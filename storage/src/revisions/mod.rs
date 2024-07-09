// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

pub mod committed;
pub mod proposed;
pub mod filestore;
pub mod memstore;

use std::io::{Error, Read};
use std::fmt::Debug;

pub use proposed::{ProposedImmutable, ProposedMutable};

pub use self::committed::Committed;

/// Represents the parent of a node store.
///
/// This enum variant is used to indicate whether the parent is a proposed immutable node or a committed node.
#[derive(Debug)]
pub enum NodeStoreParent {
    /// The parent is a proposed immutable node.
    Proposed(ProposedImmutable),
    /// The parent is a committed node.
    Committed(Committed),
}

/// Trait for readable storage.
pub trait ReadableStorage: Debug + Sync + Send {
    /// Stream data from the specified address.
    ///
    /// # Arguments
    ///
    /// * `addr` - The address from which to stream the data.
    ///
    /// # Returns
    ///
    /// A `Result` containing a boxed `Read` trait object, or an `Error` if the operation fails.
    
    fn stream_from(&self, addr: u64) -> Result<Box<dyn Read>, Error>;

    /// Return the size of the underlying storage, in bytes
    fn size(&self) -> Result<u64, Error>;
}

/// Trait for writable storage.
pub trait WritableStorage: ReadableStorage {
    /// Writes the given object at the specified offset.
    ///
    /// # Arguments
    ///
    /// * `offset` - The offset at which to write the object.
    /// * `object` - The object to write.
    ///
    /// # Returns
    ///
    /// The number of bytes written, or an error if the write operation fails.
    fn write(&self, offset: u64, object: &[u8]) -> Result<usize, Error>;
}
