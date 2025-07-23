// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![expect(
    clippy::missing_errors_doc,
    reason = "Found 1 occurrences after enabling the lint."
)]
#![expect(
    clippy::needless_continue,
    reason = "Found 1 occurrences after enabling the lint."
)]

use firewood_storage::{
    BranchNode, FileIoError, HashType, Hashable, IntoHashType, NibblesIterator, Path, PathIterItem,
    Preimage, TrieHash, ValueDigest,
};
use thiserror::Error;

use crate::merkle::{Key, Value};

#[derive(Debug, Error)]
/// Reasons why a proof is invalid
pub enum ProofError {
    /// Non-monotonic range decrease
    #[error("non-monotonic range increase")]
    NonMonotonicIncreaseRange,

    /// Unexpected hash
    #[error("unexpected hash")]
    UnexpectedHash,

    /// Unexpected value
    #[error("unexpected value")]
    UnexpectedValue,

    /// Value mismatch
    #[error("value mismatch")]
    ValueMismatch,

    /// Expected value but got None
    #[error("expected value but got None")]
    ExpectedValue,

    /// Proof is empty
    #[error("proof can't be empty")]
    Empty,

    /// Each proof node key should be a prefix of the proven key
    #[error("each proof node key should be a prefix of the proven key")]
    ShouldBePrefixOfProvenKey,

    /// Each proof node key should be a prefix of the next key
    #[error("each proof node key should be a prefix of the next key")]
    ShouldBePrefixOfNextKey,

    /// Child index is out of bounds
    #[error("child index is out of bounds")]
    ChildIndexOutOfBounds,

    /// Only nodes with even length key can have values
    #[error("only nodes with even length key can have values")]
    ValueAtOddNibbleLength,

    /// Node not in trie
    #[error("node not in trie")]
    NodeNotInTrie,

    /// Error from the merkle package
    #[error("{0:?}")]
    IO(#[from] FileIoError),

    /// Empty range
    #[error("empty range")]
    EmptyRange,
}

#[derive(Clone, Debug)]

/// A node in a proof.
pub struct ProofNode {
    /// The key this node is at. Each byte is a nibble.
    pub key: Key,
    /// The length of the key prefix that is shared with the previous node.
    #[cfg(feature = "ethhash")]
    pub partial_len: usize,
    /// None if the node does not have a value.
    /// Otherwise, the node's value or the hash of its value.
    pub value_digest: Option<ValueDigest<Value>>,
    /// The hash of each child, or None if the child does not exist.
    pub child_hashes: [Option<HashType>; BranchNode::MAX_CHILDREN],
}

impl Hashable for ProofNode {
    fn key(&self) -> impl Iterator<Item = u8> + Clone {
        self.key.as_ref().iter().copied()
    }

    #[cfg(feature = "ethhash")]
    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone {
        self.key.as_ref().iter().skip(self.partial_len).copied()
    }

    fn value_digest(&self) -> Option<ValueDigest<&[u8]>> {
        self.value_digest.as_ref().map(|vd| match vd {
            ValueDigest::Value(v) => ValueDigest::Value(v.as_ref()),
            ValueDigest::Hash(h) => ValueDigest::Hash(h.as_ref()),
        })
    }

