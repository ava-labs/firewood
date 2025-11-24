// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use fjall::{Config, Keyspace, PartitionCreateOptions, PartitionHandle, PersistMode};
use parking_lot::{Mutex, RwLock};
use std::{
    path::Path,
    sync::{Arc, Weak},
};
use weak_table::WeakValueHashMap;

use derive_where::derive_where;
use firewood_storage::{Committed, FileBacked, IntoHashType, LinearAddress, NodeStore, TrieHash};

use crate::manager::{CommittedRevision, CommittedRevisionCache};

const FJALL_PARTITION_NAME: &str = "firewood";

#[derive_where(Debug)]
#[derive_where(skip_inner)]
pub struct RootStore {
    keyspace: Keyspace,
    items: PartitionHandle,
    committed_revision_cache: Arc<RwLock<CommittedRevisionCache>>,
    /// Cache of reconstructed revisions by hash.
    cache: Mutex<WeakValueHashMap<TrieHash, Weak<NodeStore<Committed, FileBacked>>>>,
}

impl RootStore {
    /// Creates or opens an instance of `RootStore`.
    ///
    /// Args:
    /// - `path`: the directory where `RootStore` will write to.
    /// - `committed_revision_cache`: the cache of recently committed revisions.
    ///
    /// # Errors
    ///
    /// Will return an error if unable to create or open an instance of `RootStore`.
    pub fn new<P: AsRef<Path>>(
        path: P,
        committed_revision_cache: Arc<RwLock<CommittedRevisionCache>>,
    ) -> Result<RootStore, Box<dyn std::error::Error + Send + Sync>> {
        let keyspace = Config::new(path).open()?;
        let items =
            keyspace.open_partition(FJALL_PARTITION_NAME, PartitionCreateOptions::default())?;
        let cache = Mutex::new(WeakValueHashMap::new());

        Ok(Self {
            keyspace,
            items,
            committed_revision_cache,
            cache,
        })
    }

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
    pub fn add_root(
        &self,
        hash: &TrieHash,
        address: &LinearAddress,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.items.insert(**hash, address.get().to_be_bytes())?;

        self.keyspace.persist(PersistMode::Buffer)?;

        Ok(())
    }

    /// `get` retrieves a committed revision by its hash.
    ///
    /// To retrieve a committed revision involves a few steps:
    /// 1. Check if the committed revision is cached.
    /// 2. If the committed revision is not cached, query the underlying
    ///    datastore for the revision's root address.
    /// 3. Construct the committed revision.
    ///
    /// Args:
    /// - hash: the hash of the revision
    ///
    /// # Errors
    ///
    ///  Will return an error if unable to query the underlying datastore or if
    ///  the stored address is invalid.
    ///
    /// # Panics
    ///
    ///  Will panic if the latest revision does not exist.
    pub fn get(
        &self,
        hash: &TrieHash,
    ) -> Result<Option<CommittedRevision>, Box<dyn std::error::Error + Send + Sync>> {
        // 1. Check if the committed revision is cached.
        if let Some(v) = self.cache.lock().get(hash) {
            return Ok(Some(v));
        }

        // 2. If the committed revision is not cached, query the underlying
        //    datastore for the revision's root address.
        let Some(v) = self.items.get(**hash)? else {
            return Ok(None);
        };

        let array: [u8; 8] = v.as_ref().try_into()?;
        let addr = LinearAddress::new(u64::from_be_bytes(array))
            .ok_or("invalid address: empty address")?;

        let latest_nodestore = self
            .committed_revision_cache
            .read()
            .get_latest_revision()
            .expect("there is always one revision")
            .clone();

        // 3. Construct the committed revision.
        let nodestore = Arc::new(NodeStore::with_root(
            hash.clone().into_hash_type(),
            addr,
            latest_nodestore,
        ));

        // Cache for future lookups.
        self.cache.lock().insert(hash.clone(), nodestore.clone());

        Ok(Some(nodestore))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use firewood_storage::{CacheReadStrategy, FileBacked, NodeStore};
    use std::num::NonZero;
    use std::ops::Deref;
    use std::sync::Arc;

    struct TestRootStore {
        root_store: RootStore,
        tmpdir: tempfile::TempDir,
    }

    impl TestRootStore {
        fn new() -> Self {
            let tmpdir = tempfile::tempdir().unwrap();
            let root_store_dir = tmpdir.as_ref().join("root_store");

            let committed_revision_cache = Arc::new(RwLock::new(CommittedRevisionCache::default()));
            let root_store = RootStore::new(root_store_dir, committed_revision_cache).unwrap();

            Self { root_store, tmpdir }
        }
    }

    impl Deref for TestRootStore {
        type Target = RootStore;
        fn deref(&self) -> &Self::Target {
            &self.root_store
        }
    }

    #[test]
    fn test_cache_hit() {
        let root_store = TestRootStore::new();

        // Create a revision to cache.
        let db_path = root_store.tmpdir.as_ref().join("testdb");
        let file_backed = Arc::new(
            FileBacked::new(
                db_path,
                NonZero::new(1024).unwrap(),
                NonZero::new(1024).unwrap(),
                false,
                true,
                CacheReadStrategy::WritesOnly,
            )
            .unwrap(),
        );
        let revision = Arc::new(NodeStore::new_empty_committed(file_backed));

        let hash = TrieHash::from_bytes([1; 32]);
        root_store
            .cache
            .lock()
            .insert(hash.clone(), revision.clone());

        // Since the underlying datastore is empty, this should get the revision
        // from the cache.
        let retrieved_revision = root_store.get(&hash).unwrap().unwrap();

        assert!(Arc::ptr_eq(&revision, &retrieved_revision));
    }

    #[test]
    fn test_nonexistent_revision() {
        let root_store = TestRootStore::new();

        // Try to get a hash that doesn't exist in the cache nor in the underlying datastore.
        let nonexistent_hash = TrieHash::from_bytes([1; 32]);
        assert!(root_store.get(&nonexistent_hash).unwrap().is_none());
    }
}
