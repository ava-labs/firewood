// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

pub use crate::{
    config::DbConfig,
    storage::{buffer::DiskBufferConfig, WalConfig},
    v2::api::{Batch, BatchOp},
};
use crate::{
    merkle,
    shale::{self, disk_address::DiskAddress, LinearStore, ShaleError, Storable},
};
use crate::{
    merkle::{
        Bincode, Key, Merkle, MerkleError, MerkleKeyValueStream, Proof, ProofError, TrieHash,
    },
    storage::StoreRevShared,
    v2::api::{self, HashKey, KeyType, ValueType},
};
use aiofut::AioError;
use async_trait::async_trait;
use bytemuck::{Pod, Zeroable};

use metered::metered;
use std::{
    error::Error,
    fmt,
    io::{Cursor, Write},
    mem::size_of,
    path::Path,
    sync::Arc,
};

// TODO use or remove
const _MAGIC_STR: &[u8; 16] = b"firewood v0.1\0\0\0";

#[derive(Debug)]
#[non_exhaustive]
pub enum DbError {
    Aio(AioError),
    InvalidParams,
    Merkle(MerkleError),
    System(nix::Error),
    KeyNotFound,
    CreateError,
    Shale(ShaleError),
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
            DbError::Shale(e) => write!(f, "shale error: {e:?}"),
            DbError::InvalidProposal => write!(f, "invalid proposal"),
        }
    }
}

impl From<std::io::Error> for DbError {
    fn from(e: std::io::Error) -> Self {
        DbError::IO(e)
    }
}

impl From<ShaleError> for DbError {
    fn from(e: ShaleError) -> Self {
        DbError::Shale(e)
    }
}

impl Error for DbError {}

/// DbParams contains the constants that are fixed upon the creation of the DB, this ensures the
/// correct parameters are used when the DB is opened later (the parameters here will override the
/// parameters in [DbConfig] if the DB already exists).
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
struct DbParams {
    magic: [u8; 16],
    meta_file_nbit: u64,
    payload_file_nbit: u64,
    payload_regn_nbit: u64,
    wal_file_nbit: u64,
    wal_block_nbit: u64,
    root_hash_file_nbit: u64,
}

/// mutable DB-wide metadata, it keeps track of the root of the top-level trie.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct DbHeader {
    sentinel_addr: DiskAddress,
}

impl DbHeader {
    pub const MSIZE: u64 = std::mem::size_of::<Self>() as u64;
}

impl Storable for DbHeader {
    fn deserialize<T: LinearStore>(addr: usize, mem: &T) -> Result<Self, shale::ShaleError> {
        let root_bytes = mem
            .get_view(addr, Self::MSIZE)
            .ok_or(ShaleError::InvalidCacheView {
                offset: addr,
                size: Self::MSIZE,
            })?;
        let root_bytes = root_bytes.as_deref();
        let root_bytes = root_bytes.as_slice();

        Ok(Self {
            sentinel_addr: root_bytes
                .try_into()
                .expect("Self::MSIZE == DiskAddress:MSIZE"),
        })
    }

    fn serialized_len(&self) -> u64 {
        Self::MSIZE
    }

    fn serialize(&self, to: &mut [u8]) -> Result<(), ShaleError> {
        let mut cur = Cursor::new(to);
        cur.write_all(&self.sentinel_addr.to_le_bytes())?;
        Ok(())
    }
}

/// Some readable version of the DB.
#[derive(Debug)]
pub struct DbRev<T> {
    header: DbHeader,
    merkle: Merkle<T>,
}

#[async_trait]
impl<T: Sync> api::DbView for DbRev<T> {
    type Stream<'a> = MerkleKeyValueStream<'a,  T> where Self: 'a;

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
        first_key: Option<K>,
    ) -> Result<Self::Stream<'_>, api::Error> {
        Ok(match first_key {
            None => self.merkle.key_value_iter(self.header.sentinel_addr),
            Some(key) => self
                .merkle
                .key_value_iter_from_key(self.header.sentinel_addr, key.as_ref().into()),
        })
    }
}

