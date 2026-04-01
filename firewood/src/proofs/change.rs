// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Change proofs for Merkle tries.
//!
//! This module provides the [`ChangeProof`] type, which demonstrates that applying a set of
//! [`BatchOp`]s to a Merkle trie with a given start root hash produces a trie with a given
//! end root hash.
//!
//! # Overview
//!
//! A change proof consists of three components:
//!
//! 1. **Start proof**: A Merkle proof establishing that the smallest key does or doesn't
//!    exist in the start trie.
//!
//! 2. **End proof**: A Merkle proof establishing that the largest key does or doesn't
//!    exist in the start trie.
//!
//! 3. **Batch operations**: The actual [`BatchOp`]s (puts and deletes) that transform
//!    the start trie into the end trie, in lexicographic key order.
//!
//! # Use Cases
//!
//! Change proofs are particularly valuable in blockchain contexts for:
//!
//! - **State synchronization**: Nodes can efficiently sync state by exchanging and
//!   verifying change proofs between revisions.
//! - **Incremental verification**: Verifiers can confirm that a set of changes is
//!   consistent with known root hashes without replaying all transactions.

use std::fmt::Debug;
use std::num::NonZeroUsize;

use firewood_storage::PathComponentSliceExt;

use super::types::{Proof, ProofCollection, ProofError, ProofNode};
use crate::api::{self, BatchOp, FrozenChangeProof, HashKey};

/// A change proof can demonstrate that by applying the provided array of `BatchOp`s to a Merkle
/// trie with given start root hash, the resulting trie will have the given end root hash. It
/// consists of the following:
/// - A start proof: proves that the smallest key does/doesn't exist
/// - An end proof: proves the the largest key does/doesn't exist
/// - The actual `BatchOp`s that specify the difference between the start and end tries.
#[derive(Debug)]
pub struct ChangeProof<K: AsRef<[u8]> + Debug, V: AsRef<[u8]> + Debug, H> {
    start_proof: Proof<H>,
    end_proof: Proof<H>,
    batch_ops: Box<[BatchOp<K, V>]>,
}

impl<K, V, H> ChangeProof<K, V, H>
where
    K: AsRef<[u8]> + Debug,
    V: AsRef<[u8]> + Debug,
    H: ProofCollection,
{
    /// Create a new change proof with the given start and end proofs
    /// and the `BatchOp`s that are included in the proof.
    #[must_use]
    pub const fn new(
        start_proof: Proof<H>,
        end_proof: Proof<H>,
        key_values: Box<[BatchOp<K, V>]>,
    ) -> Self {
        Self {
            start_proof,
            end_proof,
            batch_ops: key_values,
        }
    }

    /// Returns a reference to the start proof, which may be empty.
    #[must_use]
    pub const fn start_proof(&self) -> &Proof<H> {
        &self.start_proof
    }

    /// Returns a reference to the end proof, which may be empty.
    #[must_use]
    pub const fn end_proof(&self) -> &Proof<H> {
        &self.end_proof
    }

    /// Returns the `BatchOp`s included in the change proof, which may be empty.
    #[must_use]
    pub const fn batch_ops(&self) -> &[BatchOp<K, V>] {
        &self.batch_ops
    }

    /// Returns true if the change proof is empty, meaning it has no start or end proof
    /// and no `BatchOp`s.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.start_proof.is_empty() && self.end_proof.is_empty() && self.batch_ops.is_empty()
    }

    /// Returns an iterator over the `BatchOp`s in this change proof.
    ///
    /// The iterator yields references to the `BatchOp`s in the order they
    /// appear in the proof (which should be lexicographic order as they appear
    /// in the trie).
    #[must_use]
    pub fn iter(&self) -> ChangeProofIter<'_, K, V> {
        ChangeProofIter(self.batch_ops.iter())
    }
}

/// An iterator over the `BatchOp`s in a `ChangeProof`.
///
/// This iterator yields references to the `BatchOp`s contained within
/// the change proof in the order they appear (lexicographic order).
///
/// This type is not re-exported at the top level; it is only accessible through
/// the iterator trait implementations on `ChangeProof`.
#[derive(Debug)]
pub struct ChangeProofIter<'a, K: AsRef<[u8]> + Debug, V: AsRef<[u8]> + Debug>(
    std::slice::Iter<'a, BatchOp<K, V>>,
);

impl<'a, K, V> Iterator for ChangeProofIter<'a, K, V>
where
    K: AsRef<[u8]> + Debug,
    V: AsRef<[u8]> + Debug,
{
    type Item = &'a BatchOp<K, V>;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.0.size_hint()
    }
}

impl<K, V> ExactSizeIterator for ChangeProofIter<'_, K, V>
where
    K: AsRef<[u8]> + Debug,
    V: AsRef<[u8]> + Debug,
{
}

