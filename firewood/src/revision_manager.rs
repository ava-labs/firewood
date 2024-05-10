// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::io::Error;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::hashednode::HashedNodeStore;
use crate::merkle::{Merkle, MerkleError};
use crate::v2::api::{Batch, BatchOp, HashKey, KeyType, ValueType};
use storage::ProposedImmutable;
use storage::{FileBacked, TrieHash};
use storage::{Historical, ProposedMutable};
use storage::{LinearStoreParent, ReadLinearStore};
use typed_builder::TypedBuilder;

#[derive(Clone, Debug, TypedBuilder)]
pub struct RollingRevisionManagerConfig {
    /// The number of historical revisions to keep in memory.
    #[builder(default = 64)]
    max_revisions: usize,
}

#[derive(Debug)]
pub(crate) struct RollingRevisionManager {
    max_revisions: usize,
    filebacked: Arc<FileBacked>,
    historicals: VecDeque<(TrieHash, Arc<Historical>)>,
    proposals: Vec<(TrieHash, Arc<ProposedImmutable>)>,
    last_commit: Arc<Historical>,
    commit_in_progress: AtomicBool,
    // TODO: maintain root hash of the most recent commit
}

impl RollingRevisionManager {
    pub fn new(
        filename: PathBuf,
        truncate: bool,
        config: RollingRevisionManagerConfig,
    ) -> Result<Self, Error> {
        let filebacked = Arc::new(FileBacked::new(filename, truncate)?);
        let base_historical = Arc::new(Historical::new(
            Default::default(),
            LinearStoreParent::FileBacked(filebacked.clone()),
            0,
        ));
        Ok(Self {
            max_revisions: config.max_revisions,
            filebacked,
            historicals: Default::default(),
            proposals: Default::default(),
            last_commit: base_historical,
            commit_in_progress: AtomicBool::new(false),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RollingRevisionManagerError {
    #[error("A commit is already is progress, only one commit can be in progress at a time")]
    CommitAlreadyInProgress,
    #[error(
        "The proposal cannot be committed with a parent of type {0:?}, only Historical are allowed"
    )]
    InvalidParentType(LinearStoreParent),
    #[error(
        "The proposal cannot be committed since it is not a direct child of the most recent commit"
    )]
    NotLatest,
    #[error("The proposal cannot be committed since a sibling was committed")]
    SiblingCommitted,
    #[error("The revision does not exist")]
    NoSuchRevision,
    #[error("An IO error occurred during the commit")]
    IO(#[from] std::io::Error),
    #[error("merkle error: {0}")]
    MerkleError(#[from] MerkleError),
}

impl RollingRevisionManager {
    fn commit(
        &mut self,
        proposal: Arc<ProposedImmutable>,
    ) -> Result<(), RollingRevisionManagerError> {
        let parent = proposal.parent();
        match parent {
            LinearStoreParent::Historical(ref h) => {
                if Arc::ptr_eq(h, &self.last_commit) {
                    return Err(RollingRevisionManagerError::NotLatest);
                }
            }
            _ => return Err(RollingRevisionManagerError::InvalidParentType(parent)),
        }

        self.commit_in_progress
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| RollingRevisionManagerError::CommitAlreadyInProgress)?;

        // TODO: WAL

        // TODO: Historical do not need to old, but need new. Proposal need to only have new.
        let historical = Arc::new(Historical::new(
            std::mem::take(&mut proposal.new.clone()), // TODO: remove clone
            parent.clone(),
            proposal.size()?,
        ));

        // TODO: get the hash of historical revision
        let hash = TrieHash::default();
        self.historicals.push_back((hash, self.last_commit.clone()));
        self.last_commit = historical;

        self.commit_in_progress
            .compare_exchange(true, false, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| RollingRevisionManagerError::CommitAlreadyInProgress)?;

        // TODO: Async flush
        Ok(())
    }

    fn expire_revisions(&mut self) -> Result<(), RollingRevisionManagerError> {
        // Check if revision expioration is needed
        if self.historicals.len() < self.max_revisions {
            return Ok(());
        }

        // TODO: Signal the writer thread to stop writing and wait for it to finish and return
        // the last written historical revision hash
        let last_hash = TrieHash::default();

        // Find the index of historical revision to expire based on the last written
        // historical revision hash
        let expiry_index = self
            .historicals
            .iter()
            .position(|(hash, _)| *hash == last_hash)
            .ok_or(RollingRevisionManagerError::NoSuchRevision)?;

        if self.historicals.len() > expiry_index {
            // create a new base revision, and then reparent the rest to be its child.
            // TODO: use the updated filename, and mutex for updating historicals.
            let filename = PathBuf::from("");
            let filebacked = Arc::new(FileBacked::new(filename, false)?);
            let base_historical = Arc::new(Historical::new(
                Default::default(),
                LinearStoreParent::FileBacked(filebacked.clone()),
                0,
            ));

            let child = self
                .historicals
                .get(expiry_index)
                .ok_or(RollingRevisionManagerError::NoSuchRevision)?
                .1
                .clone();
            child.reparent(base_historical.clone().into());
        }

        // Drain the expired historical revisions to release the references
        let _ = self
            .historicals
            .drain(0..expiry_index)
            .collect::<VecDeque<_>>();

        Ok(())
    }
}

pub type NewProposalError = (); // TODO implement

impl RollingRevisionManager {
    pub fn revision(
        &self,
        _root_hash: HashKey,
    ) -> Result<Arc<Historical>, RollingRevisionManagerError> {
        todo!()
    }

    pub fn root_hash(&self) -> Result<HashKey, RollingRevisionManagerError> {
        todo!()
    }

    pub fn propose<K: KeyType, V: ValueType>(
        &mut self,
        batch: Batch<K, V>,
    ) -> Result<Arc<ProposedImmutable>, RollingRevisionManagerError> {
        let linear = ProposedMutable::new(LinearStoreParent::Historical(self.last_commit.clone()));
        let mut merkle = Merkle::new(HashedNodeStore::initialize(linear)?);
        batch
            .into_iter()
            .try_for_each(|op| -> Result<(), RollingRevisionManagerError> {
                match op {
                    BatchOp::Put { key, value } => {
                        merkle.insert(key, value.as_ref().into())?;
                        Ok(())
                    }
                    BatchOp::Delete { key } => {
                        merkle.remove(key)?;
                        Ok(())
                    }
                }
            })?;
        let root_hash = merkle.root_hash()?;
        let node_store = merkle.freeze()?;
        let linear_store = Arc::new(node_store.consume_linear_store());
        self.proposals.push((root_hash, linear_store.clone()));
        Ok(linear_store)
    }
}

#[cfg(test)]
mod tests {
    // TODO
}
