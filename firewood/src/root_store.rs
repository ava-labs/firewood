// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#[cfg(test)]
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use std::{fmt::Debug, path::Path};

use derive_where::derive_where;
use firewood_storage::{LinearAddress, TrieHash};

const FJALL_PARTITION_NAME: &str = "firewood";

#[derive(Debug)]
pub enum RootStoreMethod {
    Add,
    Get,
    New,
}

#[derive(Debug, thiserror::Error)]
#[error("A RootStore error occurred.")]
pub struct RootStoreError {
    pub method: RootStoreMethod,
    #[source]
    pub source: Box<dyn std::error::Error + Send + Sync>,
}

pub trait RootStore: Debug {
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
#[derive(Debug, Default)]
pub struct MockStore {
    roots: Arc<Mutex<HashMap<TrieHash, LinearAddress>>>,
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
            return Err(RootStoreError {
                method: RootStoreMethod::Add,
                source: "Adding roots should fail".into(),
            });
        }

        self.roots
            .lock()
            .expect("poisoned lock")
            .insert(hash.clone(), *address);
        Ok(())
    }

    fn get(&self, hash: &TrieHash) -> Result<Option<LinearAddress>, RootStoreError> {
        if self.should_fail {
            return Err(RootStoreError {
                method: RootStoreMethod::Get,
                source: "Getting roots should fail".into(),
            });
        }

        Ok(self.roots.lock().expect("poisoned lock").get(hash).copied())
    }
}

use fjall::{Config, Keyspace, PartitionCreateOptions, PartitionHandle, PersistMode};

#[derive_where(Debug)]
#[derive_where(skip_inner)]
pub struct FjallStore {
    keyspace: Keyspace,
    items: PartitionHandle,
}

impl FjallStore {
    /// Creates or opens an instance of `FjallStore`.
    ///
    /// Args:
    /// - path: the directory where `FjallStore` will write to.
    ///
    /// # Errors
    ///
    /// Will return an error if unable to create or open an instance of `FjallStore`.
    pub fn new<P: AsRef<Path>>(path: P) -> Result<FjallStore, RootStoreError> {
        let keyspace = Config::new(path).open().map_err(|e| RootStoreError {
            method: RootStoreMethod::New,
            source: Box::new(e),
        })?;
        let items = keyspace
            .open_partition(FJALL_PARTITION_NAME, PartitionCreateOptions::default())
            .map_err(|e| RootStoreError {
                method: RootStoreMethod::New,
                source: Box::new(e),
            })?;

        Ok(Self { keyspace, items })
    }
}

impl RootStore for FjallStore {
    fn add_root(&self, hash: &TrieHash, address: &LinearAddress) -> Result<(), RootStoreError> {
        self.items
            .insert(hash.to_bytes(), address.get().to_be_bytes())
            .map_err(|e| RootStoreError {
                method: RootStoreMethod::Add,
                source: Box::new(e),
            })?;

        self.keyspace
            .persist(PersistMode::Buffer)
            .map_err(|e| RootStoreError {
                method: RootStoreMethod::Add,
                source: Box::new(e),
            })
    }

    fn get(&self, hash: &TrieHash) -> Result<Option<LinearAddress>, RootStoreError> {
        let v = self
            .items
            .get(hash.to_bytes())
            .map_err(|e| RootStoreError {
                method: RootStoreMethod::Get,
                source: Box::new(e),
            })?
            .ok_or(RootStoreError {
                method: RootStoreMethod::Add,
                source: "empty value".into(),
            })?;

        let array: [u8; 8] = v.as_ref().try_into().map_err(|e| RootStoreError {
            method: RootStoreMethod::Get,
            source: Box::new(e),
        })?;

        Ok(LinearAddress::new(u64::from_be_bytes(array)))
    }
}
