// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::cmp::Ordering;
use std::num::NonZeroUsize;

use firewood_metrics::firewood_increment;
#[cfg(feature = "ethhash")]
use firewood_storage::TrieHash;
use firewood_storage::{
    Children, HashType, NibblesIterator, Path, PathComponent, PathComponentSliceExt, PathIterItem,
};
#[cfg(feature = "ethhash")]
use rlp::Rlp;

use firewood::{
    ProofError, ProofNode,
    api::{self, BatchOp, DbView as _, FrozenChangeProof, HashKey as ApiHashKey},
};

use crate::{
    BorrowedBytes, ChangeProofResult, DatabaseHandle, HashKey, HashResult, KeyRange, Maybe,
    NextKeyRangeResult, OwnedBytes, ProposedChangeProofResult, ValueResult, VoidResult,
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
    pub start_root: HashKey,
    /// The root hash of the ending revision. This must be provided.
    /// If the root is not found in the database, the function will return
    /// [`ChangeProofResult::EndRevisionNotFound`].
    pub end_root: HashKey,
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
    /// The change proof to verify.
    pub proof: Option<Box<ChangeProofContext>>,
    /// The root hash of the starting revision.
    pub start_root: HashKey,
    /// The root hash of the ending revision.
    pub end_root: HashKey,
    /// The lower bound of the key range that the proof is expected to cover. If
    /// `None`, the proof is expected to cover from the start of the keyspace.
    pub start_key: Maybe<BorrowedBytes<'a>>,
    /// The upper bound of the key range that the proof is expected to cover. If
    /// `None`, the proof is expected to cover to the end of the keyspace.
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
    end_root: ApiHashKey,
    start_key: Option<Box<[u8]>>,
    end_key: Option<Box<[u8]>>,
}

#[derive(Debug)]
enum ProposalState<'db> {
    Proposed(crate::ProposalHandle<'db>),
    Committed(Option<ApiHashKey>),
    Failed,
}

impl From<FrozenChangeProof> for ChangeProofContext {
    fn from(proof: FrozenChangeProof) -> Self {
        Self { proof }
    }
}

