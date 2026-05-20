// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::{fmt::Debug, num::NonZeroUsize};

use firewood_storage::{Hashable, PathBuf, TriePath, TriePathFromPackedBytes};

use crate::{
    Proof, ProofCollection, ProofError,
    api::{self, FrozenChangeProof, HashKey},
    db::BatchOp,
};

use super::lex_successor;

/// A change proof can demonstrate that by applying the provided array of `BatchOp`s to a Merkle
/// trie with given start root hash, the resulting trie will have the given end root hash. It
/// consists of the following:
/// - A start proof: proves that the smallest key does/doesn't exist
/// - An end proof: proves that the largest key does/doesn't exist
/// - The actual `BatchOp`s that specify the difference between the start and end tries.
pub struct ChangeProof<K: AsRef<[u8]> + Debug, V: AsRef<[u8]> + Debug, H> {
    start_proof: Proof<H>,
    end_proof: Proof<H>,
    batch_ops: Box<[BatchOp<K, V>]>,
}

impl<K, V, H> std::fmt::Debug for ChangeProof<K, V, H>
where
    K: AsRef<[u8]> + Debug,
    V: AsRef<[u8]> + Debug,
    H: ProofCollection,
    H::Node: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangeProof")
            .field("start_proof", &self.start_proof)
            .field("end_proof", &self.end_proof)
            .field("batch_ops", &self.batch_ops)
            .finish()
    }
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
    /// The actual right edge of the proven range. This equals `end_key`
    /// when all items fit within the limit, or the last key in `batch_ops`
    /// when the proof may have been truncated (`batch_ops.len() ==
    /// max_length`). The root hash verifier uses this as the right
    /// boundary for `compute_outside_children` and reconciliation.
    pub right_edge_key: Option<Box<[u8]>>,
}

type FrozenBatchOp = BatchOp<Box<[u8]>, Box<[u8]>>;

