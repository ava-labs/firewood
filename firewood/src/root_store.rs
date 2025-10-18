// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::HashMap;

use std::path::Path;
use std::sync::Mutex;
#[cfg(test)]
use std::{cell::RefCell, rc::Rc};

use firewood_storage::{LinearAddress, TrieHash};
use heed::{
    Database, Env, EnvOpenOptions, byteorder,
    types::{self, Bytes},
};
use rocksdb::{DB, Options};
use rusqlite::Connection;

#[derive(Debug, thiserror::Error)]
pub enum RootStoreError {
    #[error("Failed to add root")]
    Add,
    #[error("Failed to get root")]
    Get,
    #[error("Open error")]
    Open,
}

pub trait RootStore {
    /// `add_root` persists a revision's address to `RootStore`.
    ///
    /// Args:
    /// - hash: the hash of the revision
    /// - address: the address of the revision
    ///
    /// # Errors
    ///
    /// Will return an error if unable to persist the revision address to the
    /// underlying datastore
    fn add_root(&self, hash: &TrieHash, address: &LinearAddress) -> Result<(), RootStoreError>;

    /// `get` returns the address of a revision.
    ///
    /// Args:
    /// - hash: the hash of the revision
    ///
    /// # Errors
    ///
    ///  Will return an error if unable to query the underlying datastore.
    fn get(&self, hash: &TrieHash) -> Result<Option<LinearAddress>, RootStoreError>;
}

#[cfg_attr(test, derive(Clone))]
#[derive(Debug)]
pub struct NoOpStore {}

impl RootStore for NoOpStore {
    fn add_root(&self, _hash: &TrieHash, _address: &LinearAddress) -> Result<(), RootStoreError> {
        Ok(())
    }

    fn get(&self, _hash: &TrieHash) -> Result<Option<LinearAddress>, RootStoreError> {
        Ok(None)
    }
}

#[cfg(test)]
#[cfg_attr(test, derive(Clone))]
#[derive(Debug, Default)]
pub struct MockStore {
    roots: Rc<RefCell<HashMap<TrieHash, LinearAddress>>>,
    should_fail: bool,
}

#[cfg(test)]
impl MockStore {
    /// Returns an instance of `MockStore` that fails for all `add_root` and `get` calls.
    #[must_use]
    pub fn with_failures() -> Self {
        Self {
            should_fail: true,
            ..Default::default()
        }
    }
}

#[cfg(test)]
impl RootStore for MockStore {
    fn add_root(&self, hash: &TrieHash, address: &LinearAddress) -> Result<(), RootStoreError> {
        if self.should_fail {
            return Err(RootStoreError::Add);
        }

        self.roots.borrow_mut().insert(hash.clone(), *address);
        Ok(())
    }

    fn get(&self, hash: &TrieHash) -> Result<Option<LinearAddress>, RootStoreError> {
        if self.should_fail {
            return Err(RootStoreError::Get);
        }

        Ok(self.roots.borrow().get(hash).copied())
    }
}

pub trait RootStoreBuilder<T: RootStore> {
    /// new creates an instance of T.
    ///
    /// Args:
    /// - path: the directory where T can operate in
    ///
    /// # Errors
    ///
    /// Will return an error if unable to create the underlying datastore.
    ///
    /// XXX: this method is a facilitator for creating instances of `RootStore`
    /// during benchmarking - this will be deleted once the final implementation
    /// of `RootStore` is decided.
    #[allow(clippy::new_ret_no_self)]
    fn new<P: AsRef<Path>>(path: P) -> Result<T, RootStoreError>;
}

pub trait RootStoreBatch {
    /// `add_roots_batch` persists a set of revision addresses to `RootStore`.
    ///
    /// Args:
    /// - entries: a mapping of revision hashes to their addresses
    ///
    /// # Errors
    ///
    /// Will return an error if unable to persist the set of revision addresses
    /// to the underlying datastore.
    ///
    /// XXX: this method is required for benchmarking against pre-populated
    /// `RootStores` - this will be deleted once `RootStore` is in production.
    fn add_roots_batch(
        &self,
        entries: &HashMap<TrieHash, LinearAddress>,
    ) -> Result<(), RootStoreError>;
}

#[derive(Debug)]
pub struct LMDBStore {
    env: Env,
    db: Database<Bytes, types::U64<byteorder::BigEndian>>,
}

impl RootStoreBuilder<LMDBStore> for LMDBStore {
    fn new<P: AsRef<Path>>(path: P) -> Result<Self, RootStoreError> {
        let dir = path.as_ref().join("lmdb_store");

        // Create the directory if it doesn't exist
        if !dir.exists() {
            std::fs::create_dir_all(&dir).map_err(|_| RootStoreError::Open)?;
        }

        // SAFETY: EnvOpenOptions::open requires unsafe
        #[allow(unsafe_code)]
        let env = unsafe { EnvOpenOptions::new().map_size(100 * 1024 * 1024).open(dir) }
            .map_err(|_| RootStoreError::Open)?;

        let mut wtxn = env.write_txn().map_err(|_| RootStoreError::Open)?;
        let db: Database<Bytes, types::U64<byteorder::BigEndian>> = env
            .create_database(&mut wtxn, None)
            .map_err(|_| RootStoreError::Open)?;
        wtxn.commit().map_err(|_| RootStoreError::Open)?;

        Ok(Self { env, db })
    }
}

