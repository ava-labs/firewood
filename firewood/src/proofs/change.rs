// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Verification logic for change proofs.
//!
//! This module provides [`verify_change_proof`], the core cryptographic
//! verification function for change proofs. The FFI layer is responsible
//! for applying `batch_ops` to the start state (producing the `applied`
//! trie) and then calling this function.

use std::cmp::Ordering;
use std::num::NonZeroUsize;

use firewood_storage::{
    Children, HashType, HashedNodeReader, NibblesIterator, PathComponent, PathIterItem, TrieHash,
    TrieReader,
};

use crate::{
    api::{self, BatchOp, FrozenChangeProof},
    merkle::Merkle,
    proofs::types::{ProofError, ProofNode},
};

/// Internal struct capturing the verified boundary keys.
struct CheckedBoundaries {
    start: Option<Box<[u8]>>,
    end: Option<Box<[u8]>>,
    /// The key the end proof was actually validated against (last batch op key
    /// for truncated proofs, `end_key` otherwise).
    resolved_end: Option<Box<[u8]>>,
}

fn key_to_nibbles(key: &[u8]) -> Vec<PathComponent> {
    NibblesIterator::new(key)
        .filter_map(PathComponent::try_new)
        .collect()
}

/// Forward-only cursor over a proposal path sorted by depth.
struct ProposalCursor<'a> {
    path: &'a [PathIterItem],
    pos: usize,
}

impl<'a> ProposalCursor<'a> {
    const fn new(path: &'a [PathIterItem]) -> Self {
        Self { path, pos: 0 }
    }

    /// Advance past nodes shallower than `depth`; return the node at exactly `depth` if present.
    fn advance_to(&mut self, depth: usize) -> Option<&'a PathIterItem> {
        // wrapping_add is safe since usize cannot overflow
        self.pos = self.pos.wrapping_add(
            self.path
                .get(self.pos..)
                .unwrap_or_default()
                .partition_point(|item| item.key_nibbles.len() < depth),
        );
        self.path
            .get(self.pos)
            .filter(|item| item.key_nibbles.len() == depth)
    }
}

