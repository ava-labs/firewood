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
    BranchNode, Children, FileIoError, HashType, Hashable, IntoHashType, NibblesIterator, Path,
    PathIterItem, Preimage, TrieHash, ValueDigest,
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

    /// Error when the first key is greater than the last key in a provided range proof
    #[error("first key must come before the last key in a range proof")]
    InvalidRange,

    /// Error when the caller provides an end key proof but no end key or key-values.
    #[error("unexpected end key proof when no end key and no key-values were provided")]
    UnexpectedEndProof,

    /// Error when the caller provides a start key proof but no start key to verify
    #[error("unexpected start key proof when no start key was provided")]
    UnexpectedStartProof,

    /// Error when the caller provided key-values or an end key but no end key proof.
    #[error("expected an end key proof when an end key is provided or key-values are present")]
    ExpectedEndProof,

    /// Error when the kev-values in in the provided proof are not in strict ascending order
    #[error("key-values in the provided proof are not in strict ascending order")]
    NonIncreasingValues,

    /// Error when the key-values are outsided of the expected [start, end] range
    /// where start and end are inclusive, but optional.
    #[error("expected key-values to be within the provided [start, end] range")]
    StateFromOutsideOfRange,

    /// Error when the exclusion proof is missing end nodes for the provided key.
    #[error("exclusion proof is missing end nodes for target path")]
    ExclusionProofMissingEndNodes,

    /// Error when the proof is invalid for an exclusion proof because the final
    /// node diverges from the penultimate node's differently than the target key.
    #[error("invalid node for exclusion proof")]
    ExclusionProofInvalidNode,

    /// Empty range
    #[error("empty range")]
    EmptyRange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    pub child_hashes: Children<HashType>,
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

    fn children(&self) -> Children<HashType> {
        self.child_hashes.clone()
    }
}

impl From<PathIterItem> for ProofNode {
    fn from(item: PathIterItem) -> Self {
        let child_hashes = if let Some(branch) = item.node.as_branch() {
            branch.children_hashes()
        } else {
            BranchNode::empty_children()
        };

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
                    .get(usize::from(next_nibble))
                    .ok_or(ProofError::ChildIndexOutOfBounds)?
                    .as_ref()
                    .ok_or(ProofError::NodeNotInTrie)?
                    .clone();
            }
        }

        if last_node.key().count() == key.len() {
            return Ok(last_node.value_digest());
        }

        // This is an exclusion proof.
        Ok(None)
    }

    /// If the last element in `proof` is `key`, this is an inclusion proof.
    /// Otherwise, this is an exclusion proof and `key` must not be in `proof`.
    ///
    /// Returns [`Ok`] if and only if all the following hold:
    ///
    ///   - Any node with a partial byte length, should not have a value associated with it
    ///     since all keys with values are written in complete bytes.
    ///
    ///   - Each key in `proof` is a strict prefix of the following key.
    ///
    ///   - Each key in `proof` is a strict prefix of `key`, except possibly the last.
    ///
    ///   - If this is an inclusion proof, the last key in `proof` is the `key`.
    ///
    ///   - If this is an exclusion proof:
    ///     - the last key in `proof` is the replacement child and is at the
    ///       corresponding index of the parent's children.
    ///     - the last key in `proof` is the possible parent and it doesn't
    ///       have a child at the corresponding index.
    pub fn verify_proof_path_structure(&self, key: &Path) -> Result<(), ProofError> {
        let proof_nodes = self.as_ref();

        let Some(last_node) = proof_nodes.last() else {
            // empty proof
            return Ok(());
        };

        // Validate all nodes except the last
        for window in proof_nodes.windows(2) {
            let [node, next_node] = window else {
                unreachable!("windows(2) will always return a slice of length 2");
            };

            // Check partial byte constraint
            #[cfg(not(feature = "branch_factor_256"))]
            if node.key().count() % 2 != 0 && node.value_digest().is_some() {
                return Err(ProofError::ValueAtOddNibbleLength);
            }

            // Each node's key should be a prefix of target key
            if next_nibble(node.key(), key.iter().copied()).is_none() {
                return Err(ProofError::ShouldBePrefixOfProvenKey);
            }

            // Each node's key should be a prefix of next node's key
            if next_nibble(node.key(), next_node.key()).is_none() {
                return Err(ProofError::ShouldBePrefixOfNextKey);
            }
        }

        // Validate last node
        let last_key = Path(last_node.key().collect());

        #[cfg(not(feature = "branch_factor_256"))]
        if last_key.len() % 2 != 0 && last_node.value_digest().is_some() {
            return Err(ProofError::ValueAtOddNibbleLength);
        }

        match key.strip_prefix(last_key.as_ref()) {
            Some(&[]) => {
                // inclusion proof, all done
                Ok(())
            }
            Some(&[next_nibble, ..]) => {
                if last_node
                    .children()
                    .get(next_nibble as usize)
                    .ok_or(ProofError::ChildIndexOutOfBounds)?
                    .is_some()
                {
                    // exclusion proof, but the last node has a child at the next nibble
                    // for our target key, so the proof is invalid because we need more
                    // proof nodes to fully verify the exclusion.
                    Err(ProofError::ExclusionProofMissingEndNodes)
                } else {
                    // exclusion proof, the last node is a parent of the target key and
                    // there is no child at the next nibble.
                    Ok(())
                }
            }
            None => {
                let [.., penultimate_node, _] = proof_nodes else {
                    // there is no penultimate node, so the last node is the only node
                    // (it is also the root node). Its existence is enough to verify
                    // that the target key is excluded.
                    return Ok(());
                };

                // replacement child, example:
                //
                // penultimate: "ab"
                // target:      "abd"
                // final:       "abef"
                // Note: shares prefix with target but diverges later
                //
                // for `last_node` to be a valid exclusion proof, it must be located
                // at the same index in penultimate_node's children as where the
                // next nibble would be in the target key.
                //
                // The example is invalid because the target key diverges
                // from the final key at the last nibble (`d != e`).
                //
                // the earlier windows loop guarantees that penultimate_node is strict prefix
                // of both the target key and the final key. Therefore, we can safely use the
                // result of `next_nibble` to check if the last node is a valid replacement
                // for the target key.

                let last_index = next_nibble(penultimate_node.key(), last_key.iter().copied());
                let target_index = next_nibble(penultimate_node.key(), key.iter().copied());

                if matches!((last_index, target_index), (Some(a), Some(b)) if a == b) {
                    // otherwise, the last node is a valid replacement for the target
                    // and it is excluded from the trie.
                    Ok(())
                } else {
                    Err(ProofError::ExclusionProofInvalidNode)
                }
            }
        }
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
pub(crate) fn next_nibble<B, C>(b: B, c: C) -> Option<u8>
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

pub(crate) fn verify_opt_value_digest(
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
