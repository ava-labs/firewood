// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::HashMap;
use std::num::NonZeroUsize;

use firewood_metrics::firewood_increment;
#[cfg(feature = "ethhash")]
use firewood_storage::TrieHash;
use firewood_storage::{Children, HashType, Hashable, PathComponent, PathIterItem};
#[cfg(feature = "ethhash")]
use rlp::Rlp;

use firewood::{
    ProofError, ProofNode,
    api::{self, DbView as _, FrozenChangeProof, HashKey},
    next_nibble,
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
    max_length: Option<NonZeroUsize>,
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
fn verify_proof(
    proof: &FrozenChangeProof,
    end_root: HashKey,
    start_key: Option<&[u8]>,
    end_key: Option<&[u8]>,
    max_length: Option<NonZeroUsize>,
) -> Result<VerificationContext, api::Error> {
    let batch_ops = proof.batch_ops();

    // Check batch_ops length <= max_length
    if let Some(max_length) = max_length
        && batch_ops.len() > max_length.into()
    {
        return Err(api::Error::ProofError(
            ProofError::ProofIsLargerThanMaxLength,
        ));
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

    // Verify keys are sorted and unique
    if !batch_ops
        .iter()
        .is_sorted_by(|a, b| b.key().cmp(a.key()) == Ordering::Greater)
    {
        return Err(api::Error::ProofError(ProofError::ChangeProofKeysNotSorted));
    }

    // Verify boundary proofs against end_root
    verify_start_proof(proof, start_key, &end_root)?;
    verify_end_proof(proof, end_key, &end_root, max_length)?;

    Ok(VerificationContext {
        end_root,
        start_key: start_key.map(Box::from),
        end_key: end_key.map(Box::from),
        max_length,
    })
}

/// Verify the start boundary proof against the end root hash.
fn verify_start_proof(
    proof: &FrozenChangeProof,
    start_key: Option<&[u8]>,
    end_root: &HashKey,
) -> Result<(), api::Error> {
    if proof.start_proof().is_empty() {
        return Ok(());
    }
    let Some(start_key) = start_key else {
        return Ok(());
    };
    proof.start_proof().value_digest(start_key, end_root)?;
    Ok(())
}

/// Verify the end boundary proof against the end root hash.
///
/// When `batch_ops.len() >= max_length`, the proof may have been truncated.
/// The generator uses the last batch op key when truncated but `end_key`
/// otherwise. Since the verifier cannot distinguish the two cases, we try
/// the last batch op key first and fall back to `end_key`.
fn verify_end_proof(
    proof: &FrozenChangeProof,
    end_key: Option<&[u8]>,
    end_root: &HashKey,
    max_length: Option<NonZeroUsize>,
) -> Result<(), api::Error> {
    if proof.end_proof().is_empty() {
        return Ok(());
    }

    let batch_ops = proof.batch_ops();
    let potentially_truncated = max_length.is_some_and(|max| batch_ops.len() >= max.get());

    if potentially_truncated
        && let Some(last_op) = batch_ops.last()
        && proof
            .end_proof()
            .value_digest(last_op.key().as_ref(), end_root)
            .is_ok()
    {
        return Ok(());
    }

    if let Some(end_key) = end_key {
        proof.end_proof().value_digest(end_key, end_root)?;
        return Ok(());
    }

    if let Some(last_op) = batch_ops.last() {
        proof
            .end_proof()
            .value_digest(last_op.key().as_ref(), end_root)?;
        return Ok(());
    }

    Ok(())
}

/// Returns true if this proof covers the full range (not truncated, not
/// bounded by start/end keys). Only complete proofs should have their
/// computed root hash compared against the expected end root.
fn is_complete_proof(verification: &VerificationContext, proof: &FrozenChangeProof) -> bool {
    // A complete proof has no start/end key bounds, and the end_proof is
    // empty (meaning we reached the end of the keyspace rather than being
    // truncated or bounded).
    verification.start_key.is_none()
        && verification.end_key.is_none()
        && proof.end_proof().is_empty()
}

/// Determine which key the end proof was generated for.
/// Replicates the try-both logic from `verify_end_proof`.
fn resolve_end_proof_key(
    verification: &VerificationContext,
    proof: &FrozenChangeProof,
) -> Option<Box<[u8]>> {
    let batch_ops = proof.batch_ops();
    let potentially_truncated = verification
        .max_length
        .is_some_and(|max| batch_ops.len() >= max.get());

    if potentially_truncated {
        if let Some(last_op) = batch_ops.last() {
            let key: &[u8] = last_op.key().as_ref();
            if proof
                .end_proof()
                .value_digest(key, &verification.end_root)
                .is_ok()
            {
                return Some(key.into());
            }
        }
    }

    if let Some(ref end_key) = verification.end_key {
        if proof
            .end_proof()
            .value_digest(end_key.as_ref(), &verification.end_root)
            .is_ok()
        {
            return Some(end_key.clone());
        }
    }

    batch_ops
        .last()
        .map(|op| -> Box<[u8]> { op.key().as_ref().into() })
}

/// Verify sub-trie hashes: compare child hashes at branch points in the
/// boundary proofs against the proposal's trie after applying batch_ops.
fn verify_subtrie_hashes(
    proof: &FrozenChangeProof,
    verification: &VerificationContext,
    proposal: &crate::ProposalHandle<'_>,
) -> Result<(), api::Error> {
    let start_proof = proof.start_proof();
    let end_proof = proof.end_proof();

    if start_proof.is_empty() || end_proof.is_empty() {
        return Ok(());
    }

    let divergence = match find_divergence(start_proof, end_proof) {
        Some(d) => d,
        None => return Ok(()),
    };

    // Verify start side: children > path nibble below divergence
    if let Some(ref start_key) = verification.start_key {
        let proposal_path = proposal.path_to_key(start_key)?;
        verify_proof_path_children(start_proof, &proposal_path, &divergence, Side::Start)?;
    }

    // Verify end side: children < path nibble below divergence
    if let Some(end_key) = resolve_end_proof_key(verification, proof) {
        let proposal_path = proposal.path_to_key(&end_key)?;
        verify_proof_path_children(end_proof, &proposal_path, &divergence, Side::End)?;
    }

    Ok(())
}

/// Verify that values at boundary keys in the proposal match the boundary
/// proofs' claims.
fn verify_boundary_values(
    proof: &FrozenChangeProof,
    verification: &VerificationContext,
    proposal: &crate::ProposalHandle<'_>,
    end_root: &HashKey,
) -> Result<(), api::Error> {
    // Check start boundary
    if !proof.start_proof().is_empty() {
        if let Some(ref start_key) = verification.start_key {
            verify_single_boundary_value(proposal, proof.start_proof(), start_key, end_root)?;
        }
    }

    // Check end boundary
    if !proof.end_proof().is_empty() {
        if let Some(end_key) = resolve_end_proof_key(verification, proof) {
            verify_single_boundary_value(proposal, proof.end_proof(), &end_key, end_root)?;
        }
    }

    Ok(())
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
    ) -> Result<ProposedChangeProofContext<'db>, (Self, api::Error)> {
        // Destructure self so we can move `proof` into either the Ok or Err
        // result without cloning.
        let proof = self.proof;

        let verification = match verify_proof(
            &proof,
            end_root.clone(),
            start_key,
            end_key,
            max_length,
        ) {
            Ok(v) => v,
            Err(e) => return Err((Self { proof }, e)),
        };

        let proposal = match db.apply_change_proof_to_parent(start_root, &proof) {
            Ok(p) => p,
            Err(e) => return Err((Self { proof }, e)),
        };

        // Only verify the root hash when the proof covers the full range.
        // Truncated proofs (partial changes) will naturally produce a different
        // root hash until all rounds have been applied.
        if is_complete_proof(&verification, &proof) {
            let computed: crate::HashKey = proposal
                .handle
                .root_hash()
                .map(crate::HashKey::from)
                .unwrap_or_default();
            if computed != crate::HashKey::from(end_root.clone()) {
                return Err((
                    Self { proof },
                    api::Error::ProofError(ProofError::EndRootMismatch),
                ));
            }
        }

        // Post-application sub-trie hash and boundary value checks
        if let Err(e) = verify_subtrie_hashes(&proof, &verification, &proposal.handle) {
            return Err((Self { proof }, e));
        }
        if let Err(e) = verify_boundary_values(&proof, &verification, &proposal.handle, &end_root)
        {
            return Err((Self { proof }, e));
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
            .map_err(|(_proof, err)| err)?;
        proposed.commit()
    }
}

// ---------------------------------------------------------------------------
// Methods on ProposedChangeProofContext
// ---------------------------------------------------------------------------

impl ProposedChangeProofContext<'_> {
    /// Commit a previously proposed change proof. Consumes the proposal handle.
    fn commit(&mut self) -> Result<Option<HashKey>, api::Error> {
        match self.proposal_state {
            ProposalState::Committed(ref hash) => Ok(hash.clone()),
            ProposalState::Proposed(_) => {
                // Take the Proposed variant out, replacing with a temporary.
                // We need to move the handle out to call commit_proposal.
                let old_state =
                    std::mem::replace(&mut self.proposal_state, ProposalState::Committed(None));
                let ProposalState::Proposed(handle) = old_state else {
                    // We just matched Proposed above, so this is unreachable in
                    // practice. Return an error rather than panicking.
                    return Err(api::Error::ProofError(ProofError::ProposalIsNone));
                };
                match handle.commit_proposal() {
                    Ok(hash) => {
                        firewood_increment!(crate::registry::MERGE_COUNT, 1, "change" => "commit");
                        self.proposal_state = ProposalState::Committed(hash.clone());
                        Ok(hash)
                    }
                    Err(err) => {
                        // Commit failed; we've already consumed the handle so
                        // leave state as Committed(None).
                        Err(err)
                    }
                }
            }
        }
    }

    /// Returns the next key range that should be fetched after processing this
    /// change proof, or `None` if there are no more keys to fetch.
    fn find_next_key(&mut self) -> Result<Option<KeyRange>, api::Error> {
        let Some(last_op) = self.proof.batch_ops().last() else {
            return Ok(None);
        };

        if self.proof.end_proof().is_empty() {
            return Ok(None);
        }

        if let Some(ref end_key) = self.verification.end_key
            && **last_op.key() >= **end_key
        {
            return Ok(None);
        }

        Ok(Some((last_op.key().clone(), self.verification.end_key.clone())))
    }
}

