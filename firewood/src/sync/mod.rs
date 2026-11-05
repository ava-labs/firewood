// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! State-sync orchestration (see `docs/src/designs/2026-06-11-state-sync.md`).
//!
//! This module hosts the network-ignorant core of Firewood's state sync: the
//! static range-proof sync toward a single fixed target root. Firewood owns
//! the keyspace bookkeeping — which ranges are verified, in flight, or cold —
//! decides what work to hand out next, and commits verified proofs as real
//! revisions, while the caller (avalanchego) remains the transport.

pub(crate) mod endpoint;
mod state;
mod syncer;
#[cfg(test)]
mod tests;

pub use state::SyncError;
pub use state::WorkId;
pub use syncer::GetWork;
pub use syncer::Submit;
pub use syncer::Syncer;
pub use syncer::WorkItem;
pub use syncer::start_sync;
