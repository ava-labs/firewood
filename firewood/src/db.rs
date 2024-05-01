// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::merkle::MerkleError;
use crate::proof::{Proof, ProofError};
use crate::storage::linear::historical::Historical as HistoricalStore;
use crate::storage::linear::proposed::ProposedMutable;
use crate::storage::linear::{self, ReadLinearStore};
use crate::stream::MerkleKeyValueStream;
use crate::trie_hash::TrieHash;
use crate::v2::api::{self, HashKey, KeyType, ValueType};
pub use crate::v2::api::{Batch, BatchOp};
use aiofut::AioError;
use async_trait::async_trait;

use crate::storage::manager::RevisionManager;
use metered::metered;
use std::error::Error;
use std::fmt;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use typed_builder::TypedBuilder;

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
pub struct HistoricalRev<T> {
    // TODO: add Merkle wrapping here
    _historical: Arc<T>,
}

#[async_trait]
impl<T: linear::ReadLinearStore> api::DbView for HistoricalRev<T> {
    type Stream<'a> = MerkleKeyValueStream<'a, T> where Self: 'a;

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

impl<T: ReadLinearStore> HistoricalRev<T> {
    pub fn stream(&self) -> MerkleKeyValueStream<'_, T> {
        todo!()
    }

    pub fn stream_from(&self, _start_key: &[u8]) -> MerkleKeyValueStream<'_, T> {
        todo!()
    }

    /// Get root hash of the generic key-value storage.
    pub fn kv_root_hash(&self) -> Result<TrieHash, DbError> {
        todo!()
    }

    /// Get a value associated with a key.
    pub fn get(&self, _key: &[u8]) -> Option<Vec<u8>> {
        todo!()
    }

    /// Dump the Trie of the generic key-value storage.
    pub fn dump(&self, _w: &mut dyn Write) -> Result<(), DbError> {
        todo!()
    }

    pub fn prove(&self, _key: &[u8]) -> Result<Proof<Vec<u8>>, MerkleError> {
        todo!()
    }

    /// Verifies a range proof is valid for a set of keys.
    pub fn verify_range_proof<N: AsRef<[u8]> + Send, V: AsRef<[u8]>>(
        &self,
        _proof: Proof<N>,
        _first_key: &[u8],
        _last_key: &[u8],
        _keys: Vec<&[u8]>,
        _values: Vec<V>,
    ) -> Result<bool, ProofError> {
        todo!()
    }
}

/// TODO danlaine: implement
pub struct Proposal<T> {
    _proposal: T,
}

#[async_trait]
impl<T: linear::ReadLinearStore + linear::WriteLinearStore> api::Proposal for Proposal<T> {
    type Proposal = Proposal<T>;

    async fn commit(self: Arc<Self>) -> Result<(), api::Error> {
        todo!()
    }

    async fn propose<K: api::KeyType, V: api::ValueType>(
        self: Arc<Self>,
        _data: api::Batch<K, V>,
    ) -> Result<Arc<Self::Proposal>, api::Error> {
        todo!()
    }
}

#[async_trait]
impl<T: linear::ReadLinearStore + linear::WriteLinearStore> api::DbView for Proposal<T> {
    type Stream<'a> = MerkleKeyValueStream<'a, T> where T: 'a;

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

/// Database configuration.
#[derive(Clone, TypedBuilder, Debug)]
pub struct DbConfig {
    /// Whether to truncate the DB when opening it. If set, the DB will be reset and all its
    /// existing contents will be lost.
    #[builder(default = false)]
    pub truncate: bool,
}

/// TODO danlaine: implement
#[derive(Debug)]
pub struct Db {
    metrics: Arc<DbMetrics>,
    _cfg: DbConfig,
    manager: RevisionManager,
}

#[async_trait]
impl api::Db for Db {
    type Historical = HistoricalRev<HistoricalStore>;

    type Proposal = Proposal<ProposedMutable>;

    async fn revision(
        &self,
        _root_hash: HashKey,
    ) -> Result<Arc<HistoricalRev<HistoricalStore>>, api::Error> {
        let store = self.manager.revision(_root_hash)?;
        Ok(Arc::new(HistoricalRev::<HistoricalStore> {
            _historical: store,
        }))
    }

    async fn root_hash(&self) -> Result<HashKey, api::Error> {
        Ok(self.manager.root_hash()?)
    }

    async fn propose<K: KeyType, V: ValueType>(
        &self,
        _batch: api::Batch<K, V>,
    ) -> Result<Arc<Self::Proposal>, api::Error> {
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
    ) -> Result<Proposal<ProposedMutable>, DbError> {
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
