// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::Arc;

use super::linear::filebacked::FileBacked;
use super::linear::historical::Historical;
use super::linear::proposed::ProposedImmutable;
use super::linear::{LinearStoreParent, ReadLinearStore, WriteLinearStore};

pub(super) struct RevisionManager {
    filebacked: FileBacked,
    historical: VecDeque<Arc<Historical>>,
    proposals: Vec<Arc<ProposedImmutable>>,
    committing_proposals: VecDeque<Arc<ProposedImmutable>>,
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
    fn commit(&mut self, proposal: Arc<ProposedImmutable>) -> Result<(), RevisionManagerError> {
        // detach FileBacked from all revisions to make writes safe
        let new_historical = self.prepare_for_writes(&proposal)?;

        for write in proposal.new.iter() {
            self.filebacked.write(*write.0, write.1)?;
        }

        self.writes_completed(new_historical)
    }

    fn writes_completed(
        &self,
        _new_historical: Arc<Historical>,
    ) -> Result<(), RevisionManagerError> {
        todo!()
    }

    fn prepare_for_writes(
        &mut self,
        proposal: &Arc<ProposedImmutable>,
    ) -> Result<Arc<Historical>, RevisionManagerError> {
        // check to see if we can commit this proposal
        let parent = proposal.parent.read().expect("poisoned lock").clone();
        match parent {
            LinearStoreParent::FileBacked(_) => {
                if !self.committing_proposals.is_empty() {
                    return Err(RevisionManagerError::NotLatest);
                }
            }
            LinearStoreParent::Proposed(ref parent_proposal) => {
                let Some(last_commiting_proposal) = self.committing_proposals.back() else {
                    return Err(RevisionManagerError::NotLatest);
                };
                if !Arc::ptr_eq(parent_proposal, last_commiting_proposal) {
                    return Err(RevisionManagerError::NotLatest);
                }
            }
            _ => return Err(RevisionManagerError::SiblingCommitted),
        }
        // safe to commit

        let new_historical = Arc::new(Historical::new(
            std::mem::take(&mut proposal.old.clone()), // TODO: remove clone
            parent.clone(),
            proposal.size()?,
        ));

        // for each outstanding proposal, see if their parent is the last committed linear store
        for candidate in &self.proposals {
            if *candidate.parent.read().expect("poisoned lock") == parent
                && !Arc::ptr_eq(candidate, proposal)
            {
                *candidate.parent.write().expect("poisoned lock") =
                    LinearStoreParent::Historical(new_historical.clone());
            }
        }

        Ok(new_historical)
    }
}

pub type NewProposalError = (); // TODO implement

impl RevisionManager {
    pub fn add_proposal(&mut self, proposal: Arc<ProposedImmutable>) {
        self.proposals.push(proposal);
    }
}

#[cfg(test)]
mod tests {
    // TODO
}
