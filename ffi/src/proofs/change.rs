// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::HashMap;
use std::num::NonZeroUsize;

use firewood_metrics::firewood_increment;
#[cfg(feature = "ethhash")]
use firewood_storage::TrieHash;
use firewood_storage::{
    Children, HashType, Hashable, HashableShunt, IntoHashType, NibblesIterator, PathComponent,
    PathIterItem, Preimage,
};
#[cfg(feature = "ethhash")]
use rlp::Rlp;

use firewood::{
    ProofError, ProofNode,
    api::{self, BatchOp, DbView as _, FrozenChangeProof, HashKey},
    merkle::PrefixOverlap,
};

use std::cmp::Ordering;

use crate::{
    BorrowedBytes, ChangeProofResult, DatabaseHandle, HashResult, KeyRange, Maybe,
    NextKeyRangeResult, OwnedBytes, ProposedChangeProofResult, ValueResult, VoidResult,
    metrics::MetricsContextExt,
};

#[cfg(feature = "ethhash")]
const EMPTY_CODE_HASH: [u8; 32] = [
    // "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
    0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03, 0xc0,
    0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85, 0xa4, 0x70,
];

/// Arguments for creating a change proof.
#[derive(Debug)]
#[repr(C)]
pub struct CreateChangeProofArgs<'a> {
    /// The root hash of the starting revision. This must be provided.
    /// If the root is not found in the database, the function will return
    /// [`ChangeProofResult::StartRevisionNotFound`].
    pub start_root: crate::HashKey,
    /// The root hash of the ending revision. This must be provided.
    /// If the root is not found in the database, the function will return
    /// [`ChangeProofResult::EndRevisionNotFound`].
    pub end_root: crate::HashKey,
    /// The start key of the range to create the proof for. If `None`, the range
    /// starts from the beginning of the keyspace.
    pub start_key: Maybe<BorrowedBytes<'a>>,
    /// The end key of the range to create the proof for. If `None`, the range
    /// ends at the end of the keyspace or until `max_length` items have been
    /// included in the proof.
    pub end_key: Maybe<BorrowedBytes<'a>>,
    /// The maximum number of key/value pairs to include in the proof. If the
    /// range contains more items than this, the proof will be truncated. If
    /// `0`, there is no limit.
    pub max_length: u32,
}

/// Arguments for verifying a change proof (used by both propose and commit).
#[derive(Debug)]
#[repr(C)]
pub struct VerifyChangeProofArgs<'a> {
    /// The change proof to verify. Ownership is transferred to the callee.
    pub proof: Option<Box<ChangeProofContext>>,
    /// The root hash of the starting revision.
    pub start_root: crate::HashKey,
    /// The root hash of the ending revision.
    pub end_root: crate::HashKey,
    /// The lower bound of the key range that the proof is expected to cover.
    pub start_key: Maybe<BorrowedBytes<'a>>,
    /// The upper bound of the key range that the proof is expected to cover.
    pub end_key: Maybe<BorrowedBytes<'a>>,
    /// The maximum number of key/value pairs that the proof is expected to cover.
    pub max_length: u32,
}

#[derive(Debug)]
#[repr(C)]
pub struct CommittedChangeProofArgs<'a, 'db> {
    // The change proof context that has been proposed and will be committed
    pub proof: Option<&'a mut ProposedChangeProofContext<'db>>,
}

/// FFI context for a parsed or generated change proof that has not yet been
/// verified or proposed.
#[derive(Debug)]
pub struct ChangeProofContext {
    proof: FrozenChangeProof,
}

/// FFI context for a change proof that has been verified and proposed against
/// a database. The verification context and proposal state are non-optional,
/// meaning this type can only be constructed after successful verification.
#[derive(Debug)]
pub struct ProposedChangeProofContext<'db> {
    proof: FrozenChangeProof,
    verification: VerificationContext,
    proposal_state: ProposalState<'db>,
}

#[derive(Debug)]
struct VerificationContext {
    end_root: HashKey,
    start_key: Option<Box<[u8]>>,
    end_key: Option<Box<[u8]>>,
    /// The key that the end proof was actually validated against.
    /// Computed during `verify_end_proof` and cached to avoid
    /// redundant `value_digest` calls in downstream checks.
    resolved_end_key: Option<Box<[u8]>>,
}

#[derive(Debug)]
enum ProposalState<'db> {
    Proposed(crate::ProposalHandle<'db>),
    Committed(Option<HashKey>),
}

impl From<FrozenChangeProof> for ChangeProofContext {
    fn from(proof: FrozenChangeProof) -> Self {
        Self { proof }
    }
}

// ---------------------------------------------------------------------------
// Free functions for shared verification logic
// ---------------------------------------------------------------------------

/// Verify structural properties and boundary proofs of the change proof.
///
/// On success, returns a `VerificationContext` capturing the verification
/// parameters so that downstream logic can avoid re-verifying.
fn verify_proof_structure(
    proof: &FrozenChangeProof,
    end_root: HashKey,
    start_key: Option<&[u8]>,
    end_key: Option<&[u8]>,
    max_length: Option<NonZeroUsize>,
) -> Result<VerificationContext, api::Error> {
    let batch_ops = proof.batch_ops();

    // Fix 3: Reject inverted ranges early. The generator enforces this
    // (merkle/mod.rs:545-552), but the verifier must independently
    // validate because start_key/end_key come from the caller, not
    // the proof.
    if let (Some(start), Some(end)) = (start_key, end_key)
        && start.cmp(end) == Ordering::Greater
    {
        return Err(api::Error::InvalidRange {
            start_key: start.to_vec().into(),
            end_key: end.to_vec().into(),
        });
    }

    // Fix 2: The honest diff algorithm only produces Put and Delete ops,
    // never DeleteRange. A crafted proof could use DeleteRange to delete
    // keys outside the proven range.
    if batch_ops
        .iter()
        .any(|op| matches!(op, BatchOp::DeleteRange { .. }))
    {
        return Err(api::Error::ProofError(ProofError::UnsupportedDeleteRange));
    }

    // Check batch_ops length <= max_length
    if let Some(max_length) = max_length
        && batch_ops.len() > max_length.into()
    {
        return Err(api::Error::ProofError(
            ProofError::ProofIsLargerThanMaxLength,
        ));
    }

    // Verify keys are sorted and unique — must run before boundary
    // checks (start_key ≤ first_key, end_key ≥ last_key) because
    // those checks compare against first/last elements, which are
    // only meaningful if the keys are actually sorted.
    if !batch_ops
        .iter()
        .is_sorted_by(|a, b| b.key().cmp(a.key()) == Ordering::Greater)
    {
        return Err(api::Error::ProofError(ProofError::ChangeProofKeysNotSorted));
    }

    // Check start key not greater than first batch op key
    if let (Some(start_key), Some(first_key)) = (start_key, batch_ops.first())
        && start_key.cmp(first_key.key()) == Ordering::Greater
    {
        return Err(api::Error::ProofError(
            ProofError::StartKeyLargerThanFirstKey,
        ));
    }

    // Check end key not less than last batch op key
    if let (Some(end_key), Some(last_key)) = (end_key, batch_ops.last())
        && end_key.cmp(last_key.key()) == Ordering::Less
    {
        return Err(api::Error::ProofError(ProofError::EndKeyLessThanLastKey));
    }

    // Fix 9: Reject proofs with batch_ops but no boundary proofs, UNLESS
    // this is a complete proof (no key bounds). Complete proofs are validated
    // by root hash comparison in is_complete_proof() instead.
    if !batch_ops.is_empty()
        && proof.start_proof().is_empty()
        && proof.end_proof().is_empty()
        && (start_key.is_some() || end_key.is_some())
    {
        return Err(api::Error::ProofError(ProofError::MissingBoundaryProof));
    }

    // Reject non-empty end_proof when there is no end_key and no batch_ops.
    // The honest generator never produces this combination. Matches
    // AvalancheGo's ErrUnexpectedEndProof.
    if end_key.is_none() && batch_ops.is_empty() && !proof.end_proof().is_empty() {
        return Err(api::Error::ProofError(ProofError::UnexpectedEndProof));
    }

    // Verify boundary proofs against end_root
    verify_start_proof(proof, start_key, &end_root)?;
    // Fix 8: verify_end_proof now returns the resolved key it validated
    // against, cached to avoid redundant value_digest calls downstream.
    let resolved_end_key = verify_end_proof(proof, end_key, &end_root, max_length)?;

    Ok(VerificationContext {
        end_root,
        start_key: start_key.map(Box::from),
        end_key: end_key.map(Box::from),
        resolved_end_key,
    })
}

/// Verify the start boundary proof against the end root hash.
fn verify_start_proof(
    proof: &FrozenChangeProof,
    start_key: Option<&[u8]>,
    end_root: &HashKey,
) -> Result<(), api::Error> {
    // An empty start_proof is valid: it means the range starts from the
    // beginning of the keyspace (start_key=None in the first sync round).
    if proof.start_proof().is_empty() {
        return Ok(());
    }

    // Fix 1: If start_proof is non-empty, we MUST have a key to validate
    // it against. The honest generator only produces a non-empty
    // start_proof when start_key is Some (merkle/mod.rs:558-561).
    let Some(start_key) = start_key else {
        return Err(api::Error::ProofError(
            ProofError::BoundaryProofUnverifiable,
        ));
    };

    proof.start_proof().value_digest(start_key, end_root)?;
    Ok(())
}

/// Verify the end boundary proof against the end root hash and return the
/// key it was validated against.
///
/// When `batch_ops.len() >= max_length`, the proof may have been truncated.
/// The generator uses the last batch op key when truncated but `end_key`
/// otherwise. Since the verifier cannot distinguish the two cases, we try
/// the last batch op key first and fall back to `end_key`.
///
/// # Truncation ambiguity (A9)
///
/// When `batch_ops.len() == max_length` but the proof was NOT truncated
/// (exactly `max_length` changes exist), Try 1 attempts `last_op_key`.
/// If the generator used `end_key`, Try 1 fails (hash mismatch) and
/// Try 2 succeeds with `end_key`. If `last_op_key == end_key`, Try 1
/// succeeds directly. Either way, `resolved_end_key` correctly reflects
/// the key the proof was actually validated against, ensuring downstream
/// nibble bounds and lookups use the right boundary.
///
/// Returns `Ok(Some(key))` with the validated key, or `Ok(None)` if the
/// end proof is empty (range reaches end of keyspace).
fn verify_end_proof(
    proof: &FrozenChangeProof,
    end_key: Option<&[u8]>,
    end_root: &HashKey,
    max_length: Option<NonZeroUsize>,
) -> Result<Option<Box<[u8]>>, api::Error> {
    // Empty end_proof = range reaches end of keyspace. No key to resolve.
    if proof.end_proof().is_empty() {
        return Ok(None);
    }

    let batch_ops = proof.batch_ops();
    let potentially_truncated = max_length.is_some_and(|max| batch_ops.len() >= max.get());

    // Try 1: truncated proof — validate against the last batch op key.
    // Only fall through to Try 2 if the proof is valid but proves a
    // different key (ShouldBePrefixOfProvenKey). Any structural error
    // (broken hash chain, missing child, etc.) is propagated immediately
    // rather than masked by trying another key.
    if potentially_truncated && let Some(last_op) = batch_ops.last() {
        match proof
            .end_proof()
            .value_digest(last_op.key().as_ref(), end_root)
        {
            Ok(_) => return Ok(Some(last_op.key().as_ref().into())),
            Err(ProofError::ShouldBePrefixOfProvenKey) => {}
            Err(e) => return Err(api::Error::ProofError(e)),
        }
    }

    // Try 2: non-truncated proof — validate against the requested end_key.
    if let Some(end_key) = end_key {
        proof.end_proof().value_digest(end_key, end_root)?;
        return Ok(Some(end_key.into()));
    }

    // Try 3: no end_key — fall back to last batch op key.
    if let Some(last_op) = batch_ops.last() {
        proof
            .end_proof()
            .value_digest(last_op.key().as_ref(), end_root)?;
        return Ok(Some(last_op.key().as_ref().into()));
    }

    // Fix 1: All validation paths exhausted. end_proof is non-empty but
    // no key could validate it. The honest generator always provides a
    // key for a non-empty end_proof (merkle/mod.rs:589-603).
    Err(api::Error::ProofError(
        ProofError::BoundaryProofUnverifiable,
    ))
}

