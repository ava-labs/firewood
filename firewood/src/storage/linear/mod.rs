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

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io::{Error, Read, Seek};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use std::fs::File;

use self::layered::{Layer, LayeredReader};

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
mod filebacked;
mod historical;
mod layered;
mod manager;
mod proposal;
mod proposed;

#[cfg(test)]
mod tests;

#[derive(Debug)]
pub(super) enum LinearStore2 {
    Historical {
        /// (offset, value) for every area of this LinearStore that is modified in
        /// the revision after this one (i.e. `parent`).
        /// For example, if the first 3 bytes of this revision are [0,1,2] and the
        /// first 3 bytes of the next revision are [4,5,6] then this map would
        /// contain [(0, [0,1,2])].
        changed_in_parent: BTreeMap<u64, Box<[u8]>>,
        /// The state of the revision after this one.
        parent: Arc<LinearStore2>,
        size: u64,
    },
    Proposed {
        new: BTreeMap<u64, Box<[u8]>>,
        old: BTreeMap<u64, Box<[u8]>>,
        parent: Arc<LinearStore2>,
        // phantom: PhantomData<M>, TODO how to handle mutability?
    },
    FileBacked {
        path: PathBuf,
        fd: Mutex<File>,
    },
}

impl LinearStore2 {
    pub fn stream_from(&self, addr: u64) -> Result<Box<dyn Read + '_>, Error> {
        match self {
            LinearStore2::Historical {
                changed_in_parent,
                parent,
                size: _,
            } => Ok(Box::new(LayeredReader::new(
                addr,
                Layer {
                    parent: parent.clone(),
                    diffs: changed_in_parent,
                },
            ))),

            LinearStore2::Proposed {
                new,
                old: _,
                parent,
            } => Ok(Box::new(LayeredReader::new(
                addr,
                Layer {
                    parent: parent.clone(),
                    diffs: new,
                },
            ))),

            LinearStore2::FileBacked { path: _, fd } => {
                let mut fd = fd.lock().expect("p");
                fd.seek(std::io::SeekFrom::Start(addr))?;
                Ok(Box::new(fd.try_clone().expect("poisoned lock")))
            }
        }
    }

    pub fn size(&self) -> Result<u64, Error> {
        match self {
            LinearStore2::Historical { size, .. } => Ok(*size),
            LinearStore2::Proposed {
                new,
                old: _,
                parent,
            } => {
                // start with the parent size
                let parent_size = parent.size()?;
                // look at the last delta, if any, and see if it will extend the file
                Ok(new
                    .range(..)
                    .next_back()
                    .map(|(k, v)| *k + v.len() as u64)
                    .map_or_else(
                        || parent_size,
                        |delta_end| std::cmp::max(parent_size, delta_end),
                    ))
            }
            LinearStore2::FileBacked { path: _, fd } => fd
                .lock()
                .expect("poisoned lock")
                .seek(std::io::SeekFrom::End(0)),
        }
    }
}

#[derive(Debug)]
pub(super) struct LinearStore<S: ReadLinearStore> {
    state: S,
}

/// All linearstores support reads
pub(super) trait ReadLinearStore: Send + Sync + Debug {
    fn stream_from(&self, addr: u64) -> Result<Box<dyn Read + '_>, Error>;
    fn size(&self) -> Result<u64, Error>;
}

impl ReadLinearStore for Arc<dyn ReadLinearStore> {
    fn stream_from(&self, addr: u64) -> Result<Box<dyn Read + '_>, Error> {
        self.deref().stream_from(addr)
    }

    fn size(&self) -> Result<u64, Error> {
        self.deref().size()
    }
}

/// Some linear stores support updates
pub(super) trait WriteLinearStore: Debug {
    fn write(&mut self, offset: u64, object: &[u8]) -> Result<usize, Error>;
}

impl<ReadWrite: ReadLinearStore + Debug> WriteLinearStore for LinearStore<ReadWrite> {
    fn write(&mut self, _offset: u64, _bytes: &[u8]) -> Result<usize, Error> {
        todo!()
    }
}

impl<S: ReadLinearStore> ReadLinearStore for LinearStore<S> {
    fn stream_from(&self, addr: u64) -> Result<Box<dyn Read + '_>, Error> {
        self.state.stream_from(addr)
    }

    fn size(&self) -> Result<u64, Error> {
        todo!()
    }
}
