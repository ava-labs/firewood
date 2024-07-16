// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

/// The [NodeStore] handles the serialization of nodes and
/// free space management of nodes in the page store. It lays out the format
/// of the [PageStore]. More specifically, it places a [FileIdentifyingMagic]
/// and a [FreeSpaceHeader] at the beginning
use std::io::{Error, ErrorKind, Write};
use std::num::NonZeroU64;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::node::Node;
use crate::revisions::proposed::{Proposed, ProposedImmutable, ProposedMutable};
use crate::revisions::Committed;
use crate::revisions::{ReadableStorage, WritableStorage};
use crate::{NodeStoreParent, ReadChangedNode};

/// [NodeStore] divides the linear store into blocks of different sizes.
/// [AREA_SIZES] is every valid block size.
const AREA_SIZES: [u64; 21] = [
    1 << MIN_AREA_SIZE_LOG, // Min block size is 8
    1 << 4,
    1 << 5,
    1 << 6,
    1 << 7,
    1 << 8,
    1 << 9,
    1 << 10,
    1 << 11,
    1 << 12,
    1 << 13,
    1 << 14,
    1 << 15,
    1 << 16,
    1 << 17,
    1 << 18,
    1 << 19,
    1 << 20,
    1 << 21,
    1 << 22,
    1 << 23, // 16 MiB
];

/// The type of an index into the [AREA_SIZES] array
/// This is not usize because we can store this as a single byte
pub type AreaIndex = u8;

// TODO danlaine: have type for index in AREA_SIZES
// Implement try_into() for it.
const MIN_AREA_SIZE_LOG: AreaIndex = 3;
const NUM_AREA_SIZES: usize = AREA_SIZES.len();
const MIN_AREA_SIZE: u64 = AREA_SIZES[0];
const MAX_AREA_SIZE: u64 = AREA_SIZES[NUM_AREA_SIZES - 1];

const SOME_FREE_LIST_ELT_SIZE: u64 = 1 + std::mem::size_of::<LinearAddress>() as u64;
const FREE_LIST_MAX_SIZE: u64 = NUM_AREA_SIZES as u64 * SOME_FREE_LIST_ELT_SIZE;

/// Returns the index in `BLOCK_SIZES` of the smallest block size >= `n`.
fn area_size_to_index(n: u64) -> Result<AreaIndex, Error> {
    if n > MAX_AREA_SIZE {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("Node size {} is too large", n),
        ));
    }

    if n <= MIN_AREA_SIZE {
        return Ok(0);
    }

    let mut log = n.ilog2();
    // If n is not a power of 2, we need to round up to the next power of 2.
    if n != 1 << log {
        log += 1;
    }

    Ok(log as AreaIndex - MIN_AREA_SIZE_LOG)
}

/// Objects cannot be stored at the zero address, so a [LinearAddress] is guaranteed not
/// to be zero. This reserved zero can be used as a [None] value for some use cases. In particular,
/// branches can use `Option<LinearAddress>` which is the same size as a [LinearAddress]
pub type LinearAddress = NonZeroU64;

/// Each [StoredArea] contains an [Area] which is either a [Node] or a [FreeArea].
#[derive(PartialEq, Eq, Clone, Debug, Deserialize, Serialize)]
enum Area<T, U> {
    Node(T),
    Free(U),
}

/// Every item stored in the [NodeStore]'s LinearStore  after the
/// [NodeStoreHeader] is a [StoredArea].
#[derive(PartialEq, Eq, Clone, Debug, Deserialize, Serialize)]
struct StoredArea<T> {
    /// Index in [AREA_SIZES] of this area's size
    area_size_index: AreaIndex,
    area: T,
}

/// [NodeStore] creates, reads, updates, and deletes [Node]s.
/// To read a node, the [NodeStore] uses the [LinearAddress] of
/// the node.
/// The first thing written in the LinearStore is a NodeStoreHeader,
/// which contains the version and the free area list heads.
/// Every subsequent write is a StoredArea containing a [Node] or a FreeArea.
/// The size of each allocation [NodeStore] makes from LinearStore is one of AREA_SIZES.
/// The first type parameter is the revision type, which can be either [Committed] or [Proposed].
/// The second type parameter indicates the backing store, either a [FileStore] or a [MemStore].
#[derive(Debug)]
pub struct NodeStore<T: ReadChangedNode, S: Send + Sync> {
    // The header from the underlying storage system
    header: NodeStoreHeader,
    deleted_nodes: Vec<(LinearAddress, AreaIndex)>,
    /// The type of revision for the NodeStore.
    pub revision_type: T,
    storage: Arc<S>,
}

