// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

pub mod committed;
pub mod filestore;
pub mod memstore;
pub mod proposed;

use std::fmt::Debug;
use std::io::{Error, Read};
use std::sync::Arc;

pub use proposed::{ProposedImmutable, ProposedMutable};

use crate::{LinearAddress, Node};

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

/// Trait for reading changed nodes. If a proposal, this recursively reads proposals until it finds a changed node
/// otherwise it returns None, indicating that the node has not changed in any parent proposal and can be read from disk
///
/// Both [Committed] and Proposed<T> implement this trait; the former always returns None.
pub trait ReadChangedNode: Debug + Send + Sync {
    /// Reads a changed node from the specified address.
    ///
    /// # Arguments
    ///
    /// * `_addr` - The linear address of the node to read.
    ///
    /// # Returns
    ///
    /// An `Option` containing an `Arc` of the read node, or `None` if the node has not changed in any parent proposal and can be read from disk.
    fn read_changed_node(&self, _addr: LinearAddress) -> Option<Arc<Node>>;
}
