// Copyright (C) 2024, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::proof::{Proof, ProofCollection};

// #[cfg(not(feature = "branch_factor_256"))]
// const TOKEN_SIZE: usize = 4;
// #[cfg(feature = "branch_factor_256")]
// const TOKEN_SIZE: usize = 8;

/// A range proof proves that a given set of key-value pairs
/// are in the trie with a given root hash.
#[derive(Debug)]
pub struct RangeProof<K, V, H> {
    start_proof: Proof<H>,
    end_proof: Proof<H>,
    key_values: Box<[(K, V)]>,
}

impl<K, V, H> RangeProof<K, V, H>
where
    K: AsRef<[u8]>,
    V: AsRef<[u8]>,
    H: ProofCollection,
{
    /// Create a new range proof with the given start and end proofs
    /// and the key-value pairs that are included in the proof.
    #[must_use]
    pub const fn new(
        start_proof: Proof<H>,
        end_proof: Proof<H>,
        key_values: Box<[(K, V)]>,
    ) -> Self {
        Self {
            start_proof,
            end_proof,
            key_values,
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

    /// Returns the key-value pairs included in the range proof, which may be empty.
    #[must_use]
    pub const fn key_values(&self) -> &[(K, V)] {
        &self.key_values
    }

    /// Returns true if the range proof is empty, meaning it has no start or end proof
    /// and no key-value pairs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.start_proof.is_empty() && self.end_proof.is_empty() && self.key_values.is_empty()
    }
}