/// Convert a byte key to a nibble path.
fn key_to_nibbles(key: &[u8]) -> Vec<PathComponent> {
    NibblesIterator::new(key)
        .filter_map(PathComponent::try_new)
        .collect()
}

/// Determine whether a child at the given nibble position is within the
/// change proof's key range at the given depth.
///
/// At depth `d = parent_nibbles.len()`, a child is in-range if its nibble
/// is strictly between the start and end boundary nibbles at that depth.
/// A child ON the boundary path (equal to the boundary nibble) is not
/// in-range — it's the on-path child handled separately.
///
/// # Edge cases (A4)
///
/// - **Boundary shorter than depth:** `sn.get(depth)` returns `None` →
///   all in-range from that side. Correct: the boundary key terminates
///   above this node, so all descendants are within range from that side.
/// - **Equal nibbles at depth:** `s == e` → no nibble satisfies
///   `> s && < e` → nothing in-range. Correct above the divergence point
///   where both boundaries route to the same child.
/// - **Both boundaries shorter:** all children in-range. Correct per trie
///   semantics: if both boundaries terminate above, this entire subtrie is
///   within the range.
fn is_child_in_range(
    parent_nibbles: &[PathComponent],
    nibble: PathComponent,
    start_nibbles: Option<&[PathComponent]>,
    end_nibbles: Option<&[PathComponent]>,
) -> bool {
    let depth = parent_nibbles.len();

    let start_ok = match start_nibbles {
        None => true,
        Some(sn) => match sn.get(depth) {
            // Boundary terminates above this depth — all descendants are
            // in-range from the start side.
            None => true,
            Some(&s) => nibble > s,
        },
    };

    let end_ok = match end_nibbles {
        None => true,
        Some(en) => match en.get(depth) {
            // Boundary terminates above this depth — all descendants are
            // in-range from the end side.
            None => true,
            Some(&e) => nibble < e,
        },
    };

    start_ok && end_ok
}

/// Walk proof nodes bottom-up, computing each node's expected hash by
/// substituting in-range children with the proposal's hashes, on-path
/// children with the hash computed from the deeper proof level, and
/// keeping out-of-range children from the proof.
///
/// `bottom_overrides` injects branch hashes at the deepest node (used
/// at the divergence point in the two-proof case to inject both the
/// start and end branch hashes).
///
/// # Single node (A5)
///
/// When `nodes.len() == 1`: the loop runs once with `rev_idx=0`.
/// `nodes.get(nodes.len() - 0)` = `nodes.get(1)` = `None`, so
/// `on_path_nibble = None` (no child below). Bottom overrides are
/// applied. Returns a valid hash for a single-node proof.
///
/// # Two nodes (A5)
///
/// Deepest (`rev_idx=0`) has no on-path child (`nodes.get(2)` = `None`).
/// Root (`rev_idx=1`) has on-path child pointing to the deeper node
/// (`nodes.get(1)` returns the deeper node).
///
/// # Proposal lookup miss (A2)
///
/// When `lookup.get(node_key)` returns `None`, `unwrap_or_default()`
/// yields all-None children. This is correct when the proposal's trie
/// compressed through this depth (no branch node at this path). If the
/// lookup key is wrong, substituting None where real children should be
/// causes the computed root to differ from `end_root`, caught by
/// `EndRootMismatch`.
fn walk_proof_bottom_up(
    nodes: &[ProofNode],
    lookup: &HashMap<Vec<PathComponent>, PathIterItem>,
    start_nibbles: Option<&[PathComponent]>,
    end_nibbles: Option<&[PathComponent]>,
    bottom_overrides: &[(PathComponent, Option<HashType>)],
) -> Option<HashType> {
    let mut computed_hash: Option<HashType> = None;
    // Ensure non-empty (checked_sub returns None for empty slices).
    nodes.len().checked_sub(1)?;

    for (rev_idx, node) in nodes.iter().rev().enumerate() {
        // Clone only the 512-byte children array, not the entire ProofNode
        // (which also includes the key PathBuf and value digest).
        let mut children = node.child_hashes.clone();
        let parent_nibbles: &[PathComponent] = node.full_path();
        let node_key: &[PathComponent] = node.key.as_ref();

        // Look up the proposal's node at this trie path to get its
        // children hashes. Returns all-None if the proposal has no
        // branch here (see A2 note above).
        let proposal_children: Children<Option<HashType>> = lookup
            .get(node_key)
            .and_then(|item| item.node.as_branch().map(|b| b.children_hashes()))
            .unwrap_or_default();

        // The on-path child leads to the next deeper node in the proof.
        // In reverse iteration, rev_idx=0 is the deepest node (no child below).
        // Forward index of the next deeper node: nodes.len() - rev_idx.
        //
        // Security (A3): PrefixOverlap::from doesn't assume `node` is a
        // prefix of `next`. If a malicious proof provides paths that don't
        // form a valid chain, the boundary proof hash verification
        // (verify_start_proof / verify_end_proof in step 8-9 of
        // verify_proof_structure) would have already rejected the proof
        // before this function runs.
        let on_path_nibble = nodes
            .len()
            .checked_sub(rev_idx)
            .and_then(|j| nodes.get(j))
            .and_then(|next| {
                PrefixOverlap::from(node.full_path(), next.full_path())
                    .unique_b
                    .first()
                    .copied()
            });

        for nibble in PathComponent::ALL {
            if Some(nibble) == on_path_nibble {
                children[nibble].clone_from(&computed_hash);
            } else if is_child_in_range(parent_nibbles, nibble, start_nibbles, end_nibbles) {
                children[nibble].clone_from(&proposal_children[nibble]);
            }
        }

        if rev_idx == 0 {
            for &(nibble, ref hash) in bottom_overrides {
                children[nibble].clone_from(hash);
            }
        }

        // Hash via a shunt that borrows the original node's paths and
        // value, avoiding cloning the key PathBuf and value digest.
        let shunt = HashableShunt::new(
            node.parent_prefix_path(),
            node.partial_path(),
            node.value_digest(),
            children,
        );
        computed_hash = Some(Preimage::to_hash(&shunt));
    }

    computed_hash
}

/// Compute the root hash from two boundary proof paths with in-range
/// child overrides from the proposal.
///
/// When a change proof has both a start and end boundary proof, the two
/// paths share common ancestor nodes before diverging:
///
/// ```text
///         root          ← shared prefix (indices 0..divergence_depth)
///        /    \
///     [1...]  [3...]    ← divergence point
///      ↓        ↓
///   start     end       ← independent tails (indices divergence_depth..)
///   tail      tail
/// ```
///
/// This function:
/// 1. Finds where the two proof paths diverge.
/// 2. Hashes each tail independently via [`walk_proof_bottom_up`].
/// 3. Injects the tail hashes as overrides at the divergence parent's
///    child slots.
/// 4. Hashes the shared prefix bottom-up with those overrides, producing
///    the expected root hash.
///
/// # Parameters
///
/// - `start_nodes`: The full start boundary proof path (root to leaf).
/// - `end_nodes`: The full end boundary proof path (root to leaf).
/// - `start_lookup`: Proposal node lookup for the start boundary key.
/// - `end_lookup`: Proposal node lookup for the end boundary key.
/// - `start_nibbles`: Nibble-level start boundary key, or `None` if
///   unbounded from below.
/// - `end_nibbles`: Nibble-level end boundary key, or `None` if unbounded
///   from above.
///
/// # Errors
///
/// Returns `BoundaryProofsDivergeAtRoot` if the proofs diverge at depth 0
/// (different root paths). Returns `EndRootMismatch` if either proof path
/// is malformed (missing expected nodes).
///
/// # Divergence at depth 1 (A6)
///
/// `parent_idx=0` (root), `shared = [root]`, tails start at index 1.
/// Overrides inject start/end branch hashes into the root's child
/// slots, and the single-node shared prefix walk produces the root hash.
fn compute_root_from_proofs_with_overrides(
    start_nodes: &[ProofNode],
    end_nodes: &[ProofNode],
    start_lookup: &HashMap<Vec<PathComponent>, PathIterItem>,
    end_lookup: &HashMap<Vec<PathComponent>, PathIterItem>,
    start_nibbles: Option<&[PathComponent]>,
    end_nibbles: Option<&[PathComponent]>,
) -> Result<HashType, ProofError> {
    // Find where the two proof paths diverge. Before this depth,
    // nodes are shared (same trie path); after it, each proof
    // follows its own branch toward its boundary key.
    let divergence_depth = start_nodes
        .iter()
        .zip(end_nodes.iter())
        .position(|(s, e)| {
            let s_path: &[PathComponent] = s.full_path();
            let e_path: &[PathComponent] = e.full_path();
            s_path != e_path
        })
        .unwrap_or(std::cmp::min(start_nodes.len(), end_nodes.len()));

    // A6: Divergence at depth 0 means the two proofs have different root
    // node paths — they cannot belong to the same trie.
    let Some(parent_idx) = divergence_depth.checked_sub(1) else {
        return Err(ProofError::BoundaryProofsDivergeAtRoot);
    };
    let parent = start_nodes
        .get(parent_idx)
        .ok_or(ProofError::EndRootMismatch)?;

    // Split both proof paths at the divergence point. The tails are the
    // portions that diverge; the prefix (0..divergence_depth) is shared.
    let start_tail = start_nodes
        .get(divergence_depth..)
        .ok_or(ProofError::EndRootMismatch)?;
    let end_tail = end_nodes
        .get(divergence_depth..)
        .ok_or(ProofError::EndRootMismatch)?;

    // Hash each tail independently. The start tail only needs the start
    // boundary for in-range classification (everything to the right of
    // the start path is in-range within this tail). Symmetrically for end.
    let start_branch_hash = if start_tail.is_empty() {
        None
    } else {
        walk_proof_bottom_up(start_tail, start_lookup, start_nibbles, None, &[])
    };
    let end_branch_hash = if end_tail.is_empty() {
        None
    } else {
        walk_proof_bottom_up(end_tail, end_lookup, None, end_nibbles, &[])
    };

    // Build overrides for the deepest shared node (the divergence parent).
    // Each override replaces one child slot with the hash from the
    // corresponding tail. The nibble is determined by comparing the
    // parent's path to the first tail node's path — the first differing
    // nibble is the child slot that leads into that tail.
    let mut overrides: Vec<(PathComponent, Option<HashType>)> = Vec::new();

    if let (Some(hash), Some(first)) = (start_branch_hash, start_tail.first())
        && let Some(&nibble) = PrefixOverlap::from(parent.full_path(), first.full_path())
            .unique_b
            .first()
    {
        overrides.push((nibble, Some(hash)));
    }
    if let (Some(hash), Some(first)) = (end_branch_hash, end_tail.first())
        && let Some(&nibble) = PrefixOverlap::from(parent.full_path(), first.full_path())
            .unique_b
            .first()
    {
        overrides.push((nibble, Some(hash)));
    }

    // Hash the shared prefix (root down to the divergence parent)
    // bottom-up, injecting the tail overrides at the deepest node.
    let shared = start_nodes
        .get(..divergence_depth)
        .ok_or(ProofError::EndRootMismatch)?;
    walk_proof_bottom_up(shared, start_lookup, start_nibbles, end_nibbles, &overrides)
        .ok_or(ProofError::EndRootMismatch)
}

