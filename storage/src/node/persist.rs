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
pub struct MaybePersistedNode(Arc<ArcSwap<MaybePersisted>>);

impl From<SharedNode> for MaybePersistedNode {
    fn from(node: SharedNode) -> Self {
        MaybePersistedNode(Arc::new(ArcSwap::new(Arc::new(
            MaybePersisted::Unpersisted(node),
        ))))
    }
}

impl From<LinearAddress> for MaybePersistedNode {
    fn from(address: LinearAddress) -> Self {
        MaybePersistedNode(Arc::new(ArcSwap::new(Arc::new(MaybePersisted::Persisted(
            address,
        )))))
    }
}

impl MaybePersistedNode {
    /// Converts this `MaybePersistedNode` to a `SharedNode` by reading from the appropriate source.
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
            MaybePersisted::Unpersisted(node) => Ok(node.clone()),
            MaybePersisted::Persisted(address) => storage.read_node(*address),
        }
    }

    /// Updates the internal state to indicate this node is persisted at the specified disk address.
    ///
    /// This method changes the internal state of the `MaybePersistedNode` from `Mem` to `Disk`,
    /// indicating that the node has been written to the specified disk location.
    ///
    /// This is done atomically using the `ArcSwap` mechanism.
    ///
    /// # Arguments
    ///
    /// * `addr` - The `LinearAddress` where the node has been persisted on disk
    pub fn persist_at(&self, addr: LinearAddress) {
        self.0.store(Arc::new(MaybePersisted::Persisted(addr)));
    }
}

/// The internal state of a `MaybePersistedNode`.
///
/// This enum represents the two possible states of a `MaybePersisted`:
/// - `Unpersisted(SharedNode)`: The node is currently in memory
/// - `Persisted(LinearAddress)`: The node is currently on disk at the specified address
#[derive(Debug)]
pub enum MaybePersisted {
    Unpersisted(SharedNode),
    Persisted(LinearAddress),
}
