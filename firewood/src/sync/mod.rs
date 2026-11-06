// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! State-sync orchestration (see `docs/src/designs/2026-06-11-state-sync.md`).
//!
//! This module hosts the network-ignorant core of Firewood's state sync: the
//! static range-proof sync toward a single fixed target root. Firewood owns
//! the keyspace bookkeeping — which ranges are verified, in flight, or cold —
//! decides what work to hand out next, and commits verified proofs as real
//! revisions, while the caller (avalanchego) remains the transport.

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "only the state machine, itself unused outside tests, calls the endpoint arithmetic"
    )
)]
pub(crate) mod endpoint;
#[expect(
    dead_code,
    reason = "nothing outside the module's tests drives the state machine"
)]
mod state;

pub use state::WorkId;