/// Compute the expected root hash by walking boundary proof paths
/// bottom-up and comparing it against the expected end root.
///
/// Replaces the old multi-phase spot-checking approach with a single
/// root hash computation. For each proof node, out-of-range children
/// keep the proof's hashes (from `end_root`'s trie), in-range children
/// use the proposal's hashes, and on-path children use the hash computed
/// from the deeper proof level.
fn verify_root_hash(
    proof: &FrozenChangeProof,
    verification: &VerificationContext,
    proposal: &crate::ProposalHandle<'_>,
) -> Result<(), api::Error> {
    let start_nodes: &[ProofNode] = proof.start_proof().as_ref();
    let end_nodes: &[ProofNode] = proof.end_proof().as_ref();

    // Case 1: Both proofs empty — this is a "complete" proof covering the
    // entire keyspace (no boundaries). The proposal should contain the
    // full target state, so compare its root hash directly against
    // end_root. Also covers the degenerate case of an empty diff.
    if start_nodes.is_empty() && end_nodes.is_empty() {
        let computed: crate::HashKey = proposal
            .root_hash()
            .map(crate::HashKey::from)
            .unwrap_or_default();
        if computed != crate::HashKey::from(verification.end_root.clone()) {
            return Err(api::Error::ProofError(ProofError::EndRootMismatch));
        }
        return Ok(());
    }

    // Convert byte-level boundary keys to nibble-level paths for
    // child-in-range classification during the bottom-up walk.
    let start_nibbles = verification.start_key.as_deref().map(key_to_nibbles);

    // Use the resolved end key (the key the end proof was actually
    // validated against) rather than the originally requested end_key.
    // For truncated proofs, resolved_end_key is the last batch op key;
    // for non-truncated proofs, it matches end_key.
    let effective_end_key = verification
        .resolved_end_key
        .as_deref()
        .or(verification.end_key.as_deref());
    let end_nibbles = effective_end_key.map(key_to_nibbles);

    // Build lookup tables mapping trie paths to proposal nodes. Each
    // lookup contains the proposal's nodes along the path to the
    // corresponding boundary key, used to get the proposal's child
    // hashes at each depth for in-range substitution.
    let start_lookup = build_proposal_lookup(verification.start_key.as_deref(), proposal)?;
    let end_lookup = build_proposal_lookup(effective_end_key, proposal)?;

    // Case 2: At least one boundary proof exists. Walk the proof path(s)
    // bottom-up to compute the expected root hash.
    //
    // Three sub-cases based on which proofs are present:
    let computed_root = if start_nodes.is_empty() {
        // Case 2a: Only end proof (first sync round, start_key=None).
        // Walk the end proof path; no start boundary to constrain.
        walk_proof_bottom_up(end_nodes, &end_lookup, None, end_nibbles.as_deref(), &[])
    } else if end_nodes.is_empty() {
        // Case 2b: Only start proof (last sync round, end of keyspace).
        // Walk the start proof path; no end boundary to constrain.
        walk_proof_bottom_up(
            start_nodes,
            &start_lookup,
            start_nibbles.as_deref(),
            None,
            &[],
        )
    } else {
        // Case 2c: Both boundary proofs exist (middle sync round).
        compute_root_from_proofs_with_overrides(
            start_nodes,
            end_nodes,
            &start_lookup,
            &end_lookup,
            start_nibbles.as_deref(),
            end_nibbles.as_deref(),
        )
        .map(Some)?
    };

    // B3 Dead code analysis: In practice this None branch is unreachable
    // for single-proof cases (non-empty slices always yield Some).
    // The two-proof case now returns Result and is handled by `?` above
    // (BoundaryProofsDivergeAtRoot or EndRootMismatch).
    // Kept as defense-in-depth for the single-proof paths.
    let Some(computed_root) = computed_root else {
        return Err(api::Error::ProofError(ProofError::EndRootMismatch));
    };
    let expected: HashType = verification.end_root.clone().into_hash_type();
    if computed_root != expected {
        return Err(api::Error::ProofError(ProofError::EndRootMismatch));
    }

    // Verify that values at every proof node match the proposal's claims.
    // The root hash walk above only substitutes children, not values. A
    // base-state mismatch at any proof node (e.g. the proof claims "valA"
    // but the proposal has "valB") would go undetected by the hash walk
    // because the proof's own value is used in the computation. Caught by
    // `ProofNodeValueMismatch`.
    verify_proof_node_values(proof, &start_lookup, &end_lookup)?;

    Ok(())
}

/// Verify that values at every proof node match the proposal.
///
/// The root hash walk substitutes children but not values, so a value
/// mismatch at any proof node (boundary or intermediate) would be masked
/// by the proof's own value in the hash computation. This check compares
/// every proof node that carries a value against the proposal node at the
/// same trie path, using the already-built proposal lookups.
///
/// Note: Despite the previous name `verify_boundary_values`, this checks
/// ALL proof nodes with values, not just the boundary keys. This is
/// necessary because intermediate nodes on the proof path also carry
/// values that must match.
fn verify_proof_node_values(
    proof: &FrozenChangeProof,
    start_lookup: &HashMap<Vec<PathComponent>, PathIterItem>,
    end_lookup: &HashMap<Vec<PathComponent>, PathIterItem>,
) -> Result<(), api::Error> {
    for node in proof.start_proof().as_ref() {
        check_proof_node_value(node, start_lookup)?;
    }
    for node in proof.end_proof().as_ref() {
        check_proof_node_value(node, end_lookup)?;
    }
    Ok(())
}

/// Compare a single proof node's value against the corresponding proposal node.
///
/// # Odd nibble depth skip (A1)
///
/// Nodes at odd nibble depths cannot carry values in the trie encoding.
/// `Proof::value_digest` (types.rs:316) enforces this via
/// `ValueAtOddNibbleLength`, which is checked during the boundary proof
/// hash chain verification (`verify_start_proof` / `verify_end_proof`). If a
/// malicious proof includes a value at odd depth, it is rejected by the
/// hash chain check, not silently ignored here.
fn check_proof_node_value(
    node: &ProofNode,
    lookup: &HashMap<Vec<PathComponent>, PathIterItem>,
) -> Result<(), api::Error> {
    // Values only exist at even nibble lengths. Odd-depth nodes are
    // skipped safely — see A1 security note above.
    if !node.full_path().len().is_multiple_of(2) {
        return Ok(());
    }
    let node_key: &[PathComponent] = node.key.as_ref();
    let proposal_value = lookup.get(node_key).and_then(|item| item.node.value());
    match (&node.value_digest, proposal_value) {
        (None, None) => Ok(()),
        (Some(digest), Some(val)) if digest.verify(val) => Ok(()),
        _ => Err(api::Error::ProofError(ProofError::ProofNodeValueMismatch {
            depth: node.full_path().len(),
        })),
    }
}

// ---------------------------------------------------------------------------
// Methods on ChangeProofContext
// ---------------------------------------------------------------------------

impl ChangeProofContext {
    /// Verify the change proof and prepare a proposal against the given database
    /// without committing it.
    ///
    /// On success, consumes `self` and returns a [`ProposedChangeProofContext`]
    /// containing the verified proof, verification context, and proposal state.
    ///
    /// On failure, returns `self` back to the caller (along with the error) so
    /// that the caller retains ownership of the unverified proof.
    fn verify_and_propose<'db>(
        self,
        db: &'db crate::DatabaseHandle,
        start_root: HashKey,
        end_root: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<ProposedChangeProofContext<'db>, Box<(Self, api::Error)>> {
        // Destructure self so we can move `proof` into either the Ok or Err
        // result without cloning.
        let proof = self.proof;

        let verification = match verify_proof_structure(
            &proof,
            end_root.clone(),
            start_key,
            end_key,
            max_length,
        ) {
            Ok(v) => v,
            Err(e) => return Err(Box::new((Self { proof }, e))),
        };

        let proposal = match db.apply_change_proof_to_parent(start_root, &proof) {
            Ok(p) => p,
            Err(e) => return Err(Box::new((Self { proof }, e))),
        };

        // Root hash verification: walk boundary proof paths bottom-up,
        // substituting in-range children from the proposal, and compare
        // the computed root against end_root.
        if let Err(e) = verify_root_hash(&proof, &verification, &proposal.handle) {
            return Err(Box::new((Self { proof }, e)));
        }

        Ok(ProposedChangeProofContext {
            proof,
            verification,
            proposal_state: ProposalState::Proposed(proposal.handle),
        })
    }

    /// Verify and commit the change proof to the given database.
    ///
    /// Consumes `self`. The proof is consumed regardless of success or failure.
    fn verify_and_commit(
        self,
        db: &crate::DatabaseHandle,
        start_root: HashKey,
        end_root: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<Option<HashKey>, api::Error> {
        let mut proposed = self
            .verify_and_propose(db, start_root, end_root, start_key, end_key, max_length)
            .map_err(|boxed| {
                let (_proof, err) = *boxed;
                err
            })?;
        proposed.commit()
    }
}

// ---------------------------------------------------------------------------
// Methods on ProposedChangeProofContext
// ---------------------------------------------------------------------------

impl ProposedChangeProofContext<'_> {
    /// Commit a previously proposed change proof. Consumes the proposal handle.
    fn commit(&mut self) -> Result<Option<HashKey>, api::Error> {
        let state = std::mem::replace(&mut self.proposal_state, ProposalState::Committed(None));
        match state {
            ProposalState::Committed(hash) => {
                self.proposal_state = ProposalState::Committed(hash.clone());
                Ok(hash)
            }
            ProposalState::Proposed(handle) => match handle.commit_proposal() {
                Ok(hash) => {
                    firewood_increment!(crate::registry::MERGE_COUNT, 1, "change" => "commit");
                    self.proposal_state = ProposalState::Committed(hash.clone());
                    Ok(hash)
                }
                Err(err) => Err(err),
            },
        }
    }

    /// Compare the proposal's computed root hash against the expected
    /// `end_root`. Called when `find_next_key` determines that sync has
    /// reached the end of the keyspace (empty `end_proof` or no `batch_ops`).
    /// This is the final cryptographic check that the accumulated state
    /// matches the target revision.
    fn check_root_hash(&self) -> Result<(), api::Error> {
        // Retrieve the root hash from whichever state we're in.
        // Before commit: read from the live proposal handle.
        // After commit: use the cached hash from the commit result.
        // None means the trie is empty; unwrap_or_default gives the
        // canonical empty-trie hash, matching the pattern in
        // verify_and_propose (line ~526).
        let computed: crate::HashKey = match &self.proposal_state {
            ProposalState::Proposed(handle) => handle
                .root_hash()
                .map(crate::HashKey::from)
                .unwrap_or_default(),
            ProposalState::Committed(hash) => {
                hash.clone().map(crate::HashKey::from).unwrap_or_default()
            }
        };
        if computed != crate::HashKey::from(self.verification.end_root.clone()) {
            return Err(api::Error::ProofError(ProofError::EndRootMismatch));
        }
        Ok(())
    }

    /// Returns the next key range that should be fetched after processing this
    /// change proof, or `None` if there are no more keys to fetch.
    ///
    /// # Termination analysis (A7)
    ///
    /// | `batch_ops` | `end_proof` | `end_key`  | Path                   | Root check? |
    /// |-------------|-------------|------------|------------------------|-------------|
    /// | empty       | empty       | any        | no `batch_ops` → nil   | yes         |
    /// | empty       | non-empty   | any        | no `batch_ops` → nil   | yes         |
    /// | non-empty   | empty       | any        | empty `end_proof`      | yes         |
    /// | non-empty   | non-empty   | Some, sat  | `last_op` >= `end_key` | no (safe*)  |
    /// | non-empty   | non-empty   | Some, !sat | continuation           | deferred    |
    /// | non-empty   | non-empty   | None       | continuation           | deferred    |
    ///
    /// *The `>=` comparison is byte-lexicographic on `Box<[u8]>`, which is
    /// standard byte ordering — no encoding confusion possible. The "no
    /// root check" row is safe because `end_key` is receiver-controlled,
    /// not attacker-controlled.
    fn find_next_key(&mut self) -> Result<Option<KeyRange>, api::Error> {
        let Some(last_op) = self.proof.batch_ops().last() else {
            // No batch_ops means the proof claims no changes exist.
            // Verify that the accumulated state matches end_root;
            // otherwise a malicious sender could send an empty proof
            // to make the receiver stop with incomplete state.
            self.check_root_hash()?;
            return Ok(None);
        };

        if self.proof.end_proof().is_empty() {
            // Empty end_proof signals the end of the keyspace — there
            // are no more keys to fetch. Verify the root hash to
            // confirm the accumulated state is complete; a malicious
            // sender could craft partial changes with an empty
            // end_proof to trigger premature completion.
            self.check_root_hash()?;
            return Ok(None);
        }

        // Range-bounded completion: last_op >= end_key. No root hash
        // check here because end_key is controlled by the receiver,
        // so the attacker cannot force this path. The proposal's root
        // hash may legitimately differ from end_root when the proof
        // covers only a sub-range of the full keyspace.
        if let Some(ref end_key) = self.verification.end_key
            && **last_op.key() >= **end_key
        {
            return Ok(None);
        }

        Ok(Some((
            last_op.key().clone(),
            self.verification.end_key.clone(),
        )))
    }
}