impl From<Committed> for NodeStoreParent {
    fn from(_: Committed) -> Self {
        NodeStoreParent::Committed(Committed {})
    }
}

impl From<ProposedImmutable> for NodeStoreParent {
    fn from(p: ProposedImmutable) -> Self {
        NodeStoreParent::Proposed(p)
    }
}

/// Methods to generate a base [Committed] NodeStore, either by creating a fresh one
/// using [NodeStore<Committed>::new] or opening an existing file
impl<S: WritableStorage> NodeStore<Committed, S> {
    /// Open an existing [NodeStore]
    ///
    /// This method reads the header previously created from a call to [NodeStore::initialize]
    pub fn open(storage: Arc<S>) -> Result<Self, Error> {
        let mut stream = storage.stream_from(0)?;

        let header: NodeStoreHeader = bincode::deserialize_from(&mut stream)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

        drop(stream);

        Ok(Self {
            header,
            deleted_nodes: Default::default(),
            revision_type: Committed,
            storage,
        })
    }

    /// Create a new, empty, [Committed] [NodeStore] and clobber
    /// the underlying store with an empty freelist and no root node
    pub fn initialize(storage: Arc<S>) -> Result<Self, Error> {
        let header = NodeStoreHeader {
            version: Version::new(),
            free_lists: Default::default(),
            root_address: None,
            size: NodeStoreHeader::SIZE,
        };

        let header_bytes = bincode::serialize(&header).map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to serialize header: {}", e),
            )
        })?;

        storage.write(0, header_bytes.as_slice())?;

        Ok(Self {
            header,
            storage,
            deleted_nodes: Default::default(),
            revision_type: Committed,
        })
    }
}

impl<T: ReadChangedNode, S: ReadableStorage> NodeStore<T, S> {
    /// Reads a node from the specified address.
    ///
    /// # Arguments
    ///
    /// * `addr` - The linear address of the node.
    ///
    /// # Returns
    ///
    /// Returns a `Result` containing an `Arc<Node>` if the node is found, or an `Error` if there was an issue reading the node.
    pub fn read_node(&self, addr: LinearAddress) -> Result<Arc<Node>, Error> {
        let proposal_nodes = &self.revision_type;
        if let Some(node) = proposal_nodes.read_changed_node(addr) {
            return Ok(node.clone());
        }
        self.read_node_from_disk(addr)
    }
}

impl<T: ReadChangedNode, S: ReadableStorage> NodeStore<T, S> {
    /// Returns (index, area_size) for the [StoredArea] at `addr`.
    /// `index` is the index of `area_size` in [AREA_SIZES].
    fn area_index_and_size(&self, addr: LinearAddress) -> Result<(AreaIndex, u64), Error> {
        let mut area_stream = self.storage.stream_from(addr.get())?;

        let index: AreaIndex = bincode::deserialize_from(&mut area_stream)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

        let size = *AREA_SIZES.get(index as usize).ok_or(Error::new(
            ErrorKind::InvalidData,
            format!("Invalid area size index {}", index),
        ))?;

        Ok((index, size))
    }

    /// Read a [Node] from the provided [LinearAddress].
    /// `addr` is the address of a StoredArea in the LinearStore.
    pub fn read_node_from_disk(&self, addr: LinearAddress) -> Result<Arc<Node>, Error> {
        debug_assert!(addr.get() % 8 == 0);

        let addr = addr.get() + 1; // Skip the index byte

        let area_stream = self.storage.stream_from(addr)?;
        let area: Area<Node, FreeArea> = bincode::deserialize_from(area_stream)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

        match area {
            Area::Node(node) => Ok(Arc::new(node)),
            Area::Free(_) => Err(Error::new(
                ErrorKind::InvalidData,
                "Attempted to read a freed area",
            )),
        }
    }

