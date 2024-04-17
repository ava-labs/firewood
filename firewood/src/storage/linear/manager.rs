// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::{collections::VecDeque, sync::Arc};

use super::{
    filebacked::FileBacked,
    historical::Historical,
    proposed::{Immutable, Mutable, Proposed},
    ImmutableLinearStore,
};

pub struct LinearStorePool {
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
    ) -> Result<Arc<ImmutableLinearStore>, CommitError> {
        // check to see if we can commit this proposal
        match proposal.parent.as_ref() {
            ImmutableLinearStore::FileBacked(_) => {
                if !self.committing_proposals.is_empty() {
                    return Err(());
                }
            }
            ImmutableLinearStore::Proposed(parent_proposal) => {
                // this proposal must be parented by the latest committing_proposal
                let Some(last_commiting_proposal) = self.committing_proposals.back().clone() else {
                    return Err(());
                };

                if !Arc::ptr_eq(parent_proposal, last_commiting_proposal) {
                    return Err(());
                }
                // ok to commit
            }
            ImmutableLinearStore::Historical(_) => return Err(()),
            ImmutableLinearStore::Invalid => return Err(()),
        }
        // safe to commit

        let new_historical = ImmutableLinearStore::Historical(Historical::from_current(
            proposal.old.clone(),
            proposal.parent.clone(),
            proposal.size().unwrap(), // todo remove unwrap
        ));

        // todo:
        // tell revision manager R4 is now backed by new_historical instead of FileBacked
        // flush all of "proposal" to disk
        // reparent all existing proposals as needed
        // tell revision manager P1 is R5

        Ok(new_historical.into())
    }
}

pub type NewProposalError = (); // TODO implement

impl LinearStorePool {
    pub fn add_proposal(&mut self, proposal: Proposed<Mutable>) {
        self.proposals.push(Arc::new(proposal.into()));
    }
}

#[cfg(test)]
mod tests {
    // TODO
}