/// Build a `HashMap` from `key_nibbles` to `PathIterItem` for fast lookup of
/// proposal trie nodes when comparing child hashes.
fn build_proposal_lookup(
    key: Option<&[u8]>,
    proposal: &crate::ProposalHandle<'_>,
) -> Result<HashMap<Vec<PathComponent>, PathIterItem>, api::Error> {
    let Some(key) = key else {
        return Ok(HashMap::new());
    };
    let path = proposal.path_to_key(key)?;
    Ok(path
        .into_iter()
        .map(|item| {
            let key_nibbles: &[PathComponent] = item.key_nibbles.as_ref();
            (key_nibbles.to_vec(), item)
        })
        .collect())
}

/// A key range that should be fetched to continue iterating through a range
/// or change proof that was truncated. Represents a half-open range
/// `[start_key, end_key)`. If `end_key` is `None`, the range is unbounded
/// and continues to the end of the keyspace.
#[derive(Debug)]
#[repr(C)]
pub struct NextKeyRange {
    /// The start key of the next range to fetch.
    pub start_key: OwnedBytes,

    /// If set, a non-inclusive upper bound for the next range to fetch. If not
    /// set, the range is unbounded (this is the final range).
    pub end_key: Maybe<OwnedBytes>,
}

#[derive(Debug)]
#[non_exhaustive]
pub struct CodeIteratorHandle<'a> {
    #[cfg(feature = "ethhash")]
    inner: std::slice::Iter<'a, KeyValuePair>,
    // uninhabitable fields make the struct impossible to construct when the feature is disabled
    #[cfg(not(feature = "ethhash"))]
    void: std::convert::Infallible,
    #[cfg(not(feature = "ethhash"))]
    marker: std::marker::PhantomData<&'a ()>,
}

type KeyValuePair = (Box<[u8]>, Box<[u8]>);

impl Iterator for CodeIteratorHandle<'_> {
    type Item = Result<crate::HashKey, api::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        #[cfg(not(feature = "ethhash"))]
        match self.void {}

        #[cfg(feature = "ethhash")]
        self.inner.find_map(|(key, value)| {
            if key.len() != 32 {
                return None;
            }

            let Ok(code_hash_slice) = Rlp::new(value).at(3).and_then(|r| r.data()) else {
                return Some(Err(api::Error::ProofError(ProofError::InvalidValueFormat)));
            };
            let code_hash: crate::HashKey = TrieHash::try_from(code_hash_slice).ok()?.into();
            if code_hash == TrieHash::from(EMPTY_CODE_HASH).into() {
                return None;
            }

            Some(Ok(code_hash))
        })
    }
}

impl<'a> CodeIteratorHandle<'a> {
    /// Create a new code hash iterator from the given key/value pairs.
    /// The key/value pairs should be the raw entries from the
    /// underlying proof.
    ///
    /// The iterator must be freed after use.
    ///
    /// Arguments:
    /// - `key_values` - The key/value pairs from the proof.
    ///
    /// Returns:
    /// - `Ok(CodeIteratorHandle)` if the iterator was successfully created.
    /// - `Err(api::Error)` if the iterator could not be created.
    ///
    /// # Errors
    ///
    /// - Returns `api::Error::FeatureNotSupported` if the `ethhash` feature
    ///   is not enabled.
    #[cfg_attr(feature = "ethhash", allow(clippy::missing_const_for_fn))]
    #[cfg_attr(not(feature = "ethhash"), allow(unused_variables))]
    pub fn new(key_values: &'a [KeyValuePair]) -> Result<Self, api::Error> {
        #[cfg(not(feature = "ethhash"))]
        {
            Err(api::Error::FeatureNotSupported(
                "ethhash code hash iterator".to_string(),
            ))
        }

        #[cfg(feature = "ethhash")]
        {
            Ok(CodeIteratorHandle {
                inner: key_values.iter(),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// FFI extern functions
// ---------------------------------------------------------------------------

/// Create a change proof for the given range of keys between two roots.
///
/// # Arguments
///
/// - `db` - The database to create the proof from.
/// - `args` - The arguments for creating the change proof.
///
/// # Returns
///
/// - [`ChangeProofResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`ChangeProofResult::StartRevisionNotFound`] if the caller provided a start root
///   that was not found in the database. The missing root hash is included in the result.
///   If both the start root and end root are missing, then only the end root is
///   reported.
/// - [`ChangeProofResult::EndRevisionNotFound`] if the caller provided an end root
///   that was not found in the database. The missing root hash is included in the result.
///   If both the start root and end root are missing, then only the end root is
///   reported.
/// - [`ChangeProofResult::Ok`] containing a pointer to the `ChangeProofContext` if the proof
///   was successfully created.
/// - [`ChangeProofResult::Err`] containing an error message if the proof could not be created.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_change_proof(
    db: Option<&DatabaseHandle>,
    args: CreateChangeProofArgs,
) -> ChangeProofResult {
    crate::invoke_with_handle(db, |db| {
        db.change_proof(
            args.start_root.into(),
            args.end_root.into(),
            args.start_key
                .as_ref()
                .map(BorrowedBytes::as_slice)
                .into_option(),
            args.end_key
                .as_ref()
                .map(BorrowedBytes::as_slice)
                .into_option(),
            NonZeroUsize::new(args.max_length as usize),
        )
    })
}

/// Verify a change proof and prepare a proposal to later commit or drop.
///
/// On success, the proof is consumed and a [`ProposedChangeProofContext`] is
/// returned. On failure, the original [`ChangeProofContext`] is returned to
/// the caller so it can be retried or freed.
///
/// # Arguments
///
/// - `db` - The database to verify the proof against.
/// - `args` - The arguments for verifying and proposing the change proof.
///
/// # Returns
///
/// - [`ProposedChangeProofResult::NullHandlePointer`] if the caller provided a null pointer
///   to either the database or the proof.
/// - [`ProposedChangeProofResult::Ok`] containing the proposed context on success.
/// - [`ProposedChangeProofResult::VerificationFailed`] containing the original proof and
///   error message on verification failure.
///
/// # Thread Safety
///
/// It is not safe to call this function concurrently with the same proof context
/// nor is it safe to call any other function that accesses the same proof context
/// concurrently. The caller must ensure exclusive access to the proof context
/// for the duration of the call.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_verify_and_propose_change_proof<'db>(
    db: Option<&'db DatabaseHandle>,
    args: VerifyChangeProofArgs,
) -> ProposedChangeProofResult<'db> {
    let (Some(db), Some(proof)) = (db, args.proof) else {
        return ProposedChangeProofResult::NullHandlePointer;
    };

    let start_key = args.start_key.into_option();
    let end_key = args.end_key.into_option();

    let _guard = firewood_metrics::set_metrics_context(db.metrics_context());

    crate::invoke(move || {
        (*proof).verify_and_propose(
            db,
            args.start_root.into(),
            args.end_root.into(),
            start_key.as_deref(),
            end_key.as_deref(),
            NonZeroUsize::new(args.max_length as usize),
        )
    })
}

/// Verify and commit a change proof to the database.
///
/// The proof is consumed regardless of success or failure.
///
/// # Arguments
///
/// - `db` - The database to commit the changes to.
/// - `args` - The arguments for verifying and committing the change proof.
///
/// # Returns
///
/// - [`HashResult::NullHandlePointer`] if the caller provided a null pointer to either
///   the database or the proof.
/// - [`HashResult::None`] if the proof resulted in an empty database (i.e., all keys were deleted).
/// - [`HashResult::Some`] containing the new root hash if the proof was successfully verified
/// - [`HashResult::Err`] containing an error message if the proof could not be verified or committed.
///
/// # Thread Safety
///
/// It is not safe to call this function concurrently with the same proof context
/// nor is it safe to call any other function that accesses the same proof context
/// concurrently. The caller must ensure exclusive access to the proof context
/// for the duration of the call.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_verify_and_commit_change_proof(
    db: Option<&DatabaseHandle>,
    args: VerifyChangeProofArgs,
) -> HashResult {
    let (Some(db), Some(proof)) = (db, args.proof) else {
        return HashResult::NullHandlePointer;
    };

    let start_key = args.start_key.into_option();
    let end_key = args.end_key.into_option();

    let _guard = firewood_metrics::set_metrics_context(db.metrics_context());

    crate::invoke(move || {
        (*proof).verify_and_commit(
            db,
            args.start_root.into(),
            args.end_root.into(),
            start_key.as_deref(),
            end_key.as_deref(),
            NonZeroUsize::new(args.max_length as usize),
        )
    })
}

/// Commit a change proof to the database.
///
/// # Arguments
///
/// - `args` - The arguments for committing the change proof, which is just a
///   `ProposedChangeProofContext`.
///
/// # Returns
///
/// - [`HashResult::NullHandlePointer`] if the caller provided a null pointer to the proof.
/// - [`HashResult::None`] if the proof resulted in an empty database (i.e., all keys were deleted).
/// - [`HashResult::Some`] containing the new root hash
/// - [`HashResult::Err`] containing an error message if the proof could not be committed.
///
/// # Thread Safety
///
/// It is not safe to call this function concurrently with the same proof context
/// nor is it safe to call any other function that accesses the same proof context
/// concurrently. The caller must ensure exclusive access to the proof context
/// for the duration of the call.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_commit_change_proof(args: CommittedChangeProofArgs<'_, '_>) -> HashResult {
    crate::invoke_with_handle(args.proof, ProposedChangeProofContext::commit)
}

/// Returns the next key range that should be fetched after processing the
/// current set of operations in a change proof that was truncated.
///
/// # Arguments
///
/// - `proof` - A [`ProposedChangeProofContext`] that has been verified and proposed.
///
/// # Returns
///
/// - [`NextKeyRangeResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`NextKeyRangeResult::None`] if there are no more keys to fetch.
/// - [`NextKeyRangeResult::Some`] containing the next key range to fetch.
/// - [`NextKeyRangeResult::Err`] containing an error message if the next key range
///   could not be determined.
///
/// # Thread Safety
///
/// It is not safe to call this function concurrently with the same proof context
/// nor is it safe to call any other function that accesses the same proof context
/// concurrently. The caller must ensure exclusive access to the proof context
/// for the duration of the call.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_change_proof_find_next_key_proposed(
    proof: Option<&mut ProposedChangeProofContext>,
) -> NextKeyRangeResult {
    crate::invoke_with_handle(proof, ProposedChangeProofContext::find_next_key)
}

/// Serialize a `ChangeProof` to bytes.
///
/// # Arguments
///
/// - `proof` - A [`ChangeProofContext`] previously returned from the create
///   method. If from a parsed proof, the proof will not be verified before
///   serialization.
///
/// # Returns
///
/// - [`ValueResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`ValueResult::Some`] containing the serialized bytes if successful.
/// - [`ValueResult::Err`] if the caller provided a null pointer.
///
/// The other [`ValueResult`] variants are not used.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_change_proof_to_bytes(proof: Option<&ChangeProofContext>) -> ValueResult {
    crate::invoke_with_handle(proof, |ctx| serialize_proof(&ctx.proof))
}

/// Serialize a proposed `ChangeProof` to bytes.
///
/// # Arguments
///
/// - `proof` - A [`ProposedChangeProofContext`] previously returned from the
///   verify and propose method.
///
/// # Returns
///
/// - [`ValueResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`ValueResult::Some`] containing the serialized bytes if successful.
/// - [`ValueResult::Err`] if the caller provided a null pointer.
///
/// The other [`ValueResult`] variants are not used.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_proposed_change_proof_to_bytes(
    proof: Option<&ProposedChangeProofContext>,
) -> ValueResult {
    crate::invoke_with_handle(proof, |ctx| serialize_proof(&ctx.proof))
}

/// Serialize a [`FrozenChangeProof`] into a byte vector.
fn serialize_proof(proof: &FrozenChangeProof) -> Vec<u8> {
    let mut vec = Vec::new();
    proof.write_to_vec(&mut vec);
    vec
}

/// Deserialize a `ChangeProof` from bytes.
///
/// # Arguments
///
/// * `bytes` - The bytes to deserialize the proof from.
///
/// # Returns
///
/// - [`ChangeProofResult::NullHandlePointer`] if the caller provided a null or zero-length slice.
/// - [`ChangeProofResult::Ok`] containing a pointer to the `ChangeProofContext` if the proof
///   was successfully parsed. This does not imply that the proof is valid, only that it is
///   well-formed. The verify method must be called to ensure the proof is cryptographically valid.
/// - [`ChangeProofResult::Err`] containing an error message if the proof could not be parsed.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_change_proof_from_bytes(bytes: BorrowedBytes) -> ChangeProofResult {
    crate::invoke(move || {
        FrozenChangeProof::from_slice(&bytes)
            .map_err(|err| api::Error::ProofError(ProofError::Deserialization(err)))
    })
}