/// Determine the next key range to fetch after this change proof.
///
/// Returns `None` when the proof's structure proves the requested range
/// is exhaustively covered. Returns
/// `Some((lex_successor(last_key), end_key))` for the structurally
/// ambiguous shape (Case 5 for Put, or any non-malformed Delete shape) —
/// the conservative continuation strictly advances the cursor past
/// `last_key` so the next fetch makes guaranteed forward progress.
///
/// As a preflight, an empty `end_proof` (which signals exhaustive
/// coverage with no boundary proof emitted) short-circuits to `None`
/// before the case analysis below.
///
/// # Cases
///
/// - **Case 1**: `batch_ops` is empty. The proof contains no diffs for
///   the requested range, so there is nothing more to fetch. → `None`.
/// - **Case 2**: `last_key == end_key`. The proof's last returned key
///   reaches the requested upper bound; the range `[start, end_key]`
///   is fully covered. → `None`.
///
/// For Put `last_op` (`last_key` is present in the end revision), the
/// remaining cases mirror the range-proof analysis:
///
/// - **Case 3**: the `end_proof`'s terminal node carries no value (an
///   exclusion proof). A truncated proof would carry `last_key`'s
///   value at the terminal, so an exclusion shape means exhaustive.
///   → `None`.
/// - **Case 4**: the `end_proof`'s terminal carries a value, but at a
///   key different from `last_key`. A truncated proof would put
///   `last_key` itself at the terminal, so a different key there
///   means exhaustive. → `None`.
/// - **Case 5**: the `end_proof`'s terminal carries a value at exactly
///   `last_key`'s path. Structurally ambiguous between a truncated
///   inclusion of `last_key` and an exhaustive exclusion of `end_key`
///   whose terminal lies at `last_key`'s node. Conservative
///   continuation. → `Some((lex_successor(last_key), end_key))`.
///
/// For Delete `last_op` (`last_key` is absent from the end revision),
/// the truncated `end_proof` is an exclusion proof of `last_key` in
/// the end revision, which is structurally indistinguishable from an
/// exhaustive exclusion of `end_key` for the non-malformed shapes.
/// The algorithm collapses these to a single conservative
/// continuation:
///
/// - **Delete malformed**: terminal carries a value at exactly
///   `last_key`'s path. This contradicts `last_key`'s absence from
///   the end revision. The change-proof verifier catches this
///   upstream via the same condition; we surface the same error for
///   defense in depth.
///   → `Err(EndProofOperationMismatch)`.
/// - **Delete other**: any other Delete shape (no value, or value at a
///   different key) is structurally ambiguous between truncated and
///   exhaustive. → `Some((lex_successor(last_key), end_key))`.
///
/// # Errors
///
/// - [`ProofError::EndKeyLessThanLastKey`] when `last_key` is strictly
///   greater than `end_key` — a malformed-proof condition that
///   verification should have caught upstream.
/// - [`ProofError::EndProofOperationMismatch`] when `last_op` is
///   Delete but the `end_proof`'s terminal carries a value at
///   `last_key`'s path — also malformed; verifier should have caught.
/// - [`ProofError::DeleteRangeFoundInChangeProof`] when `last_op` is
///   a `DeleteRange`. `verify_change_proof_structure` rejects these
///   upstream; this is defense in depth.
pub fn find_next_key_after_change_proof(
    proof: &FrozenChangeProof,
    end_key: Option<&[u8]>,
) -> Result<Option<super::range::KeyRange>, api::Error> {
    // Case 1: empty `batch_ops` → no diffs in range; range is exhausted.
    let Some(last_op) = proof.batch_ops().last() else {
        return Ok(None);
    };
    let last_key: &[u8] = last_op.key().as_ref();

    if let Some(ek) = end_key {
        // Malformed: `last_key > end_key`; verifier should have caught this.
        if last_key > ek {
            return Err(api::Error::ProofError(ProofError::EndKeyLessThanLastKey));
        }
        // Case 2: `last_key` reaches `end_key`.
        if last_key == ek {
            return Ok(None);
        }
    }

    // Preflight: empty `end_proof` signals exhaustive coverage.
    let Some(terminal) = proof.end_proof().last() else {
        return Ok(None);
    };

    let terminal_at_last_key = terminal
        .full_path()
        .path_eq(&PathBuf::path_from_packed_bytes(last_key));

    match last_op {
        BatchOp::Put { .. } => {
            // Case 3: terminal has no value → exhaustive.
            if terminal.value_digest.is_none() {
                return Ok(None);
            }
            // Case 4: terminal at different key → exhaustive.
            if !terminal_at_last_key {
                return Ok(None);
            }
            // Case 5: ambiguous → fall through to conservative continuation.
        }
        BatchOp::Delete { .. } => {
            // Malformed: Delete `last_key` means `last_key` is absent
            // from the end revision, so the terminal can't carry a
            // value at `last_key`'s path.
            if terminal.value_digest.is_some() && terminal_at_last_key {
                return Err(api::Error::ProofError(
                    ProofError::EndProofOperationMismatch,
                ));
            }
            // All other Delete shapes are structurally ambiguous →
            // fall through to conservative continuation.
        }
        BatchOp::DeleteRange { .. } => {
            // `DeleteRange` ops are rejected by
            // `verify_change_proof_structure`; a verified proof can't
            // reach this branch. Surface the same error for defense
            // in depth.
            return Err(api::Error::ProofError(
                ProofError::DeleteRangeFoundInChangeProof,
            ));
        }
    }

    // Conservative continuation: cursor strictly past `last_key`.
    Ok(Some((lex_successor(last_key), end_key.map(Box::from))))
}