impl<T> DbRev<T> {
    pub fn stream(&self) -> merkle::MerkleKeyValueStream<'_, T> {
        self.merkle.key_value_iter(self.header.sentinel_addr)
    }

    pub fn stream_from(&self, start_key: Key) -> merkle::MerkleKeyValueStream<'_, T> {
        self.merkle
            .key_value_iter_from_key(self.header.sentinel_addr, start_key)
    }

    /// Get root hash of the generic key-value storage.
    pub fn kv_root_hash(&self) -> Result<TrieHash, DbError> {
        self.merkle
            .root_hash(self.header.sentinel_addr)
            .map_err(DbError::Merkle)
    }

    /// Get a value associated with a key.
    pub fn kv_get<K: AsRef<[u8]>>(&self, key: K) -> Option<Vec<u8>> {
        let obj_ref = self.merkle.get(key, self.header.sentinel_addr);
        match obj_ref {
            Err(_) => None,
            Ok(obj) => obj.map(|o| o.to_vec()),
        }
    }

    /// Dump the Trie of the generic key-value storage.
    pub fn kv_dump(&self, w: &mut dyn Write) -> Result<(), DbError> {
        self.merkle
            .dump(self.header.sentinel_addr, w)
            .map_err(DbError::Merkle)
    }

    pub fn prove<K: AsRef<[u8]>>(&self, key: K) -> Result<Proof<Vec<u8>>, MerkleError> {
        self.merkle.prove::<K>(key, self.header.sentinel_addr)
    }

    /// Verifies a range proof is valid for a set of keys.
    pub fn verify_range_proof<N: AsRef<[u8]> + Send, K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        proof: Proof<N>,
        first_key: K,
        last_key: K,
        keys: Vec<K>,
        values: Vec<V>,
    ) -> Result<bool, ProofError> {
        let hash: [u8; 32] = *self.kv_root_hash()?;
        let valid =
            proof.verify_range_proof::<K, V, Bincode>(hash, first_key, last_key, keys, values)?;
        Ok(valid)
    }
}

pub struct Proposal {}

#[async_trait]
impl api::Proposal for Proposal {
    type Proposal = Proposal;

    #[allow(clippy::unwrap_used)]
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
    type Stream<'a> = MerkleKeyValueStream<'a, Bincode>;

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

#[async_trait]
impl api::Db for Db {
    type Historical = DbRev<StoreRevShared>;

    type Proposal = Proposal;

    /// TODO danlaine: delete or implement
    async fn revision(&self, _root_hash: HashKey) -> Result<Arc<Self::Historical>, api::Error> {
        todo!()
        // let rev = self.get_revision(&TrieHash(root_hash));
        // if let Some(rev) = rev {
        //     Ok(Arc::new(rev))
        // } else {
        //     Err(api::Error::HashNotFound {
        //         provided: root_hash,
        //     })
        // }
    }

    async fn root_hash(&self) -> Result<HashKey, api::Error> {
        todo!()
        // self.kv_root_hash().map(|hash| hash.0).map_err(Into::into)
    }

    async fn propose<K: KeyType, V: ValueType>(
        &self,
        _batch: api::Batch<K, V>,
    ) -> Result<Self::Proposal, api::Error> {
        todo!()
        // self.new_proposal(batch).map_err(Into::into)
    }
}

/// Firewood database handle.
#[derive(Debug)]
pub struct Db {
    metrics: Arc<DbMetrics>,
    _cfg: DbConfig,
}

#[metered(registry = DbMetrics, visibility = pub)]
impl Db {
    // TODO danlaine: use or remove
    const _PARAM_SIZE: u64 = size_of::<DbParams>() as u64;

    pub async fn new<P: AsRef<Path>>(_db_path: P, _cfg: &DbConfig) -> Result<Self, api::Error> {
        todo!()
    }

    /// Create a proposal.
    pub(crate) fn _new_proposal<K: KeyType, V: ValueType>(
        &self,
        _data: Batch<K, V>,
    ) -> Result<Proposal, DbError> {
        todo!()
    }

    /// Dump the Trie of the latest revision.
    pub fn kv_dump(&self, _w: &mut dyn Write) -> Result<(), DbError> {
        todo!()
    }

    /// Get root hash of the latest revision.
    pub(crate) fn _kv_root_hash(&self) -> Result<TrieHash, DbError> {
        todo!()
    }

    pub fn metrics(&self) -> Arc<DbMetrics> {
        self.metrics.clone()
    }
}
