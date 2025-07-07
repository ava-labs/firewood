// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::sync::Arc;

use arc_swap::ArcSwap;

use crate::{FileIoError, LinearAddress, NodeReader, SharedNode};

/// A node that is either in memory or on disk.
///
/// In-memory nodes can be moved to disk. This structure allows that to happen
/// atomically.
#[derive(Debug, Clone)]
pub struct MemDiskNode(Arc<ArcSwap<MemDiskEnum>>);

impl From<SharedNode> for MemDiskNode {
    fn from(node: SharedNode) -> Self {
        MemDiskNode(Arc::new(ArcSwap::new(Arc::new(MemDiskEnum::Mem(node)))))
    }
}

impl From<LinearAddress> for MemDiskNode {
    fn from(address: LinearAddress) -> Self {
        MemDiskNode(Arc::new(ArcSwap::new(Arc::new(MemDiskEnum::Disk(address)))))
    }
}

impl MemDiskNode {
    /// Converts this `MemDiskNode` to a `SharedNode` by reading from the appropriate source.
    ///
    /// If the node is in memory, it returns a clone of the in-memory node.
    /// If the node is on disk, it reads the node from storage using the provided `NodeReader`.
    ///
    /// # Arguments
    ///
    /// * `storage` - A reference to a `NodeReader` implementation that can read nodes from storage
    ///
    /// # Returns
    ///
    /// Returns a `Result<SharedNode, FileIoError>` where:
    /// - `Ok(SharedNode)` contains the node if successfully retrieved
    /// - `Err(FileIoError)` if there was an error reading from storage
    pub fn as_shared_node<S: NodeReader>(&self, storage: &S) -> Result<SharedNode, FileIoError> {
        match self.0.load().as_ref() {
            MemDiskEnum::Mem(node) => Ok(node.clone()),
            MemDiskEnum::Disk(address) => storage.read_node(*address),
        }
    }

    /// Allocates this node to disk by moving it from memory to the specified disk address.
    ///
    /// This method changes the internal state of the `MemDiskNode` from `Mem` to `Disk`,
    /// effectively moving the node from memory to the specified disk location.
    ///
    /// This is done atomically using the `ArcSwap` mechanism.
    ///
    /// # Arguments
    ///
    /// * `addr` - The `LinearAddress` where the node should be stored on disk
    pub fn allocate(&self, addr: LinearAddress) {
        self.0.swap(Arc::new(MemDiskEnum::Disk(addr)));
    }
}

/// The internal state of a `MemDiskNode`.
///
/// This enum represents the two possible states of a `MemDiskNode`:
/// - `Mem(SharedNode)`: The node is currently in memory
/// - `Disk(LinearAddress)`: The node is currently on disk at the specified address
#[derive(Debug)]
pub enum MemDiskEnum {
    Mem(SharedNode),
    Disk(LinearAddress),
}
