// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood::api;
use firewood::api::HashKey;

use crate::ProposalHandle;

/// Where a verified proof's proposal stands.
///
/// A verified change proof starts `Proposed`: checking its end root needs the
/// applied proposal. A successful commit moves to `Committed`, which caches
/// the root so a second commit returns it without touching the database. A
/// failed commit consumes the proposal and leaves `Pending`.
#[derive(Debug)]
pub(super) enum ProposalState<'db> {
    Pending,
    Proposed(ProposalHandle<'db>),
    Committed(Option<HashKey>),
}

impl<'db> ProposalState<'db> {
    /// Commit the proof and return the resulting root hash.
    ///
    /// `Committed` returns the cached root. `Proposed` commits the prepared
    /// proposal; if the database advanced since it was built
    /// (`ParentNotLatest`), `propose` builds one on the latest revision and
    /// that is committed instead. `Pending` commits a proposal `propose`
    /// builds now, and returns `ParentNotLatest` if the database advances
    /// while it is built, so one call builds at most one proposal.
    pub(super) fn commit(
        &mut self,
        mut propose: impl FnMut() -> Result<ProposalHandle<'db>, api::Error>,
    ) -> Result<Option<HashKey>, api::Error> {
        let (proposal, prepared) = match std::mem::replace(self, Self::Pending) {
            Self::Committed(hash) => {
                *self = Self::Committed(hash.clone());
                return Ok(hash);
            }
            Self::Proposed(proposal) => (proposal, true),
            Self::Pending => (propose()?, false),
        };

        let hash = match proposal.commit_proposal() {
            Err(api::Error::ParentNotLatest { .. }) if prepared => propose()?.commit_proposal()?,
            result => result?,
        };
        *self = Self::Committed(hash.clone());
        Ok(hash)
    }
}