impl ChangeProofContext {
    /// Verify structural properties and boundary proofs of the change proof.
    ///
    /// On success, returns a `VerificationContext` capturing the verification
    /// parameters so that downstream logic can avoid re-verifying.
    fn verify_proof_structure(
        proof: &FrozenChangeProof,
        end_root: ApiHashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<VerificationContext, api::Error> {
        let batch_ops = proof.batch_ops();

        // Reject inverted ranges early. The generator enforces this, but the
        // verifier must independently validate because start_key/end_key
        // come from the caller, not the proof.
        if let (Some(start), Some(end)) = (start_key, end_key)
            && start.cmp(end) == Ordering::Greater
        {
            return Err(api::Error::InvalidRange {
                start_key: start.to_vec().into(),
                end_key: end.to_vec().into(),
            });
        }

        // The honest diff algorithm only produces Put and Delete ops,
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

        // Reject proofs with batch_ops but no boundary proofs, UNLESS
        // this is a complete proof (no key bounds). Complete proofs are validated
        // by root hash comparison in verify_root_hash() instead.
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
        Self::verify_start_proof(proof, start_key, &end_root)?;
        Self::verify_end_proof(proof, end_key, &end_root)?;

        Ok(VerificationContext {
            end_root,
            start_key: start_key.map(Box::from),
            end_key: end_key.map(Box::from),
        })
    }

    /// Verify the start boundary proof against the end root hash.
    fn verify_start_proof(
        proof: &FrozenChangeProof,
        start_key: Option<&[u8]>,
        end_root: &ApiHashKey,
    ) -> Result<(), api::Error> {
        // An empty start_proof is valid: it means the range starts from the
        // beginning of the keyspace (start_key=None in the first sync round).
        if proof.start_proof().is_empty() {
            return Ok(());
        }

        // If start_proof is non-empty, we MUST have a key to validate
        // it against. The honest generator only produces a non-empty
        // start_proof when start_key is Some.
        let Some(start_key) = start_key else {
            return Err(api::Error::ProofError(
                ProofError::BoundaryProofUnverifiable,
            ));
        };

        proof.start_proof().value_digest(start_key, end_root)?;
        Ok(())
    }

    /// Verify the end boundary proof's hash chain against the end root.
    ///
    /// The generator builds the end proof for either `last_op_key`
    /// (truncated) or `end_key` (non-truncated). Rather than relying on
    /// an external hint like `max_length`, we use the proof's own content
    /// (`batch_ops`) to determine which key to try first:
    /// - Put + `Ok(Some(_))`: definitive inclusion
    /// - Delete + `Ok(None)`: expected exclusion
    /// - `end_key` fallback: any `Ok` result accepted
    ///
    /// Nibble boundaries at intermediate proof nodes are derived from
    /// the proof's own structure. At the last proof node, the boundary
    /// is derived from `batch_ops().last()` in `verify_root_hash`,
    /// independent of which key validated the hash chain here.
    fn verify_end_proof(
        proof: &FrozenChangeProof,
        end_key: Option<&[u8]>,
        end_root: &ApiHashKey,
    ) -> Result<(), api::Error> {
        if proof.end_proof().is_empty() {
            return Ok(());
        }

        // Try last_op_key first — the end proof may be for the right
        // edge of the changes (truncated proof).
        if let Some(last_op) = proof.batch_ops().last() {
            let is_delete = matches!(last_op, BatchOp::Delete { .. });
            match proof
                .end_proof()
                .value_digest(last_op.key().as_ref(), end_root)
            {
                // Put + inclusion: definitive match
                Ok(Some(_)) if !is_delete => return Ok(()),
                // Delete + exclusion: expected match
                Ok(None) if is_delete => return Ok(()),
                // Wrong key or unexpected result: fall through
                Ok(_) | Err(ProofError::ShouldBePrefixOfProvenKey) => {}
                Err(e) => return Err(api::Error::ProofError(e)),
            }
        }

        // Last batch op key didn't match (or no batch ops) — try end_key.
        // This is the non-truncated case where the generator used end_key.
        if let Some(end_key) = end_key {
            proof.end_proof().value_digest(end_key, end_root)?;
            return Ok(());
        }

        // end_proof is non-empty but no key could validate it.
        Err(api::Error::ProofError(
            ProofError::BoundaryProofUnverifiable,
        ))
    }
}

/// Convert a byte key to a nibble path.
fn key_to_nibbles(key: &[u8]) -> Vec<PathComponent> {
    NibblesIterator::new(key)
        .filter_map(PathComponent::try_new)
        .collect()
}

/// Forward-only cursor over a proposal's path-to-key result.
/// Both proof nodes and proposal path nodes are sorted by depth,
/// so we advance through the proposal path in lockstep with the
/// proof nodes instead of building a `HashMap`.
struct ProposalCursor<'a> {
    path: &'a [PathIterItem],
    pos: usize,
}

impl<'a> ProposalCursor<'a> {
    const fn new(path: &'a [PathIterItem]) -> Self {
        Self { path, pos: 0 }
    }

    /// Advance past nodes shallower than `depth`, return the node
    /// at exactly `depth` if one exists.
    fn advance_to(&mut self, depth: usize) -> Option<&'a PathIterItem> {
        while let Some(item) = self.path.get(self.pos) {
            if item.key_nibbles.len() >= depth {
                break;
            }
            self.pos = self.pos.saturating_add(1);
        }
        self.path
            .get(self.pos)
            .filter(|item| item.key_nibbles.len() == depth)
    }
}

