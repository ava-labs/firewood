// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#[cfg(test)]
use std::{cell::RefCell, collections::HashMap, rc::Rc};

use firewood_storage::{LinearAddress, TrieHash};

#[derive(Debug, thiserror::Error)]
pub enum RootStoreError {
    #[error("Failed to add root")]
    Add,
    #[error("Failed to get root")]
    Get,
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
