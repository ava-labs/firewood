// Copyright (C) 2024, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::merkle::_new_in_memory_merkle;
use crate::proof::{Proof, ProofError};
use storage::Preimage;
use storage::{Hashable, TrieHash};

/// A range proof proves that a given set of key-value pairs
/// are in the trie with a given root hash.
#[derive(Debug)]
pub struct RangeProof<K: AsRef<[u8]>, V: AsRef<[u8]>, H: Hashable> {
    pub(crate) start_proof: Option<Proof<H>>,
    pub(crate) end_proof: Option<Proof<H>>,
    pub(crate) key_values: Box<[(K, V)]>,
}

impl<K, V, H> RangeProof<K, V, H>
where
    K: AsRef<[u8]>,
    V: AsRef<[u8]>,
    H: Hashable,
{
    /// Returns the key-value pairs that this proof proves are in the trie.
    pub fn verify(
        &self,
        start_key: Option<K>,
        end_key: Option<K>,
        _expected_root_hash: TrieHash,
    ) -> Result<(), ProofError> {
        if self.key_values.is_empty() && self.start_proof.is_none() && self.end_proof.is_none() {
            return Err(ProofError::EmptyProof);
        }

        let mut keys_iter = self.key_values.iter().map(|(k, _)| k).peekable();

        // If a start key is provided, verify that the first key is >= the start key.
        // TODO clone iterator and use next
        if let Some(start_key) = &start_key {
            if let Some(first_key) = keys_iter.peek() {
                if first_key.as_ref() < start_key.as_ref() {
                    return Err(ProofError::KeyBeforeRangeStart);
                }
            }
        }

        // Verify that the keys are monotonically increasing.
        while let Some(key) = keys_iter.next() {
            if let Some(next_key) = keys_iter.peek() {
                if key.as_ref() >= next_key.as_ref() {
                    return Err(ProofError::NonIncreasingKeys);
                }
            } else {
                // Verify that the last key is <= `end_key`, if it exists,
                // which transitively proves all keys are <= `end_key`.
                // TODO danlaine: do we want to check this every key (i.e. short circuit)
                // or just here at the end of iteration?
                if let Some(end_key) = end_key.as_ref() {
                    if key.as_ref() > end_key.as_ref() {
                        return Err(ProofError::KeyAfterRangeEnd);
                    }
                }
            }
        }

        if let Some(start_proof) = &self.start_proof {
            match (&start_key, self.key_values.first()) {
                (Some(start_key), None) => {
                    start_proof.verify(start_key, None::<&[u8]>, &_expected_root_hash)?;
                }
                (None, None) => {
                    // No start key was specified and there are no key-value pairs in the range.
                    // Therefore the start proof should be just the root.
                    if start_proof.0.len() == 1 {
                        return Err(ProofError::ShouldBeJustRoot);
                    }
                    let root_hash = start_proof
                        .0
                        .first()
                        .expect("proof can't be empty")
                        .to_hash();
                    if root_hash != _expected_root_hash {
                        return Err(ProofError::UnexpectedHash);
                    }
                    return Ok(());
                }
                (None, Some((smallest_key, expected_value))) => {
                    start_proof.verify(
                        smallest_key.as_ref(),
                        Some(expected_value.as_ref()),
                        &_expected_root_hash,
                    )?;
                }
                (Some(start_key), Some((smallest_key, expected_value))) => {
                    if start_key.as_ref() == smallest_key.as_ref() {
                        start_proof.verify(
                            start_key,
                            Some(expected_value.as_ref()),
                            &_expected_root_hash,
                        )?;
                    } else {
                        // Make sure there are no keys between `start_key` and `smallest_key`
                        if !range_is_empty(
                            start_proof.implied_hashes(),
                            &Some(start_key),
                            &Some(smallest_key),
                        ) {
                            return Err(ProofError::MissingKeyValue);
                        }

                        start_proof.verify(start_key, None::<&[u8]>, &_expected_root_hash)?;
                    }
                }
            }
        } else {
            // There is no start proof iff there are no key-value pairs in the range
            // and no start key was specified.
            if start_key.is_some() || self.key_values.first().is_some() {
                return Err(ProofError::MissingStartProof);
            }
        }

        if let Some(end_proof) = &self.end_proof {
            match (&end_key, self.key_values.first()) {
                (Some(end_key), None) => {
                    end_proof.verify(end_key, None::<&[u8]>, &_expected_root_hash)?;
                }
                (None, None) => {
                    // No start key was specified and there are no key-value pairs in the range.
                    // Therefore the end proof should be just the root.
                    if end_proof.0.len() == 1 {
                        return Err(ProofError::ShouldBeJustRoot);
                    }
                    let root_hash = end_proof.0.first().expect("proof can't be empty").to_hash();
                    if root_hash != _expected_root_hash {
                        return Err(ProofError::UnexpectedHash);
                    }
                    return Ok(());
                }
                (None, Some((biggest_key, expected_value))) => {
                    end_proof.verify(
                        biggest_key.as_ref(),
                        Some(expected_value.as_ref()),
                        &_expected_root_hash,
                    )?;
                }
                (Some(end_key), Some((biggest_key, expected_value))) => {
                    if end_key.as_ref() == biggest_key.as_ref() {
                        end_proof.verify(
                            end_key,
                            Some(expected_value.as_ref()),
                            &_expected_root_hash,
                        )?;
                    } else {
                        // Make sure there are no keys between `biggest_key` and `end_key`
                        if !range_is_empty(
                            end_proof.implied_hashes(),
                            &Some(biggest_key),
                            &Some(end_key),
                        ) {
                            return Err(ProofError::MissingKeyValue);
                        }

                        end_proof.verify(end_key, None::<&[u8]>, &_expected_root_hash)?;
                    }
                }
            }
        } else {
            // There is no end proof iff there are no key-value pairs in the range
            // and no end key was specified.
            if end_key.is_some() || self.key_values.first().is_some() {
                return Err(ProofError::MissingEndProof);
            }
        }

        // Insert all key-value paris into an empty trie.
        let mut merkle = _new_in_memory_merkle();

        for (key, value) in self.key_values.iter() {
            merkle.insert(key.as_ref(), Box::from(value.as_ref()))?;
        }

        let merkle = merkle.hash();

        let Some(_root) = merkle.root() else {
            debug_assert!(self.key_values.is_empty());
            // The start and end proof, which we've verified are valid, should imply
            // there are no key-value pairs in the trie between the start and end key.
            if let Some(start_proof) = &self.start_proof {
                if !range_is_empty(start_proof.implied_hashes(), &start_key, &end_key) {
                    return Err(ProofError::MissingKeyValue);
                }
            }

            if let Some(end_proof) = &self.end_proof {
                if !range_is_empty(end_proof.implied_hashes(), &start_key, &end_key) {
                    return Err(ProofError::MissingKeyValue);
                }
            }

            return Ok(());
        };

        if self.start_proof.is_none() && self.end_proof.is_none() {
            // There are no start/end proofs to augment the hashes on the
            // left and right sides of the trie respectively.
            // The hash should already be correct.
            if let Some(root_hash) = merkle.root_hash()? {
                if root_hash != _expected_root_hash {
                    return Err(ProofError::UnexpectedHash);
                }
                return Ok(());
            } else {
                return Err(ProofError::UnexpectedEmptyTrie);
            }
        }

        todo!()
    }
}

// Returns true iff there are no keys in `implied_hashes` that are between
// `start_key` and `end_key` (exclusive) that map to Some(hash).
// A None `start_key` is considered to be before all keys.
// A None `end_key` is considered to be after all keys.
fn range_is_empty<'a, T, K>(implied_hashes: T, start_key: &Option<K>, end_key: &Option<K>) -> bool
where
    T: Iterator<Item = (Box<[u8]>, Option<&'a TrieHash>)>,
    K: AsRef<[u8]>,
{
    for (key, hash) in implied_hashes {
        if hash.is_some() {
            match (&start_key, &end_key) {
                (None, None) => {
                    return false;
                }
                (None, Some(end_key)) => {
                    if key.as_ref() < end_key.as_ref() {
                        return false;
                    }
                }
                (Some(start_key), None) => {
                    if key.as_ref() > start_key.as_ref() {
                        return false;
                    }
                }
                (Some(start_key), Some(end_key)) => {
                    if key.as_ref() > start_key.as_ref() && key.as_ref() < end_key.as_ref() {
                        return false;
                    }
                }
            }
        }
    }
    true
}