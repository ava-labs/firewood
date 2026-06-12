// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! State-sync orchestration (see `docs/plans/state-sync.md`).
//!
//! This module hosts the network-ignorant core of Firewood's state sync: the
//! static range-proof sync toward a single fixed target root. Firewood owns
//! the keyspace bookkeeping — which ranges are verified, in flight, or cold —
//! decides what work to hand out next, and commits verified proofs as real
//! revisions, while the caller (avalanchego) remains the transport. This
//! commit provides the [`Endpoint`](crate::sync::Endpoint) key type and its
//! binary-fraction split arithmetic; later commits add the `SyncState`
//! coverage machinery and the submit path.

pub(crate) mod endpoint;

pub use endpoint::Endpoint;