/// Convert a proof node's nibble key to a byte-level key for proposal lookup.
///
/// Each pair of nibbles is combined into one byte via [`Path::bytes_iter`].
/// Odd-length keys are padded with a zero nibble before conversion so that
/// `path_to_key` traverses through the odd-depth node.
fn proof_node_byte_key(node: &ProofNode) -> Vec<u8> {
    let nibbles = node.key.as_byte_slice();
    if nibbles.len().is_multiple_of(2) {
        Path::from(nibbles).bytes_iter().collect()
    } else {
        // bytes_iter() drops trailing odd nibbles, so a proof node at
        // depth 5 would produce a byte key of depth 4. path_to_key with
        // the shorter key misses the depth-5 node, causing the cursor to
        // return None and the children check to false-positive. Padding
        // with 0 ensures path_to_key traverses through the odd-depth
        // node. The extra nibble is harmless — path_to_key stops at the
        // deepest matching node.
        let mut padded = nibbles.to_vec();
        padded.push(0);
        Path::from(padded.as_slice()).bytes_iter().collect()
    }
}

/// Verify that a proof node's value matches the proposal's value at the
/// same trie path.
///
/// Nodes at odd nibble depths cannot carry values in the trie encoding.
/// Malicious proofs with values at odd depths are rejected earlier by
/// the boundary proof hash chain verification (`ValueAtOddNibbleLength`).
fn verify_proof_node_value(
    node: &ProofNode,
    lookup_item: Option<&PathIterItem>,
) -> Result<(), ProofError> {
    let depth = node.key.len();
    // Only nodes at even nibble depths can carry values.
    if !depth.is_multiple_of(2) {
        return Ok(());
    }
    let proposal_value = lookup_item.and_then(|item| item.node.value());
    match (&node.value_digest, proposal_value) {
        (None, None) => Ok(()),
        (Some(digest), Some(val)) if digest.verify(val) => Ok(()),
        _ => Err(ProofError::ProofNodeValueMismatch { depth }),
    }
}

/// Verify that in-range children of each proof node match the proposal.
///
/// The proof's hash chain to `end_root` is already verified by
/// `verify_proof_structure`. If every in-range child hash matches the
/// proposal's, the substitution in the old rehash approach would have
/// been a no-op and the root hash unchanged. A mismatch means the
/// proposal's state differs from what the proof claims.
///
/// The boundary nibble at each depth is derived from the proof's own
/// structure: the next node's key at this depth tells us which child
/// the proof navigated to. This is always correct regardless of which
/// key `verify_end_proof` validated against. At the last proof node
/// (where no next node exists), `fallback_nibbles` from the external
/// key provides the boundary.
///
/// `is_end_proof` controls both the direction of the in-range check
/// and what happens when no boundary nibble is available (key
/// exhausted at the last node): for the end proof, no children are
/// in-range (they extend the key, coming after it); for the start
/// proof, all children are in-range (same reason — after the key is
/// the in-range direction).
fn verify_in_range_children(
    nodes: &[ProofNode],
    cursor: &mut ProposalCursor<'_>,
    fallback_nibbles: &[PathComponent],
    is_end_proof: bool,
) -> Result<(), ProofError> {
    // The forward-only cursor assumes nodes are in ascending depth order,
    // which is guaranteed by the prefix checks in `verify_proof_structure`.
    for (i, node) in nodes.iter().enumerate() {
        let depth = node.key.len();

        // Advance cursor to this depth. None means the proposal has no node
        // here (trie compressed through this level). This is safe: the value
        // check passes with (None, None), and children below default to
        // all-None so any proof child hash at an in-range slot mismatches.
        let lookup_item = cursor.advance_to(depth);
        verify_proof_node_value(node, lookup_item)?;

        // Derive the boundary nibble from the proof's own structure: the
        // next node's key at this depth tells us which child the proof
        // navigated to. At the last node, fall back to the external
        // key's nibble at this depth. If the fallback also has no nibble
        // (key exhausted — inclusion proof), the end proof checks no
        // children (they're all after the key) while the start proof
        // checks all children (after the key is the in-range direction).
        let boundary_nibble = i
            .checked_add(1)
            .and_then(|next_i| nodes.get(next_i))
            .and_then(|next| next.key.get(depth).copied())
            .or_else(|| fallback_nibbles.get(depth).copied());

        // On cursor miss, defaults to all-None children (see above).
        let proposal_children: Children<Option<HashType>> = lookup_item
            .and_then(|item| item.node.as_branch().map(|b| b.children_hashes()))
            .unwrap_or_default();

        for nibble in PathComponent::ALL {
            let in_range = match boundary_nibble {
                Some(bn) if is_end_proof => nibble < bn,
                Some(bn) => nibble > bn,
                None => !is_end_proof,
            };
            if in_range && node.child_hashes[nibble] != proposal_children[nibble] {
                return Err(ProofError::InRangeChildMismatch { depth });
            }
        }
    }
    Ok(())
}