/// Verify that a proof node's value matches the proposal's value at the same path.
fn verify_proof_node_value(
    node: &ProofNode,
    lookup_item: Option<&PathIterItem>,
) -> Result<(), ProofError> {
    let depth = node.key.len();
    // Only even-nibble-depth nodes can carry values.
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

/// Verify that in-range children of each proof node match the applied trie.
///
/// `boundary_nibbles` is indexed by nibble depth. `side` is the `Ordering` that
/// a nibble must compare as against the boundary nibble to be considered in-range:
/// `Ordering::Greater` for a start boundary, `Ordering::Less` for an end boundary.
fn verify_in_range_children(
    nodes: &[ProofNode],
    path: &[PathIterItem],
    boundary_nibbles: &[PathComponent],
    side: std::cmp::Ordering,
) -> Result<(), ProofError> {
    let mut cursor = ProposalCursor::new(path);
    for node in nodes {
        let depth = node.key.len();
        let lookup_item = cursor.advance_to(depth);
        verify_proof_node_value(node, lookup_item)?;

        // None when the boundary key is shorter than this depth → all children are in-range.
        let boundary_nibble = boundary_nibbles.get(depth).copied();

        let proposal_children: Children<Option<HashType>> = lookup_item
            .and_then(|item| item.node.as_branch().map(|b| b.children_hashes()))
            .unwrap_or_default();

        for nibble in PathComponent::ALL {
            let in_range = boundary_nibble.is_none_or(|bn| nibble.cmp(&bn) == side);
            if in_range && node.child_hashes[nibble] != proposal_children[nibble] {
                eprintln!(
                    "InRangeChildMismatch at depth={depth} nibble={nibble:?}: proof={:?} applied={:?}",
                    node.child_hashes[nibble], proposal_children[nibble]
                );
                return Err(ProofError::InRangeChildMismatch { depth });
            }
        }
    }
    Ok(())
}

/// Walk the trie path from root to `key`, returning each node.
/// Returns an empty Vec when `key` is None (no boundary on that side).
fn get_path<T: TrieReader>(
    nodestore: &T,
    key: Option<&[u8]>,
) -> Result<Vec<PathIterItem>, api::Error> {
    let Some(key) = key else {
        return Ok(Vec::new());
    };
    Merkle::from(nodestore)
        .path_iter(key)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(api::Error::from)
}

/// Verify structural properties and boundary proofs of the change proof.
///
/// Returns a [`CheckedBoundaries`] capturing the resolved keys.
fn verify_proof_structure(
    proof: &FrozenChangeProof,
    end_root: &TrieHash,
    start_key: Option<&[u8]>,
    end_key: Option<&[u8]>,
    max_length: Option<NonZeroUsize>,
) -> Result<CheckedBoundaries, api::Error> {
    let batch_ops = proof.batch_ops();

    // Reject inverted ranges.
    if let (Some(start), Some(end)) = (start_key, end_key)
        && start.cmp(end) == Ordering::Greater
    {
        return Err(api::Error::InvalidRange {
            start_key: start.to_vec().into(),
            end_key: end.to_vec().into(),
        });
    }

    // Reject DeleteRange operations.
    if batch_ops
        .iter()
        .any(|op| matches!(op, BatchOp::DeleteRange { .. }))
    {
        return Err(api::Error::ProofError(ProofError::UnsupportedDeleteRange));
    }

    // Reject proofs larger than max_length.
    if let Some(max_length) = max_length
        && batch_ops.len() > max_length.into()
    {
        return Err(api::Error::ProofError(
            ProofError::ProofIsLargerThanMaxLength,
        ));
    }

    // Verify keys are sorted and unique.
    if !batch_ops
        .iter()
        .is_sorted_by(|a, b| b.key().cmp(a.key()) == Ordering::Greater)
    {
        return Err(api::Error::ProofError(ProofError::ChangeProofKeysNotSorted));
    }

    // start_key must not exceed the first batch op key.
    if let (Some(start_key), Some(first_key)) = (start_key, batch_ops.first())
        && start_key.cmp(first_key.key()) == Ordering::Greater
    {
        return Err(api::Error::ProofError(
            ProofError::StartKeyLargerThanFirstKey,
        ));
    }

    // end_key must not be less than the last batch op key.
    if let (Some(end_key), Some(last_key)) = (end_key, batch_ops.last())
        && end_key.cmp(last_key.key()) == Ordering::Less
    {
        return Err(api::Error::ProofError(ProofError::EndKeyLessThanLastKey));
    }

    // Non-empty batch_ops require at least one boundary proof (unless this is a
    // complete proof with no key bounds).
    if !batch_ops.is_empty()
        && proof.start_proof().is_empty()
        && proof.end_proof().is_empty()
        && (start_key.is_some() || end_key.is_some())
    {
        return Err(api::Error::ProofError(ProofError::MissingBoundaryProof));
    }

    // Reject non-empty end_proof when there is no end_key and no batch_ops.
    if end_key.is_none() && batch_ops.is_empty() && !proof.end_proof().is_empty() {
        return Err(api::Error::ProofError(ProofError::UnexpectedEndProof));
    }

    // Verify boundary proofs against end_root.
    verify_start_proof(proof, start_key, end_root)?;
    let resolved_end_key = verify_end_proof(proof, end_key, end_root, max_length)?;

    Ok(CheckedBoundaries {
        start: start_key.map(Box::from),
        end: end_key.map(Box::from),
        resolved_end: resolved_end_key,
    })
}

/// Verify the start boundary proof against `end_root`.
fn verify_start_proof(
    proof: &FrozenChangeProof,
    start_key: Option<&[u8]>,
    end_root: &TrieHash,
) -> Result<(), api::Error> {
    if proof.start_proof().is_empty() {
        return Ok(());
    }
    let Some(start_key) = start_key else {
        return Err(api::Error::ProofError(
            ProofError::BoundaryProofUnverifiable,
        ));
    };
    proof.start_proof().value_digest(start_key, end_root)?;
    Ok(())
}

/// Verify the end boundary proof against `end_root`.
///
/// Returns the key the end proof was validated against, or `None` if the
/// end proof is empty (range reaches end of keyspace).
///
/// When `batch_ops.len() >= max_length`, the proof may be truncated and the
/// generator used the last batch op key. Since the verifier cannot distinguish
/// truncated from non-truncated at exactly `max_length`, we try the last batch
/// op key first and fall back to `end_key`.
fn verify_end_proof(
    proof: &FrozenChangeProof,
    end_key: Option<&[u8]>,
    end_root: &TrieHash,
    max_length: Option<NonZeroUsize>,
) -> Result<Option<Box<[u8]>>, api::Error> {
    if proof.end_proof().is_empty() {
        return Ok(None);
    }

    let batch_ops = proof.batch_ops();
    let potentially_truncated = max_length.is_some_and(|max| batch_ops.len() >= max.get());

    // Try 1: potentially truncated — validate against last batch op key.
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

    // Try 2: non-truncated — validate against requested end_key.
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

    Err(api::Error::ProofError(
        ProofError::BoundaryProofUnverifiable,
    ))
}

/// Case 2c: both start and end proofs are present. Finds the depth where they diverge,
/// verifies shared-prefix node values, in-range children at the divergence parent, and
/// the independent tails below the divergence point.
fn verify_dual_proofs(
    start_nodes: &[ProofNode],
    end_nodes: &[ProofNode],
    start_path: &[PathIterItem],
    end_path: &[PathIterItem],
    start_nibbles: Option<&[PathComponent]>,
    end_nibbles: Option<&[PathComponent]>,
) -> Result<(), api::Error> {
    let mut start_cursor = ProposalCursor::new(start_path);
    let mut start_rest = start_nodes;
    let mut end_rest = end_nodes;

    // Walk the shared prefix: verify values at each node common to both paths.
    // The last shared node is the divergence parent.
    let mut parent: Option<&ProofNode> = None;
    loop {
        match (start_rest, end_rest) {
            ([s, new_start @ ..], [e, new_end @ ..]) if s.key == e.key => {
                let item = start_cursor.advance_to(s.key.len());
                verify_proof_node_value(s, item)?;
                parent = Some(s);
                start_rest = new_start;
                end_rest = new_end;
            }
            _ => break,
        }
    }

    let Some(parent) = parent else {
        return Err(ProofError::BoundaryProofsDivergeAtRoot.into());
    };

    // Verify in-range children at the divergence parent.
    let parent_depth = parent.key.len();
    let start_bn = start_nibbles.and_then(|sn| sn.get(parent_depth).copied());
    let end_bn = end_nibbles.and_then(|en| en.get(parent_depth).copied());

    let proposal_children = start_cursor
        .advance_to(parent_depth)
        .and_then(|i| i.node.as_branch().map(|b| b.children_hashes()))
        .unwrap_or_default();
    for nibble in PathComponent::ALL {
        let after_start = start_bn.is_none_or(|s| nibble > s);
        let before_end = end_bn.is_none_or(|e| nibble < e);
        if after_start && before_end && parent.child_hashes[nibble] != proposal_children[nibble] {
            eprintln!(
                "InRangeChildMismatch at depth={parent_depth} nibble={nibble:?}: proof={:?} applied={:?}",
                parent.child_hashes[nibble], proposal_children[nibble]
            );
            return Err(ProofError::InRangeChildMismatch {
                depth: parent_depth,
            }
            .into());
        }
    }

    // Walk the remaining tails below the divergence point.
    let start_tail = start_rest;
    let end_tail = end_rest;
    if !start_tail.is_empty() {
        verify_in_range_children(
            start_tail,
            start_path,
            start_nibbles.unwrap_or(&[]),
            std::cmp::Ordering::Greater,
        )?;
    }
    if !end_tail.is_empty() {
        verify_in_range_children(
            end_tail,
            end_path,
            end_nibbles.unwrap_or(&[]),
            std::cmp::Ordering::Less,
        )?;
    }
    Ok(())
}

/// Verify boundary proof nodes against the applied trie using in-range child comparison.
///
/// The proof's hash chain to `end_root` is already verified by
/// `verify_proof_structure`. If every in-range child hash matches the
/// applied trie's, the substitution in the rehash approach would have been
/// a no-op — same verification, less work.
fn verify_root_hash<T: HashedNodeReader>(
    proof: &FrozenChangeProof,
    boundaries: &CheckedBoundaries,
    end_root: &TrieHash,
    applied: &T,
) -> Result<(), api::Error> {
    let start_nodes: &[ProofNode] = proof.start_proof().as_ref();
    let end_nodes: &[ProofNode] = proof.end_proof().as_ref();

    // Case 1: Both proofs empty — complete proof covering the entire keyspace.
    // Compare the applied trie's root hash directly against end_root.
    if start_nodes.is_empty() && end_nodes.is_empty() {
        if applied.root_hash().as_ref() != Some(end_root) {
            return Err(api::Error::ProofError(ProofError::EndRootMismatch));
        }
        return Ok(());
    }

    let start_nibbles = boundaries.start.as_deref().map(key_to_nibbles);
    let effective_end = boundaries
        .resolved_end
        .as_deref()
        .or(boundaries.end.as_deref());
    let end_nibbles = effective_end.map(key_to_nibbles);

    let start_path = get_path(applied, boundaries.start.as_deref())?;
    let end_path = get_path(applied, effective_end)?;

    if start_nodes.is_empty() {
        // Case 2a: Only end proof (first sync round, start_key=None).
        verify_in_range_children(
            end_nodes,
            &end_path,
            end_nibbles.as_deref().unwrap_or(&[]),
            std::cmp::Ordering::Less,
        )?;
    } else if end_nodes.is_empty() {
        // Case 2b: Only start proof (last sync round, end of keyspace).
        verify_in_range_children(
            start_nodes,
            &start_path,
            start_nibbles.as_deref().unwrap_or(&[]),
            std::cmp::Ordering::Greater,
        )?;
    } else {
        verify_dual_proofs(
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

/// Verify a change proof against an already-applied trie.
///
/// # Parameters
///
/// * `proof` — the change proof to verify.
/// * `applied` — the trie produced by applying `proof.batch_ops()` to the start state.
/// * `end_root` — the expected root hash of the end state.
/// * `start_key` / `end_key` — the key bounds that were requested.
/// * `max_length` — the maximum number of batch ops the generator was allowed to include.
///
/// # Errors
///
/// Returns [`api::Error::ProofError`] if structural validation, boundary proof
/// verification, or root hash verification fails.
pub fn verify_change_proof<T: HashedNodeReader>(
    proof: &FrozenChangeProof,
    applied: &T,
    end_root: &TrieHash,
    start_key: Option<&[u8]>,
    end_key: Option<&[u8]>,
    max_length: Option<NonZeroUsize>,
) -> Result<(), api::Error> {
    let boundaries = verify_proof_structure(proof, end_root, start_key, end_key, max_length)?;
    verify_root_hash(proof, &boundaries, end_root, applied)?;
    Ok(())
}
