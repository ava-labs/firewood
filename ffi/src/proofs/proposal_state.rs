// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood::api::HashKey;

use crate::ProposalHandle;

/// Where a verified proof's proposal stands.
///
/// Verification always produces `Proposed`. A successful commit moves to
/// `Committed`, which caches the root so a second commit returns it without
/// touching the database. `Pending` exists only between a failed commit, which
/// consumes the proposal, and the next commit, which rebuilds one from the
/// proof.
#[derive(Debug)]
pub(super) enum ProposalState<'db> {
    Pending,
    Proposed(ProposalHandle<'db>),
    Committed(Option<HashKey>),
}