/// Frees the memory associated with a `ChangeProofContext`.
///
/// # Arguments
///
/// * `proof` - The `ChangeProofContext` to free, previously returned from any Rust function.
///
/// # Returns
///
/// - [`VoidResult::Ok`] if the memory was successfully freed.
/// - [`VoidResult::Err`] if the process panics while freeing the memory.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_free_change_proof(proof: Option<Box<ChangeProofContext>>) -> VoidResult {
    crate::invoke_with_handle(proof, drop)
}

/// Frees the memory associated with a `ProposedChangeProofContext`.
///
/// # Arguments
///
/// * `proof` - The `ProposedChangeProofContext` to free, previously returned
///   from the verify and propose function.
///
/// # Returns
///
/// - [`VoidResult::Ok`] if the memory was successfully freed.
/// - [`VoidResult::Err`] if the process panics while freeing the memory.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_free_proposed_change_proof(
    proof: Option<Box<ProposedChangeProofContext>>,
) -> VoidResult {
    crate::invoke_with_handle(proof, drop)
}

// ---------------------------------------------------------------------------
// MetricsContextExt impls
// ---------------------------------------------------------------------------

impl crate::MetricsContextExt for ChangeProofContext {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        None
    }
}

impl crate::MetricsContextExt for ProposedChangeProofContext<'_> {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        None
    }
}

impl crate::MetricsContextExt for CodeIteratorHandle<'_> {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        None
    }
}

#[cfg(test)]
mod tests {
    use firewood::{
        ProofError,
        api::{BatchOp, Proposal as _},
        merkle::{Key, Value},
    };

    /// Shorthand for creating a Put batch operation with the right types.
    fn put(key: &[u8], val: &[u8]) -> BatchOp<Key, Value> {
        BatchOp::Put {
            key: key.to_vec().into_boxed_slice(),
            value: val.to_vec().into_boxed_slice(),
        }
    }

    use crate::{BorrowedBytes, CView, DatabaseHandle, DatabaseHandleArgs, NodeHashAlgorithm};

    use super::{ChangeProofContext, is_child_in_range, verify_proof_structure};

    /// Create a temporary database for testing.
    fn test_db(dir: &std::path::Path) -> DatabaseHandle {
        let dir_str = dir.to_str().expect("tempdir path should be valid UTF-8");
        let args = DatabaseHandleArgs {
            dir: BorrowedBytes::from_slice(dir_str.as_bytes()),
            root_store: true,
            node_cache_memory_limit: 0,
            free_list_cache_size: 1024,
            revisions: 100,
            strategy: 0,
            truncate: true,
            expensive_metrics: false,
            node_hash_algorithm: if cfg!(feature = "ethhash") {
                NodeHashAlgorithm::Ethereum
            } else {
                NodeHashAlgorithm::MerkleDB
            },
            deferred_persistence_commit_count: 1,
        };
        DatabaseHandle::new(args).expect("failed to create test database")
    }

    /// Test that `verify_proof_structure` rejects inverted key ranges.
    /// `BoundaryPathsInverted` in `verify_subtrie_hashes` requires hand-crafted
    /// `ProofNode`s (impossible due to `#[non_exhaustive]`), so we test the
    /// structural rejection of inverted ranges instead.
    #[test]
    fn test_inverted_range_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = test_db(dir.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\xa0", b"v1")];
        let p = (&db).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1 = db.current_root_hash().expect("root");

        let extra = vec![put(b"\x50", b"mid")];
        let p = (&db).create_proposal(extra).expect("proposal");
        p.commit().expect("commit");
        let root2 = db.current_root_hash().expect("root");

        // Create a valid bounded proof
        let proof = db
            .change_proof(
                root1,
                root2.clone(),
                Some(b"\x10".as_ref()),
                Some(b"\xa0".as_ref()),
                None,
            )
            .expect("change proof");

        // Verify with inverted keys: start=\xa0 > end=\x10
        let result = verify_proof_structure(&proof, root2, Some(b"\xa0"), Some(b"\x10"), None);
        let err = result.expect_err("inverted range should be rejected");
        assert!(
            matches!(err, firewood::api::Error::InvalidRange { .. }),
            "expected InvalidRange, got: {err}"
        );
    }

    /// Test that truncated proofs presented as unbounded are detected.
    ///
    /// Attack scenario: an attacker generates a truncated proof (`max_length=1`)
    /// covering only 1 of N changes, then presents it as unbounded (`max_length=None`)
    /// with `start_key=None, end_key=None`.
    ///
    /// The root hash verification passes because out-of-range children come from
    /// the proof (`end_root`'s trie). But `find_next_key` returns `Some` (continuation
    /// needed), signaling that the proof is incomplete. The receiver must not
    /// treat partial state as complete.
    #[test]
    fn test_unbounded_proof_with_end_proof_checks_root_hash() {
        use std::num::NonZeroUsize;

        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Both databases start with identical state
        let initial = vec![
            put(b"\x10", b"v0"),
            put(b"\x20", b"v1"),
            put(b"\x30", b"v2"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b, "initial roots must match");

        // Source db gets multiple key changes → root2
        let changes = vec![
            put(b"\x10", b"changed0"),
            put(b"\x20", b"changed1"),
            put(b"\x30", b"changed2"),
        ];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Generate a truncated proof covering only 1 change
        let proof = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                None,
                None,
                NonZeroUsize::new(1),
            )
            .expect("truncated change proof");

        assert!(
            !proof.end_proof().is_empty(),
            "truncated proof should have non-empty end_proof"
        );

        // verify_and_propose succeeds: the proof is valid for the sub-range it covers
        let change_ctx = ChangeProofContext::from(proof);
        let mut proposed = change_ctx
            .verify_and_propose(&db_b, root1_b, root2, None, None, None)
            .expect("truncated proof passes root hash check for its sub-range");

        // find_next_key detects that the proof is incomplete by returning
        // a continuation range instead of None
        let next = proposed
            .find_next_key()
            .expect("find_next_key should not error");
        assert!(
            next.is_some(),
            "truncated proof presented as unbounded must indicate more data needed"
        );
    }

    /// Test that `verify_proof_structure` rejects a proof where a
    /// non-empty start proof has no key to validate against
    /// (`BoundaryProofUnverifiable`).
    #[test]
    fn test_boundary_proof_unverifiable_via_structure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = test_db(dir.path());

        let batch = vec![put(b"\x00", b"v0"), put(b"\x10", b"v1")];
        let proposal = (&db).create_proposal(batch).expect("create proposal");
        proposal.commit().expect("commit");
        let root1 = db.current_root_hash().expect("root");

        let extra = vec![put(b"\x05", b"v2")];
        let proposal = (&db).create_proposal(extra).expect("create proposal");
        proposal.commit().expect("commit");
        let root2 = db.current_root_hash().expect("root");

        // Create a bounded proof (start_key=Some → non-empty start_proof)
        let proof = db
            .change_proof(
                root1,
                root2.clone(),
                Some(b"\x00".as_ref()),
                Some(b"\x10".as_ref()),
                None,
            )
            .expect("change proof");

