// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::io::Error;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::v2::api::HashKey;
use storage::FileBacked;
use storage::Historical;
use storage::ProposedImmutable;
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
    historical: VecDeque<Arc<Historical>>,
    proposals: Vec<Arc<ProposedImmutable>>,
    last_commit: Arc<Historical>,
    commit_in_progress: AtomicBool,
    // TODO: by_hash: HashMap<TrieHash, LinearStore>
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
            historical: Default::default(),
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
    #[error("An IO error occurred during the commit")]
    IO(#[from] std::io::Error),
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
        self.last_commit = historical;

        self.commit_in_progress
            .compare_exchange(true, false, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| RollingRevisionManagerError::CommitAlreadyInProgress)?;

        // TODO: Async flush
        Ok(())
    }
}

pub type NewProposalError = (); // TODO implement

impl RollingRevisionManager {
    pub fn add_proposal(&mut self, proposal: Arc<ProposedImmutable>) {
        self.proposals.push(proposal);
    }

    pub fn revision(
        &self,
        _root_hash: HashKey,
    ) -> Result<Arc<Historical>, RollingRevisionManagerError> {
        todo!()
    }

    pub fn root_hash(&self) -> Result<HashKey, RollingRevisionManagerError> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    // TODO
}
