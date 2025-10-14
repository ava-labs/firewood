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
#[derive(Debug, Clone)]
pub struct MockStore {
    pub roots: Rc<RefCell<HashMap<TrieHash, LinearAddress>>>,
    pub should_add_root_fail: bool,
    pub should_get_fail: bool,
}

#[allow(clippy::new_without_default)]
#[cfg(test)]
impl MockStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            roots: Rc::new(RefCell::new(HashMap::new())),
            should_add_root_fail: false,
            should_get_fail: false,
        }
    }
}

#[cfg(test)]
impl RootStore for MockStore {
    fn add_root(&self, hash: &TrieHash, address: &LinearAddress) -> Result<(), RootStoreError> {
        if self.should_add_root_fail {
            return Err(RootStoreError::Add);
        }

        self.roots.borrow_mut().insert(hash.clone(), *address);
        Ok(())
    }

    fn get(&self, hash: &TrieHash) -> Result<Option<LinearAddress>, RootStoreError> {
        if self.should_get_fail {
            return Err(RootStoreError::Get);
        }

        Ok(self.roots.borrow().get(hash).copied())
    }
}
