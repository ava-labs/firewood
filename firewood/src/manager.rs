// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::{collections::VecDeque, io::Error};

use typed_builder::TypedBuilder;

use crate::v2::api::HashKey;

use storage::{Committed, FileBacked, ImmutableProposal, NodeStore, TrieHash};

#[derive(Clone, Debug, TypedBuilder)]
pub struct RevisionManagerConfig {
    /// The number of historical revisions to keep in memory.
    #[builder(default = 64)]
    max_revisions: usize,
}

#[derive(Debug)]
pub(crate) struct RevisionManager {
    max_revisions: usize,
    filebacked: Arc<FileBacked>,
    historical: VecDeque<Arc<NodeStore<Committed, FileBacked>>>,
    proposals: Vec<Arc<NodeStore<ImmutableProposal, FileBacked>>>,
    // committing_proposals: VecDeque<Arc<ProposedImmutable>>,
    by_hash: HashMap<TrieHash, Arc<NodeStore<Committed, FileBacked>>>,
    // TODO: maintain root hash of the most recent commit
}

impl RevisionManager {
    pub fn new(
        filename: PathBuf,
        truncate: bool,
        config: RevisionManagerConfig,
    ) -> Result<Self, Error> {
        let storage = Arc::new(FileBacked::new(filename, truncate)?);
        let nodestore = Arc::new(NodeStore::new_empty_committed(storage.clone())?);
        let manager = Self {
            max_revisions: config.max_revisions,
            filebacked: storage,
            historical: VecDeque::from([nodestore]),
            by_hash: Default::default(),
            proposals: Default::default(),
            // committing_proposals: Default::default(),
        };
        Ok(manager)
    }

    pub fn latest_revision(&self) -> Option<Arc<NodeStore<Committed, FileBacked>>> {
        self.historical.back().cloned()
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
    pub fn commit(
        &mut self,
        _proposal: Arc<NodeStore<ImmutableProposal, FileBacked>>,
    ) -> Result<(), RevisionManagerError> {
        todo!()
    }
}

pub type NewProposalError = (); // TODO implement

impl RevisionManager {
    // TODO fix this or remove it. It should take in a proposal.
    pub fn add_proposal(&mut self, proposal: Arc<NodeStore<ImmutableProposal, FileBacked>>) {
        self.proposals.push(proposal);
    }

    pub fn revision(
        &self,
        _root_hash: HashKey,
    ) -> Result<Arc<NodeStore<Committed, FileBacked>>, RevisionManagerError> {
        todo!()
    }

    pub fn root_hash(&self) -> Result<Option<HashKey>, RevisionManagerError> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    // TODO
}
