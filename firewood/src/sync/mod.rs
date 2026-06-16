// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! State-sync orchestration (see `docs/plans/state-sync.md`).
//!
//! This module hosts the network-ignorant core of Firewood's state sync: the
//! static range-proof sync toward a single fixed target root. Firewood owns
//! the keyspace bookkeeping — which ranges are verified, in flight, or cold —
//! decides what work to hand out next, and commits verified proofs as real
//! revisions, while the caller (avalanchego) remains the transport. This
//! module provides the [`Endpoint`](crate::sync::Endpoint) key type and its
//! binary-fraction split arithmetic, the pure `SyncState` coverage machine
//! (work handout, truncation continuations, shed and refeed), and the
//! Db-coupled [`Syncer`](crate::sync::Syncer) driver created by
//! [`start_sync`](crate::sync::start_sync), which verifies and commits
//! submitted range proofs and detects completion.

pub(crate) mod endpoint;
mod state;
mod syncer;

pub use endpoint::Endpoint;
pub use state::{SyncError, WorkId};
pub use syncer::{GetWork, Submit, Syncer, WorkItem, start_sync};