    /// Get the root address of the [NodeStore]
    pub const fn root_address(&self) -> Option<LinearAddress> {
        self.header.root_address
    }
}

impl<S: WritableStorage> NodeStore<ProposedMutable, S> {
    /// Creates a new proposed instance of `NodeStore`.
    ///
    /// # Arguments
    ///
    /// * `parent` - The parent node store.
    /// * `storage` - The storage implementation.
    pub fn new<T: Into<NodeStoreParent> + ReadChangedNode>(parent: NodeStore<T, S>) -> Self {
        let mut store = NodeStore {
            header: parent.header.clone(),
            deleted_nodes: Default::default(),
            revision_type: Proposed::new(parent.revision_type.into().into()),
            storage: parent.storage.clone(),
        };
        store.write_header().expect("failed to write header");
        store
    }

    // TODO danlaine: Write only the parts of the header that have changed instead of the whole thing
    fn write_header(&mut self) -> Result<(), Error> {
        let header_bytes = bincode::serialize(&self.header).map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to serialize free lists: {}", e),
            )
        })?;

        self.storage.write(0, header_bytes.as_slice())?;

        Ok(())
    }

    /// Attempts to allocate `n` bytes from the free lists.
    /// If successful returns the address of the newly allocated area
    /// and the index of the free list that was used.
    /// If there are no free areas big enough for `n` bytes, returns None.
    /// TODO danlaine: If we return a larger area than requested, we should split it.
    fn allocate_from_freed(&mut self, n: u64) -> Result<Option<(LinearAddress, AreaIndex)>, Error> {
        // Find the smallest free list that can fit this size.
        let index = area_size_to_index(n)?;

        // rustify: rewrite using self.header.free_lists.iter_mut().find(...)
        for index in index as usize..NUM_AREA_SIZES {
            // Get the first free block of sufficient size.
            let free_stored_area_addr = self.header.free_lists[index];
            if let Some(free_stored_area_addr) = free_stored_area_addr {
                // Update the free list head.
                // Skip the index byte and Area discriminant byte
                let free_area_addr = free_stored_area_addr.get() + 2;
                let free_head_stream = self.storage.stream_from(free_area_addr)?;
                let free_head: FreeArea = bincode::deserialize_from(free_head_stream)
                    .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

                // Update the free list to point to the next free block.
                self.header.free_lists[index] = free_head.next_free_block;

                // Return the address of the newly allocated block.
                return Ok(Some((free_stored_area_addr, index as AreaIndex)));
            }
            // No free blocks in this list, try the next size up.
        }

        Ok(None)
    }

    fn allocate_from_end(&mut self, n: u64) -> Result<(LinearAddress, AreaIndex), Error> {
        let index = area_size_to_index(n)?;
        let area_size = AREA_SIZES[index as usize];
        let addr = LinearAddress::new(self.header.size).expect("node store size can't be 0");
        self.header.size += area_size;
        debug_assert!(addr.get() % 8 == 0);
        Ok((addr, index))
    }

    /// Allocates an area in the LinearStore large enough for the provided Area.
    /// Returns the address of the allocated area.
    pub fn create_node(&mut self, node: Node) -> Result<LinearAddress, Error> {
        let addr = self.create_node_inner(node)?;
        Ok(addr)
    }

    fn stored_len(node: &Node) -> u64 {
        // TODO: calculate length without serializing!
        let area: Area<&Node, FreeArea> = Area::Node(node);
        let area_bytes = bincode::serialize(&area)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e))
            .expect("fixme");

        // +1 for the size index byte
        // TODO: do a better job packing the boolean (freed) with the possible node sizes
        // A reasonable option is use a i8 and negative values indicate it's freed whereas positive values are non-free
        // This would still allow for 127 different sizes
        area_bytes.len() as u64 + 1
    }

    /// Allocates an area in the LinearStore but does not write it; returns
    /// where the node will go and the freelist size of the node
    ///
    /// Note: The node is removed from the freelist but it still contains a freed status
    /// in the linearstore. The caller must eventually call [NodeStore::update_node] or
    /// [NodeStore::delete_node] on this node
    pub fn allocate_node(&mut self, node: &Node) -> Result<(LinearAddress, AreaIndex), Error> {
        let stored_area_size = Self::stored_len(node);

        // Attempt to allocate from a free list.
        // If we can't allocate from a free list, allocate past the existing
        // of the LinearStore.
        let (addr, index) = match self.allocate_from_freed(stored_area_size)? {
            Some((addr, index)) => (addr, index),
            None => {
                // if we just allocated new space, we might free it, so we need to write the
                // size of this freelist item into the linear store
                // TODO: This works, but think about if we can do better than this somehow.
                let (addr, index) = self.allocate_from_end(stored_area_size)?;
                (addr, index)
            }
        };

        Ok((addr, index))
    }

    /// Checks to see if a new Node will fit into an existing allocated node space
    pub fn still_fits(&mut self, old_size: AreaIndex, node: &Node) -> Result<bool, Error> {
        let stored_area_size = Self::stored_len(node);
        let required_index = area_size_to_index(stored_area_size)?;
        Ok(required_index <= old_size)
    }

    /// Get the size of the space allocated to a node a specific address
    pub fn node_size(&self, addr: LinearAddress) -> Result<AreaIndex, Error> {
        let size = 0;
        self.storage
            .stream_from(addr.into())?
            .read_exact(&mut [size])?;
        Ok(size)
    }

    /// The inner implementation of `create_node` that doesn't update the free lists.
    fn create_node_inner(&mut self, node: Node) -> Result<LinearAddress, Error> {
        let (addr, index) = self.allocate_node(&node)?;

        let stored_area = StoredArea {
            area_size_index: index,
            area: Area::<_, FreeArea>::Node(node),
        };

        let stored_area_bytes =
            bincode::serialize(&stored_area).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

        self.storage
            .write(addr.get(), stored_area_bytes.as_slice())?;

        Ok(addr)
    }

    /// Update a [Node] that was previously at the provided address.
    /// This is complicated by the fact that a node might grow and not be able to fit a the given
    /// address, in which case we return [UpdateError::NodeMoved].
    /// `addr` is the address of a StoredArea in the LinearStore.
    pub fn update_node(&mut self, addr: LinearAddress, node: Node) -> Result<(), UpdateError> {
        debug_assert!(addr.get() % 8 == 0);

        let (_, old_stored_area_size) = self.area_index_and_size(addr)?;

        let new_area: Area<&Node, FreeArea> = Area::Node(&node);

        let new_area_bytes =
            bincode::serialize(&new_area).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

        let new_stored_area_size = new_area_bytes.len() as u64 + 1; // +1 for the size index byte

        if new_stored_area_size <= old_stored_area_size {
            // the new node fits in the old node's area
            let addr = addr.get() + 1; // Skip the index byte
            self.storage.write(addr, new_area_bytes.as_slice())?;
            return Ok(());
        }

        // the new node is larger than the old node, so we need to allocate a new area
        let new_node_addr = self.create_node_inner(node)?;
        self.delete_node(addr)?;
        Err(UpdateError::NodeMoved(new_node_addr))
    }

    /// Update a node in-place. This should only be used when the node was allocated using
    /// allocate_node.
    /// TODO: We should enforce this by having a new type for allocated nodes, which could
    /// carry the size information too
    pub fn update_in_place(&mut self, addr: LinearAddress, node: &Node) -> Result<(), Error> {
        let new_area: Area<&Node, FreeArea> = Area::Node(node);
        let new_area_bytes =
            bincode::serialize(&new_area).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

        let addr = addr.get() + 1; // Skip the index byte
        self.storage.write(addr, new_area_bytes.as_slice())?;
        Ok(())
    }

    /// Deletes the [Node] at the given address.
    pub fn delete_node(&mut self, addr: LinearAddress) -> Result<(), Error> {
        debug_assert!(addr.get() % 8 == 0);

        if let Some((_, area_size_index)) = self.revision_type.new_nodes.remove(&addr) {
            // we created this node before, so we can return it to the free list for possible reuse
            // in this revision
            let _next_free_block = self.header.free_lists[area_size_index as usize];
            self.header.free_lists[area_size_index as usize] = Some(addr);
            // TODO: what do we do with next_free_block here?
        } else {
            // we didn't create this node, so mark it as a deleted node for this revision
            self.deleted_nodes.push((addr, self.area_index_and_size(addr)?.0));
        }
        Ok(())
    }

    /// Write the root [LinearAddress] of the [NodeStore]
    pub fn set_root(&mut self, addr: Option<LinearAddress>) {
        self.header.root_address = addr;
    }
}

