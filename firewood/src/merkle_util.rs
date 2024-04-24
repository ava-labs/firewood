// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{
    merkle::{
        proof::{Proof, ProofError},
        Merkle, TrieHash,
    },
    storage::node::LinearAddress,
};
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum DataStoreError {
    #[error("failed to insert data")]
    InsertionError,
    #[error("failed to remove data")]
    RemovalError,
    #[error("failed to get data")]
    GetError,
    #[error("failed to generate root hash")]
    RootHashError,
    #[error("failed to dump data")]
    DumpError,
    #[error("invalid utf8")]
    UTF8Error,
    #[error("bad proof")]
    ProofError,
    #[error("failed to verify proof")]
    ProofVerificationError,
    #[error("no keys or values found in proof")]
    ProofEmptyKeyValuesError,
}

pub struct InMemoryMerkle {
    sentinel_addr: LinearAddress,
    merkle: Merkle,
}

impl InMemoryMerkle {
    pub fn new() -> Self {
        todo!()
    }

    pub fn insert<K: AsRef<[u8]>>(&mut self, key: K, val: Vec<u8>) -> Result<(), DataStoreError> {
        self.merkle
            .insert(key, val, self.sentinel_addr)
            .map_err(|_err| DataStoreError::InsertionError)
    }

    pub fn remove<K: AsRef<[u8]>>(&mut self, key: K) -> Result<Option<Vec<u8>>, DataStoreError> {
        self.merkle
            .remove(key, self.sentinel_addr)
            .map_err(|_err| DataStoreError::RemovalError)
    }

    pub fn get<K: AsRef<[u8]>>(&self, key: K) -> Result<Option<Box<[u8]>>, DataStoreError> {
        self.merkle
            .get(key, self.sentinel_addr)
            .map_err(|_err| DataStoreError::GetError)
    }

    pub const fn get_sentinel_address(&self) -> LinearAddress {
        self.sentinel_addr
    }

    pub fn get_merkle_mut(&mut self) -> &mut Merkle {
        &mut self.merkle
    }

    pub fn root_hash(&self) -> Result<TrieHash, DataStoreError> {
        self.merkle
            .root_hash(self.sentinel_addr)
            .map_err(|_err| DataStoreError::RootHashError)
    }

    pub fn dump(&self) -> Result<String, DataStoreError> {
        let mut s = Vec::new();
        self.merkle
            .dump(self.sentinel_addr, &mut s)
            .map_err(|_err| DataStoreError::DumpError)?;
        String::from_utf8(s).map_err(|_err| DataStoreError::UTF8Error)
    }

    pub fn prove<K: AsRef<[u8]>>(&self, key: K) -> Result<Proof<Vec<u8>>, DataStoreError> {
        self.merkle
            .prove(key, self.sentinel_addr)
            .map_err(|_err| DataStoreError::ProofError)
    }

    pub fn verify_proof<N: AsRef<[u8]> + Send, K: AsRef<[u8]>>(
        &self,
        key: K,
        proof: &Proof<N>,
    ) -> Result<Option<Vec<u8>>, DataStoreError> {
        let hash: [u8; 32] = *self.root_hash()?;
        proof
            .verify(key, hash)
            .map_err(|_err| DataStoreError::ProofVerificationError)
    }

    pub fn verify_range_proof<N: AsRef<[u8]> + Send, K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        proof: &Proof<N>,
        first_key: K,
        last_key: K,
        keys: Vec<K>,
        vals: Vec<V>,
    ) -> Result<bool, ProofError> {
        let hash: [u8; 32] = *self.root_hash()?;
        proof.verify_range_proof(hash, first_key, last_key, keys, vals)
    }
}
