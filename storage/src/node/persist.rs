// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use parking_lot::Mutex;
use std::{fmt::Display, ops::Deref, sync::Arc};

use crate::{AreaIndex, FileIoError, LinearAddress, Node, NodeReader, SharedNode};

/// A node that is either in memory or on disk.
///
/// In-memory nodes that can be moved to disk. This structure allows that to happen
/// atomically.
///
/// `MaybePersistedNode` owns a reference-counted pointer to a mutex-protected
/// enum representing either an un-persisted (or allocating) node or the
/// linear address of a persisted node.
///
/// This type is complicated, so here is a breakdown of the types involved:
///
/// | Item                   | Description                                            |
/// |------------------------|--------------------------------------------------------|
/// | [`MaybePersistedNode`] | Newtype wrapper around the remaining items.            |
/// | [Arc]                  | Reference counted pointer to a mutexed enum            |
/// | `Mutex`                | Protects the inner enum during updates                 |
/// | `MaybePersisted`       | Enum of either `Unpersisted` or `Persisted`            |
/// | variant `Unpersisted`  | The shared node, in memory, for unpersisted nodes      |
/// | -> [`SharedNode`]      | A `triomphe::Arc` of a [Node](`crate::Node`)           |
/// | variant `Persisted`    | The address of a persisted node.                       |
/// | -> [`LinearAddress`]   | A 64-bit address for a persisted node on disk.         |
///
/// Traversing these pointers does incur a runtime penalty.
///
/// When an `Unpersisted` node is `Persisted` using [`MaybePersistedNode::persist_at`],
/// the enum value inside the mutex is replaced under the lock. Subsequent accesses
/// to any instance of it, including any clones, will see the `Persisted` node address.
#[derive(Debug, Clone)]
pub struct MaybePersistedNode(Arc<Mutex<MaybePersisted>>);

impl PartialEq<MaybePersistedNode> for MaybePersistedNode {
    fn eq(&self, other: &MaybePersistedNode) -> bool {
        // if underlying mutex is same, this is necessary to avoid deadlock
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        *self.0.lock() == *other.0.lock()
    }
}

impl Eq for MaybePersistedNode {}

impl From<SharedNode> for MaybePersistedNode {
    fn from(node: SharedNode) -> Self {
        MaybePersistedNode(Arc::new(Mutex::new(MaybePersisted::Unpersisted(node))))
    }
}

impl From<LinearAddress> for MaybePersistedNode {
    fn from(address: LinearAddress) -> Self {
        MaybePersistedNode(Arc::new(Mutex::new(MaybePersisted::Persisted {
            address,
            area_index: None,
        })))
    }
}

impl From<&MaybePersistedNode> for Option<LinearAddress> {
    fn from(node: &MaybePersistedNode) -> Option<LinearAddress> {
        match &*node.0.lock() {
            MaybePersisted::Unpersisted(_) => None,
            MaybePersisted::Allocated { address, .. }
            | MaybePersisted::Persisted { address, .. } => Some(*address),
        }
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
        match &*self.0.lock() {
            MaybePersisted::Allocated { node, .. } | MaybePersisted::Unpersisted(node) => {
                Ok(node.clone())
            }
            MaybePersisted::Persisted { address, .. } => storage.read_node(*address),
        }
    }