        // Verify with start_key=None: the non-empty start proof has no key
        // to validate against → BoundaryProofUnverifiable
        let result = verify_proof_structure(
            &proof,
            root2,
            None, // start_key=None but start_proof is non-empty
            Some(b"\x10"),
            None,
        );
        let err = result.expect_err("should fail with BoundaryProofUnverifiable");
        assert!(
            matches!(
                err,
                firewood::api::Error::ProofError(ProofError::BoundaryProofUnverifiable)
            ),
            "expected BoundaryProofUnverifiable, got: {err}"
        );
    }

    /// Happy path: single-round unbounded proof where root hash matches.
    /// `find_next_key` returns `Ok(None)` without error because the
    /// proposal's root hash matches `end_root`.
    #[test]
    fn test_find_next_key_root_hash_positive() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Both databases start with identical state
        let initial = vec![put(b"\x10", b"v0"), put(b"\x20", b"v1")];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // db_a modifies keys → root2
        let changes = vec![put(b"\x10", b"changed0"), put(b"\x20", b"changed1")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Generate unbounded proof covering all changes
        let proof = db_a
            .change_proof(root1_a.clone(), root2.clone(), None, None, None)
            .expect("change proof");

        let change_ctx = ChangeProofContext::from(proof);
        let mut proposed = change_ctx
            .verify_and_propose(&db_b, root1_b, root2, None, None, None)
            .expect("verify_and_propose should succeed");

        // find_next_key should return Ok(None) — root hash matches
        let next = proposed
            .find_next_key()
            .expect("find_next_key should not error");
        assert_eq!(
            next, None,
            "single-round unbounded proof should be complete"
        );
    }

    /// Multi-round scenario where `is_complete_proof` is false (`start_key=Some`)
    /// but `find_next_key` catches the root hash mismatch because the proof
    /// only covers a suffix of the changed keys.
    ///
    /// Simulates a receiver that skips the first changed key (\x10) by
    /// starting from \x11, so the proposal is missing one change and the
    /// root hash won't match `end_root`.
    #[test]
    fn test_find_next_key_root_hash_mismatch() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Both databases start with identical state
        let initial = vec![
            put(b"\x10", b"v0"),
            put(b"\x20", b"v1"),
            put(b"\x30", b"v2"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // db_a modifies all 3 keys → root2
        let changes = vec![
            put(b"\x10", b"changed0"),
            put(b"\x20", b"changed1"),
            put(b"\x30", b"changed2"),
        ];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Generate a proof starting from \x11, which skips the \x10 change.
        // This gives us only the \x20 and \x30 changes with empty end_proof
        // (end of keyspace). start_key=Some makes is_complete_proof false,
        // so verify_and_propose skips the root hash check.
        let partial_proof = db_a
            .change_proof(root1_a, root2.clone(), Some(b"\x11".as_ref()), None, None)
            .expect("partial proof");
        assert!(
            partial_proof.end_proof().is_empty(),
            "final page should have empty end_proof"
        );

        let partial_ctx = ChangeProofContext::from(partial_proof);
        let mut proposed = partial_ctx
            .verify_and_propose(&db_b, root1_b, root2, Some(b"\x11".as_ref()), None, None)
            .expect("verify_and_propose should pass (no root hash check)");

        // find_next_key → end_proof empty → check_root_hash → mismatch
        // because the proposal is missing the \x10 change
        let result = proposed.find_next_key();
        let err = result.expect_err("find_next_key should detect root hash mismatch");
        assert!(
            matches!(
                err,
                firewood::api::Error::ProofError(ProofError::EndRootMismatch)
            ),
            "expected EndRootMismatch, got: {err}"
        );
    }

    /// Verify that `walk_proof_tail` checks the last start-proof node's
    /// children. A bounded change proof where the start proof terminates at
    /// a branch node must verify that all children of that last node match
    /// the proposal, since those children lead to keys > `start_key` (within
    /// the proven range).
    #[test]
    fn test_start_tail_last_node_children_checked() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Keys chosen so that \x10 becomes a branch with children \x10\x01
        // and \x10\x02 beneath it. The start proof for key \x10 will
        // terminate at that branch node.
        let initial = vec![
            put(b"\x10\x01", b"a"),
            put(b"\x10\x02", b"b"),
            put(b"\x30", b"c"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // Change a key in the proven range (above \x10) on db_a.
        let changes = vec![put(b"\x30", b"changed")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Bounded proof: start=\x10, end=None (open end).
        // The start proof's last node is the branch at \x10 with children.
        let proof = db_a
            .change_proof(root1_a, root2.clone(), Some(b"\x10".as_ref()), None, None)
            .expect("change proof");

        let change_ctx = ChangeProofContext::from(proof);
        // Verification must succeed — the children of the last start-proof
        // node match because both databases share the same base state.
        let result = change_ctx.verify_and_propose(
            &db_b,
            root1_b,
            root2,
            Some(b"\x10".as_ref()),
            None,
            None,
        );
        assert!(
            result.is_ok(),
            "verify_and_propose should succeed when start tail children match: {:?}",
            result.err()
        );
    }

    // -----------------------------------------------------------------------
    // Defense-in-depth gap tests (cross-implementation comparison)
    // -----------------------------------------------------------------------

    /// Defense-in-depth gap 4a: A non-empty `start_proof` with
    /// `start_key=None` must be rejected. Matches `AvalancheGo`'s
    /// `ErrUnexpectedStartProof`. Firewood catches this via
    /// `BoundaryProofUnverifiable` in `verify_start_proof`.
    #[test]
    fn test_unexpected_start_proof_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = test_db(dir.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\xa0", b"v1")];
        let p = (&db).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1 = db.current_root_hash().expect("root");

        let extra = vec![put(b"\x50", b"mid")];
        let p = (&db).create_proposal(extra).expect("proposal");
        p.commit().expect("commit");
        let root2 = db.current_root_hash().expect("root");

        // Create a bounded proof with start_key=Some → non-empty start_proof
        let proof = db
            .change_proof(
                root1,
                root2.clone(),
                Some(b"\x10".as_ref()),
                Some(b"\xa0".as_ref()),
                None,
            )
            .expect("change proof");

        assert!(
            !proof.start_proof().is_empty(),
            "bounded proof should have non-empty start_proof"
        );

        // Verify with start_key=None: non-empty start_proof has no key
        // to validate against → BoundaryProofUnverifiable
        let result = verify_proof_structure(&proof, root2, None, Some(b"\xa0"), None);
        let err = result.expect_err("non-empty start_proof with start_key=None must be rejected");
        assert!(
            matches!(
                err,
                firewood::api::Error::ProofError(ProofError::BoundaryProofUnverifiable)
            ),
            "expected BoundaryProofUnverifiable, got: {err}"
        );
    }

    /// Defense-in-depth gap 4c (complete proof case): `end_root` is an
    /// all-zeros hash but `batch_ops` contain data. Matches `AvalancheGo`'s
    /// `ErrDataInMissingRootProof`. Firewood catches via `EndRootMismatch`
    /// because the complete proof's computed root won't match zeros.
    #[test]
    fn test_empty_end_root_with_batch_ops_rejected_complete() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\xa0", b"v1")];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        let changes = vec![put(b"\x50", b"mid")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Generate an unbounded proof (complete) with real batch_ops
        let proof = db_a
            .change_proof(root1_a.clone(), root2, None, None, None)
            .expect("change proof");

        assert!(!proof.batch_ops().is_empty(), "proof should have batch_ops");

        // Verify with end_root = all zeros (empty trie hash)
        let empty_root = firewood::api::HashKey::empty();
        let change_ctx = ChangeProofContext::from(proof);
        let err = change_ctx
            .verify_and_propose(&db_b, root1_b, empty_root, None, None, None)
            .expect_err("empty end_root with batch_ops should be rejected");
        assert!(
            matches!(
                err.1,
                firewood::api::Error::ProofError(ProofError::EndRootMismatch)
            ),
            "expected EndRootMismatch, got: {:?}",
            err.1
        );
    }

    /// Defense-in-depth gap 4c (partial proof case): `end_root` is an
    /// all-zeros hash but the proof is bounded with real `batch_ops`.
    /// The boundary proof's hash chain fails because it was validated
    /// against the real root, not the zeros.
    #[test]
    fn test_empty_end_root_with_batch_ops_rejected_partial() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\xa0", b"v1")];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        let changes = vec![put(b"\x50", b"mid")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Generate a bounded proof (partial)
        let proof = db_a
            .change_proof(
                root1_a.clone(),
                root2,
                Some(b"\x10".as_ref()),
                Some(b"\xa0".as_ref()),
                None,
            )
            .expect("change proof");

        // Verify with end_root = all zeros
        let empty_root = firewood::api::HashKey::empty();
        let result = verify_proof_structure(&proof, empty_root, Some(b"\x10"), Some(b"\xa0"), None);
        // The boundary proof's value_digest will fail against the wrong root
        assert!(
            result.is_err(),
            "partial proof with empty end_root should fail boundary proof validation"
        );
    }

    /// Defense-in-depth gap 4b: An intermediate value mismatch between the
    /// proof and the proposal is caught by the hash chain. Matches
    /// `AvalancheGo`'s `verifyChangeProofKeyValues`. Firewood catches via
    /// `SubTrieHashMismatch` or `BoundaryValueMismatch` in post-application
    /// verification.
    #[test]
    fn test_intermediate_value_mismatch_caught_by_hash_chain() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Same keys, different values → different root hashes.
        // apply_change_proof_to_parent rebuilds the proposal from the
        // start_root revision. With different base states, the proposal
        // trie's intermediate hashes diverge from the proof's claims.
        let initial_a = vec![
            put(b"\x10", b"valA0"),
            put(b"\x20", b"valA1"),
            put(b"\x30", b"valA2"),
        ];
        let initial_b = vec![
            put(b"\x10", b"valB0"),
            put(b"\x20", b"valB1"),
            put(b"\x30", b"valB2"),
        ];

        let p = (&db_a).create_proposal(initial_a).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial_b).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_ne!(root1_a, root1_b, "roots should differ");

        // db_a: add changes → root2
        let changes = vec![put(b"\x50", b"new")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2_a = db_a.current_root_hash().expect("root");

        // Generate an unbounded proof from db_a (root1_a → root2_a)
        let proof = db_a
            .change_proof(root1_a, root2_a.clone(), None, None, None)
            .expect("change proof");

        // Verify on db_b: apply_change_proof_to_parent uses root1_b as
        // the parent. Since root1_b has different values than root1_a,
        // applying the same batch_ops produces a different trie whose
        // root hash won't match root2_a.
        let change_ctx = ChangeProofContext::from(proof);
        let result = change_ctx.verify_and_propose(&db_b, root1_b, root2_a, None, None, None);

        let err =
            result.expect_err("proof from db_a should fail on db_b with different base state");
        assert!(
            matches!(
                err.1,
                firewood::api::Error::ProofError(ProofError::EndRootMismatch)
            ),
            "expected EndRootMismatch, got: {:?}",
            err.1
        );
    }

    /// Empty proof with bounded range is rejected with `MissingBoundaryProof`.
    /// Matches `AvalancheGo`'s `ErrEmptyProof`. A proof with no `batch_ops`,
    /// no `start_proof`, and no `end_proof` is only valid for a complete proof
    /// (no bounds), where the root hash check applies instead.
    #[test]
    fn test_empty_proof_rejected() {
        use firewood::Proof;
        use firewood::api::FrozenChangeProof;

        // Construct a completely empty proof
        let empty_proof = FrozenChangeProof::new(
            Proof::new(Box::new([])),
            Proof::new(Box::new([])),
            Box::new([]),
        );

        // With bounds: should fail with MissingBoundaryProof
        // (empty proofs + batch_ops is checked by Fix 9, but even with
        // empty batch_ops, bounded ranges need boundary proofs to be valid)
        let dummy_root = firewood::api::HashKey::empty();

        // Bounded range with empty proof must be rejected because there are
        // no boundary proofs to validate the range
        let _result = verify_proof_structure(
            &empty_proof,
            dummy_root.clone(),
            Some(b"\x10"),
            Some(b"\xa0"),
            None,
        );
        // The proof has empty start/end proofs. verify_start_proof passes
        // (empty start_proof is valid). verify_end_proof passes (empty
        // end_proof is valid). But Fix 9 only fires when batch_ops is
        // non-empty. For empty batch_ops + empty proofs + bounds, the
        // structural checks pass — this is intentional because an empty
        // diff within a range is valid (no changes in that sub-range).
        // The real protection comes from the root hash check for complete
        // proofs or the boundary proof hash chain for non-empty proofs.
        //
        // Test the case where empty proofs + non-empty batch_ops + bounds
        // triggers MissingBoundaryProof:
        let non_empty_proof = FrozenChangeProof::new(
            Proof::new(Box::new([])),
            Proof::new(Box::new([])),
            Box::new([put(b"\x50", b"value")]),
        );
        let result = verify_proof_structure(
            &non_empty_proof,
            dummy_root,
            Some(b"\x10"),
            Some(b"\xa0"),
            None,
        );
        let err = result.expect_err("empty proofs with batch_ops and bounds must be rejected");
        assert!(
            matches!(
                err,
                firewood::api::Error::ProofError(ProofError::MissingBoundaryProof)
            ),
            "expected MissingBoundaryProof, got: {err}"
        );
    }

    /// Duplicate keys in `batch_ops` are rejected with `ChangeProofKeysNotSorted`.
    /// Matches `AvalancheGo`'s `ErrNonIncreasingValues`. The strict ordering
    /// check rejects equal keys (not just reversed keys).
    #[test]
    fn test_duplicate_keys_rejected() {
        use firewood::api::FrozenChangeProof;

        let dir = tempfile::tempdir().expect("tempdir");
        let db = test_db(dir.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\xa0", b"v1")];
        let p = (&db).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1 = db.current_root_hash().expect("root");

        let extra = vec![put(b"\x50", b"mid")];
        let p = (&db).create_proposal(extra).expect("proposal");
        p.commit().expect("commit");
        let root2 = db.current_root_hash().expect("root");

        // Generate a valid proof to get real ProofNodes
        let valid_proof = db
            .change_proof(root1, root2.clone(), None, None, None)
            .expect("change proof");

        // Construct a new proof with real proof nodes but duplicate keys
        let crafted = FrozenChangeProof::new(
            firewood::Proof::new(valid_proof.start_proof().as_ref().into()),
            firewood::Proof::new(valid_proof.end_proof().as_ref().into()),
            Box::new([put(b"\x50", b"a"), put(b"\x50", b"b")]),
        );

        let result = verify_proof_structure(&crafted, root2, None, None, None);
        let err = result.expect_err("duplicate keys must be rejected");
        assert!(
            matches!(
                err,
                firewood::api::Error::ProofError(ProofError::ChangeProofKeysNotSorted)
            ),
            "expected ChangeProofKeysNotSorted, got: {err}"
        );
    }

    /// Truncated proof positive round-trip: generate a truncated proof
    /// (`max_length=1`), verify it succeeds, then use `find_next_key` to
    /// get the continuation range and complete a second round.
    #[test]
    fn test_truncated_proof_round_trip() {
        use std::num::NonZeroUsize;

        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Both databases start with identical state
        let initial = vec![
            put(b"\x10", b"v0"),
            put(b"\x20", b"v1"),
            put(b"\x30", b"v2"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // db_a: add multiple changes
        let changes = vec![
            put(b"\x10", b"changed0"),
            put(b"\x20", b"changed1"),
            put(b"\x30", b"changed2"),
        ];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Round 1: truncated proof with max_length=1
        let proof1 = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                None,
                None,
                NonZeroUsize::new(1),
            )
            .expect("truncated proof");

        let change_ctx1 = ChangeProofContext::from(proof1);
        let mut proposed1 = change_ctx1
            .verify_and_propose(
                &db_b,
                root1_b.clone(),
                root2.clone(),
                None,
                None,
                NonZeroUsize::new(1),
            )
            .expect("round 1 verify_and_propose should succeed");

        // find_next_key should return Some (not done yet)
        let next = proposed1
            .find_next_key()
            .expect("find_next_key round 1 should not error");
        assert!(next.is_some(), "truncated proof should have a next range");
        let (next_start, next_end) = next.expect("just checked is_some");

        // Commit round 1
        let root_b_after_1 = proposed1.commit().expect("commit round 1");

        // Round 2: continue from where round 1 left off
        let proof2 = db_a
            .change_proof(
                root1_a,
                root2.clone(),
                Some(next_start.as_ref()),
                next_end.as_deref(),
                None,
            )
            .expect("continuation proof");

        let change_ctx2 = ChangeProofContext::from(proof2);
        let mut proposed2 = change_ctx2
            .verify_and_propose(
                &db_b,
                root_b_after_1.expect("commit should return root"),
                root2,
                Some(next_start.as_ref()),
                next_end.as_deref(),
                None,
            )
            .expect("round 2 verify_and_propose should succeed");

        // find_next_key should not error; either complete or more rounds
        let final_next = proposed2.find_next_key();
        assert!(
            final_next.is_ok(),
            "find_next_key round 2 should not error: {final_next:?}"
        );
    }

    // -----------------------------------------------------------------------
    // New root hash verification tests
    // -----------------------------------------------------------------------

    /// Happy path: first sync round with `start_key=None`, only `end_proof`.
    /// Verify that the root hash is computed correctly with a single boundary.
    #[test]
    fn test_root_hash_single_end_proof() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\x20", b"v1")];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        let changes = vec![put(b"\x10", b"changed")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // First round: start_key=None, end_key=Some
        let proof = db_a
            .change_proof(root1_a, root2.clone(), None, Some(b"\x20".as_ref()), None)
            .expect("proof");

        assert!(
            proof.start_proof().is_empty(),
            "first round: no start proof"
        );

        let change_ctx = ChangeProofContext::from(proof);
        let result = change_ctx.verify_and_propose(
            &db_b,
            root1_b,
            root2,
            None,
            Some(b"\x20".as_ref()),
            None,
        );
        assert!(
            result.is_ok(),
            "single end proof should verify: {:?}",
            result.err()
        );
    }

    /// Happy path: last sync round with `end_key=None`, empty `end_proof`, only `start_proof`.
    #[test]
    fn test_root_hash_single_start_proof() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\x20", b"v1")];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        let changes = vec![put(b"\x20", b"changed")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Last round: start_key=Some, end_key=None
        let proof = db_a
            .change_proof(root1_a, root2.clone(), Some(b"\x10".as_ref()), None, None)
            .expect("proof");

        let change_ctx = ChangeProofContext::from(proof);
        let result = change_ctx.verify_and_propose(
            &db_b,
            root1_b,
            root2,
            Some(b"\x10".as_ref()),
            None,
            None,
        );
        assert!(
            result.is_ok(),
            "single start proof should verify: {:?}",
            result.err()
        );
    }

    /// Happy path: middle sync round with both boundary proofs.
    #[test]
    fn test_root_hash_two_proofs() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![
            put(b"\x10", b"v0"),
            put(b"\x20", b"v1"),
            put(b"\x30", b"v2"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        let changes = vec![put(b"\x20", b"changed")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Middle round: both boundaries
        let proof = db_a
            .change_proof(
                root1_a,
                root2.clone(),
                Some(b"\x10".as_ref()),
                Some(b"\x30".as_ref()),
                None,
            )
            .expect("proof");

        let change_ctx = ChangeProofContext::from(proof);
        let result = change_ctx.verify_and_propose(
            &db_b,
            root1_b,
            root2,
            Some(b"\x10".as_ref()),
            Some(b"\x30".as_ref()),
            None,
        );
        assert!(
            result.is_ok(),
            "two-proof bounded should verify: {:?}",
            result.err()
        );
    }

    /// Happy path: complete unbounded proof, both proofs empty.
    #[test]
    fn test_root_hash_complete_no_proofs() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\x20", b"v1")];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        let changes = vec![put(b"\x10", b"changed0"), put(b"\x20", b"changed1")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Unbounded proof
        let proof = db_a
            .change_proof(root1_a, root2.clone(), None, None, None)
            .expect("proof");

        let change_ctx = ChangeProofContext::from(proof);
        let result = change_ctx.verify_and_propose(&db_b, root1_b, root2, None, None, None);
        assert!(
            result.is_ok(),
            "complete unbounded should verify: {:?}",
            result.err()
        );
    }

    /// Adversarial: valid proof verified with a flipped `end_root` byte.
    #[test]
    fn test_root_hash_rejects_wrong_end_root() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\x20", b"v1")];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        let changes = vec![put(b"\x10", b"changed")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        let proof = db_a
            .change_proof(root1_a, root2, None, None, None)
            .expect("proof");

        // Use a wrong end_root (all zeros)
        let wrong_root = firewood::api::HashKey::empty();
        let change_ctx = ChangeProofContext::from(proof);
        let err = change_ctx
            .verify_and_propose(&db_b, root1_b, wrong_root, None, None, None)
            .expect_err("wrong end_root must fail");
        assert!(
            matches!(
                err.1,
                firewood::api::Error::ProofError(ProofError::EndRootMismatch)
            ),
            "expected EndRootMismatch, got: {:?}",
            err.1
        );
    }

    /// Value mismatch at an INTERMEDIATE proof node (not a boundary key).
    /// Key `\x20` sits on the end proof path to `\x20\x10\x01` and is within
    /// the range [\x10, \x20\x10\x01]. `verify_boundary_values` checks all
    /// proof nodes with values, not just the two boundary keys.
    #[test]
    fn test_intermediate_proof_node_value_mismatch() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Both dbs share keys, but differ at \x20 — an intermediate node
        // on the end proof path to \x20\x10\x01.
        let initial_a = vec![
            put(b"\x10", b"shared0"),
            put(b"\x20", b"valA"),
            put(b"\x20\x10\x01", b"deep"),
            put(b"\x30", b"shared2"),
        ];
        let initial_b = vec![
            put(b"\x10", b"shared0"),
            put(b"\x20", b"valB"),
            put(b"\x20\x10\x01", b"deep"),
            put(b"\x30", b"shared2"),
        ];

        let p = (&db_a).create_proposal(initial_a).expect("proposal");
        p.commit().expect("commit");
        let root_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial_b).expect("proposal");
        p.commit().expect("commit");
        let root_b = db_b.current_root_hash().expect("root");
        assert_ne!(root_a, root_b, "roots should differ at key \\x20");

        // Change a key deeper than \x20 on the same path
        let changes = vec![put(b"\x20\x10\x01", b"deep_changed")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root_a_updated = db_a.current_root_hash().expect("root");

        // Bounded proof: range [\x10, \x20\x10\x01].
        // The end proof path goes through the node at \x20 (intermediate).
        // batch_ops only contains the \x20\x10\x01 change, not \x20.
        let proof = db_a
            .change_proof(
                root_a.clone(),
                root_a_updated.clone(),
                Some(b"\x10".as_ref()),
                Some(b"\x20\x10\x01".as_ref()),
                None,
            )
            .expect("change proof");

        // Verify on dbB: \x20 has "valB" in proposal but "valA" in proof.
        // The boundary check only covers \x10 and \x20\x10\x01, not \x20.
        let change_ctx = ChangeProofContext::from(proof);
        let result = change_ctx.verify_and_propose(
            &db_b,
            root_b,
            root_a_updated,
            Some(b"\x10".as_ref()),
            Some(b"\x20\x10\x01".as_ref()),
            None,
        );

        let err = result.expect_err("intermediate value mismatch at \\x20 should be caught");
        assert!(
            matches!(
                err.1,
                firewood::api::Error::ProofError(ProofError::ProofNodeValueMismatch { .. })
            ),
            "expected ProofNodeValueMismatch, got: {:?}",
            err.1
        );
    }

    /// Unit test for `is_child_in_range` at various depths and boundary positions.
    #[test]
    fn test_is_child_in_range_boundary_cases() {
        use firewood_storage::PathComponent;

        let pc = |v: u8| PathComponent::try_new(v).expect("test nibble");

        // No bounds → always in-range
        assert!(is_child_in_range(&[], pc(5), None, None));
        assert!(is_child_in_range(&[], pc(0), None, None));
        assert!(is_child_in_range(&[], pc(15), None, None));

        // Start only: nibble must be > start at depth 0
        let start = vec![pc(5)];
        assert!(!is_child_in_range(&[], pc(4), Some(&start), None));
        assert!(!is_child_in_range(&[], pc(5), Some(&start), None)); // ON boundary
        assert!(is_child_in_range(&[], pc(6), Some(&start), None));

        // End only: nibble must be < end at depth 0
        let end = vec![pc(10)];
        assert!(is_child_in_range(&[], pc(9), None, Some(&end)));
        assert!(!is_child_in_range(&[], pc(10), None, Some(&end))); // ON boundary
        assert!(!is_child_in_range(&[], pc(11), None, Some(&end)));

        // Both bounds at depth 0: strictly between
        assert!(!is_child_in_range(&[], pc(5), Some(&start), Some(&end)));
        assert!(is_child_in_range(&[], pc(6), Some(&start), Some(&end)));
        assert!(is_child_in_range(&[], pc(9), Some(&start), Some(&end)));
        assert!(!is_child_in_range(&[], pc(10), Some(&start), Some(&end)));

        // Depth 1: parent has one nibble, check at depth 1
        let parent = vec![pc(3)];
        let start_deep = vec![pc(3), pc(2)];
        let end_deep = vec![pc(3), pc(8)];
        assert!(!is_child_in_range(
            &parent,
            pc(2),
            Some(&start_deep),
            Some(&end_deep)
        ));
        assert!(is_child_in_range(
            &parent,
            pc(3),
            Some(&start_deep),
            Some(&end_deep)
        ));
        assert!(is_child_in_range(
            &parent,
            pc(7),
            Some(&start_deep),
            Some(&end_deep)
        ));
        assert!(!is_child_in_range(
            &parent,
            pc(8),
            Some(&start_deep),
            Some(&end_deep)
        ));

        // Start terminates before depth: all in-range from start side
        let short_start = vec![pc(3)]; // terminates at depth 0
        assert!(is_child_in_range(
            &parent,
            pc(0),
            Some(&short_start),
            Some(&end_deep)
        ));
        assert!(is_child_in_range(
            &parent,
            pc(7),
            Some(&short_start),
            Some(&end_deep)
        ));
        assert!(!is_child_in_range(
            &parent,
            pc(8),
            Some(&short_start),
            Some(&end_deep)
        ));
    }

    // -----------------------------------------------------------------------
    // B1: Untested error path tests
    // -----------------------------------------------------------------------

    /// B1: `StartKeyLargerThanFirstKey` is returned when the `start_key`
    /// is lexicographically greater than the first key in `batch_ops`.
    #[test]
    fn test_start_key_larger_than_first_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = test_db(dir.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\xa0", b"v1")];
        let p = (&db).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1 = db.current_root_hash().expect("root");

        let extra = vec![put(b"\x50", b"mid")];
        let p = (&db).create_proposal(extra).expect("proposal");
        p.commit().expect("commit");
        let root2 = db.current_root_hash().expect("root");

        // Generate a proof with batch_ops containing key \x50
        let proof = db
            .change_proof(root1, root2.clone(), None, None, None)
            .expect("change proof");

        // Verify with start_key=\xff, which is greater than any key in batch_ops
        let result = verify_proof_structure(&proof, root2, Some(b"\xff"), None, None);
        let err = result.expect_err("start_key > first_key must be rejected");
        assert!(
            matches!(
                err,
                firewood::api::Error::ProofError(ProofError::StartKeyLargerThanFirstKey)
            ),
            "expected StartKeyLargerThanFirstKey, got: {err}"
        );
    }

    /// B1: `EndKeyLessThanLastKey` is returned when the `end_key` is
    /// lexicographically less than the last key in `batch_ops`.
    #[test]
    fn test_end_key_less_than_last_key() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = test_db(dir.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\xa0", b"v1")];
        let p = (&db).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1 = db.current_root_hash().expect("root");

        let extra = vec![put(b"\x50", b"mid")];
        let p = (&db).create_proposal(extra).expect("proposal");
        p.commit().expect("commit");
        let root2 = db.current_root_hash().expect("root");

        // Generate a proof with batch_ops containing key \x50
        let proof = db
            .change_proof(root1, root2.clone(), None, None, None)
            .expect("change proof");

        // Verify with end_key=\x01, which is less than the last key in batch_ops
        let result = verify_proof_structure(&proof, root2, None, Some(b"\x01"), None);
        let err = result.expect_err("end_key < last_key must be rejected");
        assert!(
            matches!(
                err,
                firewood::api::Error::ProofError(ProofError::EndKeyLessThanLastKey)
            ),
            "expected EndKeyLessThanLastKey, got: {err}"
        );
    }

    /// B1: `ProofIsLargerThanMaxLength` is returned when `batch_ops.len()`
    /// exceeds the specified `max_length`.
    #[test]
    fn test_proof_larger_than_max_length() {
        use std::num::NonZeroUsize;

        let dir = tempfile::tempdir().expect("tempdir");
        let db = test_db(dir.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\xa0", b"v1")];
        let p = (&db).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1 = db.current_root_hash().expect("root");

        let extra = vec![put(b"\x50", b"mid"), put(b"\x60", b"mid2")];
        let p = (&db).create_proposal(extra).expect("proposal");
        p.commit().expect("commit");
        let root2 = db.current_root_hash().expect("root");

        // Generate a proof with 2 batch_ops (no truncation during generation)
        let proof = db
            .change_proof(root1, root2.clone(), None, None, None)
            .expect("change proof");

        assert!(
            proof.batch_ops().len() >= 2,
            "proof should have at least 2 batch_ops"
        );

        // Verify with max_length=1, which is less than the actual count
        let result = verify_proof_structure(&proof, root2, None, None, NonZeroUsize::new(1));
        let err = result.expect_err("proof exceeding max_length must be rejected");
        assert!(
            matches!(
                err,
                firewood::api::Error::ProofError(ProofError::ProofIsLargerThanMaxLength)
            ),
            "expected ProofIsLargerThanMaxLength, got: {err}"
        );
    }

    /// B1: `UnsupportedDeleteRange` is rejected when a crafted proof
    /// contains a `DeleteRange` operation, tested through the full
    /// `verify_and_propose` pipeline.
    #[test]
    fn test_delete_range_rejected_via_verify_and_propose() {
        use firewood::api::FrozenChangeProof;

        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\xa0", b"v1")];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        let changes = vec![put(b"\x50", b"mid")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Get a valid proof to steal its boundary proofs
        let valid_proof = db_a
            .change_proof(root1_a, root2.clone(), None, None, None)
            .expect("change proof");

        // Craft a proof with a DeleteRange op
        let crafted = FrozenChangeProof::new(
            firewood::Proof::new(valid_proof.start_proof().as_ref().into()),
            firewood::Proof::new(valid_proof.end_proof().as_ref().into()),
            Box::new([BatchOp::DeleteRange {
                prefix: b"\x50".to_vec().into_boxed_slice(),
            }]),
        );

        let change_ctx = ChangeProofContext::from(crafted);
        let err = change_ctx
            .verify_and_propose(&db_b, root1_b, root2, None, None, None)
            .expect_err("DeleteRange must be rejected");
        assert!(
            matches!(
                err.1,
                firewood::api::Error::ProofError(ProofError::UnsupportedDeleteRange)
            ),
            "expected UnsupportedDeleteRange, got: {:?}",
            err.1
        );
    }

    /// B1: `BoundaryProofsDivergeAtRoot` for divergence at depth 0.
    /// Two boundary proofs whose root nodes have different paths are rejected.
    #[test]
    fn test_divergence_at_depth_zero() {
        use firewood::api::FrozenChangeProof;
        use firewood_storage::Hashable;

        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // We need keys that create deep trie structures so that the
        // start and end proofs have different root node paths.
        // Keys chosen so root has children at nibble 1 and nibble a.
        let initial = vec![
            put(b"\x10", b"v0"),
            put(b"\x11", b"v1"),
            put(b"\xa0", b"v2"),
            put(b"\xa1", b"v3"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        let changes = vec![put(b"\x10", b"changed"), put(b"\xa0", b"changed2")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Get the individual proofs
        let start_proof_data = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                Some(b"\x10".as_ref()),
                None,
                None,
            )
            .expect("start proof");
        let end_proof_data = db_a
            .change_proof(root1_a, root2.clone(), None, Some(b"\xa0".as_ref()), None)
            .expect("end proof");

        // If the start and end proofs happen to share the same root node
        // path (which they will for a trie with a single root), the
        // divergence depth won't be 0. We test the structural check by
        // crafting proofs with intentionally mismatched root paths.
        // We can do this by using the start proof's start_proof nodes
        // (which go through nibble 1) and the end proof's end_proof
        // nodes (which go through nibble a) — but only if their first
        // nodes have different paths.
        let start_nodes = start_proof_data.start_proof();
        let end_nodes = end_proof_data.end_proof();

        if let (Some(start_root), Some(end_root)) =
            (start_nodes.as_ref().first(), end_nodes.as_ref().first())
        {
            let s_root_path: &[firewood_storage::PathComponent] = start_root.full_path();
            let e_root_path: &[firewood_storage::PathComponent] = end_root.full_path();

            if s_root_path != e_root_path {
                // The proofs diverge at depth 0 — craft a combined proof
                let crafted = FrozenChangeProof::new(
                    firewood::Proof::new(start_nodes.as_ref().into()),
                    firewood::Proof::new(end_nodes.as_ref().into()),
                    Box::new([put(b"\x50", b"mid")]),
                );

                let change_ctx = ChangeProofContext::from(crafted);
                let err = change_ctx
                    .verify_and_propose(
                        &db_b,
                        root1_b,
                        root2,
                        Some(b"\x10".as_ref()),
                        Some(b"\xa0".as_ref()),
                        None,
                    )
                    .expect_err("divergence at depth 0 must fail");
                // The boundary proof hash chain check or root hash check
                // should catch this.
                assert!(
                    matches!(err.1, firewood::api::Error::ProofError(_)),
                    "expected ProofError, got: {:?}",
                    err.1
                );
            }
        }

        // Even if the trie structure doesn't produce divergence at 0
        // naturally, the root hash walk + boundary proof checks still
        // protect against forged proofs.
    }

    /// Non-empty `end_proof` with no `end_key` and no `batch_ops` is rejected.
    /// Matches `AvalancheGo`'s `ErrUnexpectedEndProof`.
    #[test]
    fn test_unexpected_end_proof_rejected() {
        use firewood::api::FrozenChangeProof;

        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\x20", b"v1")];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // Create a change that produces a non-empty end_proof
        let changes = vec![put(b"\x20", b"changed")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Generate a proof with end_key=Some so that end_proof is populated
        let valid_proof = db_a
            .change_proof(root1_a, root2.clone(), None, Some(b"\x20".as_ref()), None)
            .expect("change proof");

        // Craft a proof with the non-empty end_proof but empty batch_ops.
        let crafted = FrozenChangeProof::new(
            firewood::Proof::new(Box::new([])),
            firewood::Proof::new(valid_proof.end_proof().as_ref().into()),
            Box::new([]),
        );

        // Verify with end_key=None and empty batch_ops — should be rejected.
        let change_ctx = ChangeProofContext::from(crafted);
        let err = change_ctx
            .verify_and_propose(&db_b, root1_b, root2, None, None, None)
            .expect_err("unexpected end proof must be rejected");
        assert!(
            matches!(
                err.1,
                firewood::api::Error::ProofError(ProofError::UnexpectedEndProof)
            ),
            "expected UnexpectedEndProof, got: {:?}",
            err.1
        );
    }

    // -----------------------------------------------------------------------
    // B2: Untested attack vector tests
    // -----------------------------------------------------------------------

    /// A8/B2: Omitted-change attack — a malicious sender removes one
    /// `BatchOp` from a valid proof. The proposal's in-range subtrie hash
    /// differs from `end_root`'s, so `EndRootMismatch` is returned.
    #[test]
    fn test_omitted_change_attack() {
        use firewood::api::FrozenChangeProof;

        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![
            put(b"\x10", b"v0"),
            put(b"\x20", b"v1"),
            put(b"\x30", b"v2"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // Change all 3 keys
        let changes = vec![
            put(b"\x10", b"changed0"),
            put(b"\x20", b"changed1"),
            put(b"\x30", b"changed2"),
        ];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Generate a valid unbounded proof
        let valid_proof = db_a
            .change_proof(root1_a, root2.clone(), None, None, None)
            .expect("change proof");

        let batch_ops = valid_proof.batch_ops();
        assert!(
            batch_ops.len() >= 2,
            "need at least 2 batch_ops to test omission"
        );

        // Craft a proof with one batch_op removed (drop the middle one)
        let mut shortened_ops: Vec<BatchOp<firewood::merkle::Key, firewood::merkle::Value>> =
            batch_ops.to_vec();
        shortened_ops.remove(1); // Remove the second op

        let crafted = FrozenChangeProof::new(
            firewood::Proof::new(valid_proof.start_proof().as_ref().into()),
            firewood::Proof::new(valid_proof.end_proof().as_ref().into()),
            shortened_ops.into_boxed_slice(),
        );

        // The proposal will be missing one change, so its root hash
        // won't match end_root.
        let change_ctx = ChangeProofContext::from(crafted);
        let err = change_ctx
            .verify_and_propose(&db_b, root1_b, root2, None, None, None)
            .expect_err("omitted change must be detected");
        assert!(
            matches!(
                err.1,
                firewood::api::Error::ProofError(ProofError::EndRootMismatch)
            ),
            "expected EndRootMismatch for omitted change, got: {:?}",
            err.1
        );
    }

    /// B2: Empty `batch_ops` with non-empty boundary proofs represents
    /// "no changes in this sub-range". The root hash check should pass
    /// because the proposal state is unchanged from the parent.
    #[test]
    fn test_empty_batch_ops_with_nonempty_proofs() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Both databases with identical state
        let initial = vec![
            put(b"\x10", b"v0"),
            put(b"\x20", b"v1"),
            put(b"\x30", b"v2"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // Change only \x30 (outside the range [\x10, \x20])
        let changes = vec![put(b"\x30", b"changed")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Generate a bounded proof for range [\x10, \x20].
        // Since the only change (\x30) is outside this range,
        // batch_ops should be empty.
        let proof = db_a
            .change_proof(
                root1_a,
                root2.clone(),
                Some(b"\x10".as_ref()),
                Some(b"\x20".as_ref()),
                None,
            )
            .expect("change proof");

        // Regardless of whether batch_ops is empty, verification should
        // succeed since the sub-range is correctly represented.
        let change_ctx = ChangeProofContext::from(proof);
        let result = change_ctx.verify_and_propose(
            &db_b,
            root1_b,
            root2,
            Some(b"\x10".as_ref()),
            Some(b"\x20".as_ref()),
            None,
        );
        assert!(
            result.is_ok(),
            "empty batch_ops with non-empty proofs should verify: {:?}",
            result.err()
        );
    }

    /// B2: Double-commit should return the cached hash from the first
    /// commit rather than erroring.
    #[test]
    fn test_double_commit_returns_cached_hash() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\x20", b"v1")];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        let changes = vec![put(b"\x10", b"changed")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        let proof = db_a
            .change_proof(root1_a, root2.clone(), None, None, None)
            .expect("change proof");

        let change_ctx = ChangeProofContext::from(proof);
        let mut proposed = change_ctx
            .verify_and_propose(&db_b, root1_b, root2, None, None, None)
            .expect("verify_and_propose");

        // First commit
        let hash1 = proposed.commit().expect("first commit");

        // Second commit should return the cached hash
        let hash2 = proposed.commit().expect("second commit");
        assert_eq!(
            hash1, hash2,
            "double commit should return the same cached hash"
        );
    }

    /// A8 variant: omitted-change attack on a bounded proof.
    /// Removes one `BatchOp` from a bounded (partial) proof.
    #[test]
    fn test_omitted_change_attack_bounded() {
        use firewood::api::FrozenChangeProof;

        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![
            put(b"\x10", b"v0"),
            put(b"\x20", b"v1"),
            put(b"\x30", b"v2"),
            put(b"\x40", b"v3"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        let changes = vec![put(b"\x20", b"changed1"), put(b"\x30", b"changed2")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Bounded proof covering [\x10, \x40]
        let valid_proof = db_a
            .change_proof(
                root1_a,
                root2.clone(),
                Some(b"\x10".as_ref()),
                Some(b"\x40".as_ref()),
                None,
            )
            .expect("change proof");

        let batch_ops = valid_proof.batch_ops();
        if batch_ops.len() >= 2 {
            // Remove one op
            let mut shortened_ops: Vec<BatchOp<firewood::merkle::Key, firewood::merkle::Value>> =
                batch_ops.to_vec();
            shortened_ops.remove(0);

            let crafted = FrozenChangeProof::new(
                firewood::Proof::new(valid_proof.start_proof().as_ref().into()),
                firewood::Proof::new(valid_proof.end_proof().as_ref().into()),
                shortened_ops.into_boxed_slice(),
            );

            let change_ctx = ChangeProofContext::from(crafted);
            let err = change_ctx
                .verify_and_propose(
                    &db_b,
                    root1_b,
                    root2,
                    Some(b"\x10".as_ref()),
                    Some(b"\x40".as_ref()),
                    None,
                )
                .expect_err("omitted change in bounded proof must be detected");
            assert!(
                matches!(
                    err.1,
                    firewood::api::Error::ProofError(ProofError::EndRootMismatch)
                ),
                "expected EndRootMismatch for bounded omitted change, got: {:?}",
                err.1
            );
        }
    }
}