    fn children(&self) -> impl Iterator<Item = (usize, &HashType)> + Clone {
        self.child_hashes
            .iter()
            .enumerate()
            .filter_map(|(i, hash)| hash.as_ref().map(|h| (i, h)))
    }
}

impl From<PathIterItem> for ProofNode {
    fn from(item: PathIterItem) -> Self {
        let mut child_hashes: [Option<HashType>; BranchNode::MAX_CHILDREN] =
            [const { None }; BranchNode::MAX_CHILDREN];

        if let Some(branch) = item.node.as_branch() {
            // TODO danlaine: can we avoid indexing?
            #[expect(clippy::indexing_slicing)]
            for (i, hash) in branch.children_hashes() {
                child_hashes[i] = Some(hash.clone());
            }
        }

        #[cfg(feature = "ethhash")]
        let partial_len = item
            .key_nibbles
            .len()
            .saturating_sub(item.node.partial_path().0.len());

        Self {
            key: item.key_nibbles,
            #[cfg(feature = "ethhash")]
            partial_len,
            value_digest: item
                .node
                .value()
                .map(|value| ValueDigest::Value(value.to_vec().into_boxed_slice())),
            child_hashes,
        }
    }
}

/// A proof that a given key-value pair either exists or does not exist in a trie.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Proof<T: ?Sized>(T);

impl<T: ProofCollection + ?Sized> Proof<T> {
    /// Verify a proof
    pub fn verify<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        key: K,
        expected_value: Option<V>,
        root_hash: &TrieHash,
    ) -> Result<(), ProofError> {
        verify_opt_value_digest(expected_value, self.value_digest(key, root_hash)?)
    }

    /// Returns the value digest associated with the given `key` in the trie revision
    /// with the given `root_hash`. If the key does not exist in the trie, returns `None`.
    /// Returns an error if the proof is invalid or doesn't prove the key for the
    /// given revision.
    pub fn value_digest<K: AsRef<[u8]>>(
        &self,
        key: K,
        root_hash: &TrieHash,
    ) -> Result<Option<ValueDigest<&[u8]>>, ProofError> {
        let key = Path(NibblesIterator::new(key.as_ref()).collect());

        let Some(last_node) = self.0.as_ref().last() else {
            return Err(ProofError::Empty);
        };

        let mut expected_hash = root_hash.clone().into_hash_type();

        let mut iter = self.0.as_ref().iter().peekable();
        while let Some(node) = iter.next() {
            if node.to_hash() != expected_hash {
                return Err(ProofError::UnexpectedHash);
            }

            // Assert that only nodes whose keys are an even number of nibbles
            // have a `value_digest`.
            #[cfg(not(feature = "branch_factor_256"))]
            if node.key().count() % 2 != 0 && node.value_digest().is_some() {
                return Err(ProofError::ValueAtOddNibbleLength);
            }

            if let Some(next_node) = iter.peek() {
                // Assert that every node's key is a prefix of `key`, except for the last node,
                // whose key can be equal to or a suffix of `key` in an exclusion proof.
                if next_nibble(node.key(), key.iter().copied()).is_none() {
                    return Err(ProofError::ShouldBePrefixOfProvenKey);
                }

                // Assert that every node's key is a prefix of the next node's key.
                let next_node_index = next_nibble(node.key(), next_node.key());

                let Some(next_nibble) = next_node_index else {
                    return Err(ProofError::ShouldBePrefixOfNextKey);
                };

                expected_hash = node
                    .children()
                    .find_map(|(i, hash)| {
                        if i == next_nibble as usize {
                            Some(hash.clone())
                        } else {
                            None
                        }
                    })
                    .ok_or(ProofError::NodeNotInTrie)?;
            }
        }

        if last_node.key().count() == key.len() {
            return Ok(last_node.value_digest());
        }

        // This is an exclusion proof.
        Ok(None)
    }

    /// Returns the length of the proof.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.as_ref().len()
    }

    /// Returns true if the proof is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.as_ref().is_empty()
    }
}

impl<T: ProofCollection + ?Sized> std::ops::Deref for Proof<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: ProofCollection + ?Sized> std::ops::DerefMut for Proof<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: ProofCollection> Proof<T> {
    /// Constructs a new proof from a collection of proof nodes.
    #[inline]
    #[must_use]
    pub const fn new(proof: T) -> Self {
        Self(proof)
    }
}

impl Proof<EmptyProofCollection> {
    /// Constructs a new empty proof.
    #[inline]
    #[must_use]
    pub const fn empty() -> Self {
        Self::new(EmptyProofCollection)
    }