/// Verify Case 2c: both start and end boundary proofs exist.
/// Finds the divergence point, checks shared prefix values, the
/// divergence parent's in-range children, and walks both tails.
fn verify_divergent_proofs(
    start_nodes: &[ProofNode],
    end_nodes: &[ProofNode],
    start_path: &[PathIterItem],
    end_path: &[PathIterItem],
    start_nibbles: Option<&[PathComponent]>,
    end_nibbles: Option<&[PathComponent]>,
) -> Result<(), api::Error> {
    // Find where the two proof paths diverge.
    let divergence_depth = start_nodes
        .iter()
        .zip(end_nodes.iter())
        .position(|(s, e)| s.key != e.key)
        .unwrap_or(std::cmp::min(start_nodes.len(), end_nodes.len()));

    let Some(parent_idx) = divergence_depth.checked_sub(1) else {
        return Err(ProofError::BoundaryProofsDivergeAtRoot.into());
    };
    let parent = start_nodes
        .get(parent_idx)
        .ok_or(api::Error::ProofError(ProofError::EndRootMismatch))?;

    // Verify values at shared prefix nodes (including divergence parent).
    // At shared prefix nodes both proofs route through the same nibble,
    // so no children are in-range from either side — only values matter.
    let shared = start_nodes
        .get(..divergence_depth)
        .ok_or(api::Error::ProofError(ProofError::EndRootMismatch))?;
    // The start cursor advances through shared prefix nodes first,
    // then continues into the divergence parent and start tail.
    let mut start_cursor = ProposalCursor::new(start_path);
    for node in shared {
        // None means the proposal compressed through this depth (no
        // explicit node). verify_proof_node_value allows (None, None)
        // — if the proof node claims a value exists but the proposal
        // has none, the mismatch is caught.
        let item = start_cursor.advance_to(node.key.len());
        verify_proof_node_value(node, item)?;
    }

    // At the divergence parent: children between the two boundary
    // nibbles are in-range. Derive boundary nibbles from the proof
    // nodes below the divergence point — the node at divergence_depth
    // in each proof is the first node after the parent, and its key
    // at the parent's depth is the nibble the proof navigated to.
    // If a proof has no node after divergence, fall back to the
    // external key's nibble at the parent depth.
    let parent_depth = parent.key.len();
    let start_bn = start_nodes
        .get(divergence_depth)
        .and_then(|next| next.key.get(parent_depth).copied())
        .or_else(|| start_nibbles.and_then(|sn| sn.get(parent_depth).copied()));
    let end_bn = end_nodes
        .get(divergence_depth)
        .and_then(|next| next.key.get(parent_depth).copied())
        .or_else(|| end_nibbles.and_then(|en| en.get(parent_depth).copied()));

    // Reuse the start cursor to look up the divergence parent's children.
    // None means the proposal compressed through this depth; children
    // default to all-None, so any proof child with a hash at an in-range
    // slot will produce an InRangeChildMismatch, correctly detecting the
    // structural difference between the proof and proposal.
    let item = start_cursor.advance_to(parent_depth);
    let proposal_children = item
        .and_then(|i| i.node.as_branch().map(|b| b.children_hashes()))
        .unwrap_or_default();
    for nibble in PathComponent::ALL {
        let after_start = start_bn.is_none_or(|s| nibble > s);
        let before_end = end_bn.is_none_or(|e| nibble < e);
        if after_start && before_end && parent.child_hashes[nibble] != proposal_children[nibble] {
            return Err(ProofError::InRangeChildMismatch {
                depth: parent_depth,
            }
            .into());
        }
    }

    // Walk tails independently below the divergence point.
    // start_cursor naturally continues past shared prefix nodes.
    let start_tail = start_nodes
        .get(divergence_depth..)
        .ok_or(api::Error::ProofError(ProofError::EndRootMismatch))?;
    let end_tail = end_nodes
        .get(divergence_depth..)
        .ok_or(api::Error::ProofError(ProofError::EndRootMismatch))?;
    if !start_tail.is_empty() {
        verify_in_range_children(
            start_tail,
            &mut start_cursor,
            start_nibbles.unwrap_or(&[]),
            false,
        )?;
    }
    if !end_tail.is_empty() {
        // Separate cursor for the end tail since it walks a different
        // proposal path than the start cursor.
        let mut end_cursor = ProposalCursor::new(end_path);
        verify_in_range_children(end_tail, &mut end_cursor, end_nibbles.unwrap_or(&[]), true)?;
    }

    Ok(())
}