impl RootStore for LMDBStore {
    fn add_root(&self, hash: &TrieHash, address: &LinearAddress) -> Result<(), RootStoreError> {
        let mut wtxn = self.env.write_txn().map_err(|_| RootStoreError::Add)?;

        let hash_bytes: [u8; 32] = hash.to_bytes();

        self.db
            .put(&mut wtxn, &hash_bytes, &address.get())
            .map_err(|_| RootStoreError::Add)?;
        wtxn.commit().map_err(|_| RootStoreError::Add)?;

        Ok(())
    }

    fn get(&self, hash: &TrieHash) -> Result<Option<LinearAddress>, RootStoreError> {
        let rtxn = self.env.read_txn().map_err(|_| RootStoreError::Get)?;
        let hash_bytes: [u8; 32] = hash.to_bytes();

        let v = self
            .db
            .get(&rtxn, &hash_bytes)
            .map_err(|_| RootStoreError::Get)?
            .ok_or(RootStoreError::Get)?;

        Ok(LinearAddress::new(v))
    }
}

impl RootStoreBatch for LMDBStore {
    fn add_roots_batch(
        &self,
        entries: &HashMap<TrieHash, LinearAddress>,
    ) -> Result<(), RootStoreError> {
        let mut wtxn = self.env.write_txn().map_err(|_| RootStoreError::Add)?;

        for (hash, address) in entries {
            let hash_bytes: [u8; 32] = hash.to_bytes();
            self.db
                .put(&mut wtxn, &hash_bytes, &address.get())
                .map_err(|_| RootStoreError::Add)?;
        }

        wtxn.commit().map_err(|_| RootStoreError::Add)?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct SQLiteStore {
    db: Mutex<Connection>,
}

impl RootStoreBuilder<SQLiteStore> for SQLiteStore {
    fn new<P: AsRef<Path>>(path: P) -> Result<Self, RootStoreError> {
        let db_path = path.as_ref().join("sqlite.db");
        let conn = Connection::open(db_path).map_err(|_| RootStoreError::Open)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS roots (
                hash BLOB PRIMARY KEY NOT NULL,
                address BLOB NOT NULL
            )",
            [],
        )
        .map_err(|_| RootStoreError::Open)?;

        Ok(Self {
            db: Mutex::new(conn),
        })
    }
}

impl RootStore for SQLiteStore {
    fn add_root(&self, hash: &TrieHash, address: &LinearAddress) -> Result<(), RootStoreError> {
        let hash_bytes: [u8; 32] = hash.to_bytes();
        let addr_bytes = address.get().to_be_bytes();

        let db = self.db.lock().expect("poisoned lock");
        db.execute(
            "INSERT OR REPLACE INTO roots (hash, address) VALUES (?1, ?2)",
            (&hash_bytes, &addr_bytes),
        )
        .map_err(|_| RootStoreError::Add)?;

        Ok(())
    }

