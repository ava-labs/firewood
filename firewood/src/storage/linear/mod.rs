// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// TODO: remove this once we have code that uses it
#![allow(dead_code)]

//! A [LinearStore] provides a view of a set of bytes at
//! a given time. A [LinearStore] has three different types,
//! which refer to another base type, as follows:
//! ```mermaid
//! stateDiagram-v2
//!     R1(Committed) --> R2(Committed)
//!     R2(Committed) --> R3(FileBacked)
//!     P1(Proposed) --> R3(FileBacked)
//!     P2(Proposed) --> P1(Proposed)
//! ```
//!
//! Each type is described in more detail below.

use std::fmt::Debug;
use std::io::{Cursor, Error, Read};

mod committed;
/// A linear store used for proposals
///
/// A [Proposed] [LinearStore] supports read operations which look for the
/// changed bytes in the `new` member, and if not present, delegate to
/// their parent.
///
/// The old map is maintained because it is needed if this proposal commits.
/// The old and new maps contain the same entries unless the address extends
/// beyond the end of the base linear store, in which case only new bytes
/// exist
///
/// The parent can either be another [Proposed] store or a [FileBacked] store.
///
/// The second generic parameter specifies whether this proposal is mutable or
/// not. Mutable proposals implement the [ReadWriteLinearStore] trait
///
/// The possible combinations are:
///  - `Proposed<Proposed, ReadWrite>`
///  - `Proposed<Proposed, ReadOnly>`
///  - `Proposed<FileBacked, ReadWrite>`
///  - `Proposed<FileBacked, ReadOnly>`
///
/// Transitioning from ReadWrite to ReadOnly just prevents future mutations to
/// the proposal maps
///
// As an example of how this works, lets say that you have a [FileBased] Layer
// base: [0, 1, 2, 3, 4, 5, 6, 7, 8] <-- this is on disk AKA R2
//   us: [0, 1, 2, 3, 4, 6, 6, 7, 9, 10] <-- part of a proposal AKA P1 this is the "virtual" state. Not stored in entirety in this struct.
// changes: {old: 5:[5] 8:[8]}, new: {5:[6] : 8:[9 10]}   <-- this is what is actually is stored in this struct (in `changes`).
// start streaming from index 4
// first byte I read comes from base
// second byte I read comes from change
// third byte I read comes from base

// commit means: what is on disk is no longer actually R2 and vice versa
// commit means: what is about to be on disk is in the changes list of P1 (specificially new regions)
// commit means: what was on disk is in the changes list of P1 for changed byte indices of P1 (specificaly old regions)
//  Every byte index is one of:
// * Unchanged in P1
// * Changed in P1

// read 1 byte from R1:
//  -- check and see if R2 has an "old" change at this index, if so, use that
//  -- check and see if P1 (R3) has an "old" change at this index, if so use that
//  -- not: read from disk (cache)

// physical structure - set of linear bytes
//   -- set of physical changes A
// logical structure - set of nodes interconnected
//   -- set of logical changes B
mod filebacked;
mod proposed;

#[derive(Debug)]
pub(super) struct LinearStore<T> {
    state: T,
}

/// All linearstores support reads
pub(super) trait ReadOnlyLinearStore: Debug {
    fn stream_from(&self, addr: u64) -> Result<impl Read, Error>;
}

/// Some linear stores support updates
pub(super) trait ReadWriteLinearStore: Debug {
    fn write(&mut self, offset: u64, object: &[u8]) -> Result<usize, Error>;
    fn size(&self) -> Result<u64, Error>;
}

impl<ReadWrite: Debug> ReadWriteLinearStore for LinearStore<ReadWrite> {
    fn write(&mut self, _offset: u64, _bytes: &[u8]) -> Result<usize, Error> {
        todo!()
    }

    fn size(&self) -> Result<u64, Error> {
        todo!()
    }
}

/// TODO: remove this once all the ReadOnlyLinearStore implementations are complete
impl<T: std::fmt::Debug> ReadOnlyLinearStore for LinearStore<T> {
    fn stream_from(&self, _addr: u64) -> Result<impl Read, Error> {
        Ok(Cursor::new([]))
    }
}