/// Verify that boundary proof nodes are consistent with the proposal
/// by comparing in-range children directly.
///
/// The proof's hash chain to `end_root` is already verified in
/// `verify_proof_structure`. Substituting a child hash with an
/// identical value doesn't change the hash input, so rehashing is
/// redundant. Instead we compare in-range children directly — same
/// verification, less work.
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
        let computed: HashKey = proposal.root_hash().map(HashKey::from).unwrap_or_default();
        if computed != HashKey::from(verification.end_root.clone()) {
            return Err(api::Error::ProofError(ProofError::EndRootMismatch));
        }
        return Ok(());
    }

    // Fallback nibbles from external keys — used at the last proof node
    // where no next node exists to derive the boundary from. Intermediate
    // nodes derive their boundary from the next proof node's key, which
    // is always correct regardless of which key verify_end_proof validated.
    let start_nibbles = verification.start_key.as_deref().map(key_to_nibbles);

    // The end proof's last-node boundary comes from the last batch_op's
    // key, not from end_key or the key verify_end_proof validated. The
    // proposal only applies changes up to last_op — children beyond
    // last_op's nibble are unchanged from start_root and should not be
    // compared against end_root (which may differ due to truncation).
    let end_nibbles = proof.batch_ops().last().map(|op| key_to_nibbles(op.key()));

    // Retrieve the proposal's path aligned with each boundary proof.
    // The lookup key is derived from the proof nodes themselves, so the
    // proposal traversal follows the same trie path as the proof.
    let start_path = get_proposal_path_for_proof(start_nodes, proposal)?;
    let end_path = get_proposal_path_for_proof(end_nodes, proposal)?;

    if start_nodes.is_empty() {
        // Case 2a: Only end proof (first sync round, start_key=None).
        let mut cursor = ProposalCursor::new(&end_path);
        verify_in_range_children(
            end_nodes,
            &mut cursor,
            end_nibbles.as_deref().unwrap_or(&[]),
            true,
        )?;
    } else if end_nodes.is_empty() {
        // Case 2b: Only start proof (last sync round, end of keyspace).
        let mut cursor = ProposalCursor::new(&start_path);
        verify_in_range_children(
            start_nodes,
            &mut cursor,
            start_nibbles.as_deref().unwrap_or(&[]),
            false,
        )?;
    } else {
        // Case 2c: Both boundary proofs exist (middle sync round).
        verify_divergent_proofs(
            start_nodes,
            end_nodes,
            &start_path,
            &end_path,
            start_nibbles.as_deref(),
            end_nibbles.as_deref(),
        )?;
    }

    Ok(())
}

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
        db: &'db DatabaseHandle,
        start_root: ApiHashKey,
        end_root: ApiHashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<ProposedChangeProofContext<'db>, Box<(Self, api::Error)>> {
        // Destructure self so we can move `proof` into either the Ok or Err
        // result without cloning.
        let proof = self.proof;

        let verification = match Self::verify_proof_structure(
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
        db: &DatabaseHandle,
        start_root: ApiHashKey,
        end_root: ApiHashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<Option<ApiHashKey>, api::Error> {
        let mut proposed = self
            .verify_and_propose(db, start_root, end_root, start_key, end_key, max_length)
            .map_err(|boxed| {
                let (_proof, err) = *boxed;
                err
            })?;
        proposed.commit()
    }
}

