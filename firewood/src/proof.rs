// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::hashednode::{Preimage, ValueDigest};
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
    /// None if the node has no value.
    /// The value associated with `key` if the value is < 32 bytes.
    /// The hash of the value if the node's value is >= 32 bytes.
    /// TODO danlaine: Can we enforce that this is at most 32 bytes?
    pub value_digest: Option<Box<[u8]>>,
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
            value_digest: item.node.value().map(|value| value.into()),
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
pub struct Proof(pub Box<[ProofNode]>);

impl Proof {
    pub fn verify<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        key: K,
        expected_value: Option<V>,
        root_hash: &TrieHash,
    ) -> Result<(), ProofError> {
        match self.value_digest(key, root_hash)? {
            None => {
                // This proof proves that `key` maps to None.
                if expected_value.is_some() {
                    return Err(ProofError::ExpectedValue);
                }
            }
            Some(ValueDigest::Value(got_value)) => {
                // This proof proves that `key` maps to `got_value`.
                let Some(expected_value) = expected_value else {
                    // We were expecting `key` to map to None.
                    return Err(ProofError::UnexpectedValue);
                };

                if got_value != expected_value.as_ref() {
                    // `key` maps to an unexpected value.
                    return Err(ProofError::ValueMismatch);
                }
            }
            Some(ValueDigest::Hash(got_hash)) => {
                // This proof proves that `key` maps to a value
                // whose hash is `got_hash`.
                let Some(expected_value) = expected_value else {
                    // We were expecting `key` to map to None.
                    return Err(ProofError::UnexpectedValue);
                };

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
    ) -> Result<Option<ValueDigest>, ProofError> {
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
            if node.key.len() % 2 != 0 && node.value_digest.is_some() {
                return Err(ProofError::ValueAtOddNibbleLength);
            }

            if i != self.0.len() - 1 {
                // Assert that every node's key is a prefix of the proven key,
                // with the exception of the last node, which is a suffix of the
                // proven key in exclusion proofs.
                let next_nibble = next_nibble(&mut node.key.iter(), &mut key.iter())?;

                let Some(next_nibble) = next_nibble else {
                    return Err(ProofError::ShouldBePrefixOfProvenKey);
                };

                expected_hash = node
                    .child_hashes
                    .get(next_nibble as usize)
                    .ok_or(ProofError::ChildIndexOutOfBounds)?
                    .as_ref()
                    .ok_or(ProofError::NodeNotInTrie)?;

                // Assert that each node's key is a prefix of the next node's key.
                #[allow(clippy::indexing_slicing)]
                let next_node_key = &self.0[i + 1].key;
                if !is_prefix(&mut node.key.iter(), &mut next_node_key.iter()) {
                    return Err(ProofError::ShouldBePrefixOfNextKey);
                }
            }
        }

        if last_node.key.len() == key.len() {
            // This is an inclusion proof.
            return Ok(last_node.value_digest.as_ref().map(|value| {
                if value.len() < 32 {
                    ValueDigest::Value(value)
                } else {
                    // TODO danlaine: I think we can remove this copy by not
                    // requiring Hash to own its data.
                    ValueDigest::Hash(value.to_vec().into_boxed_slice())
                }
            }));
        }

        todo!()
    }
}

/// Returns the next nibble in `c` after `b`.
/// Returns an error if `b` is not a prefix of `c`.
fn next_nibble<'a, I>(b: &mut I, c: &mut I) -> Result<Option<u8>, ProofError>
where
    I: Iterator<Item = &'a u8>,
{
    // Check if b is a prefix of c
    for b_item in b {
        match c.next() {
            Some(c_item) if b_item == c_item => continue,
            _ => return Err(ProofError::ShouldBePrefixOfNextKey),
        }
    }

    // If a is a prefix, return the first element in c after b
    Ok(c.next().copied())
}

fn is_prefix<'a, I>(b: &mut I, c: &mut I) -> bool
where
    I: Iterator<Item = &'a u8>,
{
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