impl<S: Send + Sync> NodeStore<ProposedMutable, S> {
    /// Freeze a writable NodeStore into a ProposedImmutable
    pub fn freeze(self) -> NodeStore<ProposedImmutable, S> {
        NodeStore {
            header: self.header,
            storage: self.storage,
            deleted_nodes: self.deleted_nodes,
            revision_type: self.revision_type.freeze(),
        }
    }
}

/// An error from doing an update. One special error is [UpdateError::NodeMoved]
/// There are no implementations of [Into::into] here because we want to be sure
/// the caller thinks about how to handle the node moving to a new address when
/// it changes size.
///
/// TODO: Callers shouldn't care whether or not the node changed size and should
/// handle the moving of a node in all cases. This may allow the storage layer to
/// be smarter about moving related nodes closer together on disk (i.e., dynamic
/// defragmentation)
#[derive(Debug)]
pub enum UpdateError {
    /// An IO error occurred during the update
    Io(Error),
    /// The update succeeded, but the node moved to a new address. Any parent pointers
    /// should be updated as a result.
    NodeMoved(LinearAddress),
}

impl From<Error> for UpdateError {
    fn from(value: Error) -> Self {
        UpdateError::Io(value)
    }
}

/// Can be used by filesystem tooling such as "file" to identify
/// the version of firewood used to create this [NodeStore] file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
struct Version {
    bytes: [u8; 16],
}