    /// Resolve this node into a concrete `Node`, performing I/O when needed.
    ///
    /// Attempts to move the in-memory node without cloning when this is the sole
    /// reference. If multiple references exist, it falls back to cloning the
    /// node (similar to `reconstruct_with_parent`). If the node is persisted,
    /// it reads from storage.
    ///
    /// A clone here could be expensive, particularly if the trie contains many
    /// materialized Unpersisted nodes, which can happen when reconstructing from
    /// an already reconstructed root. We avoid the clone unless the reconstructed
    /// revision is referenced elsewhere at the time of this call, which is
    /// unlikely in our typical use cases.
    pub fn try_into_node<S: NodeReader>(self, storage: &S) -> Result<Node, FileIoError> {
        match Arc::try_unwrap(self.0) {
            Ok(mutex) => match mutex.into_inner() {
                MaybePersisted::Unpersisted(shared_node)
                | MaybePersisted::Allocated {
                    node: shared_node, ..
                } => Ok(triomphe::Arc::unwrap_or_clone(shared_node)),
                MaybePersisted::Persisted { address, .. } => {
                    let shared = storage.read_node(address)?;
                    Ok(shared.deref().clone())
                }
            },
            Err(arc) => match &*arc.lock() {
                MaybePersisted::Unpersisted(shared_node)
                | MaybePersisted::Allocated {
                    node: shared_node, ..
                } => Ok(shared_node.deref().clone()),
                MaybePersisted::Persisted { address, .. } => {
                    let shared = storage.read_node(*address)?;
                    Ok(shared.deref().clone())
                }
            },
        }
    }
    /// Returns the linear address of the node if it is persisted on disk.
    ///
    /// # Returns
    ///
    /// Returns `Some(LinearAddress)` if the node is persisted on disk, otherwise `None`.
    #[must_use]
    pub fn as_linear_address(&self) -> Option<LinearAddress> {
        match &*self.0.lock() {
            MaybePersisted::Unpersisted(_) => None,
            MaybePersisted::Allocated { address, .. }
            | MaybePersisted::Persisted { address, .. } => Some(*address),
        }
    }

    /// Returns the persisted storage information if available.
    #[must_use]
    pub(crate) fn storage_info(&self) -> Option<(LinearAddress, Option<AreaIndex>)> {
        match &*self.0.lock() {
            MaybePersisted::Unpersisted(_) => None,
            MaybePersisted::Allocated {
                address,
                area_index,
                ..
            } => Some((*address, Some(*area_index))),
            MaybePersisted::Persisted {
                address,
                area_index,
            } => Some((*address, *area_index)),
        }
    }

    /// Returns a reference to the unpersisted node if it is unpersisted.
    ///
    /// # Returns
    ///
    /// Returns `Some(&Self)` if the node is unpersisted, otherwise `None`.
    #[must_use]
    pub fn unpersisted(&self) -> Option<&Self> {
        match &*self.0.lock() {
            MaybePersisted::Allocated { .. } | MaybePersisted::Unpersisted(_) => Some(self),
            MaybePersisted::Persisted { .. } => None,
        }
    }

    /// Updates the internal state to indicate this node is persisted at the specified disk address.
    ///
    /// This method changes the internal state of the `MaybePersistedNode` from `Mem` to `Disk`,
    /// indicating that the node has been written to the specified disk location.
    ///
    /// This is done under a `Mutex` lock.
    ///
    /// # Arguments
    ///
    /// * `addr` - The `LinearAddress` where the node has been persisted on disk
    pub fn persist_at(&self, addr: LinearAddress) {
        let mut guard = self.0.lock();
        let area_index = match &*guard {
            MaybePersisted::Allocated { area_index, .. } => Some(*area_index),
            MaybePersisted::Persisted { area_index, .. } => *area_index,
            MaybePersisted::Unpersisted(_) => None,
        };
        *guard = MaybePersisted::Persisted {
            address: addr,
            area_index,
        };
    }

    /// Updates the internal state to indicate this node is allocated at the specified disk address.
    ///
    /// This method changes the internal state of the `MaybePersistedNode` to `Allocated`,
    /// indicating that the node has been allocated on disk but is still in memory.
    ///
    /// This is done under a `Mutex` lock.
    ///
    /// # Arguments
    ///
    /// * `addr` - The `LinearAddress` where the node has been allocated on disk
    /// * `area_index` - The storage area index allocated for the node
    pub fn allocate_at(&self, addr: LinearAddress, area_index: AreaIndex) {
        let mut guard = self.0.lock();
        let node = {
            match &*guard {
                MaybePersisted::Unpersisted(node) | MaybePersisted::Allocated { node, .. } => {
                    node.clone()
                }
                MaybePersisted::Persisted { .. } => {
                    unreachable!("Cannot allocate a node that is already persisted on disk");
                }
            }
        };
        *guard = MaybePersisted::Allocated {
            address: addr,
            area_index,
            node,
        };
    }

