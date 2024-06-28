// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.


pub mod committed;
pub mod proposed;
pub mod filebacked;
pub mod memory;

pub use proposed::{ProposedImmutable, ProposedMutable};

pub use self::committed::Committed;

#[derive(Debug)]
enum NodeStoreParent {
    Proposed(ProposedImmutable),
    Committed(Committed),
}

pub trait Immutable {}
pub trait Mutable {}

impl Immutable for ProposedImmutable {}
impl Immutable for Committed {}
impl Mutable for ProposedMutable {}