impl<K, V> std::iter::FusedIterator for ChangeProofIter<'_, K, V>
where
    K: AsRef<[u8]> + Debug,
    V: AsRef<[u8]> + Debug,
{
}

impl<'a, K, V, H> IntoIterator for &'a ChangeProof<K, V, H>
where
    K: AsRef<[u8]> + Debug,
    V: AsRef<[u8]> + Debug,
    H: ProofCollection,
{
    type Item = &'a BatchOp<K, V>;
    type IntoIter = ChangeProofIter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

// ── Change proof verification ──────────────────────────────────────────────

/// Verification context captured after structural validation of a change proof.
/// Stored so that downstream logic (root hash verification, `find_next_key`) can
/// reference the original verification parameters without re-validating.
#[derive(Debug)]
pub struct ChangeProofVerificationContext {
    /// The expected root hash of the ending revision.
    pub end_root: HashKey,
    /// The lower bound of the verified key range, if any.
    pub start_key: Option<Box<[u8]>>,
    /// The upper bound of the verified key range, if any.
    pub end_key: Option<Box<[u8]>>,
}

/// Verify structural properties and boundary proofs of a change proof.
///
/// Performs the following checks:
/// - Range validity (`start_key` < `end_key`)
/// - No `DeleteRange` operations
/// - `batch_ops` length does not exceed `max_length`
/// - Keys are sorted and unique
/// - Boundary key constraints (`start_key` ≤ first batch key, `end_key` ≥ last batch key)
/// - Boundary proof completeness (non-empty `batch_ops` with bounds requires at least one proof)
/// - Start and end proof hash chain verification against `end_root`
/// - End proof inclusion/exclusion consistency with the last batch operation
///
/// # Errors
///
/// Returns [`api::Error::ProofError`] if the proof is structurally invalid
/// or boundary proof hash chains fail verification.
///
/// On success, returns a [`ChangeProofVerificationContext`] capturing the
/// verification parameters for use by downstream root hash verification.
#[expect(clippy::too_many_lines)]
pub fn verify_change_proof_structure(
    proof: &FrozenChangeProof,
    end_root: HashKey,
    start_key: Option<&[u8]>,
    end_key: Option<&[u8]>,
    max_length: Option<NonZeroUsize>,
) -> Result<ChangeProofVerificationContext, api::Error> {
    let batch_ops = proof.batch_ops();

    // --- O(1) checks first ---

    // Check batch_ops length <= max_length
    if let Some(max_length) = max_length
        && batch_ops.len() > max_length.into()
    {
        return Err(api::Error::ProofError(
            ProofError::ProofIsLargerThanMaxLength,
        ));
    }

    // Reject inverted ranges early. The generator enforces this, but the
    // verifier must independently validate because start_key/end_key
    // come from the caller, not the proof.
    if let (Some(start), Some(end)) = (start_key, end_key)
        && start > end
    {
        return Err(api::Error::InvalidRange {
            start_key: start.to_vec().into(),
            end_key: end.to_vec().into(),
        });
    }

    // Validate boundary proof presence against batch_ops, start_key,
    // and end_key. The honest generator follows strict rules about when
    // proofs should be present. These O(1) checks reject malformed
    // proofs before expensive O(n) scans.

    // Non-empty start_proof without a start_key is unverifiable — the
    // honest generator only produces start_proof when start_key is Some.
    if !proof.start_proof().is_empty() && start_key.is_none() {
        return Err(api::Error::ProofError(ProofError::UnexpectedStartProof));
    }

    match (batch_ops.is_empty(), proof.end_proof().is_empty()) {
        // batch_ops present but no end_proof — always an error, but
        // distinguish "no boundary proofs at all" from "just missing end".
        (false, true) => {
            if proof.start_proof().is_empty() && (start_key.is_some() || end_key.is_some()) {
                return Err(api::Error::ProofError(ProofError::MissingBoundaryProof));
            }
            return Err(api::Error::ProofError(ProofError::MissingEndProof));
        }
        // No batch_ops, end_proof present but no end_key — the honest
        // generator never produces this.
        (true, false) if end_key.is_none() => {
            return Err(api::Error::ProofError(ProofError::UnexpectedEndProof));
        }
        // No batch_ops, no end_proof, but end_key present — missing.
        (true, true) if end_key.is_some() => {
            return Err(api::Error::ProofError(ProofError::MissingEndProof));
        }
        // all other cases are fine
        _ => {}
    }

    // Check start key not greater than first batch op key
    if let (Some(start_key), Some(first_key)) = (start_key, batch_ops.first())
        && *start_key > **first_key.key()
    {
        return Err(api::Error::ProofError(
            ProofError::StartKeyLargerThanFirstKey,
        ));
    }

    // Check end key not less than last batch op key
    if let (Some(end_key), Some(last_key)) = (end_key, batch_ops.last())
        && *end_key < **last_key.key()
    {
        return Err(api::Error::ProofError(ProofError::EndKeyLessThanLastKey));
    }

    // Verify start boundary proof against end_root.
    // When start_key is None, the start proof must be empty (enforced by
    // the UnexpectedStartProof check above), so there is nothing to
    // verify and we skip this block.
    // value_digest returns:
    //   Ok(Some(_)) → inclusion proof (key exists at the last proof node)
    //   Ok(None)    → exclusion proof (key does not exist)
    //   Err(Empty)  → start_proof is empty, valid here (range starts
    //                  from beginning of keyspace)
    if let Some(start_key) = start_key {
        let result = match proof.start_proof().value_digest(start_key, &end_root) {
            Ok(result) => result,
            Err(ProofError::Empty) => None,
            Err(e) => return Err(api::Error::ProofError(e)),
        };

        // When the first batch op key equals start_key, verify that
        // the start proof's inclusion/exclusion result matches the op
        // type. A Put expects inclusion (key exists in end_root); a
        // Delete expects exclusion (key absent). A mismatch means the
        // attacker added a spurious key at start_key (e.g., Put when
        // start_key doesn't exist) or converted a Put to Delete.
        //
        // When first_op_key != start_key (or batch_ops is empty), the
        // start proof is for start_key which is an arbitrary range
        // bound — both inclusion and exclusion are valid.
        if let Some(first_op) = batch_ops.first()
            && first_op.key().as_ref() == start_key
        {
            // Reject DeleteRange early with the specific error
            // before the generic mismatch check. The O(n) scan
            // below also catches DeleteRange, but this provides
            // a better error for the first-op position.
            if matches!(first_op, BatchOp::DeleteRange { .. }) {
                return Err(api::Error::ProofError(
                    ProofError::DeleteRangeFoundInChangeProof,
                ));
            }
            let consistent = matches!(
                (first_op, &result),
                (BatchOp::Put { .. }, Some(_)) | (BatchOp::Delete { .. }, None)
            );
            if !consistent {
                return Err(api::Error::ProofError(
                    ProofError::StartProofOperationMismatch,
                ));
            }
        }
    }

    // Single-pass O(n) scan: reject DeleteRange ops and verify keys are
    // sorted and unique. The honest diff algorithm only produces Put and
    // Delete ops; a crafted proof could use DeleteRange to delete keys
    // outside the proven range. After the loop, last_op holds the last
    // batch op for end proof verification.
    let mut last_op: Option<&BatchOp<_, _>> = None;
    for op in batch_ops {
        if matches!(op, BatchOp::DeleteRange { .. }) {
            return Err(api::Error::ProofError(
                ProofError::DeleteRangeFoundInChangeProof,
            ));
        }
        let key = op.key();
        if let Some(prev) = last_op
            && key <= prev.key()
        {
            return Err(api::Error::ProofError(ProofError::ChangeProofKeysNotSorted));
        }
        last_op = Some(op);
    }

    // Verify end boundary proof against end_root.
    // The key used is last_op_key when batch_ops is non-empty, or end_key
    // when batch_ops is empty. When both are None, this block is skipped
    // — the structural match block above guarantees the end proof is also
    // empty in that case (UnexpectedEndProof rejects non-empty end proofs
    // when batch_ops is empty and end_key is None).
    if let Some(key) = last_op.map(|op| op.key().as_ref()).or(end_key) {
        // value_digest returns:
        //   Ok(Some(_)) → inclusion proof (key exists at the last proof node)
        //   Ok(None)    → exclusion proof (key does not exist)
        //   Err(Empty)  → end_proof is empty, valid here only when
        //                  batch_ops is empty (no operation to check)
        let result = match proof.end_proof().value_digest(key, &end_root) {
            Ok(result) => result,
            Err(ProofError::Empty) => None,
            Err(e) => return Err(api::Error::ProofError(e)),
        };

        // When batch_ops is non-empty, the generator built the end proof
        // for last_op_key. A Put means the key was inserted in end_root
        // (expect inclusion); a Delete means it was removed (expect
        // exclusion). When batch_ops is empty (last_op is None), the key
        // came from end_key (an arbitrary range bound) — both inclusion
        // and exclusion are valid, so we fall through to the wildcard.
        match last_op {
            Some(BatchOp::Put { .. }) if result.is_none() => {
                return Err(api::Error::ProofError(
                    ProofError::EndProofOperationMismatch,
                ));
            }
            Some(BatchOp::Delete { .. }) if result.is_some() => {
                return Err(api::Error::ProofError(
                    ProofError::EndProofOperationMismatch,
                ));
            }
            _ => {}
        }
    }

    Ok(ChangeProofVerificationContext {
        end_root,
        start_key: start_key.map(Box::from),
        end_key: end_key.map(Box::from),
    })
}

/// Convert a proof node's nibble key to a byte-level key for path lookup.
///
/// Each pair of nibbles is combined into one byte. Odd-length keys are
/// implicitly padded with a zero nibble so that `path_to_key` traverses
/// through the odd-depth node. Only valid for the last node in a proof
/// path — the zero padding would misalign intermediate nodes.
#[must_use]
pub fn change_proof_node_byte_key(node: &ProofNode) -> Box<[u8]> {
    // Pair nibbles into bytes directly. For odd-length keys, the
    // trailing nibble is padded with 0 so path_to_key traverses
    // through odd-depth nodes. The extra nibble is harmless since
    // path_to_key stops at the deepest matching node.
    let (pairs, remainder) = node.key.as_byte_slice().as_chunks::<2>();
    pairs
        .iter()
        .map(|&[hi, lo]| hi << 4 | lo)
        .chain(remainder.iter().map(|&nibble| nibble << 4))
        .collect()
}

/// Return the byte key needed to look up a proposal path aligned with the
/// given boundary proof nodes. Returns `None` when the proof is empty.
#[must_use]
pub fn change_proof_boundary_key(proof_nodes: &[ProofNode]) -> Option<Box<[u8]>> {
    proof_nodes.last().map(change_proof_node_byte_key)
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, reason = "Tests can use unwrap")]
    #![expect(clippy::indexing_slicing, reason = "Tests can use indexing")]

    use super::*;
    use crate::merkle::{Key, Value};

    #[test]
    fn test_change_proof_iterator() {
        let key_values: Box<[BatchOp<Key, Value>]> = Box::new([
            BatchOp::Put {
                key: b"key1".to_vec().into_boxed_slice(),
                value: b"value1".to_vec().into_boxed_slice(),
            },
            BatchOp::Put {
                key: b"key2".to_vec().into_boxed_slice(),
                value: b"value2".to_vec().into_boxed_slice(),
            },
            BatchOp::Put {
                key: b"key3".to_vec().into_boxed_slice(),
                value: b"value3".to_vec().into_boxed_slice(),
            },
        ]);

        let start_proof = Proof::empty();
        let end_proof = Proof::empty();

        let change_proof = ChangeProof::new(start_proof, end_proof, key_values);

        let mut iter = change_proof.iter();
        assert_eq!(iter.len(), 3);

        let first = iter.next().unwrap();
        assert!(
            matches!(first, BatchOp::Put { key, value } if **key == *b"key1" && **value == *b"value1"),
        );

        let second = iter.next().unwrap();
        assert!(
            matches!(second, BatchOp::Put { key, value } if **key == *b"key2" && **value == *b"value2"),
        );

        let third = iter.next().unwrap();
        assert!(
            matches!(third, BatchOp::Put { key, value } if **key == *b"key3" && **value == *b"value3"),
        );

        assert!(iter.next().is_none());
    }

    #[test]
    fn test_change_proof_into_iterator() {
        let key_values: Box<[BatchOp<Key, Value>]> = Box::new([
            BatchOp::Put {
                key: b"a".to_vec().into_boxed_slice(),
                value: b"alpha".to_vec().into_boxed_slice(),
            },
            BatchOp::Put {
                key: b"b".to_vec().into_boxed_slice(),
                value: b"beta".to_vec().into_boxed_slice(),
            },
        ]);

        let start_proof = Proof::empty();
        let end_proof = Proof::empty();
        let change_proof = ChangeProof::new(start_proof, end_proof, key_values);

        let mut items = Vec::new();
        for item in &change_proof {
            items.push(item);
        }

        assert_eq!(items.len(), 2);
        assert!(
            matches!(items[0], BatchOp::Put{ key, value } if **key == *b"a" && **value == *b"alpha"),
        );
        assert!(
            matches!(items[1], BatchOp::Put{ key, value } if **key == *b"b" && **value == *b"beta"),
        );
    }

    #[test]
    fn test_empty_change_proof_iterator() {
        let key_values: Box<[BatchOp<Key, Value>]> = Box::new([]);
        let start_proof = Proof::empty();
        let end_proof = Proof::empty();
        let change_proof = ChangeProof::new(start_proof, end_proof, key_values);

        let mut iter = change_proof.iter();
        assert_eq!(iter.len(), 0);
        assert!(iter.next().is_none());

        let items: Vec<_> = change_proof.into_iter().collect();
        assert!(items.is_empty());
    }
}
