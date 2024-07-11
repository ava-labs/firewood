// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::hashednode::{Hashable, Preimage, ValueDigest};
use crate::merkle::MerkleError;
use sha2::{Digest, Sha256};
use storage::{BranchNode, NibblesIterator, PathIterItem, TrieHash};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProofError {
    #[error("non-monotonic range increase")]
    NonMonotonicIncreaseRange,
    #[error("unexpected hash")]
    UnexpectedHash,
    #[error("unexpected value")]
    UnexpectedValue,
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
    #[error("empty range")]
    EmptyRange,
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
                if got_value.as_ref() != expected_value.as_ref() {
                    // `key` maps to an unexpected value.
                    return Err(ProofError::ValueMismatch);
                }
            }
            ValueDigest::_Hash(got_hash) => {
                // This proof proves that `key` maps to a value
                // whose hash is `got_hash`.
                let value_hash = Sha256::digest(expected_value.as_ref());
                if got_hash.as_ref() != value_hash.as_slice() {
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
    ) -> Result<Option<ValueDigest<T::T>>, ProofError> {
        let key: Vec<u8> = NibblesIterator::new(key.as_ref()).collect();

        let Some(last_node) = self.0.last() else {
            return Err(ProofError::Empty);
        };

        let mut expected_hash = root_hash;

        // TODO danlaine: Is there a better way to do this loop?
        for i in 0..self.0.len() {
            #[allow(clippy::indexing_slicing)]
            let node = &self.0[i];

            if node.to_hash() != *expected_hash {
                return Err(ProofError::UnexpectedHash);
            }

            // Assert that only nodes whose keys are an even number of nibbles
            // have a `value_digest`.
            if node.key().count() % 2 != 0 && node.value_digest().is_some() {
                return Err(ProofError::ValueAtOddNibbleLength);
            }

            if i != self.0.len() - 1 {
                // Assert that every node's key is a prefix of the proven key,
                // with the exception of the last node, which is a suffix of the
                // proven key in exclusion proofs.
                let next_nibble = next_nibble(node.key(), key.iter().copied())?;

                let Some(next_nibble) = next_nibble else {
                    return Err(ProofError::ShouldBePrefixOfProvenKey);
                };

                expected_hash = node
                    .children()
                    .find(|(i, _)| *i == next_nibble as usize)
                    .map(|(_, hash)| hash)
                    .ok_or(ProofError::ChildIndexOutOfBounds)?;

                // Assert that each node's key is a prefix of the next node's key.
                #[allow(clippy::indexing_slicing)]
                let next_node_key = self.0[i + 1].key();
                if !is_prefix(node.key(), next_node_key) {
                    return Err(ProofError::ShouldBePrefixOfNextKey);
                }
            }
        }

        if last_node.key().count() == key.len() {
            return Ok(last_node.value_digest());
        }

        // This is an exclusion proof.
        Ok(None)
    }
}

/// Returns the next nibble in `c` after `b`.
/// Returns an error if `b` is not a prefix of `c`.
fn next_nibble<I, J>(b: I, c: J) -> Result<Option<u8>, ProofError>
where
    I: IntoIterator<Item = u8>,
    J: IntoIterator<Item = u8>,
{
    let b = b.into_iter();
    let mut c = c.into_iter();

    // Check if b is a prefix of c
    for b_item in b {
        match c.next() {
            Some(c_item) if b_item == c_item => continue,
            _ => return Err(ProofError::ShouldBePrefixOfNextKey),
        }
    }

    // If a is a prefix, return the first element in c after b
    Ok(c.next())
}

fn is_prefix<I, J>(b: I, c: J) -> bool
where
    I: IntoIterator<Item = u8>,
    J: IntoIterator<Item = u8>,
{
    let b = b.into_iter();
    let mut c = c.into_iter();
    for b_item in b {
        let Some(c_item) = c.next() else {
            return false;
        };
        if b_item != c_item {
            return false;
        }
    }
    true
}
