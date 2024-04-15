// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::proposal::ProposalID;

pub struct Manager {}

pub type CommitError = (); // TODO implement

impl Manager {
    // TODO implement.
    pub fn new_proposal(&mut self) -> ProposalID {
        0
    }

    // TODO implement.
    pub fn commit_proposal(&mut self, _id: ProposalID) -> Result<ProposalID, CommitError> {
        Err(())
    }
}

#[cfg(test)]
mod tests {
    // TODO
}
