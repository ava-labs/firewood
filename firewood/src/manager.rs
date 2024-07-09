// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::io::Error;
use std::path::PathBuf;
use std::sync::Arc;

use typed_builder::TypedBuilder;

use crate::v2::api::HashKey;

use storage::{Committed, FileStore, NodeStore, ProposedImmutable};

#[derive(Clone, Debug, TypedBuilder)]
pub struct RevisionManagerConfig {
    /// The number of historical revisions to keep in memory.
    #[builder(default = 64)]
    max_revisions: usize,
}

#[derive(Debug)]
pub(crate) struct RevisionManager {
    max_revisions: usize,
    filebacked: Arc<FileStore>,
    historical: VecDeque<Arc<NodeStore<Committed, FileStore>>>,
    proposals: Vec<Arc<ProposedImmutable>>, // TODO: Should be Vec<Weak<ProposedImmutable>>
    committing_proposals: VecDeque<Arc<ProposedImmutable>>,
    // TODO: by_hash: HashMap<TrieHash, LinearStore>
}

impl RevisionManager {
    pub fn new(
        filename: PathBuf,
        truncate: bool,
        config: RevisionManagerConfig,
    ) -> Result<Self, Error> {
        Ok(Self {
            max_revisions: config.max_revisions,
            filebacked: FileStore::new(filename, truncate)?.into(),
            historical: Default::default(),
            proposals: Default::default(),
            committing_proposals: Default::default(),
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RevisionManagerError {
    #[error("The proposal cannot be committed since a sibling was committed")]
    SiblingCommitted,
    #[error(
        "The proposal cannot be committed since it is not a direct child of the most recent commit"
    )]
    NotLatest,
    #[error("An IO error occurred during the commit")]
    IO(#[from] std::io::Error),
}

impl RevisionManager {
    fn commit(&mut self, _proposal: Arc<ProposedImmutable>) -> Result<(), RevisionManagerError> {
        todo!()
    }
}

pub type NewProposalError = (); // TODO implement

impl RevisionManager {
    pub fn add_proposal(&mut self, proposal: Arc<ProposedImmutable>) {
        self.proposals.push(proposal);
    }

    pub fn revision(
        &self,
        _root_hash: HashKey,
    ) -> Result<Arc<NodeStore<Committed, FileStore>>, RevisionManagerError> {
        todo!()
    }

    pub fn root_hash(&self) -> Result<HashKey, RevisionManagerError> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    // TODO
}
