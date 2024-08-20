// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

use std::collections::HashMap;
use std::num::NonZero;
use std::path::PathBuf;
use std::sync::Arc;
use std::{collections::VecDeque, io::Error};

use typed_builder::TypedBuilder;

use crate::v2::api::HashKey;

use storage::{Committed, FileBacked, ImmutableProposal, NodeStore, Parentable, TrieHash};

#[derive(Clone, Debug, TypedBuilder)]
pub struct RevisionManagerConfig {
    /// The number of historical revisions to keep in memory.
    #[builder(default = 64)]
    max_revisions: usize,

    #[builder(default_code = "NonZero::new(1024).expect(\"non-zero\")")]
    node_cache_size: NonZero<usize>,
}

#[derive(Debug)]
pub(crate) struct RevisionManager {
    /// Maximum number of revisions to keep on disk
    max_revisions: usize,

    /// The underlying file storage
    filebacked: Arc<FileBacked>,

    /// The list of revisions that are on disk; these point to the different roots
    /// stored in the filebacked storage.
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
        let storage = Arc::new(FileBacked::new(filename, config.node_cache_size, truncate)?);
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
    /// Commit a proposal
    /// To commit a proposal involves a few steps:
    /// 1. Commit check.
    ///    The proposal’s parent must be the last committed revision, otherwise the commit fails.
    /// 2. Persist delete list.
    ///    The list of all nodes that were to be deleted for this proposal must be fully flushed to disk.
    ///    The address of the root node and the root hash is also persisted.
    ///    Note that this is *not* a write ahead log.
    ///    It only contains the address of the nodes that are deleted, which should be very small.
    /// 3. Set last committed revision.
    ///    Set last committed revision in memory.
    ///    Another commit can start after this but before the node flush is completed.
    /// 4. Free list flush.
    ///    Persist/write the free list header.
    ///    The free list is flushed first to prevent future allocations from using the space allocated to this proposal.
    ///    This should be done in a single write since the free list headers are small, and must be persisted to disk before starting the next step.
    /// 5. Node flush.
    ///    Persist/write all the nodes to disk.
    ///    Note that since these are all freshly allocated nodes, they will never be referred to by any prior commit.
    ///    After flushing all nodes, the file should be flushed to disk (fsync) before performing the next step.
    /// 6. Root move.
    ///    The root address on disk must be updated.
    ///    This write can be delayed, but would mean that recovery will not roll forward to this revision.
    /// 7. Proposal Cleanup.
    ///    Any other proposals that have this proposal as a parent should be reparented to the committed version.
    /// 8. Revision reaping.
    ///    If more than the configurable number of revisions is available, the oldest revision can be forgotten.
    pub fn commit(
        &mut self,
        proposal: Arc<NodeStore<ImmutableProposal, FileBacked>>,
    ) -> Result<(), RevisionManagerError> {
        // 1. Commit check
        let current_revision = self.current_revision();
        if !proposal
            .kind
            .parent_is(&current_revision.kind.as_nodestore_parent())
        {
            return Err(RevisionManagerError::NotLatest);
        }
        // 2. Persist delete list
        // TODO

        // 3. Set last committed revision
        let committed: Arc<NodeStore<Committed, FileBacked>> = proposal.as_committed().into();
        self.historical.push_back(committed.clone());
        if let Some(hash) = committed.kind.root_hash() {
            self.by_hash.insert(hash, committed.clone());
        }
        // TODO: We could allow other commits to start here using the pending list

        // 4. Free list flush
        proposal.flush_freelist()?;

        // 5. Node flush
        proposal.flush_nodes()?;

        // 6. Root move
        proposal.flush_header()?;

        // 7. Proposal Cleanup
        self.proposals.retain(|p| {
            // TODO: reparent proposals; this needs a lock on the parent element of immutable proposals
            // if p
            //     .kind
            //     .parent_is(&proposal.kind.as_nodestore_parent())
            // {
            //     p.kind.reparent_to(&committed.kind.as_nodestore_parent());
            // }
            !Arc::ptr_eq(&proposal, p)
        });

        Ok(())
    }
}

pub type NewProposalError = (); // TODO implement

impl RevisionManager {
    pub fn add_proposal(&mut self, proposal: Arc<NodeStore<ImmutableProposal, FileBacked>>) {
        self.proposals.push(proposal);
    }

    pub fn revision(
        &self,
        root_hash: HashKey,
    ) -> Result<Arc<NodeStore<Committed, FileBacked>>, RevisionManagerError> {
        self.by_hash
            .get(&root_hash)
            .cloned()
            .ok_or(RevisionManagerError::IO(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Revision not found",
            )))
    }

    pub fn root_hash(&self) -> Result<Option<HashKey>, RevisionManagerError> {
        self.current_revision()
            .kind
            .root_hash()
            .map(Option::Some)
            .ok_or(RevisionManagerError::IO(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Root hash not found",
            )))
    }

    fn current_revision(&self) -> Arc<NodeStore<Committed, FileBacked>> {
        self.historical
            .back()
            .expect("there is always one revision")
            .clone()
    }
}

#[cfg(test)]
mod tests {
    // TODO
}
