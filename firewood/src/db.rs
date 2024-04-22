// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

pub use crate::{
    config::DbConfig,
    storage::{buffer::DiskBufferConfig, WalConfig},
    v2::api::{Batch, BatchOp, Proposal},
};
use crate::{
    merkle,
    shale::{self, disk_address::DiskAddress, LinearStore, ShaleError, Storable, StoreId},
};
use crate::{
    merkle::{
        Bincode, Key, Merkle, MerkleError, MerkleKeyValueStream, Proof, ProofError, TrieHash,
    },
    storage::{CachedStore, MemStoreR, StoreDelta, StoreRevMut, StoreRevShared, StoreWrite},
    v2::api::{self, HashKey, KeyType, ValueType},
};
use aiofut::AioError;
use async_trait::async_trait;
use bytemuck::{Pod, Zeroable};

use metered::metered;
use std::{
    error::Error,
    fmt,
    io::{Cursor, ErrorKind, Write},
    mem::size_of,
    ops::Deref,
    path::Path,
    sync::Arc,
};

const MERKLE_META_STORE_ID: StoreId = 0x0;
const MERKLE_PAYLOAD_STORE_ID: StoreId = 0x1;
const ROOT_HASH_STORE_ID: StoreId = 0x2;
const RESERVED_STORE_ID: u64 = 0x1000;

const MAGIC_STR: &[u8; 16] = b"firewood v0.1\0\0\0";

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

#[derive(Clone, Debug)]
/// Necessary linear store instances bundled for a `Store`.
struct SubUniverse<T> {
    meta: T,
    payload: T,
}

impl<T> SubUniverse<T> {
    const fn new(meta: T, payload: T) -> Self {
        Self { meta, payload }
    }
}

impl SubUniverse<StoreRevShared> {
    fn to_mem_store_r(&self) -> SubUniverse<Arc<impl MemStoreR>> {
        SubUniverse {
            meta: self.meta.inner().clone(),
            payload: self.payload.inner().clone(),
        }
    }
}

impl SubUniverse<StoreRevMut> {
    fn new_from_other(&self) -> SubUniverse<StoreRevMut> {
        SubUniverse {
            meta: StoreRevMut::new_from_other(&self.meta),
            payload: StoreRevMut::new_from_other(&self.payload),
        }
    }
}

impl<T: MemStoreR + 'static> SubUniverse<Arc<T>> {
    fn rewind(
        &self,
        meta_writes: &[StoreWrite],
        payload_writes: &[StoreWrite],
    ) -> SubUniverse<StoreRevShared> {
        SubUniverse::new(
            StoreRevShared::from_ash(self.meta.clone(), meta_writes),
            StoreRevShared::from_ash(self.payload.clone(), payload_writes),
        )
    }
}

impl SubUniverse<Arc<CachedStore>> {
    fn to_mem_store_r(&self) -> SubUniverse<Arc<impl MemStoreR>> {
        SubUniverse {
            meta: self.meta.clone(),
            payload: self.payload.clone(),
        }
    }
}

fn get_sub_universe_from_deltas(
    sub_universe: &SubUniverse<Arc<CachedStore>>,
    meta_delta: StoreDelta,
    payload_delta: StoreDelta,
) -> SubUniverse<StoreRevShared> {
    SubUniverse::new(
        StoreRevShared::from_delta(sub_universe.meta.clone(), meta_delta),
        StoreRevShared::from_delta(sub_universe.payload.clone(), payload_delta),
    )
}

fn get_sub_universe_from_empty_delta(
    sub_universe: &SubUniverse<Arc<CachedStore>>,
) -> SubUniverse<StoreRevShared> {
    get_sub_universe_from_deltas(sub_universe, StoreDelta::default(), StoreDelta::default())
}

/// mutable DB-wide metadata, it keeps track of the root of the top-level trie.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct DbHeader {
    sentinel_addr: DiskAddress,
}

impl DbHeader {
    pub const MSIZE: u64 = std::mem::size_of::<Self>() as u64;

    pub const fn new_empty() -> Self {
        Self {
            sentinel_addr: DiskAddress::null(),
        }
    }
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
    header: shale::Obj<DbHeader>,
    merkle: Merkle<T, Bincode>,
}

#[async_trait]
impl<T: LinearStore> api::DbView for DbRev<T> {
    type Stream<'a> = MerkleKeyValueStream<'a, T, Bincode> where Self: 'a;

