// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// TODO: remove this once we have code that uses it
#![allow(dead_code)]

//! A LinearStore provides a view of a set of bytes at
//! a given time. A LinearStore has three different types,
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
use std::io::{Error, Read};
use std::sync::Arc;

/// A linear store used for proposals
///
/// A Proposed LinearStore supports read operations which look for the
/// changed bytes in the `new` member, and if not present, delegate to
/// their parent.
///
/// The old map is maintained because it is needed if this proposal commits.
/// The old and new maps contain the same entries unless the address extends
/// beyond the end of the base linear store, in which case only new bytes
/// exist
///
/// The parent can either be another `Proposed` store or a `FileBacked` store.
///
/// The second generic parameter specifies whether this proposal is mutable or
/// not. Mutable proposals implement the `ReadWriteLinearStore` trait
///
/// The possible combinations are:
///  - `Proposed<Proposed, ReadWrite>` (in-progress nested proposal)
///  - `Proposed<Proposed, ReadOnly>` (completed nested proposal)
///  - `Proposed<FileBacked, ReadWrite>` (first proposal on base revision)
///  - `Proposed<FileBacked, ReadOnly>` (completed first proposal on base)
///
/// Transitioning from ReadWrite to ReadOnly just prevents future mutations to
/// the proposal maps. ReadWrite proposals only exist during the application of
/// a Batch to the proposal, and are subsequently changed to ReadOnly
///
/// # How a commit works
///
/// Lets assume we have the following:
///  - bytes "on disk":   (0, 1, 2) `LinearStore<FileBacked>`
///  - bytes in proposal: (   3   ) `LinearStore<Proposed<FileBacked, ReadOnly>>`
/// that is, we're changing the second byte (1) to (3)
///
/// To commit:
///  - Convert the `LinearStore<FileBacked>` to `LinearStore<Committed>` taking the
/// old pages from the `LinearStore<Proposed<FileBacked, Readonly>>`
///  - Change any direct child proposals from `LinearStore<Proposed<Proposed, Readonly>>`
/// into `LinearStore<FileBacked>`
///  - Invalidate any other `LinearStore` that is a child of `LinearStore<FileBacked>`
///  - Flush all the `Proposed<FileBacked, ReadOnly>::new` bytes to disk
///  - Convert the `LinearStore<Proposed<FileBacked, Readonly>>` to `LinearStore<FileBacked>`
pub(super) mod filebacked;
pub mod historical;
pub mod proposed;

use crate::MemStore;

use self::filebacked::FileBacked;
use self::historical::Historical;
use self::proposed::ProposedImmutable;
mod layered;

/// All linear stores support reads
pub trait ReadLinearStore: Send + Sync + Debug {
    /// Return a `Read` object for a stream of data from a [ReadLinearStore] at a
    /// given address. Note this does not use LinearAddress to allow for reading
    /// at address 0
    fn stream_from(&self, addr: u64) -> Result<Box<dyn Read + '_>, Error>;

    /// Return the size of the underlying [ReadLinearStore]
    fn size(&self) -> Result<u64, Error>;
}

/// Some linear stores support writes
pub trait WriteLinearStore: ReadLinearStore {
    /// Write a new object at a given offset
    fn write(&mut self, offset: u64, object: &[u8]) -> Result<usize, Error>;

    /// Convert this [WriteLinearStore] into a [ProposedImmutable]
    /// which only implements ReadLinearStore, not [WriteLinearStore]
    fn freeze(self) -> crate::ProposedImmutable;
}

/// The parent of a [ReadLinearStore]
#[derive(Debug, Clone)]
pub enum LinearStoreParent {
    /// The parent is on disk
    FileBacked(Arc<FileBacked>),

    /// The parent is a proposal
    Proposed(Arc<crate::ProposedImmutable>),

    /// The parent is a historical revision
    Historical(Arc<historical::Historical>),

    /// The parent is in memory (primarily for testing)
    MemBacked(Arc<MemStore>),
}

impl PartialEq for LinearStoreParent {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::FileBacked(l0), Self::FileBacked(r0)) => Arc::ptr_eq(l0, r0),
            (Self::Proposed(l0), Self::Proposed(r0)) => Arc::ptr_eq(l0, r0),
            (Self::Historical(l0), Self::Historical(r0)) => Arc::ptr_eq(l0, r0),
            (Self::MemBacked(l0), Self::MemBacked(r0)) => Arc::ptr_eq(l0, r0),
            _ => false,
        }
    }
}

impl From<FileBacked> for LinearStoreParent {
    fn from(value: FileBacked) -> Self {
        LinearStoreParent::FileBacked(value.into())
    }
}
impl From<ProposedImmutable> for LinearStoreParent {
    fn from(value: ProposedImmutable) -> Self {
        LinearStoreParent::Proposed(value.into())
    }
}

impl From<Historical> for LinearStoreParent {
    fn from(value: Historical) -> Self {
        LinearStoreParent::Historical(value.into())
    }
}

impl From<Arc<Historical>> for LinearStoreParent {
    fn from(value: Arc<Historical>) -> Self {
        LinearStoreParent::Historical(value)
    }
}

impl From<Arc<MemStore>> for LinearStoreParent {
    fn from(value: Arc<MemStore>) -> Self {
        LinearStoreParent::MemBacked(value)
    }
}

#[cfg(test)]
impl From<MemStore> for LinearStoreParent {
    fn from(value: MemStore) -> Self {
        LinearStoreParent::MemBacked(value.into())
    }
}

impl ReadLinearStore for LinearStoreParent {
    fn stream_from(&self, addr: u64) -> Result<Box<dyn Read + '_>, Error> {
        match self {
            LinearStoreParent::FileBacked(filebacked) => filebacked.stream_from(addr),
            LinearStoreParent::Proposed(proposed) => proposed.stream_from(addr),
            LinearStoreParent::Historical(historical) => historical.stream_from(addr),
            LinearStoreParent::MemBacked(memstore) => memstore.stream_from(addr),
        }
    }

    fn size(&self) -> Result<u64, Error> {
        match self {
            LinearStoreParent::FileBacked(filebacked) => filebacked.size(),
            LinearStoreParent::Proposed(proposed) => proposed.size(),
            LinearStoreParent::Historical(historical) => historical.size(),
            LinearStoreParent::MemBacked(memstore) => memstore.size(),
        }
    }
}

pub mod memory;