impl Version {
    const SIZE: u64 = std::mem::size_of::<Self>() as u64;

    /// construct a [Version] header from the firewood version
    fn new() -> Self {
        let mut version_bytes: [u8; Self::SIZE as usize] = Default::default();
        let version = env!("CARGO_PKG_VERSION");
        let _ = version_bytes
            .as_mut_slice()
            .write_all(format!("firewood {}", version).as_bytes());
        Self {
            bytes: version_bytes,
        }
    }
}

pub type FreeLists = [Option<LinearAddress>; NUM_AREA_SIZES];

/// Persisted metadata for a [NodeStore].
/// The [NodeStoreHeader] is at the start of the LinearStore.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NodeStoreHeader {
    /// Identifies the version of firewood used to create this [NodeStore].
    version: Version,
    size: u64,
    /// Element i is the pointer to the first free block of size `BLOCK_SIZES[i]`.
    free_lists: FreeLists,
    root_address: Option<LinearAddress>,
}

impl NodeStoreHeader {
    /// The first SIZE bytes of the LinearStore are the [NodeStoreHeader].
    /// The serialized NodeStoreHeader may be less than SIZE bytes but we
    /// reserve this much space for it since it can grow and it must always be
    /// at the start of the LinearStore so it can't be moved in a resize.
    const SIZE: u64 = {
        let max_size = Version::SIZE + 8 + 9 + FREE_LIST_MAX_SIZE;
        // Round up to the nearest multiple of MIN_AREA_SIZE
        let remainder = max_size % MIN_AREA_SIZE;
        if remainder == 0 {
            max_size
        } else {
            max_size + MIN_AREA_SIZE - remainder
        }
    };
}

/// A [FreeArea] is stored at the start of the area that contained a node that
/// has been freed.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
struct FreeArea {
    next_free_block: Option<LinearAddress>,
}