/// Verify a boundary proof against `end_root` and optionally check that the
/// proof's inclusion/exclusion result is consistent with `boundary_op`.
///
/// When `boundary_op` is `Some`, a `Put` must be an inclusion proof (key
/// present) and a `Delete` must be an exclusion proof (key absent).
/// When `boundary_op` is `None` the key is an arbitrary range bound and
/// both outcomes are valid.
fn verify_boundary_proof<C: ProofCollection>(
    proof: &Proof<C>,
    key: &[u8],
    end_root: &HashKey,
    boundary_op: Option<&FrozenBatchOp>,
    mismatch_error: ProofError,
) -> Result<(), api::Error> {
    let result = match proof.value_digest(key, end_root) {
        Ok(result) => result,
        Err(ProofError::Empty) => None,
        Err(e) => return Err(api::Error::ProofError(e)),
    };

    match boundary_op {
        Some(BatchOp::Put { .. }) if result.is_none() => {
            Err(api::Error::ProofError(mismatch_error))
        }
        Some(BatchOp::Delete { .. }) if result.is_some() => {
            Err(api::Error::ProofError(mismatch_error))
        }
        _ => Ok(()),
    }
}

/// Compute the right edge of the proven range.
///
/// The proof may have been truncated by the generator's limit when:
///   1. `batch_ops.len() == max_length` (limit was potentially hit)
///   2. the end proof is an inclusion proof of the last batch op key
///      (the generator produced the proof at that key, not at `end_key`)
///
/// When truncated, the right edge is `last_op_key`. Otherwise it is
/// `end_key` (falling back to `last_op_key` for unbounded ranges).
fn compute_right_edge_key<'a>(
    proof: &FrozenChangeProof,
    end_root: &HashKey,
    last_op_key: Option<&'a [u8]>,
    end_key: Option<&'a [u8]>,
    max_length: Option<NonZeroUsize>,
) -> Option<&'a [u8]> {
    let possibly_truncated = max_length.is_some_and(|n| proof.batch_ops().len() == n.get());
    let truncated = possibly_truncated
        && last_op_key.is_some_and(|k| {
            proof
                .end_proof()
                .value_digest(k, end_root)
                .ok()
                .flatten()
                .is_some()
        });
    if truncated {
        last_op_key
    } else {
        end_key.or(last_op_key)
    }
}