    fn get(&self, hash: &TrieHash) -> Result<Option<LinearAddress>, RootStoreError> {
        let hash_bytes: [u8; 32] = hash.to_bytes();

        let db = self.db.lock().expect("poisoned lock");

        let addr_bytes: Vec<u8> = db
            .query_row(
                "SELECT address FROM roots WHERE hash = ?1",
                [&hash_bytes],
                |row| row.get(0),
            )
            .map_err(|_| RootStoreError::Get)?;

        let addr_array: [u8; 8] = addr_bytes
            .as_slice()
            .try_into()
            .map_err(|_| RootStoreError::Get)?;
        let addr_u64 = u64::from_be_bytes(addr_array);

        Ok(LinearAddress::new(addr_u64))
    }
}

impl RootStoreBatch for SQLiteStore {
    fn add_roots_batch(
        &self,
        entries: &HashMap<TrieHash, LinearAddress>,
    ) -> Result<(), RootStoreError> {
        let db = self.db.lock().expect("poisoned lock");

        let tx = db
            .unchecked_transaction()
            .map_err(|_| RootStoreError::Add)?;

        for (hash, address) in entries {
            let hash_bytes: [u8; 32] = hash.to_bytes();
            let addr_bytes = address.get().to_be_bytes();

            tx.execute(
                "INSERT OR REPLACE INTO roots (hash, address) VALUES (?1, ?2)",
                (&hash_bytes, &addr_bytes),
            )
            .map_err(|_| RootStoreError::Add)?;
        }

        tx.commit().map_err(|_| RootStoreError::Add)?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct RocksDBStore {
    db: DB,
}

impl RootStoreBuilder<RocksDBStore> for RocksDBStore {
    fn new<P: AsRef<Path>>(path: P) -> Result<Self, RootStoreError> {
        let dir = path.as_ref().join("rocksdb_store");

        // Create the directory if it doesn't exist
        if !dir.exists() {
            std::fs::create_dir_all(&dir).map_err(|_| RootStoreError::Open)?;
        }

        let mut opts = Options::default();
        opts.create_if_missing(true);

        let db = DB::open(&opts, dir).map_err(|_| RootStoreError::Open)?;

        Ok(Self { db })
    }
}

impl RootStore for RocksDBStore {
    fn add_root(&self, hash: &TrieHash, address: &LinearAddress) -> Result<(), RootStoreError> {
        let hash_bytes: [u8; 32] = hash.to_bytes();
        let addr_bytes = address.get().to_be_bytes();

        self.db
            .put(hash_bytes, addr_bytes)
            .map_err(|_| RootStoreError::Add)?;

        Ok(())
    }

    fn get(&self, hash: &TrieHash) -> Result<Option<LinearAddress>, RootStoreError> {
        let hash_bytes: [u8; 32] = hash.to_bytes();

        let value = self
            .db
            .get(hash_bytes)
            .map_err(|_| RootStoreError::Get)?
            .ok_or(RootStoreError::Get)?;

        let addr_bytes: [u8; 8] = value
            .as_slice()
            .try_into()
            .map_err(|_| RootStoreError::Get)?;
        let addr_u64 = u64::from_be_bytes(addr_bytes);

        Ok(LinearAddress::new(addr_u64))
    }
}

impl RootStoreBatch for RocksDBStore {
    fn add_roots_batch(
        &self,
        entries: &HashMap<TrieHash, LinearAddress>,
    ) -> Result<(), RootStoreError> {
        let mut batch = rocksdb::WriteBatch::default();

        for (hash, address) in entries {
            let hash_bytes: [u8; 32] = hash.to_bytes();
            let addr_bytes = address.get().to_be_bytes();
            batch.put(hash_bytes, addr_bytes);
        }

        self.db.write(batch).map_err(|_| RootStoreError::Add)?;

        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use firewood_storage::{LinearAddress, TrieHash};

    use crate::root_store::{LMDBStore, RocksDBStore, RootStore, RootStoreBuilder, SQLiteStore};

    #[test]
    fn test_new_lmdb_store() {
        let tmpdir = tempfile::tempdir().unwrap();
        let lmdb_store = LMDBStore::new(tmpdir.path()).unwrap();

        test_simple_kv_pair(lmdb_store);

        tmpdir.close().unwrap();
    }

    #[test]
    fn test_fuzz_lmdb_store() {
        let tmpdir = tempfile::tempdir().unwrap();
        let lmdb_store = LMDBStore::new(tmpdir.path()).unwrap();

        test_fuzz(lmdb_store);

        tmpdir.close().unwrap();
    }

    #[test]
    fn test_new_sqlite_store() {
        let tmpdir = tempfile::tempdir().unwrap();
        let sqlite_store = SQLiteStore::new(&tmpdir).unwrap();

        test_simple_kv_pair(sqlite_store);

        tmpdir.close().unwrap();
    }

    #[test]
    fn test_fuzz_sqlite_store() {
        let tmpdir = tempfile::tempdir().unwrap();
        let sqlite_store = SQLiteStore::new(&tmpdir).unwrap();

        test_fuzz(sqlite_store);

        tmpdir.close().unwrap();
    }

    #[test]
    fn test_new_rocksdb_store() {
        let tmpdir = tempfile::tempdir().unwrap();
        let rocksdb_store = RocksDBStore::new(&tmpdir).unwrap();

        test_simple_kv_pair(rocksdb_store);

        tmpdir.close().unwrap();
    }

    #[test]
    fn test_fuzz_rocksdb_store() {
        let tmpdir = tempfile::tempdir().unwrap();
        let rocksdb_store = RocksDBStore::new(&tmpdir).unwrap();

        test_fuzz(rocksdb_store);

        tmpdir.close().unwrap();
    }

    fn test_simple_kv_pair<T: RootStore>(root_store: T) {
        // First, store KV-pair
        let hash = TrieHash::from_bytes([1; 32]);
        let address = LinearAddress::new(1).unwrap();

        root_store.add_root(&hash, &address).unwrap();

        // Now, get KV-pair
        assert_eq!(address, root_store.get(&hash).unwrap().unwrap());
    }

    fn test_fuzz<T: RootStore>(root_store: T) {
        let rng = firewood_storage::SeededRng::new(1234);

        // Insert 256 random key-value pairs
        for _ in 0..256 {
            let mut hash_bytes = [0u8; 32];
            rng.fill_bytes(&mut hash_bytes);
            let hash = TrieHash::from_bytes(hash_bytes);

            let addr_value = rng.random_range(1..u64::MAX);
            let address = LinearAddress::new(addr_value).unwrap();

            root_store.add_root(&hash, &address).unwrap();

            let retrieved_address = root_store.get(&hash).unwrap();
            assert_eq!(address, retrieved_address.unwrap());
        }
    }
}