    async fn root_hash(&self) -> Result<api::HashKey, api::Error> {
        self.merkle
            .root_hash(self.header.sentinel_addr)
            .map(|h| *h)
            .map_err(|e| api::Error::IO(std::io::Error::new(ErrorKind::Other, e)))
    }

    async fn val<K: api::KeyType>(&self, key: K) -> Result<Option<Vec<u8>>, api::Error> {
        let obj_ref = self.merkle.get(key, self.header.sentinel_addr);
        match obj_ref {
            Err(e) => Err(api::Error::IO(std::io::Error::new(ErrorKind::Other, e))),
            Ok(obj) => Ok(obj.map(|inner| inner.deref().to_owned())),
        }
    }

    async fn single_key_proof<K: api::KeyType>(
        &self,
        key: K,
    ) -> Result<Option<Proof<Vec<u8>>>, api::Error> {
        self.merkle
            .prove(key, self.header.sentinel_addr)
            .map(Some)
            .map_err(|e| api::Error::IO(std::io::Error::new(ErrorKind::Other, e)))
    }

    async fn range_proof<K: api::KeyType, V>(
        &self,
        first_key: Option<K>,
        last_key: Option<K>,
        limit: Option<usize>,
    ) -> Result<Option<api::RangeProof<Vec<u8>, Vec<u8>>>, api::Error> {
        self.merkle
            .range_proof(self.header.sentinel_addr, first_key, last_key, limit)
            .await
            .map_err(|e| api::Error::InternalError(Box::new(e)))
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

impl<T: LinearStore> DbRev<T> {
    pub fn stream(&self) -> merkle::MerkleKeyValueStream<'_, T, Bincode> {
        self.merkle.key_value_iter(self.header.sentinel_addr)
    }

    pub fn stream_from(&self, start_key: Key) -> merkle::MerkleKeyValueStream<'_, T, Bincode> {
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

impl DbRev<StoreRevMut> {
    fn borrow_split(&mut self) -> (&mut shale::Obj<DbHeader>, &mut Merkle<StoreRevMut, Bincode>) {
        (&mut self.header, &mut self.merkle)
    }

    fn flush_dirty(&mut self) -> Option<()> {
        self.header.flush_dirty();
        self.merkle.flush_dirty()?;
        Some(())
    }
}

impl From<DbRev<StoreRevMut>> for DbRev<StoreRevShared> {
    fn from(mut value: DbRev<StoreRevMut>) -> Self {
        value.flush_dirty();
        DbRev {
            header: value.header,
            merkle: value.merkle.into(),
        }
    }
}

// TODO danlaine: delete or implement
type proposal = ();

#[async_trait]
impl api::Db for Db {
    type Historical = DbRev<StoreRevShared>;

    type Proposal = proposal;

    /// TODO danlaine: delete or implement
    async fn revision(&self, root_hash: HashKey) -> Result<Arc<Self::Historical>, api::Error> {
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
        self.kv_root_hash().map(|hash| hash.0).map_err(Into::into)
    }

    async fn propose<K: KeyType, V: ValueType>(
        &self,
        batch: api::Batch<K, V>,
    ) -> Result<Self::Proposal, api::Error> {
        self.new_proposal(batch).map_err(Into::into)
    }
}

/// Firewood database handle.
#[derive(Debug)]
pub struct Db {
    metrics: Arc<DbMetrics>,
    cfg: DbConfig,
}

#[metered(registry = DbMetrics, visibility = pub)]
impl Db {
    const PARAM_SIZE: u64 = size_of::<DbParams>() as u64;

    pub async fn new<P: AsRef<Path>>(_db_path: P, _cfg: &DbConfig) -> Result<Self, api::Error> {
        todo!()
    }

    /// Create a proposal.
    pub(crate) fn new_proposal<K: KeyType, V: ValueType>(
        &self,
        data: Batch<K, V>,
    ) -> Result<proposal::Proposal, DbError> {
        todo!()
    }

    /// Dump the Trie of the latest revision.
    pub fn kv_dump(&self, w: &mut dyn Write) -> Result<(), DbError> {
        todo!()
    }

    /// Get root hash of the latest revision.
    pub(crate) fn kv_root_hash(&self) -> Result<TrieHash, DbError> {
        todo!()
    }

    pub fn metrics(&self) -> Arc<DbMetrics> {
        self.metrics.clone()
    }
}
