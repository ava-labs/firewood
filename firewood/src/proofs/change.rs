// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::fmt::Debug;

use crate::{Proof, ProofCollection, db::BatchOp};

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
