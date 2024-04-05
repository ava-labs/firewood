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

mod committed;
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
mod proposed;

#[derive(Debug)]
pub(super) struct LinearStore<S: ReadLinearStore> {
    state: S,
}

/// All linearstores support reads
pub(super) trait ReadLinearStore: Debug {
    fn stream_from(&self, addr: u64) -> Result<impl Read, Error>;
    fn size(&self) -> Result<u64, Error>;
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
    fn stream_from(&self, addr: u64) -> Result<impl Read, Error> {
        self.state.stream_from(addr)
    }

    fn size(&self) -> Result<u64, Error> {
        todo!()
    }
}

#[cfg(test)]
pub mod tests {
    use super::{ReadLinearStore, WriteLinearStore};
    use std::io::Read;

    #[derive(Debug)]
    pub struct InMemReadWriteLinearStore {
        bytes: Vec<u8>,
    }

    impl InMemReadWriteLinearStore {
        pub fn new() -> Self {
            Self { bytes: vec![] }
        }
    }

    impl WriteLinearStore for InMemReadWriteLinearStore {
        fn write(&mut self, offset: u64, object: &[u8]) -> Result<usize, std::io::Error> {
            let offset = offset as usize;
            if offset + object.len() > self.bytes.len() {
                self.bytes.resize(offset + object.len(), 0);
            }
            self.bytes[offset..offset + object.len()].copy_from_slice(object);
            Ok(object.len())
        }
    }

    impl ReadLinearStore for InMemReadWriteLinearStore {
        fn stream_from(&self, addr: u64) -> Result<impl Read, std::io::Error> {
            Ok(&self.bytes[addr as usize..])
        }

        fn size(&self) -> Result<u64, std::io::Error> {
            Ok(self.bytes.len() as u64)
        }
    }

    #[test]
    fn test_in_mem_write_linear_store() {
        let mut store = InMemReadWriteLinearStore { bytes: vec![] };
        assert_eq!(store.size().unwrap(), 0);

        // Write to an empty store
        store.write(0, &[1, 2, 3]).unwrap();
        assert_eq!(store.bytes, vec![1, 2, 3]);
        assert_eq!(store.size().unwrap(), 3);

        // Write at the end of the store
        store.write(3, &[4, 5, 6]).unwrap();
        assert_eq!(store.bytes, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(store.size().unwrap(), 6);

        // Overwrite the beginning of the store
        store.write(0, &[7, 8]).unwrap();
        assert_eq!(store.bytes, vec![7, 8, 3, 4, 5, 6]);
        assert_eq!(store.size().unwrap(), 6);

        // Overwrite the middle of the store
        store.write(1, &[9, 10]).unwrap();
        assert_eq!(store.bytes, vec![7, 9, 10, 4, 5, 6]);
        assert_eq!(store.size().unwrap(), 6);

        // Write past the end of the store
        store.write(7, &[11, 12]).unwrap();
        assert_eq!(store.bytes, vec![7, 9, 10, 4, 5, 6, 0, 11, 12]);
        assert_eq!(store.size().unwrap(), 9);

        // Read from the start of the store
        let mut reader = store.stream_from(0).unwrap();
        let mut read_bytes = vec![];
        let num_read_bytes = reader.read_to_end(&mut read_bytes).unwrap();
        assert_eq!(read_bytes, vec![7, 9, 10, 4, 5, 6, 0, 11, 12]);
        assert_eq!(num_read_bytes, 9);

        // Read from the middle of the store
        let mut reader = store.stream_from(3).unwrap();
        let mut read_bytes = vec![];
        let num_read_bytes = reader.read_to_end(&mut read_bytes).unwrap();
        assert_eq!(read_bytes, vec![4, 5, 6, 0, 11, 12]);
        assert_eq!(num_read_bytes, 6);

        // Read from the end of the store
        let mut reader = store.stream_from(store.size().unwrap() - 1).unwrap();
        let mut read_bytes = vec![];
        let num_read_bytes = reader.read_to_end(&mut read_bytes).unwrap();
        assert_eq!(read_bytes, vec![12]);
        assert_eq!(num_read_bytes, 1);

        // Read from past the end of the store
        let mut reader = store.stream_from(store.size().unwrap()).unwrap();
        let mut read_bytes = vec![];
        let num_read_bytes = reader.read_to_end(&mut read_bytes).unwrap();
        assert_eq!(read_bytes, vec![]);
        assert_eq!(num_read_bytes, 0);
    }
}