/// TODO: enable nodestore tests
#[cfg(never)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::linear::memory::MemStore;
    use crate::Path;
    use crate::{BranchNode, LeafNode};

    use super::*;

    #[test]
    fn test_area_size_to_index() {
        // TODO: rustify using: for size in AREA_SIZES
        for (i, &area_size) in AREA_SIZES.iter().enumerate() {
            // area size is at top of range
            assert_eq!(area_size_to_index(area_size).unwrap(), i as AreaIndex);

            if i > 0 {
                // 1 less than top of range stays in range
                assert_eq!(area_size_to_index(area_size - 1).unwrap(), i as AreaIndex);
            }

            if i < NUM_AREA_SIZES - 1 {
                // 1 more than top of range goes to next range
                assert_eq!(
                    area_size_to_index(area_size + 1).unwrap(),
                    (i + 1) as AreaIndex
                );
            }
        }

        for i in 0..=MIN_AREA_SIZE {
            assert_eq!(area_size_to_index(i).unwrap(), 0);
        }

        assert!(area_size_to_index(MAX_AREA_SIZE + 1).is_err());
    }

    #[test]
    fn test_create() {
        let linear_store = MemStore::new(vec![]);
        let mut node_store = NodeStore::initialize(linear_store).unwrap();

        let leaf = Node::Leaf(LeafNode {
            partial_path: Path::from([0, 1, 2]),
            value: Box::new([3, 4, 5]),
        });

        let leaf_addr = node_store.create_node(leaf.clone()).unwrap();
        let (_, leaf_area_size) = node_store.area_index_and_size(leaf_addr).unwrap();

        {
            // The header should be unchanged
            let mut header_bytes = node_store.linear_store.stream_from(0).unwrap();
            let header: NodeStoreHeader = bincode::deserialize_from(&mut header_bytes).unwrap();
            assert_eq!(header.version, Version::new());
            let empty_free_lists: FreeLists = Default::default();
            assert_eq!(header.free_lists, empty_free_lists);
            assert_eq!(header.root_address, None);

            // Leaf should go right after the header
            assert_eq!(leaf_addr.get(), NodeStoreHeader::SIZE);

            // Size should be updated
            assert_eq!(
                node_store.header.size,
                NodeStoreHeader::SIZE + leaf_area_size
            );

            // Should be able to read the leaf back
            let read_leaf = node_store.read_node(leaf_addr).unwrap();
            assert_eq!(*read_leaf, leaf);
        }

        // Create another node
        let branch = Node::Branch(Box::new(BranchNode {
            partial_path: Path::from([6, 7, 8]),
            value: Some(vec![9, 10, 11].into_boxed_slice()),
            children: Default::default(),
        }));

        let old_size = node_store.header.size;
        let branch_addr = node_store.create_node(branch.clone()).unwrap();

        {
            // The header  should be unchanged
            let mut header_bytes = node_store.linear_store.stream_from(0).unwrap();
            let header: NodeStoreHeader = bincode::deserialize_from(&mut header_bytes).unwrap();
            assert_eq!(header.version, Version::new());
            let empty_free_lists: FreeLists = Default::default();
            assert_eq!(header.free_lists, empty_free_lists);
            assert_eq!(header.root_address, None);
            assert!(header.size > old_size);

            // branch should go right after leaf
            assert_eq!(branch_addr.get(), NodeStoreHeader::SIZE + leaf_area_size);

            // Size should be updated
            let (_, branch_area_size) = node_store.area_index_and_size(branch_addr).unwrap();
            assert_eq!(
                node_store.header.size,
                NodeStoreHeader::SIZE + leaf_area_size + branch_area_size
            );

            // Should be able to read the branch back
            let read_leaf2 = node_store.read_node(branch_addr).unwrap();
            assert_eq!(*read_leaf2, branch);
        }
    }

    #[test]
    fn test_update_resize() {
        let linear_store = MemStore::new(vec![]);
        let mut node_store = NodeStore::initialize(linear_store).unwrap();

        // Create a leaf
        let leaf = Node::Leaf(LeafNode {
            partial_path: Path::new(),
            value: Box::new([1]),
        });
        let leaf_addr = node_store.create_node(leaf).unwrap();
        let (leaf_index, leaf_area_size) = node_store.area_index_and_size(leaf_addr).unwrap();

        // Update the node
        let branch = Node::Branch(Box::new(BranchNode {
            partial_path: Path::from([6, 7, 8]),
            value: Some(vec![9, 10, 11].into_boxed_slice()),
            children: Default::default(),
        }));

        // The new node is larger than the old node, so we need to allocate a new area
        let (branch_addr, _) = match node_store.update_node(leaf_addr, branch.clone()) {
            Ok(_) => panic!("Expected NodeMoved"),
            Err(UpdateError::NodeMoved(new_addr)) => {
                let (_, branch_area_size) = node_store.area_index_and_size(new_addr).unwrap();
                (new_addr, branch_area_size)
            }
            Err(e) => panic!("Unexpected error: {:?}", e),
        };

        // The new area should be at the end of the store
        assert_eq!(branch_addr.get(), NodeStoreHeader::SIZE + leaf_area_size);

        // The free list should be updated
        let mut header_bytes = node_store.linear_store.stream_from(0).unwrap();
        let header: NodeStoreHeader = bincode::deserialize_from(&mut header_bytes).unwrap();
        assert_eq!(header.free_lists[leaf_index as usize], Some(leaf_addr));

        // The new node should be readable
        let read_branch = node_store.read_node(branch_addr).unwrap();
        assert_eq!(*read_branch, branch);

        // The old node should be deleted
        assert!(node_store.read_node(leaf_addr).is_err());
    }

    #[test]
    fn test_update_dont_resize() {
        let linear_store = MemStore::new(vec![]);
        let mut node_store = NodeStore::initialize(linear_store).unwrap();

        // Create a leaf
        let leaf1 = Node::Leaf(LeafNode {
            partial_path: Path::new(),
            value: Box::new([1]),
        });
        let leaf1_addr = node_store.create_node(leaf1).unwrap();

        // Update the node
        let leaf2 = Node::Leaf(LeafNode {
            partial_path: Path::new(),
            value: Box::new([2]),
        });

        // The new node should fit in the old area
        let old_size = node_store.header.size;
        node_store.update_node(leaf1_addr, leaf2.clone()).unwrap();

        // The header should be unchanged
        let mut header_bytes = node_store.linear_store.stream_from(0).unwrap();
        let header: NodeStoreHeader = bincode::deserialize_from(&mut header_bytes).unwrap();
        assert_eq!(header.version, Version::new());
        let empty_free_lists: FreeLists = Default::default();
        assert_eq!(header.free_lists, empty_free_lists);
        assert_eq!(header.root_address, None);
        assert_eq!(header.size, old_size);

        // The new node should be readable
        let read_leaf2 = node_store.read_node(leaf1_addr).unwrap();
        assert_eq!(*read_leaf2, leaf2);
    }

    #[test]
    fn test_delete() {
        let linear_store = MemStore::new(vec![]);
        let mut node_store = NodeStore::initialize(linear_store).unwrap();

        // Create a leaf
        let leaf = Node::Leaf(LeafNode {
            partial_path: Path::new(),
            value: Box::new([1]),
        });
        let leaf_addr = node_store.create_node(leaf.clone()).unwrap();
        let (leaf_index, _) = node_store.area_index_and_size(leaf_addr).unwrap();

        // Delete the node
        node_store.delete_node(leaf_addr).unwrap();

        {
            // The header should have the freed node in the free list
            let mut header_bytes = node_store.linear_store.stream_from(0).unwrap();
            let header: NodeStoreHeader = bincode::deserialize_from(&mut header_bytes).unwrap();
            assert_eq!(header.free_lists[leaf_index as usize], Some(leaf_addr));
        }

        // The node shouldn't be readable
        assert!(node_store.read_node(leaf_addr).is_err());

        // Create a new node with the same size
        let new_leaf_addr = node_store.create_node(leaf).unwrap();

        // The new node should be at the same address
        assert_eq!(new_leaf_addr, leaf_addr);

        // The header should be updated
        let mut header_bytes = node_store.linear_store.stream_from(0).unwrap();
        let header: NodeStoreHeader = bincode::deserialize_from(&mut header_bytes).unwrap();
        assert_eq!(header.free_lists[leaf_index as usize], None);
    }

    #[test]
    fn test_node_store_new() {
        let linear_store = MemStore::new(vec![]);
        let node_store = NodeStore::initialize(linear_store).unwrap();

        // Check the empty header is written at the start of the LinearStore.
        let mut header_bytes = node_store.linear_store.stream_from(0).unwrap();
        let header: NodeStoreHeader = bincode::deserialize_from(&mut header_bytes).unwrap();
        assert_eq!(header.version, Version::new());
        let empty_free_list: FreeLists = Default::default();
        assert_eq!(header.free_lists, empty_free_list);
    }
}