    /// Converts an empty immutable proof into an empty mutable proof.
    #[inline]
    #[must_use]
    pub const fn into_mutable<T: Hashable>(self) -> Proof<Vec<T>> {
        Proof::new(Vec::new())
    }
}

impl<T: Hashable> Proof<Box<[T]>> {
    /// Converts an immutable proof into a mutable proof.
    #[inline]
    #[must_use]
    pub fn into_mutable(self) -> Proof<Vec<T>> {
        Proof::new(self.0.into_vec())
    }
}

impl<T: Hashable> Proof<Vec<T>> {
    /// Converts a mutable proof into an immutable proof.
    #[inline]
    #[must_use]
    pub fn into_immutable(self) -> Proof<Box<[T]>> {
        Proof::new(self.0.into_boxed_slice())
    }
}

impl<T, V> Proof<V>
where
    T: Hashable,
    V: ProofCollection<Node = T> + IntoIterator<Item = T> + FromIterator<T>,
{
    /// Joins two proofs into one.
    #[inline]
    #[must_use]
    pub fn join<O: ProofCollection<Node = T> + IntoIterator<Item = T>>(
        self,
        other: Proof<O>,
    ) -> Proof<V> {
        self.into_iter().chain(other).collect()
    }
}

impl<V: ProofCollection + FromIterator<V::Node>> FromIterator<V::Node> for Proof<V> {
    #[inline]
    fn from_iter<I: IntoIterator<Item = V::Node>>(iter: I) -> Self {
        Proof(iter.into_iter().collect())
    }
}

impl<V: ProofCollection + Extend<V::Node>> Extend<V::Node> for Proof<V> {
    #[inline]
    fn extend<I: IntoIterator<Item = V::Node>>(&mut self, iter: I) {
        self.0.extend(iter);
    }
}

impl<V: ProofCollection + IntoIterator<Item = V::Node>> IntoIterator for Proof<V> {
    type Item = V::Node;
    type IntoIter = V::IntoIter;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// A trait representing a collection of proof nodes.
///
/// This allows [`Proof`] to be generic over different types of collections such
/// a `Box<[T]>` or `Vec<T>`, where `T` implements the `Hashable` trait.
pub trait ProofCollection: AsRef<[Self::Node]> {
    /// The type of nodes in the proof collection.
    type Node: Hashable;
}

impl<T: Hashable> ProofCollection for [T] {
    type Node = T;
}

impl<T: Hashable> ProofCollection for Box<[T]> {
    type Node = T;
}

impl<T: Hashable> ProofCollection for Vec<T> {
    type Node = T;
}

/// A zero-sized type to represent an empty proof collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct EmptyProofCollection;

impl AsRef<[ProofNode]> for EmptyProofCollection {
    #[inline]
    fn as_ref(&self) -> &[ProofNode] {
        &[]
    }
}

impl ProofCollection for EmptyProofCollection {
    type Node = ProofNode;
}

/// Returns the next nibble in `c` after `b`.
/// Returns None if `b` is not a strict prefix of `c`.
fn next_nibble<B, C>(b: B, c: C) -> Option<u8>
where
    B: IntoIterator<Item = u8>,
    C: IntoIterator<Item = u8>,
{
    let b = b.into_iter();
    let mut c = c.into_iter();

    // Check if b is a prefix of c
    for b_item in b {
        match c.next() {
            Some(c_item) if b_item == c_item => continue,
            _ => return None,
        }
    }

    c.next()
}

fn verify_opt_value_digest(
    expected_value: Option<impl AsRef<[u8]>>,
    found_value: Option<ValueDigest<impl AsRef<[u8]>>>,
) -> Result<(), ProofError> {
    match (expected_value, found_value) {
        (None, None) => Ok(()),
        (Some(_), None) => Err(ProofError::ExpectedValue),
        (None, Some(_)) => Err(ProofError::UnexpectedValue),
        (Some(ref expected), Some(found)) if found.verify(expected) => Ok(()),
        (Some(_), Some(_)) => Err(ProofError::ValueMismatch),
    }
}