/// Verify structural properties and boundary proofs of a change proof.
///
/// Performs the following checks:
/// - Range validity (`start_key` ≤ `end_key`)
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
        && batch_ops.len() > max_length.get()
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

    // A start_proof anchors the first batch op to end_root at start_key.
    // Without start_key we have no key to verify the proof against, so a
    // non-empty start_proof is rejected as unverifiable.
    if !proof.start_proof().is_empty() && start_key.is_none() {
        return Err(api::Error::ProofError(ProofError::UnexpectedStartProof));
    }

    match (batch_ops.is_empty(), proof.end_proof().is_empty()) {
        // batch_ops present but no end_proof — always an error. The end
        // proof anchors the last batch key to end_root; without it an
        // attacker could truncate batch_ops and the verifier couldn't
        // detect the omission. This applies even when proving through the
        // end of the DB, because the proof still needs to bind the last
        // key's inclusion/exclusion to the claimed root hash.
        // Distinguish "no boundary proofs at all" from "just missing end".
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
    // the UnexpectedStartProof check above), so there is nothing to verify.
    // When first_op_key == start_key, the proof must be consistent with
    // the op type (Put→inclusion, Delete→exclusion). Otherwise start_key
    // is an arbitrary range bound and both outcomes are valid.
    if let Some(start_key) = start_key {
        let boundary_op = batch_ops
            .first()
            .filter(|op| op.key().as_ref() == start_key);
        verify_boundary_proof(
            proof.start_proof(),
            start_key,
            &end_root,
            boundary_op,
            ProofError::StartProofOperationMismatch,
        )?;
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

    let last_op_key = last_op.map(|op| op.key().as_ref());
    let right_edge_key = compute_right_edge_key(proof, &end_root, last_op_key, end_key, max_length);

    // Verify end boundary proof against end_root. The end proof was
    // generated for right_edge_key. The boundary_op check applies when
    // the right edge matches the last batch op key (meaning the proof
    // must be consistent with the op type); otherwise the key is just a
    // range bound and both inclusion/exclusion are valid.
    let end_boundary_op =
        last_op.filter(|op| right_edge_key.is_some_and(|k| op.key().as_ref() == k));
    if let Some(key) = right_edge_key {
        verify_boundary_proof(
            proof.end_proof(),
            key,
            &end_root,
            end_boundary_op,
            ProofError::EndProofOperationMismatch,
        )?;
    }

    Ok(ChangeProofVerificationContext {
        end_root,
        start_key: start_key.map(Box::from),
        end_key: end_key.map(Box::from),
        right_edge_key: right_edge_key.map(Box::from),
    })
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, reason = "Tests can use unwrap")]
    #![expect(clippy::indexing_slicing, reason = "Tests can use indexing")]

    use firewood_storage::{Children, ValueDigest};

    use super::*;
    use crate::ProofNode;
    use crate::merkle::{Key, Value};

    /// An `end_key` larger than any test key, used when the test should
    /// not exercise the `last_key == end_key` (Case 2) or `last_key >
    /// end_key` (malformed) branches.
    const FAR_END_KEY: &[u8] = b"\xff\xff";

    /// Build a terminal `ProofNode` at the path corresponding to
    /// `key_bytes`. If `value` is `Some`, the node carries a value
    /// digest wrapping those bytes; otherwise the node represents an
    /// exclusion-shaped terminal with no value.
    fn make_terminal(key_bytes: &[u8], value: Option<&[u8]>) -> ProofNode {
        ProofNode {
            key: PathBuf::path_from_packed_bytes(key_bytes),
            partial_len: 0,
            value_digest: value.map(|v| ValueDigest::Value(Box::from(v))),
            child_hashes: Children::new(),
        }
    }

    /// Build a `BatchOp::Put` operation for `key` and `value`.
    fn make_put(key: &[u8], value: &[u8]) -> BatchOp<Key, Value> {
        BatchOp::Put {
            key: Box::from(key),
            value: Box::from(value),
        }
    }

    /// Build a `BatchOp::Delete` operation for `key`.
    fn make_delete(key: &[u8]) -> BatchOp<Key, Value> {
        BatchOp::Delete {
            key: Box::from(key),
        }
    }

    /// Build a `FrozenChangeProof` from explicit batch ops and an
    /// optional terminal node for the `end_proof`. The `start_proof`
    /// is always empty (irrelevant to `find_next_key`'s decisions).
    fn make_change_proof(
        batch_ops: Vec<BatchOp<Key, Value>>,
        end_terminal: Option<ProofNode>,
    ) -> FrozenChangeProof {
        ChangeProof::new(
            Proof::new(Box::default()),
            Proof::new(end_terminal.into_iter().collect()),
            batch_ops.into_boxed_slice(),
        )
    }

    #[test]
    fn test_find_next_key_change_empty_batch_ops() {
        // Case 1: empty `batch_ops` → no diffs in range.
        let proof = make_change_proof(vec![], None);
        assert_eq!(
            find_next_key_after_change_proof(&proof, Some(FAR_END_KEY)).unwrap(),
            None
        );
    }

    #[test]
    fn test_find_next_key_change_empty_end_proof() {
        // Empty `end_proof` → exhaustive.
        let proof = make_change_proof(vec![make_put(b"key1", b"v1")], None);
        assert_eq!(
            find_next_key_after_change_proof(&proof, Some(FAR_END_KEY)).unwrap(),
            None
        );
    }

    #[test]
    fn test_find_next_key_change_last_equals_end() {
        // Case 2: last_key reaches end_key.
        let proof = make_change_proof(
            vec![make_put(b"key1", b"v1")],
            Some(make_terminal(b"key1", Some(b"v"))),
        );
        assert_eq!(
            find_next_key_after_change_proof(&proof, Some(b"key1")).unwrap(),
            None
        );
    }

    #[test]
    fn test_find_next_key_change_last_greater_than_end() {
        // Malformed: last_key > end_key → Err.
        let proof = make_change_proof(
            vec![make_put(b"key2", b"v2")],
            Some(make_terminal(b"key2", Some(b"v"))),
        );
        assert!(matches!(
            find_next_key_after_change_proof(&proof, Some(b"key1")),
            Err(api::Error::ProofError(ProofError::EndKeyLessThanLastKey))
        ));
    }

    #[test]
    fn test_find_next_key_change_put_terminal_no_value() {
        // Case 3 (Put): terminal has no value → exhaustive.
        let proof = make_change_proof(
            vec![make_put(b"key1", b"v1")],
            Some(make_terminal(b"key1", None)),
        );
        assert_eq!(
            find_next_key_after_change_proof(&proof, Some(FAR_END_KEY)).unwrap(),
            None
        );
    }

    #[test]
    fn test_find_next_key_change_put_terminal_value_different_key() {
        // Case 4 (Put): terminal at a key different from last_key.
        let proof = make_change_proof(
            vec![make_put(b"key1", b"v1")],
            Some(make_terminal(b"key9", Some(b"v"))),
        );
        assert_eq!(
            find_next_key_after_change_proof(&proof, Some(FAR_END_KEY)).unwrap(),
            None
        );
    }

    #[test]
    fn test_find_next_key_change_put_terminal_value_at_last_key() {
        // Case 5 (Put): ambiguous shape → conservative continuation.
        let last_key: &[u8] = b"key1";
        let proof = make_change_proof(
            vec![make_put(last_key, b"v1")],
            Some(make_terminal(last_key, Some(b"v"))),
        );
        let result = find_next_key_after_change_proof(&proof, Some(FAR_END_KEY)).unwrap();
        let (cursor, returned_end) = result.expect("ambiguous shape must return Some");
        assert!(
            cursor.as_ref() > last_key,
            "cursor {cursor:?} must be > {last_key:?}"
        );
        assert_eq!(returned_end.as_deref(), Some(FAR_END_KEY));
    }

    #[test]
    fn test_find_next_key_change_delete_terminal_no_value() {
        // Delete with terminal carrying no value: structurally
        // ambiguous between truncated and exhaustive → conservative.
        let last_key: &[u8] = b"key1";
        let proof = make_change_proof(
            vec![make_delete(last_key)],
            Some(make_terminal(last_key, None)),
        );
        let result = find_next_key_after_change_proof(&proof, Some(FAR_END_KEY)).unwrap();
        let (cursor, returned_end) = result.expect("Delete must return Some");
        assert!(
            cursor.as_ref() > last_key,
            "cursor {cursor:?} must be > {last_key:?}"
        );
        assert_eq!(returned_end.as_deref(), Some(FAR_END_KEY));
    }

    #[test]
    fn test_find_next_key_change_delete_terminal_value_different_key() {
        // Delete with terminal at a sibling leaf carrying a value:
        // structurally ambiguous → conservative.
        let last_key: &[u8] = b"key1";
        let proof = make_change_proof(
            vec![make_delete(last_key)],
            Some(make_terminal(b"key9", Some(b"v"))),
        );
        let result = find_next_key_after_change_proof(&proof, Some(FAR_END_KEY)).unwrap();
        let (cursor, returned_end) = result.expect("Delete must return Some");
        assert!(
            cursor.as_ref() > last_key,
            "cursor {cursor:?} must be > {last_key:?}"
        );
        assert_eq!(returned_end.as_deref(), Some(FAR_END_KEY));
    }

    #[test]
    fn test_find_next_key_change_delete_terminal_value_at_last_key() {
        // Malformed Delete: terminal carries a value at last_key's
        // path, contradicting last_key's absence from the end revision.
        // Should surface the same error the verifier raises.
        let proof = make_change_proof(
            vec![make_delete(b"key1")],
            Some(make_terminal(b"key1", Some(b"v"))),
        );
        assert!(matches!(
            find_next_key_after_change_proof(&proof, Some(FAR_END_KEY)),
            Err(api::Error::ProofError(
                ProofError::EndProofOperationMismatch
            ))
        ));
    }

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