    /// Returns the address and shared node if this node is in the Allocated state.
    ///
    /// # Returns
    ///
    /// Returns `Some((LinearAddress, SharedNode, AreaIndex))` if the node is in the
    /// Allocated state, otherwise `None`.
    #[must_use]
    pub fn allocated_info(&self) -> Option<(LinearAddress, SharedNode, AreaIndex)> {
        match &*self.0.lock() {
            MaybePersisted::Allocated {
                address,
                area_index,
                node,
            } => Some((*address, node.clone(), *area_index)),
            _ => None,
        }
    }
}

/// Display the `MaybePersistedNode` as a string.
///
/// This is used in the dump utility.
///
/// We render these:
///   For disk addresses, just the address
///   For shared nodes, the address of the [`SharedNode`] object, prefixed with a 'M'
///
/// If instead you want the node itself, use [`MaybePersistedNode::as_shared_node`] first.
impl Display for MaybePersistedNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let guard = self.0.lock();
        match &*guard {
            MaybePersisted::Unpersisted(node) => write!(f, "M{:p}", (*node).as_ptr()),
            MaybePersisted::Allocated { address, node, .. } => {
                write!(f, "A{:p}@{address}", (*node).as_ptr())
            }
            MaybePersisted::Persisted { address, .. } => write!(f, "{address}"),
        }
    }
}

/// The internal state of a `MaybePersistedNode`.
///
/// This enum represents the three possible states of a `MaybePersisted`:
/// - `Unpersisted(SharedNode)`: The node is currently in memory
/// - `Allocated`: The node is allocated on disk but being flushed to disk
/// - `Persisted`: The node is currently on disk at the specified address
#[derive(Debug)]
enum MaybePersisted {
    Unpersisted(SharedNode),
    Allocated {
        address: LinearAddress,
        area_index: AreaIndex,
        node: SharedNode,
    },
    Persisted {
        address: LinearAddress,
        area_index: Option<AreaIndex>,
    },
}

impl PartialEq for MaybePersisted {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (MaybePersisted::Unpersisted(lhs), MaybePersisted::Unpersisted(rhs)) => lhs == rhs,
            (
                MaybePersisted::Allocated {
                    address: lhs_address,
                    node: lhs_node,
                    ..
                },
                MaybePersisted::Allocated {
                    address: rhs_address,
                    node: rhs_node,
                    ..
                },
            ) => lhs_address == rhs_address && lhs_node == rhs_node,
            (
                MaybePersisted::Persisted {
                    address: lhs_address,
                    ..
                },
                MaybePersisted::Persisted {
                    address: rhs_address,
                    ..
                },
            ) => lhs_address == rhs_address,
            _ => false,
        }
    }
}

impl Eq for MaybePersisted {}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod test {
    use nonzero_ext::nonzero;

    use crate::{AreaIndex, LeafNode, MemStore, Node, NodeStore, Path};

    use super::*;

    #[test]
    fn test_maybe_persisted_node() -> Result<(), FileIoError> {
        let mem_store = MemStore::default().into();
        let store = NodeStore::new_empty_committed(mem_store);
        let node = SharedNode::new(Node::Leaf(LeafNode {
            partial_path: Path::new(),
            value: vec![0].into(),
        }));
        // create as unpersisted
        let maybe_persisted_node = MaybePersistedNode::from(node.clone());
        let addr = nonzero!(2048u64);
        assert_eq!(maybe_persisted_node.as_shared_node(&store).unwrap(), node);
        assert_eq!(
            Option::<LinearAddress>::None,
            Option::from(&maybe_persisted_node)
        );

        let addr = LinearAddress::new(addr.get()).unwrap();
        maybe_persisted_node.persist_at(addr);
        assert!(maybe_persisted_node.as_shared_node(&store).is_err());
        assert_eq!(Some(addr), Option::from(&maybe_persisted_node));
        Ok(())
    }

