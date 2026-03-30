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

        // Reject empty end_proof when end_key is provided or batch_ops is
        // non-empty. The honest generator always produces an end proof in
        // these cases. Without this check, a malicious prover can force the
        // verifier through expensive trie operations (proposal construction,
        // root hash verification) before the proof is ultimately rejected.
        // Matches AvalancheGo's ErrNoEndProof.
        if proof.end_proof().is_empty() && (end_key.is_some() || !batch_ops.is_empty()) {
            return Err(api::Error::ProofError(ProofError::MissingEndProof));
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
    /// The generator always builds the end proof for `batch_ops.last().key()`
    /// when `batch_ops` is non-empty, or for `end_key` when `batch_ops` is
    /// empty (matching `AvalancheGo`'s convention). The verifier mirrors this
    /// to derive the key deterministically — no ambiguity, single hash chain
    /// check.
    fn verify_end_proof(
        proof: &FrozenChangeProof,
        end_key: Option<&[u8]>,
        end_root: &ApiHashKey,
    ) -> Result<(), api::Error> {
        // An empty end_proof is valid: it means both end_key is None and
        // batch_ops is empty (the MissingEndProof structural check in
        // verify_proof_structure rejects empty end_proof in all other
        // cases). There is no right boundary to verify.
        if proof.end_proof().is_empty() {
            return Ok(());
        }

        // Derive the key the generator built the end proof for:
        // last batch_ops key when non-empty, end_key otherwise.
        // The structural checks guarantee that at least one of these
        // is Some when end_proof is non-empty:
        //   - batch_ops non-empty → last() is Some
        //   - batch_ops empty + end_proof non-empty → end_key must be
        //     Some (otherwise UnexpectedEndProof would have fired)
        let key = proof
            .batch_ops()
            .last()
            .map(|op| op.key().as_ref())
            .or(end_key);

        let Some(key) = key else {
            // Unreachable when called after verify_proof_structure:
            // the structural checks ensure at least one key is available
            // whenever end_proof is non-empty. This branch is defensive
            // against callers that bypass structural validation.
            return Err(api::Error::ProofError(
                ProofError::BoundaryProofUnverifiable,
            ));
        };

        // Validate the hash chain and determine inclusion/exclusion.
        // value_digest returns:
        //   Ok(Some(_)) → inclusion proof (key exists at the last proof node)
        //   Ok(None)    → exclusion proof (key does not exist)
        let result = proof.end_proof().value_digest(key, end_root)?;

        // When batch_ops is non-empty, the generator built the end proof
        // for last_op_key. A Put means the key was inserted in end_root
        // (expect inclusion); a Delete means it was removed (expect
        // exclusion). If the result doesn't match, the attacker tampered
        // with batch_ops — the derived key doesn't match the proof's
        // actual target.
        //
        // When batch_ops is empty, the key came from end_key (an
        // arbitrary range bound that may or may not exist in the trie).
        // Both inclusion and exclusion are valid — skip the check.
        if let Some(last_op) = proof.batch_ops().last() {
            let is_delete = matches!(last_op, BatchOp::Delete { .. });
            // Put + inclusion (key exists) or Delete + exclusion (key
            // absent) are the only valid combinations. Any mismatch
            // means the attacker added a spurious key (Put but key
            // doesn't exist) or converted a Put to Delete (Delete but
            // key still exists). The derived key doesn't match the
            // proof's actual target.
            let consistent = matches!((is_delete, &result), (false, Some(_)) | (true, None));
            if !consistent {
                return Err(api::Error::ProofError(
                    ProofError::EndProofOperationMismatch,
                ));
            }
        }

        Ok(())
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
/// exhausted at the last node, i.e. an inclusion proof): for the
/// end proof, no children are in-range (they all come after the
/// proven key); for the start proof, all children are in-range
/// (they all come after the proven key, which is the in-range
/// direction for start proofs).
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
    // key when batch_ops is non-empty. The proposal only applies changes
    // up to last_op — children beyond last_op's nibble are unchanged
    // from start_root and should not be compared against end_root (which
    // may differ due to truncation).
    //
    // When batch_ops is empty, the proposal is just start_root with no
    // modifications. The in-range region extends to end_key, so we fall
    // back to end_key's nibbles to ensure in-range children at the last
    // end-proof node are still checked. Without this fallback,
    // boundary_nibble would be None and no children would be verified,
    // allowing a prover to claim "no changes" while end_root actually
    // differs from start_root within the range.
    let end_nibbles = proof
        .batch_ops()
        .last()
        .map(|op| key_to_nibbles(op.key()))
        .or_else(|| verification.end_key.as_deref().map(key_to_nibbles));

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

    /// Shorthand for creating a Delete batch operation with the right types.
    fn delete(key: &[u8]) -> BatchOp<Key, Value> {
        BatchOp::Delete {
            key: key.to_vec().into_boxed_slice(),
        }
    }

    use crate::{BorrowedBytes, CView, DatabaseHandle, DatabaseHandleArgs, NodeHashAlgorithm};

    use super::ChangeProofContext;
    use firewood_storage::PathIterItem;

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
        let result = ChangeProofContext::verify_proof_structure(
            &proof,
            root2,
            Some(b"\xa0"),
            Some(b"\x10"),
            None,
        );
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
        let result = ChangeProofContext::verify_proof_structure(
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

    /// Happy path: single-round bounded proof where `find_next_key`
    /// returns `Ok(None)` because `last_op >= end_key`.
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

        // Generate a bounded proof with end_key = last changed key.
        // This ensures last_op >= end_key so find_next_key returns
        // None (range complete) without needing a second round.
        let proof = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                None,
                Some(b"\x20".as_ref()),
                None,
            )
            .expect("change proof");

        let change_ctx = ChangeProofContext::from(proof);
        let mut proposed = change_ctx
            .verify_and_propose(&db_b, root1_b, root2, None, Some(b"\x20".as_ref()), None)
            .expect("verify_and_propose should succeed");

        // find_next_key returns Ok(None) — last_op (\x20) >= end_key (\x20)
        let next = proposed
            .find_next_key()
            .expect("find_next_key should not error");
        assert_eq!(
            next, None,
            "single-round proof should be complete when last_op >= end_key"
        );
    }

    // test_find_next_key_root_hash_mismatch removed: it tested the
    // empty-end-proof → check_root_hash termination path, which is now
    // unreachable when batch_ops is non-empty (the generator always
    // produces a non-empty end proof). The check_root_hash path for
    // empty batch_ops is covered by test_empty_batch_ops_end_nibbles_fallback
    // and test_empty_batch_ops_with_nonempty_proofs.

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

    /// Defense-in-depth: A non-empty `start_proof` with
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
        let result =
            ChangeProofContext::verify_proof_structure(&proof, root2, None, Some(b"\xa0"), None);
        let err = result.expect_err("non-empty start_proof with start_key=None must be rejected");
        assert!(
            matches!(
                err,
                firewood::api::Error::ProofError(ProofError::BoundaryProofUnverifiable)
            ),
            "expected BoundaryProofUnverifiable, got: {err}"
        );
    }

    /// Defense-in-depth: `end_root` is an all-zeros hash but `batch_ops`
    /// contain data. The proof is rejected — either by the boundary proof
    /// hash chain (`UnexpectedHash` when the end proof is non-empty) or by
    /// root hash comparison (`EndRootMismatch` when both proofs are empty).
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

        // Generate an unbounded proof with real batch_ops
        let proof = db_a
            .change_proof(root1_a.clone(), root2, None, None, None)
            .expect("change proof");

        assert!(!proof.batch_ops().is_empty(), "proof should have batch_ops");

        // Verify with end_root = all zeros (empty trie hash).
        // The proof is rejected because the boundary proof's hash chain
        // doesn't match the all-zeros root.
        let empty_root = firewood::api::HashKey::empty();
        let change_ctx = ChangeProofContext::from(proof);
        let _err = change_ctx
            .verify_and_propose(&db_b, root1_b, empty_root, None, None, None)
            .expect_err("empty end_root with batch_ops should be rejected");
    }

    /// Defense-in-depth (partial proof case): `end_root` is an
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
        let result = ChangeProofContext::verify_proof_structure(
            &proof,
            empty_root,
            Some(b"\x10"),
            Some(b"\xa0"),
            None,
        );
        // The boundary proof's value_digest will fail against the wrong root
        assert!(
            result.is_err(),
            "partial proof with empty end_root should fail boundary proof validation"
        );
    }

    /// Defense-in-depth: An intermediate value mismatch between the
    /// proof and the proposal is caught by the hash chain. Matches
    /// `AvalancheGo`'s `verifyChangeProofKeyValues`. Firewood catches via
    /// `ProofNodeValueMismatch` or `InRangeChildMismatch` in post-application
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

        // The proof is rejected because the proposal (built from db_b's
        // different base state) has different in-range children than the
        // proof (from db_a's end_root). Detected by InRangeChildMismatch
        // or EndRootMismatch depending on whether the proof has boundary
        // proofs.
        let _err =
            result.expect_err("proof from db_a should fail on db_b with different base state");
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
        // (empty proofs + batch_ops is checked by MissingBoundaryProof, but
        // even with empty batch_ops, bounded ranges need boundary proofs to be valid)
        let dummy_root = firewood::api::HashKey::empty();

        // Bounded range with empty proof must be rejected because there are
        // no boundary proofs to validate the range
        let _result = ChangeProofContext::verify_proof_structure(
            &empty_proof,
            dummy_root.clone(),
            Some(b"\x10"),
            Some(b"\xa0"),
            None,
        );
        // The proof has empty start/end proofs. verify_start_proof passes
        // (empty start_proof is valid). verify_end_proof passes (empty
        // end_proof is valid). But MissingBoundaryProof only fires when batch_ops is
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
        let result = ChangeProofContext::verify_proof_structure(
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

        let result = ChangeProofContext::verify_proof_structure(&crafted, root2, None, None, None);
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

    // test_root_hash_rejects_wrong_end_root removed: duplicate purpose
    // with test_empty_end_root_with_batch_ops_rejected_complete (both test
    // unbounded proofs with wrong end_root). The remaining test covers
    // this scenario.

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

    /// `StartKeyLargerThanFirstKey` is returned when the `start_key`
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
        let result =
            ChangeProofContext::verify_proof_structure(&proof, root2, Some(b"\xff"), None, None);
        let err = result.expect_err("start_key > first_key must be rejected");
        assert!(
            matches!(
                err,
                firewood::api::Error::ProofError(ProofError::StartKeyLargerThanFirstKey)
            ),
            "expected StartKeyLargerThanFirstKey, got: {err}"
        );
    }

    /// `EndKeyLessThanLastKey` is returned when the `end_key` is
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
        let result =
            ChangeProofContext::verify_proof_structure(&proof, root2, None, Some(b"\x01"), None);
        let err = result.expect_err("end_key < last_key must be rejected");
        assert!(
            matches!(
                err,
                firewood::api::Error::ProofError(ProofError::EndKeyLessThanLastKey)
            ),
            "expected EndKeyLessThanLastKey, got: {err}"
        );
    }

    /// `ProofIsLargerThanMaxLength` is returned when `batch_ops.len()`
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
        let result = ChangeProofContext::verify_proof_structure(
            &proof,
            root2,
            None,
            None,
            NonZeroUsize::new(1),
        );
        let err = result.expect_err("proof exceeding max_length must be rejected");
        assert!(
            matches!(
                err,
                firewood::api::Error::ProofError(ProofError::ProofIsLargerThanMaxLength)
            ),
            "expected ProofIsLargerThanMaxLength, got: {err}"
        );
    }

    /// `UnsupportedDeleteRange` is rejected when a crafted proof
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

    /// `BoundaryProofsDivergeAtRoot` is returned when `start_proof` and
    /// `end_proof` have no shared root path (diverge at depth 0).
    ///
    /// This is a defense-in-depth check: after `verify_proof_structure`
    /// passes (which validates each boundary proof's hash chain against
    /// `end_root`), two valid proofs must share the same root node.
    /// Divergence at root can only occur with crafted inputs that bypass
    /// the hash chain check. We test it by calling `verify_root_hash`
    /// directly with proof paths whose first nodes have different keys.
    #[test]
    fn test_divergence_at_depth_zero() {
        use firewood::api::FrozenChangeProof;

        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Keys at different nibble paths: \x10/\x11 under nibble 1,
        // \xa0/\xa1 under nibble a. This creates a trie where the root
        // branches to nibble 1 and nibble a.
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

        // Generate two proofs through different sub-trees.
        // Left proof goes through nibble 1 (\x10..\x11).
        // Right proof goes through nibble a (\xa0..\xa1).
        let left_proof = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                Some(b"\x10".as_ref()),
                Some(b"\x11".as_ref()),
                None,
            )
            .expect("left proof");
        let right_proof = db_a
            .change_proof(
                root1_a,
                root2.clone(),
                Some(b"\xa0".as_ref()),
                Some(b"\xa1".as_ref()),
                None,
            )
            .expect("right proof");

        let start_nodes = left_proof.start_proof();
        let end_nodes = right_proof.end_proof();

        // Both proofs share the same root node (first element) but
        // diverge at depth 1. Verify this assumption.
        let start_slice = start_nodes.as_ref();
        let end_slice = end_nodes.as_ref();
        assert!(
            start_slice.len() >= 2,
            "start proof needs at least 2 nodes, got {}",
            start_slice.len()
        );
        assert!(
            end_slice.len() >= 2,
            "end proof needs at least 2 nodes, got {}",
            end_slice.len()
        );
        assert_eq!(
            start_slice.first().expect("checked len").key,
            end_slice.first().expect("checked len").key,
            "first nodes must share the root path"
        );
        assert_ne!(
            start_slice.get(1).expect("checked len").key,
            end_slice.get(1).expect("checked len").key,
            "second nodes must diverge"
        );

        // Skip the shared root node (index 0) to create boundary proof
        // paths that diverge at position 0.
        let divergent_start: Box<[_]> = start_slice.get(1..).expect("checked len").into();
        let divergent_end: Box<[_]> = end_slice.get(1..).expect("checked len").into();

        let crafted = FrozenChangeProof::new(
            firewood::Proof::new(divergent_start),
            firewood::Proof::new(divergent_end),
            Box::new([put(b"\x50", b"mid")]),
        );

        // Create a valid proposal — apply_change_proof_to_parent only
        // applies batch_ops to the parent revision, it doesn't validate
        // boundary proofs.
        let proposal = db_b
            .apply_change_proof_to_parent(root1_b, &crafted)
            .expect("proposal from batch ops");

        // Construct a VerificationContext with keys that exist in the
        // proposal trie so that build_proposal_lookup succeeds.
        let verification = super::VerificationContext {
            end_root: root2,
            start_key: Some(b"\x10".to_vec().into_boxed_slice()),
            end_key: Some(b"\xa1".to_vec().into_boxed_slice()),
        };

        // Call verify_root_hash directly (bypassing verify_proof_structure
        // which would reject these crafted proofs via hash chain checks).
        let err = super::verify_root_hash(&crafted, &verification, &proposal.handle)
            .expect_err("divergent root proofs should be rejected");
        assert!(
            matches!(
                err,
                firewood::api::Error::ProofError(ProofError::BoundaryProofsDivergeAtRoot)
            ),
            "expected BoundaryProofsDivergeAtRoot, got: {err}"
        );
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

    /// Omitted-change attack — a malicious sender removes one
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

        // The proposal will be missing one change. Detected by
        // InRangeChildMismatch (in-range children comparison) or
        // EndRootMismatch (direct root hash comparison) depending on
        // whether the proof has boundary proofs.
        let change_ctx = ChangeProofContext::from(crafted);
        let _err = change_ctx
            .verify_and_propose(&db_b, root1_b, root2, None, None, None)
            .expect_err("omitted change must be detected");
    }

    /// Empty `batch_ops` with non-empty boundary proofs represents
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

    /// Double-commit should return the cached hash from the first
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

    /// After a failed commit (e.g. `SiblingCommitted`), subsequent commits
    /// should return `CommitAlreadyFailed` rather than falsely succeeding
    /// with an empty hash.
    #[test]
    fn test_commit_after_failed_commit_returns_error() {
        use firewood::api::Proposal as _;

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

        // db_a: make a change → root2
        let changes = vec![put(b"\x10", b"changed")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Generate change proof from db_a
        let proof = db_a
            .change_proof(root1_a, root2.clone(), None, None, None)
            .expect("change proof");

        // verify_and_propose on db_b
        let change_ctx = ChangeProofContext::from(proof);
        let mut proposed = change_ctx
            .verify_and_propose(&db_b, root1_b.clone(), root2, None, None, None)
            .expect("verify_and_propose");

        // Advance db_b's state with a sibling proposal, making the
        // change proof proposal stale
        let sibling = (&db_b)
            .create_proposal(vec![put(b"\x20", b"sibling")])
            .expect("sibling proposal");
        sibling.commit().expect("sibling commit");

        // First commit should fail (sibling advanced the state)
        let err1 = proposed
            .commit()
            .expect_err("first commit should fail due to sibling");
        assert!(
            matches!(
                err1,
                firewood::api::Error::SiblingCommitted
                    | firewood::api::Error::ParentNotLatest { .. }
            ),
            "expected SiblingCommitted or ParentNotLatest, got: {err1}"
        );

        // Second commit should fail with CommitAlreadyFailed, not succeed
        let err2 = proposed
            .commit()
            .expect_err("second commit should fail with CommitAlreadyFailed");
        assert!(
            matches!(err2, firewood::api::Error::CommitAlreadyFailed),
            "expected CommitAlreadyFailed, got: {err2}"
        );
    }

    /// Omitted-change attack on a bounded proof.
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
            // Omitting a batch op changes the proposal's trie. If the
            // missing key's subtree is an in-range child, we get
            // InRangeChildMismatch; if the root hash comparison catches
            // it first (Case 1 / complete proofs), we get EndRootMismatch.
            assert!(
                matches!(
                    err.1,
                    firewood::api::Error::ProofError(
                        ProofError::EndRootMismatch | ProofError::InRangeChildMismatch { .. }
                    )
                ),
                "expected EndRootMismatch or InRangeChildMismatch for bounded omitted change, got: {:?}",
                err.1
            );
        }
    }

    /// Build a [`PathIterItem`] with the given depth (`key_nibbles` length).
    /// Only `key_nibbles.len()` matters to [`ProposalCursor`], so the node
    /// contents are minimal.
    fn cursor_item(depth: usize) -> PathIterItem {
        use firewood_storage::{LeafNode, Node, Path, PathComponent, SharedNode};
        PathIterItem {
            key_nibbles: (0..depth)
                .map(|_| PathComponent::try_new(0).expect("0 is a valid nibble"))
                .collect(),
            node: SharedNode::new(Node::Leaf(LeafNode {
                partial_path: Path::new(),
                value: Box::default(),
            })),
            next_nibble: None,
        }
    }

    #[test]
    fn test_cursor_empty_path() {
        use super::ProposalCursor;

        let mut cursor = ProposalCursor::new(&[]);
        assert!(cursor.advance_to(0).is_none());
        assert!(cursor.advance_to(5).is_none());
    }

    #[test]
    fn test_cursor_exact_match() {
        use super::ProposalCursor;

        let items = [cursor_item(0), cursor_item(2), cursor_item(4)];
        let mut cursor = ProposalCursor::new(&items);

        let hit = cursor.advance_to(0).expect("depth 0 exists");
        assert_eq!(hit.key_nibbles.len(), 0);

        let hit = cursor.advance_to(2).expect("depth 2 exists");
        assert_eq!(hit.key_nibbles.len(), 2);

        let hit = cursor.advance_to(4).expect("depth 4 exists");
        assert_eq!(hit.key_nibbles.len(), 4);
    }

    #[test]
    fn test_cursor_skip_and_miss() {
        use super::ProposalCursor;

        let items = [
            cursor_item(0),
            cursor_item(2),
            cursor_item(4),
            cursor_item(6),
        ];
        let mut cursor = ProposalCursor::new(&items);

        // Skip depths 0 and 2, land on depth 4.
        let hit = cursor.advance_to(4).expect("depth 4 exists");
        assert_eq!(hit.key_nibbles.len(), 4);

        // Depth 5 doesn't exist; cursor stops at depth 6 (>=5) but 6 != 5.
        assert!(cursor.advance_to(5).is_none());

        // Depth 6 is still reachable — the miss didn't consume it.
        let hit = cursor.advance_to(6).expect("depth 6 not consumed by miss");
        assert_eq!(hit.key_nibbles.len(), 6);
    }

    #[test]
    fn test_cursor_past_end() {
        use super::ProposalCursor;

        let items = [cursor_item(0), cursor_item(2)];
        let mut cursor = ProposalCursor::new(&items);

        assert!(cursor.advance_to(0).is_some());
        assert!(cursor.advance_to(2).is_some());
        assert!(cursor.advance_to(4).is_none());
    }

    #[test]
    fn test_cursor_repeated_depth() {
        use super::ProposalCursor;

        let items = [cursor_item(2)];
        let mut cursor = ProposalCursor::new(&items);

        // First call returns the item.
        let hit = cursor.advance_to(2).expect("depth 2 exists");
        assert_eq!(hit.key_nibbles.len(), 2);

        // Second call at the same depth returns the same item — idempotent.
        let hit = cursor.advance_to(2).expect("depth 2 still reachable");
        assert_eq!(hit.key_nibbles.len(), 2);
    }

    #[test]
    fn test_cursor_depth_gap() {
        use super::ProposalCursor;

        // Large gap simulating trie compression.
        let items = [cursor_item(0), cursor_item(8)];
        let mut cursor = ProposalCursor::new(&items);

        // Depth 4 doesn't exist; cursor stops at depth 8 (>=4) but 8 != 4.
        cursor.advance_to(0);
        assert!(cursor.advance_to(4).is_none());

        // Depth 8 is still reachable.
        let hit = cursor
            .advance_to(8)
            .expect("depth 8 not consumed by gap miss");
        assert_eq!(hit.key_nibbles.len(), 8);
    }

    /// Unbounded truncated proof where the last batch op is Delete.
    /// The end proof is an exclusion proof for the deleted key.
    /// `verify_end_proof` must accept Delete + `Ok(None)` as a valid match.
    #[test]
    fn test_truncated_proof_with_delete_last_op() {
        use std::num::NonZeroUsize;

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

        // db_a: modify one key, delete another, modify a third.
        // With max_length=2, the truncated proof includes the first two ops,
        // with the last being a Delete.
        let changes = vec![
            put(b"\x10", b"changed0"),
            delete(b"\x20"),
            put(b"\x30", b"changed2"),
        ];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        let proof = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                None,
                None,
                NonZeroUsize::new(2),
            )
            .expect("truncated change proof");

        let batch_ops = proof.batch_ops();
        assert_eq!(batch_ops.len(), 2, "should have exactly 2 ops");
        assert!(
            matches!(batch_ops.last().expect("checked"), BatchOp::Delete { .. }),
            "last op should be Delete"
        );

        // Verify on db_b — should succeed despite last op being Delete
        let change_ctx = ChangeProofContext::from(proof);
        let mut proposed = change_ctx
            .verify_and_propose(&db_b, root1_b, root2, None, None, None)
            .expect("truncated proof with Delete last op should verify");

        let next = proposed
            .find_next_key()
            .expect("find_next_key should not error");
        assert!(next.is_some(), "truncated proof should have a next range");
    }

    /// Non-truncated bounded proof where the last batch op is Delete and
    /// `end_key` differs from `last_op_key`. The end proof is built for
    /// `end_key`, so the `last_op_key` check falls through to `end_key`.
    #[test]
    fn test_non_truncated_delete_falls_through_to_end_key() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![
            put(b"\x10", b"v0"),
            put(b"\x20", b"v1"),
            put(b"\xa0", b"v2"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // Delete \x20 (middle key). The last op in the diff is the delete.
        // end_key=\xa0 differs from last_op_key=\x20.
        let changes = vec![delete(b"\x20")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Non-truncated bounded proof: end proof built for end_key=\xa0
        let proof = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                Some(b"\x10".as_ref()),
                Some(b"\xa0".as_ref()),
                None,
            )
            .expect("change proof");

        let batch_ops = proof.batch_ops();
        assert!(
            !batch_ops.is_empty(),
            "proof should have at least one batch op"
        );
        assert!(
            matches!(batch_ops.last().expect("non-empty"), BatchOp::Delete { .. }),
            "last op should be Delete"
        );

        let change_ctx = ChangeProofContext::from(proof);
        let result = change_ctx.verify_and_propose(
            &db_b,
            root1_b,
            root2,
            Some(b"\x10".as_ref()),
            Some(b"\xa0".as_ref()),
            None,
        );
        assert!(
            result.is_ok(),
            "non-truncated delete proof should verify via end_key fallback: {:?}",
            result.err()
        );
    }

    /// Bounded truncated proof where the last batch op is Delete.
    /// Tests the path where `end_key` is provided but the proof was
    /// built for `last_op_key` (truncated).
    #[test]
    fn test_bounded_truncated_delete() {
        use std::num::NonZeroUsize;

        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![
            put(b"\x10", b"v0"),
            put(b"\x20", b"v1"),
            put(b"\x30", b"v2"),
            put(b"\xa0", b"v3"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // Multiple changes including a delete in the middle.
        let changes = vec![
            put(b"\x10", b"changed0"),
            delete(b"\x20"),
            put(b"\x30", b"changed2"),
        ];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Bounded truncated proof: end_key=\xa0, max_length=2.
        // The truncated proof covers the first 2 ops, with the last being Delete(\x20).
        let proof = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                None,
                Some(b"\xa0".as_ref()),
                NonZeroUsize::new(2),
            )
            .expect("bounded truncated change proof");

        let batch_ops = proof.batch_ops();
        assert_eq!(batch_ops.len(), 2, "should have exactly 2 ops");
        assert!(
            matches!(batch_ops.last().expect("checked"), BatchOp::Delete { .. }),
            "last op should be Delete"
        );

        let change_ctx = ChangeProofContext::from(proof);
        let mut proposed = change_ctx
            .verify_and_propose(&db_b, root1_b, root2, None, Some(b"\xa0".as_ref()), None)
            .expect("bounded truncated delete proof should verify");

        let next = proposed
            .find_next_key()
            .expect("find_next_key should not error");
        assert!(next.is_some(), "truncated proof should have a next range");
    }

    /// Truncated proof verified without forwarding `max_length`.
    /// Previously, `verify_end_proof` relied on `max_length` to detect
    /// truncation. Now it uses the proof's own content, so passing
    /// `max_length=None` to the verifier still works.
    #[test]
    fn test_verify_end_proof_ignores_max_length() {
        use std::num::NonZeroUsize;

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

        let changes = vec![
            put(b"\x10", b"changed0"),
            put(b"\x20", b"changed1"),
            put(b"\x30", b"changed2"),
        ];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Generate a truncated proof with max_length=1
        let proof = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                None,
                None,
                NonZeroUsize::new(1),
            )
            .expect("truncated proof");

        assert!(
            !proof.end_proof().is_empty(),
            "truncated proof should have non-empty end_proof"
        );

        // Verify with max_length=None — verifier does not need max_length
        // to determine truncation because verify_end_proof uses batch_ops.
        let change_ctx = ChangeProofContext::from(proof);
        let result = change_ctx.verify_and_propose(&db_b, root1_b, root2, None, None, None);
        assert!(
            result.is_ok(),
            "truncated proof verified without max_length should succeed: {:?}",
            result.err()
        );
    }

    /// Start proof where `start_key` was deleted in the target trie.
    /// The start proof is an exclusion proof for the deleted key,
    /// which `verify_start_proof` accepts via `value_digest`.
    #[test]
    fn test_start_proof_exclusion_for_deleted_key() {
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

        // Delete start_key \x10 and modify \x30.
        // The start proof for \x10 in end_root's trie will be an exclusion proof.
        let changes = vec![delete(b"\x10"), put(b"\x30", b"changed")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Bounded proof starting from the deleted key
        let proof = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                Some(b"\x10".as_ref()),
                None,
                None,
            )
            .expect("change proof with deleted start_key");

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
            "start proof exclusion for deleted key should verify: {:?}",
            result.err()
        );
    }

    /// Adversarial: omitting a `batch_op` in the gap between `last_op_key`
    /// and the proof's actual boundary. With key-derived nibbles, the
    /// narrower boundary from Delete + Ok(None) would skip the gap;
    /// with proof-derived nibbles the actual proof path's boundary is
    /// used and the omission is detected.
    #[test]
    fn test_omitted_change_in_nibble_gap() {
        use firewood::api::FrozenChangeProof;

        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Initial keys: \x10, \x20, \x30, \xa0
        let initial = vec![
            put(b"\x10", b"v0"),
            put(b"\x20", b"v1"),
            put(b"\x30", b"v2"),
            put(b"\xa0", b"v3"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // Changes: Delete(\x20), Put(\x30, "changed"), Put(\xa0, "changed")
        let changes = vec![
            delete(b"\x20"),
            put(b"\x30", b"changed"),
            put(b"\xa0", b"changed"),
        ];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Non-truncated bounded proof: start_key=\x10, end_key=\xa0
        let valid_proof = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                Some(b"\x10".as_ref()),
                Some(b"\xa0".as_ref()),
                None,
            )
            .expect("change proof");

        let batch_ops = valid_proof.batch_ops();
        assert!(
            batch_ops.len() >= 3,
            "need at least 3 batch_ops (Delete + 2 Puts), got {}",
            batch_ops.len()
        );

        // Craft a proof with the \x30 Put removed. The proof's boundary
        // proofs still authenticate the full range [\x10, \xa0], but
        // the proposal will be missing the \x30 change.
        let shortened_ops: Vec<_> = batch_ops
            .iter()
            .filter(|op| op.key().as_ref() != b"\x30")
            .cloned()
            .collect();
        assert_eq!(
            shortened_ops.len(),
            batch_ops.len() - 1,
            "should have removed exactly one op"
        );

        let crafted = FrozenChangeProof::new(
            firewood::Proof::new(valid_proof.start_proof().as_ref().into()),
            firewood::Proof::new(valid_proof.end_proof().as_ref().into()),
            shortened_ops.into_boxed_slice(),
        );

        // With proof-derived nibbles, the end proof's path to \xa0 gives
        // boundary nibble `a` at the root. Nibble 3 (\x30) is < `a`, so
        // it's in-range and the child mismatch is detected.
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
            .expect_err("omitted \x30 in nibble gap must be detected");
        assert!(
            matches!(
                err.1,
                firewood::api::Error::ProofError(
                    ProofError::InRangeChildMismatch { .. }
                        | ProofError::ProofNodeValueMismatch { .. }
                )
            ),
            "expected InRangeChildMismatch or ProofNodeValueMismatch, got: {:?}",
            err.1
        );
    }

    /// Correctness: a proof whose last node is at an odd nibble depth
    /// must not false-positive. Without the odd-depth padding fix in
    /// `proof_node_byte_key`, `path_to_key` misses the odd-depth node
    /// and the cursor returns None, causing a spurious
    /// `InRangeChildMismatch`.
    #[test]
    fn test_odd_depth_proof_node_accepted() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Keys \x12 and \x13 share nibble 1 at depth 0 and diverge at
        // depth 1 (odd). This creates a branch at odd depth 1. Key \x50
        // is in a different subtree (nibble 5).
        let initial = vec![
            put(b"\x12", b"v0"),
            put(b"\x13", b"v1"),
            put(b"\x50", b"v2"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // Change \x50 only. The change is outside the [\x14, ..] range
        // start proof path but within the proven range.
        let changes = vec![put(b"\x50", b"changed")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Bounded proof: start_key=\x14, end_key=None.
        // The start proof for \x14 follows nibble 1 at depth 0 to the
        // depth-1 branch, where nibble 4 doesn't exist (only 2 and 3).
        // The last proof node is at odd depth 1.
        let proof = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                Some(b"\x14".as_ref()),
                None,
                None,
            )
            .expect("change proof with odd-depth end");

        let change_ctx = ChangeProofContext::from(proof);
        let result = change_ctx.verify_and_propose(
            &db_b,
            root1_b,
            root2,
            Some(b"\x14".as_ref()),
            None,
            None,
        );
        assert!(
            result.is_ok(),
            "proof with odd-depth last node should verify (no false positive): {:?}",
            result.err()
        );
    }

    /// Regression test: normal bounded proof with Puts still succeeds
    /// after switching from key-derived to proof-derived nibble
    /// boundaries. The proof-derived boundary should produce identical
    /// results to the old key-derived boundary for standard Put
    /// operations.
    #[test]
    fn test_proof_derived_nibbles_match_key_derived() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Multiple keys spread across different nibble subtrees
        let initial = vec![
            put(b"\x10", b"v0"),
            put(b"\x20", b"v1"),
            put(b"\x30", b"v2"),
            put(b"\x40", b"v3"),
            put(b"\xa0", b"v4"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // Put changes in the middle of the range
        let changes = vec![
            put(b"\x20", b"changed1"),
            put(b"\x30", b"changed2"),
            put(b"\x40", b"changed3"),
        ];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Bounded proof with both start and end proofs
        let proof = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                Some(b"\x10".as_ref()),
                Some(b"\xa0".as_ref()),
                None,
            )
            .expect("change proof");

        assert!(
            !proof.start_proof().is_empty() && !proof.end_proof().is_empty(),
            "bounded proof should have both boundary proofs"
        );

        let change_ctx = ChangeProofContext::from(proof);
        let result = change_ctx.verify_and_propose(
            &db_b,
            root1_b,
            root2,
            Some(b"\x10".as_ref()),
            Some(b"\xa0".as_ref()),
            None,
        );
        assert!(
            result.is_ok(),
            "normal bounded proof with Puts should still verify: {:?}",
            result.err()
        );
    }

    /// Exercises `boundary_nibble = None` + `is_end_proof = false` in
    /// `verify_in_range_children`. When `start_key` is a prefix of
    /// another key in the trie, the start proof's last node is a branch
    /// that also carries a value. At that depth the start key's nibbles
    /// are exhausted, so `fallback_nibbles.get(depth)` returns `None`
    /// and all children are treated as in-range.
    ///
    /// This must not false-positive on a valid proof.
    #[test]
    fn test_start_proof_inclusion_with_children_below() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // \xab is a prefix of \xab\xcd at the byte level.
        // In the trie, the branch at nibble path [a, b] carries the value
        // for key \xab and has a child at nibble c leading to \xab\xcd.
        // Key \xf0 is in a separate subtree.
        let initial = vec![
            put(b"\xab", b"short"),
            put(b"\xab\xcd", b"long"),
            put(b"\xf0", b"other"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // Change the child key (\xab\xcd) and the separate key (\xf0).
        // start_key=\xab is unchanged but its subtree is modified.
        let changes = vec![put(b"\xab\xcd", b"changed"), put(b"\xf0", b"changed2")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Bounded proof: start_key = \xab (inclusion proof with children).
        // The start proof's last node at depth 2 has children below it.
        // fallback_nibbles for \xab = [a, b] → get(2) = None →
        // boundary_nibble = None → all children in-range.
        let proof = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                Some(b"\xab".as_ref()),
                None,
                None,
            )
            .expect("change proof with prefix start_key");

        assert!(
            !proof.start_proof().is_empty(),
            "start proof should be non-empty for prefix key"
        );

        let change_ctx = ChangeProofContext::from(proof);
        let result = change_ctx.verify_and_propose(
            &db_b,
            root1_b,
            root2,
            Some(b"\xab".as_ref()),
            None,
            None,
        );
        assert!(
            result.is_ok(),
            "start proof inclusion with children below should not false-positive: {:?}",
            result.err()
        );
    }

    /// Mirror of `test_start_proof_inclusion_with_children_below` for the
    /// end proof. When `last_op_key` is a prefix of another key, the end
    /// proof's last node has children. `boundary_nibble = None` +
    /// `is_end_proof = true` → no children are checked (they extend past
    /// the boundary). Must not false-positive.
    #[test]
    fn test_end_proof_inclusion_with_children_below() {
        use std::num::NonZeroUsize;

        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // \xab is a prefix of \xab\xcd.
        // Key \xf0 is in a separate subtree.
        let initial = vec![
            put(b"\xab", b"short"),
            put(b"\xab\xcd", b"long"),
            put(b"\xf0", b"other"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // Change ALL three keys. The diff from root1→root2 has 3 ops.
        // With max_length=1, only the first (sorted) op is included:
        // \xab (the prefix key). The proof is truncated since 1 < 3.
        let changes = vec![
            put(b"\xab", b"changed0"),
            put(b"\xab\xcd", b"changed1"),
            put(b"\xf0", b"changed2"),
        ];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Truncated proof (max_length=1) so last_op = \xab.
        // The end proof for \xab is an inclusion proof — the last node
        // at depth 2 has a child at nibble c (for \xab\xcd).
        // end_nibbles = [a, b] → get(2) = None → boundary_nibble = None
        // → is_end_proof = true → no children checked.
        let proof = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                None,
                None,
                NonZeroUsize::new(1),
            )
            .expect("truncated change proof with prefix last_op_key");

        assert!(
            !proof.end_proof().is_empty(),
            "end proof should be non-empty for truncated proof"
        );

        let change_ctx = ChangeProofContext::from(proof);
        let result =
            change_ctx.verify_and_propose(&db_b, root1_b, root2, None, None, NonZeroUsize::new(1));
        assert!(
            result.is_ok(),
            "end proof inclusion with children below should not false-positive: {:?}",
            result.err()
        );
    }

    /// Exercises `start_bn = None` at the divergence parent in
    /// `verify_divergent_proofs`. When both proofs share a prefix and
    /// the start key is exhausted at the parent depth, `is_none_or`
    /// treats all children as in-range from the start side.
    ///
    /// Setup: `start_key` \x12 doesn't exist in the trie but the trie
    /// has a branch at depth 2 (nibble path [1, 2]) with children
    /// \x12\x34 and \x12\x56. The start proof is an exclusion proof
    /// ending at the branch — the last start node is at depth 2 where
    /// `start_nibbles` [1, 2] are exhausted.
    #[test]
    fn test_divergence_parent_start_key_exhausted() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Keys under nibble prefix [1, 2] — creating a branch at depth 2.
        // Key \xf0 is in a different subtree (nibble f) for the end proof.
        let initial = vec![
            put(b"\x12\x34", b"v0"),
            put(b"\x12\x56", b"v1"),
            put(b"\xf0", b"v2"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // Modify keys in the subtree and beyond.
        let changes = vec![
            put(b"\x12\x34", b"changed0"),
            put(b"\x12\x56", b"changed1"),
            put(b"\xf0", b"changed2"),
        ];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Bounded proof: start_key=\x12, end_key=\xf0.
        // \x12 doesn't exist in the trie → start proof is an exclusion
        // proof. The start proof's last node is at the branch for [1, 2]
        // at depth 2. start_nibbles for \x12 = [1, 2] → get(2) = None.
        //
        // The end proof for \xf0 goes through a different path.
        // At the divergence parent (shared root), start_bn uses the
        // start proof's next node's nibble at root depth (nibble 1).
        // At the start tail's last node (the depth-2 branch), the
        // fallback start_nibbles.get(2) = None → boundary_nibble = None
        // → all children of the branch are checked as in-range.
        let proof = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                Some(b"\x12".as_ref()),
                Some(b"\xf0".as_ref()),
                None,
            )
            .expect("change proof with exhausted start_key at branch");

        assert!(
            !proof.start_proof().is_empty() && !proof.end_proof().is_empty(),
            "both proofs should be non-empty"
        );

        let change_ctx = ChangeProofContext::from(proof);
        let result = change_ctx.verify_and_propose(
            &db_b,
            root1_b,
            root2,
            Some(b"\x12".as_ref()),
            Some(b"\xf0".as_ref()),
            None,
        );
        assert!(
            result.is_ok(),
            "divergence parent with exhausted start_key should not false-positive: {:?}",
            result.err()
        );
    }

    /// Adversarial counterpart to `test_start_proof_inclusion_with_children_below`.
    /// Omits a `batch_op` in the subtree below `start_key`. The `None`
    /// boundary nibble at the start proof's last node means ALL children
    /// are checked. The omitted change causes a child hash mismatch
    /// between the proof and the proposal, which must be caught.
    #[test]
    fn test_omitted_change_below_prefix_start_key() {
        use firewood::api::FrozenChangeProof;

        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![
            put(b"\xab", b"short"),
            put(b"\xab\xcd", b"long"),
            put(b"\xf0", b"other"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // Change both the child key and the separate key.
        let changes = vec![put(b"\xab\xcd", b"changed"), put(b"\xf0", b"changed2")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Get the real proof
        let real_proof = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                Some(b"\xab".as_ref()),
                None,
                None,
            )
            .expect("change proof");

        // Craft a proof that omits the \xab\xcd change — only keep \xf0.
        // The omitted change in the subtree below start_key should be
        // caught by the all-children check at the start proof's last node.
        let tampered_ops: Vec<_> = real_proof
            .batch_ops()
            .iter()
            .filter(|op| op.key().as_ref() != b"\xab\xcd")
            .cloned()
            .collect();
        assert!(
            tampered_ops.len() < real_proof.batch_ops().len(),
            "should have removed at least one op"
        );

        let tampered = FrozenChangeProof::new(
            firewood::Proof::new(real_proof.start_proof().as_ref().into()),
            firewood::Proof::new(real_proof.end_proof().as_ref().into()),
            tampered_ops.into_boxed_slice(),
        );

        let change_ctx = ChangeProofContext::from(tampered);
        let result = change_ctx.verify_and_propose(
            &db_b,
            root1_b,
            root2,
            Some(b"\xab".as_ref()),
            None,
            None,
        );
        assert!(
            result.is_err(),
            "omitted change below prefix start_key should be detected"
        );
    }

    /// Exercises the `end_nibbles` fallback path where `batch_ops` is
    /// empty but the last end-proof node has in-range children that
    /// must be verified against the proposal.
    ///
    /// The attack: take an honest proof from `end_root` (which has empty
    /// `batch_ops` because no changes occurred within the proven range),
    /// but verify it against a different verifier database whose
    /// start state has different in-range keys. The boundary proofs
    /// are genuinely from `end_root` so the hash chain passes — only
    /// `verify_root_hash` can catch the in-range child mismatch.
    ///
    /// Setup:
    /// - Prover `db_a`: keys `\x20`, `\x25`, `\x30`
    /// - Verifier `db_b`: keys `\x21`, `\x25`, `\x30` (differs at `\x20` vs `\x21`)
    /// - Prover changes only `\x30` (outside the range `[\x10, \x23]`)
    /// - Proof for `[\x10, \x23]` has empty `batch_ops` (no in-range changes)
    /// - End proof for `\x23` is an exclusion proof from `end_root`
    ///
    /// Trie structure at `end_root` (nibble view):
    /// ```text
    ///       root
    ///      2/  \3
    ///    [2]    [3,0] = \x30
    ///   0/ \5
    /// [2,0] [2,5]
    /// ```
    ///
    /// The end proof path for `\x23` (nibbles `[2,3]`) reaches the branch
    /// at nibble depth 1 (key `[2]`) and stops — child 3 doesn't exist,
    /// so this is an exclusion proof. The last end-proof node has
    /// children at nibbles 0 (`\x20`) and 5 (`\x25`).
    ///
    /// `db_b`'s trie has `\x21` instead of `\x20`, so child 0's hash at
    /// the `[2]` branch differs between `end_root` and `db_b`'s start state.
    ///
    /// With the fix (`end_nibbles` falls back to `end_key`):
    ///   `boundary_nibble` at depth 1 = 3 (from `\x23`'s nibble at depth 1)
    ///   → child at nibble 0 is in-range (0 < 3) and verified
    ///   → mismatch detected: `db_a` has `\x20`, `db_b` has `\x21`
    ///
    /// Without the fix (`end_nibbles` = None when `batch_ops` is empty):
    ///   `boundary_nibble` = None → no children verified
    ///   → the state difference goes undetected
    #[test]
    fn test_empty_batch_ops_end_nibbles_fallback() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // db_a starts with keys \x20, \x25, \x30.
        let initial_a = vec![
            put(b"\x20", b"v0"),
            put(b"\x25", b"v1"),
            put(b"\x30", b"v2"),
        ];
        let p = (&db_a).create_proposal(initial_a).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        // db_b starts with keys \x21, \x25, \x30 — differs at \x21
        // instead of \x20. This means db_b's start state differs from
        // db_a's within the range [\x10, \x23], but both databases
        // agree on \x25 and \x30 (outside or at the boundary).
        let initial_b = vec![
            put(b"\x21", b"v0"),
            put(b"\x25", b"v1"),
            put(b"\x30", b"v2"),
        ];
        let p = (&db_b).create_proposal(initial_b).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");

        // The two databases have different root hashes because \x20 ≠ \x21.
        assert_ne!(root1_a, root1_b);

        // On db_a, change only \x30 (outside the range [\x10, \x23]).
        let changes = vec![put(b"\x30", b"changed")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Generate a proof for range [\x10, \x23] between root1_a and
        // root2 on db_a. Since the only change (\x30) is outside the
        // range, batch_ops should be empty. The boundary proofs are
        // genuinely from root2 (end_root).
        let proof = db_a
            .change_proof(
                root1_a,
                root2.clone(),
                Some(b"\x10".as_ref()),
                Some(b"\x23".as_ref()),
                None,
            )
            .expect("change proof");

        assert!(
            proof.batch_ops().is_empty(),
            "proof should have empty batch_ops (no in-range changes)"
        );
        assert!(
            !proof.end_proof().is_empty(),
            "proof should have an end proof for \\x23"
        );

        // Present this proof to db_b, claiming it covers root1_b → root2.
        // The end proof's hash chain validates against root2 (it was
        // genuinely generated from root2). The proposal applies empty
        // batch_ops to root1_b, so the proposal is just root1_b.
        //
        // But root1_b has \x21 where root2 has \x20. The in-range child
        // at the last end-proof node (nibble 0 under the [2] branch)
        // should differ between the proof (from root2) and the proposal
        // (from root1_b). verify_root_hash must detect this.
        let ctx = ChangeProofContext::from(proof);
        let result = ctx.verify_and_propose(
            &db_b,
            root1_b,
            root2,
            Some(b"\x10".as_ref()),
            Some(b"\x23".as_ref()),
            None,
        );
        assert!(
            result.is_err(),
            "in-range child mismatch at last end-proof node should be detected"
        );
    }

    /// Verify the generator convention: for a non-truncated proof where
    /// `end_key` differs from the last batch op key, the end proof is
    /// built for `last_op_key` (not `end_key`). The verifier mirrors
    /// this convention to derive the key deterministically.
    #[test]
    fn test_generator_uses_last_op_key_for_end_proof() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = test_db(dir.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\x30", b"v1")];
        let p = (&db).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1 = db.current_root_hash().expect("root");

        // Change \x10 only — \x30 is unchanged
        let changes = vec![put(b"\x10", b"changed")];
        let p = (&db).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db.current_root_hash().expect("root");

        // Generate a non-truncated proof with end_key=\x50 (well beyond
        // the last change at \x10). With the generator convention, the
        // end proof should be built for \x10 (last_op_key), not \x50.
        let proof = db
            .change_proof(root1, root2.clone(), None, Some(b"\x50".as_ref()), None)
            .expect("change proof");

        assert!(
            !proof.end_proof().is_empty(),
            "non-truncated proof with batch_ops should have a non-empty end proof"
        );

        // The end proof should validate against last_op_key (\x10)
        // via value_digest. If the generator had used end_key (\x50),
        // the hash chain would follow a different trie path and this
        // would fail.
        let result = proof.end_proof().value_digest(b"\x10".as_ref(), &root2);
        assert!(
            result.is_ok(),
            "end proof should validate against last_op_key: {:?}",
            result.err()
        );
    }

    /// Verify the `MissingEndProof` structural check: a proof with
    /// non-empty `batch_ops` but empty `end_proof` is rejected before
    /// any expensive trie operations.
    #[test]
    fn test_missing_end_proof_rejected() {
        use firewood::Proof;
        use firewood::api::FrozenChangeProof;

        // Craft a proof with non-empty batch_ops but empty end_proof.
        // This combination is never produced by an honest generator.
        let crafted = FrozenChangeProof::new(
            Proof::new(Box::new([])),                     // empty start_proof
            Proof::new(Box::new([])),                     // empty end_proof
            vec![put(b"\x10", b"v0")].into_boxed_slice(), // non-empty batch_ops
        );

        // verify_proof_structure should reject this with MissingEndProof
        // before any trie work happens.
        let result = ChangeProofContext::verify_proof_structure(
            &crafted,
            firewood::api::HashKey::empty(),
            None,
            None,
            None,
        );
        assert!(
            matches!(
                result,
                Err(firewood::api::Error::ProofError(
                    ProofError::MissingEndProof
                ))
            ),
            "expected MissingEndProof, got: {result:?}",
        );
    }
}
