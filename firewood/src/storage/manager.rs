// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::{collections::VecDeque, sync::Arc};

use super::linear::{
    filebacked::FileBacked,
    historical::Historical,
    proposed::{Proposed, ProposedImmutable},
    Immutable, LinearStoreParent, Mutable, ReadLinearStore,
};

pub(super) struct LinearStorePool {
    filebacked: Arc<FileBacked>,
    historical: VecDeque<Arc<Historical>>,
    proposals: Vec<Arc<Proposed<Immutable>>>,
    committing_proposals: VecDeque<Arc<Proposed<Immutable>>>,
}

type CommitError = ();

impl LinearStorePool {
    // fn commit(&mut self, proposal: Arc<Proposed<Immutable>>) {
    //     let new_historical = self.prepare_for_writes(proposal);
    //     // tell revision manager that R4 is new_historical
    //     // do the writes
    //     self.writes_completed();
    // }

    fn prepare_for_writes(
        &mut self,
        proposal: Arc<Proposed<Immutable>>,
    ) -> Result<Arc<Historical>, CommitError> {
        // check to see if we can commit this proposal
        match proposal.parent {
            LinearStoreParent::FileBacked(_) => {
                if !self.committing_proposals.is_empty() {
                    return Err(());
                }
            }
            LinearStoreParent::Proposed(ref parent_proposal) => {
                let Some(last_commiting_proposal) = self.committing_proposals.back() else {
                    return Err(());
                };
                if !Arc::ptr_eq(parent_proposal, last_commiting_proposal) {
                    return Err(());
                }
            }
            _ => return Err(()),
        }
        // safe to commit

        let new_historical = Historical::new(
            std::mem::take(&mut proposal.old.clone()), // TODO: remove clone
            proposal.parent.clone(),
            proposal.size().map_err(|_| ())?,
        );

        // todo:
        // tell revision manager R4 is now backed by new_historical instead of FileBacked
        // flush all of "proposal" to disk
        // reparent all existing proposals as needed
        // tell revision manager P1 is R5

        Ok(Arc::new(new_historical))
    }
}

pub type NewProposalError = (); // TODO implement

impl LinearStorePool {
    pub fn add_proposal(&mut self, proposal: Arc<ProposedImmutable>) {
        self.proposals.push(proposal);
    }
}

#[cfg(test)]
mod tests {
    // TODO
}