impl ProposedChangeProofContext<'_> {
    /// Commit a previously proposed change proof. Consumes the proposal handle.
    fn commit(&mut self) -> Result<Option<ApiHashKey>, api::Error> {
        // Pessimistically set state to Failed; success paths below restore it.
        let state = std::mem::replace(&mut self.proposal_state, ProposalState::Failed);
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
            ProposalState::Failed => Err(api::Error::CommitAlreadyFailed),
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
        let computed: HashKey = match &self.proposal_state {
            ProposalState::Proposed(handle) => {
                handle.root_hash().map(HashKey::from).unwrap_or_default()
            }
            ProposalState::Committed(hash) => hash.clone().map(HashKey::from).unwrap_or_default(),
            ProposalState::Failed => {
                return Err(api::Error::CommitAlreadyFailed);
            }
        };
        if computed != HashKey::from(self.verification.end_root.clone()) {
            return Err(api::Error::ProofError(ProofError::EndRootMismatch));
        }
        Ok(())
    }

    /// Returns the next key range that should be fetched after processing this
    /// change proof, or `None` if there are no more keys to fetch.
    ///
    /// # Termination analysis
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

/// Retrieve the proposal's path aligned with the given proof nodes.
///
/// The lookup key is derived from the last proof node's nibble path
/// (converted to bytes), so the proposal traversal follows the same
/// trie path as the proof regardless of which key was verified.
/// Returns an empty Vec when the proof is empty.
fn get_proposal_path_for_proof(
    proof_nodes: &[ProofNode],
    proposal: &crate::ProposalHandle<'_>,
) -> Result<Vec<PathIterItem>, api::Error> {
    let Some(last) = proof_nodes.last() else {
        return Ok(Vec::new());
    };
    let key = proof_node_byte_key(last);
    proposal.path_to_key(&key)
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
    type Item = Result<HashKey, api::Error>;

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
            let code_hash: HashKey = TrieHash::try_from(code_hash_slice).ok()?.into();
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
    let start_key = args.start_key.into_option();
    let end_key = args.end_key.into_option();
    let handle = db.and_then(|db| args.proof.map(|p| (db, p)));
    crate::invoke_with_handle(handle, |(db, ctx)| {
        ctx.verify_and_propose(
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
    let start_key = args.start_key.into_option();
    let end_key = args.end_key.into_option();
    let handle = db.and_then(|db| args.proof.map(|p| (db, p)));
    crate::invoke_with_handle(handle, |(db, ctx)| {
        ctx.verify_and_commit(
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
/// - [`ValueResult::Err`] containing an error message if the `ChangeProof`
///   cannot be serialized.
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
/// - [`ValueResult::Err`] containing an error message if the proposed `ChangeProof`
///   cannot be serialized.
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

impl crate::MetricsContextExt for (&DatabaseHandle, Box<ChangeProofContext>) {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        self.0.metrics_context()
    }
}
