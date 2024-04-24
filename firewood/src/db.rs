// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::merkle::{self};
pub use crate::{
    config::DbConfig,
    v2::api::{Batch, BatchOp},
};
use crate::{
    merkle::{MerkleError, MerkleKeyValueStream, Proof, ProofError, TrieHash},
    v2::api::{self, HashKey, KeyType, ValueType},
};
use aiofut::AioError;
use async_trait::async_trait;

use metered::metered;
use std::{error::Error, fmt, io::Write, path::Path, sync::Arc};

// TODO use or remove
const _VERSION_STR: &[u8; 16] = b"firewood v0.1\0\0\0";

#[derive(Debug)]
#[non_exhaustive]
pub enum DbError {
    Aio(AioError),
    InvalidParams,
    Merkle(MerkleError),
    System(nix::Error),
    KeyNotFound,
    CreateError,
    IO(std::io::Error),
    InvalidProposal,
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DbError::Aio(e) => write!(f, "aio error: {e:?}"),
            DbError::InvalidParams => write!(f, "invalid parameters provided"),
            DbError::Merkle(e) => write!(f, "merkle error: {e:?}"),
            DbError::System(e) => write!(f, "system error: {e:?}"),
            DbError::KeyNotFound => write!(f, "not found"),
            DbError::CreateError => write!(f, "database create error"),
            DbError::IO(e) => write!(f, "I/O error: {e:?}"),
            DbError::InvalidProposal => write!(f, "invalid proposal"),
        }
    }
}

impl From<std::io::Error> for DbError {
    fn from(e: std::io::Error) -> Self {
        DbError::IO(e)
    }
}

impl Error for DbError {}

#[derive(Debug)]
pub struct Historical {}

#[async_trait]
impl api::DbView for Historical {
    type Stream<'a> = MerkleKeyValueStream<'a> where Self: 'a;

    async fn root_hash(&self) -> Result<api::HashKey, api::Error> {
        todo!()
    }

    async fn val<K: api::KeyType>(&self, _key: K) -> Result<Option<Vec<u8>>, api::Error> {
        todo!()
    }

    async fn single_key_proof<K: api::KeyType>(
        &self,
        _key: K,
    ) -> Result<Option<Proof<Vec<u8>>>, api::Error> {
        todo!()
    }

    async fn range_proof<K: api::KeyType, V>(
        &self,
        _first_key: Option<K>,
        _last_key: Option<K>,
        _limit: Option<usize>,
    ) -> Result<Option<api::RangeProof<Vec<u8>, Vec<u8>>>, api::Error> {
        todo!()
    }

    fn iter_option<K: KeyType>(
        &self,
        _first_key: Option<K>,
    ) -> Result<Self::Stream<'_>, api::Error> {
        todo!()
    }
}

impl Historical {
    pub fn stream(&self) -> merkle::MerkleKeyValueStream<'_> {
        todo!()
    }

    pub fn stream_from(&self, _start_key: &[u8]) -> merkle::MerkleKeyValueStream<'_> {
        todo!()
    }

    /// Get root hash of the generic key-value storage.
    pub fn kv_root_hash(&self) -> Result<TrieHash, DbError> {
        todo!()
    }

    /// Get a value associated with a key.
    pub fn get<K: AsRef<[u8]>>(&self, _key: K) -> Option<Vec<u8>> {
        todo!()
    }

    /// Dump the Trie of the generic key-value storage.
    pub fn dump(&self, _w: &mut dyn Write) -> Result<(), DbError> {
        todo!()
    }

    pub fn prove<K: AsRef<[u8]>>(&self, _key: K) -> Result<Proof<Vec<u8>>, MerkleError> {
        todo!()
    }

    /// Verifies a range proof is valid for a set of keys.
    pub fn verify_range_proof<N: AsRef<[u8]> + Send, K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        _proof: Proof<N>,
        _first_key: K,
        _last_key: K,
        _keys: Vec<K>,
        _values: Vec<V>,
    ) -> Result<bool, ProofError> {
        todo!()
    }
}

/// TODO danlaine: implement
pub struct Proposal {}

#[async_trait]
impl api::Proposal for Proposal {
    type Proposal = Proposal;

    async fn commit(self: Arc<Self>) -> Result<(), api::Error> {
        todo!()
    }

    async fn propose<K: api::KeyType, V: api::ValueType>(
        self: Arc<Self>,
        _data: api::Batch<K, V>,
    ) -> Result<Self::Proposal, api::Error> {
        todo!()
    }
}

#[async_trait]
impl api::DbView for Proposal {
    type Stream<'a> = MerkleKeyValueStream<'a>;

    async fn root_hash(&self) -> Result<api::HashKey, api::Error> {
        todo!()
    }

    async fn val<K>(&self, _key: K) -> Result<Option<Vec<u8>>, api::Error>
    where
        K: api::KeyType,
    {
        todo!()
    }

    async fn single_key_proof<K>(&self, _key: K) -> Result<Option<Proof<Vec<u8>>>, api::Error>
    where
        K: api::KeyType,
    {
        todo!()
    }

    async fn range_proof<K, V>(
        &self,
        _first_key: Option<K>,
        _last_key: Option<K>,
        _limit: Option<usize>,
    ) -> Result<Option<api::RangeProof<Vec<u8>, Vec<u8>>>, api::Error>
    where
        K: api::KeyType,
    {
        todo!();
    }

    fn iter_option<K: KeyType>(
        &self,
        _first_key: Option<K>,
    ) -> Result<Self::Stream<'_>, api::Error> {
        todo!()
    }
}

/// TODO danlaine: implement
#[derive(Debug)]
pub struct Db {
    metrics: Arc<DbMetrics>,
    _cfg: DbConfig,
}

#[async_trait]
impl api::Db for Db {
    type Historical = Historical;

    type Proposal = Proposal;

    async fn revision(&self, _root_hash: HashKey) -> Result<Arc<Self::Historical>, api::Error> {
        todo!()
    }

    async fn root_hash(&self) -> Result<HashKey, api::Error> {
        todo!()
    }

    async fn propose<K: KeyType, V: ValueType>(
        &self,
        _batch: api::Batch<K, V>,
    ) -> Result<Self::Proposal, api::Error> {
        todo!()
    }
}

#[metered(registry = DbMetrics, visibility = pub)]
impl Db {
    pub async fn new<P: AsRef<Path>>(_db_path: P, _cfg: &DbConfig) -> Result<Self, api::Error> {
        // TODO danlaine: Do intialization here and return Ok(Self { ... })
        todo!()
    }

    /// Create a proposal.
    pub fn new_proposal<K: KeyType, V: ValueType>(
        &self,
        _data: Batch<K, V>,
    ) -> Result<Proposal, DbError> {
        todo!()
    }

    /// Dump the Trie of the latest revision.
    pub fn dump(&self, _w: &mut dyn Write) -> Result<(), DbError> {
        todo!()
    }

    pub fn metrics(&self) -> Arc<DbMetrics> {
        self.metrics.clone()
    }
}