// ---------------------------------------------------------------------------
// Helper types and functions (unchanged)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
enum Side {
    Start,
    End,
}

struct DivergenceInfo {
    /// Index into the proof nodes where the proofs diverge.
    node_index: usize,
    /// Nibble the start proof takes at the divergence node.
    start_nibble: PathComponent,
    /// Nibble the end proof takes at the divergence node.
    end_nibble: PathComponent,
}

/// Find where two boundary proofs (from the same end trie) diverge.
fn find_divergence(
    start_proof: &firewood::Proof<Box<[ProofNode]>>,
    end_proof: &firewood::Proof<Box<[ProofNode]>>,
) -> Option<DivergenceInfo> {
    let start_nodes: &[ProofNode] = start_proof;
    let end_nodes: &[ProofNode] = end_proof;

    for (i, (s_node, e_node)) in start_nodes.iter().zip(end_nodes.iter()).enumerate() {
        let s_next = start_nodes.get(i + 1);
        let e_next = end_nodes.get(i + 1);

        let s_nibble = s_next.and_then(|n| next_nibble(s_node.full_path(), n.full_path()));
        let e_nibble = e_next.and_then(|n| next_nibble(e_node.full_path(), n.full_path()));

        match (s_nibble, e_nibble) {
            (Some(sn), Some(en)) if sn != en => {
                return Some(DivergenceInfo {
                    node_index: i,
                    start_nibble: sn,
                    end_nibble: en,
                });
            }
            // One proof continues deeper but the other stops — the key of the
            // shorter proof *is* the divergence point.
            (Some(sn), None) => {
                // end proof stops here; use next_nibble from start to the end
                // proof's last node key to determine end side.
                let en = next_nibble(
                    e_node.full_path(),
                    end_nodes
                        .last()
                        .map_or_else(|| e_node.full_path(), |n| n.full_path()),
                );
                if let Some(en) = en {
                    if sn != en {
                        return Some(DivergenceInfo {
                            node_index: i,
                            start_nibble: sn,
                            end_nibble: en,
                        });
                    }
                }
            }
            (None, Some(en)) => {
                let sn = next_nibble(
                    s_node.full_path(),
                    start_nodes
                        .last()
                        .map_or_else(|| s_node.full_path(), |n| n.full_path()),
                );
                if let Some(sn) = sn {
                    if sn != en {
                        return Some(DivergenceInfo {
                            node_index: i,
                            start_nibble: sn,
                            end_nibble: en,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    None
}

/// Compare child hashes at branch points for one boundary proof side
/// against the proposal's trie path nodes.
fn verify_proof_path_children(
    boundary_proof: &firewood::Proof<Box<[ProofNode]>>,
    proposal_path: &[PathIterItem],
    divergence: &DivergenceInfo,
    side: Side,
) -> Result<(), api::Error> {
    let proof_nodes: &[ProofNode] = boundary_proof.as_ref();

    // Build lookup: key_nibbles → &PathIterItem
    let proposal_lookup: HashMap<&[firewood_storage::PathComponent], &PathIterItem> = proposal_path
        .iter()
        .map(|item| (item.key_nibbles.as_ref(), item))
        .collect();

    for (depth, proof_node) in proof_nodes.iter().enumerate() {
        if depth < divergence.node_index {
            // Above divergence: both proofs follow same child, no in-range children
            continue;
        }

        // Determine in-range nibbles based on depth and side.
        // The "path nibble" is the nibble the current node uses to continue
        // deeper into the proof — i.e., the nibble leading to the NEXT node.
        let in_range: Vec<PathComponent> = if depth == divergence.node_index {
            // At divergence: nibbles strictly between start_nibble and end_nibble
            in_range_at_divergence(divergence)
        } else {
            // Below divergence: need the nibble going forward from this node
            let forward_nibble = proof_nodes
                .get(depth + 1)
                .and_then(|next| next_nibble(proof_node.key.as_ref(), next.key.as_ref()));

            let Some(path_nibble) = forward_nibble else {
                // Last node in the proof path — no forward nibble, skip
                continue;
            };

            match side {
                Side::Start => in_range_after(path_nibble),
                Side::End => in_range_before(path_nibble),
            }
        };

        if in_range.is_empty() {
            continue;
        }

        // Look up corresponding proposal node by key_nibbles
        let proof_key: &[firewood_storage::PathComponent] = proof_node.key.as_ref();
        let proposal_children: Children<Option<HashType>> =
            if let Some(item) = proposal_lookup.get(proof_key) {
                item.node
                    .as_branch()
                    .map(|b| b.children_hashes())
                    .unwrap_or_default()
            } else {
                // Proposal is compressed through this depth — no branch here.
                // If the boundary proof claims non-None child hashes at in-range
                // nibbles, the proposal is missing expected sub-tries.
                for nibble in &in_range {
                    if proof_node.child_hashes[*nibble].is_some() {
                        return Err(api::Error::ProofError(ProofError::SubTrieHashMismatch));
                    }
                }
                continue;
            };

        // Compare in-range child hashes
        for nibble in &in_range {
            if proof_node.child_hashes[*nibble] != proposal_children[*nibble] {
                return Err(api::Error::ProofError(ProofError::SubTrieHashMismatch));
            }
        }
    }

    Ok(())
}

/// Verify that a single boundary key's value in the proposal matches the
/// boundary proof's claim.
fn verify_single_boundary_value(
    proposal: &crate::ProposalHandle<'_>,
    proof: &firewood::Proof<Box<[ProofNode]>>,
    key: &[u8],
    end_root: &HashKey,
) -> Result<(), api::Error> {
    let proposal_value = proposal.val(key)?;
    let proof_digest = proof.value_digest(key, end_root)?;

    match (&proposal_value, &proof_digest) {
        (None, None) => Ok(()),
        (Some(val), Some(digest)) if digest.verify(val) => Ok(()),
        _ => Err(api::Error::ProofError(ProofError::BoundaryValueMismatch)),
    }
}

/// Nibbles strictly between `start_nibble` and `end_nibble` at divergence.
fn in_range_at_divergence(d: &DivergenceInfo) -> Vec<PathComponent> {
    PathComponent::ALL
        .into_iter()
        .filter(|&n| n > d.start_nibble && n < d.end_nibble)
        .collect()
}

/// Nibbles strictly after the given path nibble (for start side below divergence).
fn in_range_after(path_nibble: PathComponent) -> Vec<PathComponent> {
    PathComponent::ALL
        .into_iter()
        .filter(|&n| n > path_nibble)
        .collect()
}

/// Nibbles strictly before the given path nibble (for end side below divergence).
fn in_range_before(path_nibble: PathComponent) -> Vec<PathComponent> {
    PathComponent::ALL
        .into_iter()
        .filter(|&n| n < path_nibble)
        .collect()
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