    #[test]
    fn test_from_linear_address() {
        let addr: LinearAddress = nonzero!(1024u64).into();
        let maybe_persisted_node = MaybePersistedNode::from(addr);
        assert_eq!(Some(addr), Option::from(&maybe_persisted_node));
    }

    #[test]
    fn test_clone_shares_underlying_shared_node() -> Result<(), FileIoError> {
        let mem_store = MemStore::default().into();
        let store = NodeStore::new_empty_committed(mem_store);
        let node = SharedNode::new(Node::Leaf(LeafNode {
            partial_path: Path::new(),
            value: vec![42].into(),
        }));

        let original = MaybePersistedNode::from(node.clone());
        let cloned = original.clone();

        // Both should be unpersisted initially
        assert_eq!(original.as_shared_node(&store).unwrap(), node);
        assert_eq!(cloned.as_shared_node(&store).unwrap(), node);
        assert_eq!(Option::<LinearAddress>::None, Option::from(&original));
        assert_eq!(Option::<LinearAddress>::None, Option::from(&cloned));

        // First reference is 'node', second is shared by original and cloned
        assert_eq!(triomphe::Arc::strong_count(&node), 2);

        // Persist the original
        let addr = nonzero!(1024u64).into();
        original.persist_at(addr);

        // Both original and clone should now be persisted since they share the same
        // mutex-protected pointer
        assert!(original.as_shared_node(&store).is_err());
        assert!(cloned.as_shared_node(&store).is_err());
        assert_eq!(Some(addr), Option::from(&original));
        assert_eq!(Some(addr), Option::from(&cloned));

        // 'node' is no longer referenced by anyone but our local copy,
        // so the count should be 1
        assert_eq!(triomphe::Arc::strong_count(&node), 1);

        Ok(())
    }

    #[test]
    fn test_allocated_info() {
        let node = SharedNode::new(Node::Leaf(LeafNode {
            partial_path: Path::new(),
            value: vec![123].into(),
        }));

        let maybe_persisted = MaybePersistedNode::from(node.clone());

        // Initially unpersisted, so allocated_info should return None
        assert!(maybe_persisted.allocated_info().is_none());

        // Allocate the node
        let addr = LinearAddress::new(2048).unwrap();
        let area_index = AreaIndex::MIN;
        maybe_persisted.allocate_at(addr, area_index);

        // Now allocated_info should return Some
        let (retrieved_addr, retrieved_node, retrieved_area_index) =
            maybe_persisted.allocated_info().unwrap();
        assert_eq!(retrieved_addr, addr);
        assert_eq!(retrieved_node, node);
        assert_eq!(retrieved_area_index, area_index);

        // Persist the node
        maybe_persisted.persist_at(addr);

        assert_eq!(
            maybe_persisted.storage_info(),
            Some((addr, Some(area_index)))
        );

        // After persisting, allocated_info should return None again
        assert!(maybe_persisted.allocated_info().is_none());
    }

    #[test]
    fn test_persisted_equality_ignores_cached_area_index() {
        let node = SharedNode::new(Node::Leaf(LeafNode {
            partial_path: Path::new(),
            value: vec![123].into(),
        }));
        let addr = LinearAddress::new(2048).unwrap();

        let from_address = MaybePersistedNode::from(addr);
        let from_allocation = MaybePersistedNode::from(node);
        from_allocation.allocate_at(addr, AreaIndex::MIN);
        from_allocation.persist_at(addr);

        assert_eq!(from_address, from_allocation);
    }
}
