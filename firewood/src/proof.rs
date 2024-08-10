// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::iter::once;

use crate::merkle::MerkleError;
use sha2::{Digest, Sha256};
use storage::{
    BranchNode, Hashable, NibblesIterator, PathIterItem, Preimage, TrieHash, ValueDigest,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProofError {
    #[error("expected start proof but got None")]
    MissingStartProof,
    #[error("expected end proof but got None")]
    MissingEndProof,
    #[error("proof keys should be monotonically increasing")]
    NonIncreasingKeys,
    #[error("key before range start")]
    KeyBeforeRangeStart,
    #[error("key after range end")]
    KeyAfterRangeEnd,
    #[error("unexpected hash")]
    UnexpectedHash,
    #[error("unexpected value")]
    UnexpectedValue,
    #[error("proof should contain only the root")]
    ShouldBeJustRoot,
    #[error("expected non-empty trie")]
    UnexpectedEmptyTrie,
    #[error("missing key-value pair impled by proof")]
    MissingKeyValue,
    #[error("value mismatch")]
    ValueMismatch,
    #[error("expected value but got None")]
    ExpectedValue,
    #[error("proof can't be empty")]
    Empty,
    #[error("each proof node key should be a prefix of the proven key")]
    ShouldBePrefixOfProvenKey,
    #[error("each proof node key should be a prefix of the next key")]
    ShouldBePrefixOfNextKey,
    #[error("child index is out of bounds")]
    ChildIndexOutOfBounds,
    #[error("only nodes with even length key can have values")]
    ValueAtOddNibbleLength,
    #[error("node not in trie")]
    NodeNotInTrie,
    #[error("{0:?}")]
    Merkle(#[from] MerkleError),
    #[error("proof is empty; should have key-values or a start/end proof")]
    EmptyProof,
}

#[derive(Clone, Debug)]
pub struct ProofNode {
    /// The key this node is at. Each byte is a nibble.
    pub key: Box<[u8]>,
    /// None if the node does not have a value.
    /// Otherwise, the node's value or the hash of its value.
    pub value_digest: Option<ValueDigest<Box<[u8]>>>,
    /// The hash of each child, or None if the child does not exist.
    pub child_hashes: [Option<TrieHash>; BranchNode::MAX_CHILDREN],
}

impl Hashable for ProofNode {
    fn key(&self) -> impl Iterator<Item = u8> + Clone {
        self.key.as_ref().iter().copied()
    }

    fn value_digest(&self) -> Option<ValueDigest<&[u8]>> {
        self.value_digest.as_ref().map(|vd| match vd {
            ValueDigest::Value(v) => ValueDigest::Value(v.as_ref()),
            ValueDigest::_Hash(h) => ValueDigest::_Hash(h.as_ref()),
        })
    }

    fn children(&self) -> impl Iterator<Item = (usize, &TrieHash)> + Clone {
        self.child_hashes
            .iter()
            .enumerate()
            .filter_map(|(i, hash)| hash.as_ref().map(|h| (i, h)))
    }
}

impl From<PathIterItem> for ProofNode {
    fn from(item: PathIterItem) -> Self {
        let mut child_hashes: [Option<TrieHash>; BranchNode::MAX_CHILDREN] = Default::default();

        if let Some(branch) = item.node.as_branch() {
            // TODO danlaine: can we avoid indexing?
            #[allow(clippy::indexing_slicing)]
            for (i, hash) in branch.children_iter() {
                child_hashes[i] = Some(hash.clone());
            }
        }

        Self {
            key: item.key_nibbles,
            value_digest: item
                .node
                .value()
                .map(|value| ValueDigest::Value(value.to_vec().into_boxed_slice())),
            child_hashes,
        }
    }
}

impl From<&ProofNode> for TrieHash {
    fn from(node: &ProofNode) -> Self {
        node.to_hash()
    }
}

/// A proof that a given key-value pair either exists or does not exist in a trie.
#[derive(Clone, Debug)]
pub struct Proof<T: Hashable>(pub Box<[T]>);

impl<T: Hashable> Proof<T> {
    pub fn verify<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        key: K,
        expected_value: Option<V>,
        root_hash: &TrieHash,
    ) -> Result<(), ProofError> {
        let value_digest = self.value_digest(key, root_hash)?;

        let Some(value_digest) = value_digest else {
            // This proof proves that `key` maps to None.
            if expected_value.is_some() {
                return Err(ProofError::ExpectedValue);
            }
            return Ok(());
        };

        let Some(expected_value) = expected_value else {
            // We were expecting `key` to map to None.
            return Err(ProofError::UnexpectedValue);
        };

        match value_digest {
            ValueDigest::Value(got_value) => {
                // This proof proves that `key` maps to `got_value`.
                if got_value != expected_value.as_ref() {
                    // `key` maps to an unexpected value.
                    return Err(ProofError::ValueMismatch);
                }
            }
            ValueDigest::_Hash(got_hash) => {
                // This proof proves that `key` maps to a value
                // whose hash is `got_hash`.
                let value_hash = Sha256::digest(expected_value.as_ref());
                if got_hash != value_hash.as_slice() {
                    // `key` maps to an unexpected value.
                    return Err(ProofError::ValueMismatch);
                }
            }
        }
        Ok(())
    }

    /// Returns the value digest associated with the given `key` in the trie revision
    /// with the given `root_hash`. If the key does not exist in the trie, returns `None`.
    /// Returns an error if the proof is invalid or doesn't prove the key for the
    /// given revision.
    fn value_digest<K: AsRef<[u8]>>(
        &self,
        key: K,
        root_hash: &TrieHash,
    ) -> Result<Option<ValueDigest<&[u8]>>, ProofError> {
        let key: Box<[u8]> = NibblesIterator::new(key.as_ref()).collect();

        let Some(last_node) = self.0.last() else {
            return Err(ProofError::Empty);
        };

        let mut expected_hash = root_hash;

        let mut iter = self.0.iter().peekable();
        while let Some(node) = iter.next() {
            if node.to_hash() != *expected_hash {
                return Err(ProofError::UnexpectedHash);
            }

            // Assert that only nodes whose keys are an even number of nibbles
            // have a `value_digest`.
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
                            Some(hash)
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

    /// Returns an iterator that returns (key,hash) for all child hashes (or lack thereof)
    /// implied by this proof in increasing lexicographic order by key.
    /// If the hash is None, the trie doesn't contain any keys with the given key prefix.
    pub(super) fn implied_hashes(&self) -> impl Iterator<Item = (Box<[u8]>, Option<&TrieHash>)> {
        ImpliedHashIterator::FromNode {
            node: &self.0[0], // TODO don't use indexing
            child: None,
            next_index: 0,
        }
    }
}

/// Iterates over all implied hashes of a node in lexicographic order.
enum ImpliedHashIterator<'a, T: Hashable> {
    /// Next iteration returns the hash, if any, of the child at the `next_index`.
    FromNode {
        node: &'a T,
        child: Option<(&'a [T], u8)>,
        next_index: u8,
    },
    /// Next iteration returns a (key,hash) pair from the child iterator.
    Recursive {
        node: &'a T,
        child: Box<ImpliedHashIterator<'a, T>>,
        next_index: u8,
    },
    Exhausted,
}

impl<'a, T: Hashable> Iterator for ImpliedHashIterator<'a, T> {
    type Item = (Box<[u8]>, Option<&'a TrieHash>);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            ImpliedHashIterator::FromNode {
                node,
                child,
                next_index,
            } => {
                if let Some((remaining_nodes, child_index)) = child {
                    // Check if we should traverse to the child.
                    if next_index == child_index {
                        // Traverse to the child.
                        let child_key = remaining_nodes.first().expect("TODO").key();

                        let child = remaining_nodes.get(1).map(|grandchild| {
                            let grandchild_key = grandchild.key();
                            let grandchild_index =
                                next_nibble(child_key.clone(), grandchild_key).expect("TODO");
                            (&remaining_nodes[1..], grandchild_index)
                        });

                        let child = Box::new(ImpliedHashIterator::FromNode {
                            node: &remaining_nodes[0],
                            child,
                            next_index: 0,
                        });

                        let hash = node
                            .children()
                            .find_map(|(i, hash)| {
                                if i == *next_index as usize {
                                    Some(hash)
                                } else {
                                    None
                                }
                            })
                            .expect("TODO");

                        *self = ImpliedHashIterator::Recursive {
                            node,
                            child,
                            next_index: *next_index + 1,
                        };

                        return Some((child_key.collect(), Some(hash)));
                    }
                }

                let hash = node.children().find_map(|(i, hash)| {
                    if i == *next_index as usize {
                        Some(hash)
                    } else {
                        None
                    }
                });

                let key: Box<[u8]> = node.key().chain(once(*next_index)).collect();

                *next_index += 1;

                if *next_index == BranchNode::MAX_CHILDREN as u8 {
                    *self = ImpliedHashIterator::Exhausted;
                }

                Some((key, hash))
            }
            ImpliedHashIterator::Recursive {
                node,
                child: child_iter,
                next_index,
            } => {
                if let Some(next) = child_iter.next() {
                    return Some(next);
                }

                // The child iterator is exhausted.
                *self = ImpliedHashIterator::FromNode {
                    node,
                    child: None,
                    next_index: *next_index,
                };

                self.next()
            }
            ImpliedHashIterator::Exhausted => None,
        }
    }
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
