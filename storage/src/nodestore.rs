// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Debug;
/// The [NodeStore] handles the serialization of nodes and
/// free space management of nodes in the page store. It lays out the format
/// of the [PageStore]. More specifically, it places a [FileIdentifyingMagic]
/// and a [FreeSpaceHeader] at the beginning
use std::io::{Error, ErrorKind, Write};
use std::iter::once;
use std::num::NonZeroU64;
use std::sync::Arc;

use crate::hashednode::hash_node;
use crate::node::Node;
use crate::{Child, Path, ReadableStorage, TrieHash};

use super::linear::WritableStorage;

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

/// Number of children in a branch
const BRANCH_CHILDREN: usize = 16;

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

/// Every item stored in the [NodeStore]'s ReadableStorage  after the
/// [NodeStoreHeader] is a [StoredArea].
#[derive(PartialEq, Eq, Clone, Debug, Deserialize, Serialize)]
struct StoredArea<T> {
    /// Index in [AREA_SIZES] of this area's size
    area_size_index: AreaIndex,
    area: T,
}

impl<T: ReadInMemoryNode, S: ReadableStorage> NodeStore<T, S> {
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
    /// `addr` is the address of a StoredArea in the ReadableStorage.
    pub fn read_node_from_disk(&self, addr: LinearAddress) -> Result<Arc<Node>, Error> {
        debug_assert!(addr.get() % 8 == 0);

        let addr = addr.get() + 1; // Skip the index byte

        let area_stream = self.storage.stream_from(addr)?;
        let area: Area<Node, FreeArea> = bincode::deserialize_from(area_stream)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

        match area {
            Area::Node(node) => Ok(node.into()),
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

impl<S: ReadableStorage> NodeStore<Committed, S> {
    /// Open an existing [NodeStore]
    ///
    /// This method reads the header previously created from a call to [NodeStore::new]
    pub fn open(storage: Arc<S>) -> Result<Self, Error> {
        let mut stream = storage.stream_from(0)?;

        let header: NodeStoreHeader = bincode::deserialize_from(&mut stream)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

        drop(stream);

        Ok(Self {
            header,
            kind: Committed {
                deleted: Default::default(),
            },
            storage,
        })
    }

    /// Create a new, empty, Committed [NodeStore] and clobber
    /// the underlying store with an empty freelist and no root node
    pub fn new_empty_committed(storage: Arc<S>) -> Result<Self, Error> {
        let header = NodeStoreHeader::new();

        // let header_bytes = bincode::serialize(&header).map_err(|e| {
        //     Error::new(
        //         ErrorKind::InvalidData,
        //         format!("Failed to serialize header: {}", e),
        //     )
        // })?;

        // storage.write(0, header_bytes.as_slice())?;

        Ok(Self {
            header,
            storage,
            kind: Committed {
                deleted: Default::default(),
            },
        })
    }
}

impl<S: WritableStorage> NodeStore<ImmutableProposal, S> {
    /// Creatse a new NodeStore proposal from a given `parent`.
    pub fn new<T: Into<NodeStoreParent> + ReadInMemoryNode>(parent: NodeStore<T, S>) -> Self {
        NodeStore {
            header: parent.header.clone(),
            kind: ImmutableProposal {
                new: HashMap::new(),
                deleted: Default::default(),
                parent: parent.kind.into(),
            },
            storage: parent.storage.clone(),
        }
    }

    // TODO danlaine: Use this code in the revision management code.
    // TODO danlaine: Write only the parts of the header that have changed instead of the whole thing
    // fn write_header(&mut self) -> Result<(), Error> {
    //     let header_bytes = bincode::serialize(&self.header).map_err(|e| {
    //         Error::new(
    //             ErrorKind::InvalidData,
    //             format!("Failed to serialize free lists: {}", e),
    //         )
    //     })?;

    //     self.storage.write(0, header_bytes.as_slice())?;

    //     Ok(())
    // }
}

impl<S: WritableStorage> NodeStore<ProposedMutable2, S> {
    /// Creates a new, empty, [NodeStore] and clobbers the underlying `storage` with an empty header.
    pub fn new_empty_proposal(storage: Arc<S>) -> Self {
        let header = NodeStoreHeader::new();
        let header_bytes = bincode::serialize(&header).expect("failed to serialize header");
        storage
            .write(0, header_bytes.as_slice())
            .expect("failed to write header");
        NodeStore {
            header,
            kind: ProposedMutable2 {
                root: None,
                deleted: Default::default(),
                parent: NodeStoreParent::Committed,
            },
            storage,
        }
    }
}

impl<S: ReadableStorage> NodeStore<ImmutableProposal, S> {
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
            if let Some(free_stored_area_addr) = self.header.free_lists[index] {
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

    /// Returns the length of the serialized area for a node.
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

    /// Returns an address that can be used to store the given `node` and updates
    /// `self.header` to reflect the allocation. Doesn't actually write the node to storage.
    /// Also returns the index of the free list the node was allocated from.
    pub fn allocate_node(&mut self, node: &Node) -> Result<(LinearAddress, AreaIndex), Error> {
        let stored_area_size = Self::stored_len(node);

        // Attempt to allocate from a free list.
        // If we can't allocate from a free list, allocate past the existing
        // of the ReadableStorage.
        let (addr, index) = match self.allocate_from_freed(stored_area_size)? {
            Some((addr, index)) => (addr, index),
            None => self.allocate_from_end(stored_area_size)?,
        };

        Ok((addr, index))
    }

    // TODO danlaine: Use this code inside the revision management code or delete it.
    /// The inner implementation of `create_node` that doesn't update the free lists.
    // fn create_node_inner(&mut self, node: Node) -> Result<LinearAddress, Error> {
    //     let (addr, index) = self.allocate_node(&node)?;

    //     let stored_area = StoredArea {
    //         area_size_index: index,
    //         area: Area::<_, FreeArea>::Node(node),
    //     };

    //     let stored_area_bytes =
    //         bincode::serialize(&stored_area).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

    //     self.storage
    //         .write(addr.get(), stored_area_bytes.as_slice())?;

    //     Ok(addr)
    // }

    // TODO danlaine: use this code in the revision management code or delete it.
    /// Update a node in-place. This should only be used when the node was allocated using
    /// allocate_node.
    /// TODO: We should enforce this by having a new type for allocated nodes, which could
    /// carry the size information too
    // pub fn update_in_place(&mut self, addr: LinearAddress, node: &Node) -> Result<(), Error> {
    //     let new_area: Area<&Node, FreeArea> = Area::Node(node);
    //     let new_area_bytes =
    //         bincode::serialize(&new_area).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

    //     let addr = addr.get() + 1; // Skip the index byte
    //     self.storage.write(addr, new_area_bytes.as_slice())?;
    //     Ok(())
    // }

    // TODO danlaine: use this code in the revision management code.
    /// Deletes the [Node] at the given address.
    // pub fn delete_node(&mut self, addr: LinearAddress) -> Result<(), Error> {
    //     debug_assert!(addr.get() % 8 == 0);

    //     let (area_size_index, _) = self.area_index_and_size(addr)?;

    //     // The area that contained the node is now free.
    //     let area: Area<Node, FreeArea> = Area::Free(FreeArea {
    //         next_free_block: self.header.free_lists[area_size_index as usize],
    //     });

    //     let stored_area = StoredArea {
    //         area_size_index,
    //         area,
    //     };

    //     let stored_area_bytes =
    //         bincode::serialize(&stored_area).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

    //     self.storage.write(addr.into(), &stored_area_bytes)?;

    //     // The newly freed block is now the head of the free list.
    //     self.header.free_lists[area_size_index as usize] = Some(addr);

    //     Ok(())
    // }

    /// Write the root [LinearAddress] of the [NodeStore]
    pub fn set_root(&mut self, addr: Option<LinearAddress>) -> Result<(), Error> {
        self.header.root_address = addr;
        Ok(())
    }
}

/// An error from doing an update
#[derive(Debug)]
pub enum UpdateError {
    /// An IO error occurred during the update
    Io(Error),
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
/// The [NodeStoreHeader] is at the start of the ReadableStorage.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize, Clone)]
struct NodeStoreHeader {
    /// Identifies the version of firewood used to create this [NodeStore].
    version: Version,
    size: u64,
    /// Element i is the pointer to the first free block of size `BLOCK_SIZES[i]`.
    free_lists: FreeLists,
    root_address: Option<LinearAddress>,
}

impl NodeStoreHeader {
    /// The first SIZE bytes of the ReadableStorage are the [NodeStoreHeader].
    /// The serialized NodeStoreHeader may be less than SIZE bytes but we
    /// reserve this much space for it since it can grow and it must always be
    /// at the start of the ReadableStorage so it can't be moved in a resize.
    const SIZE: u64 = {
        // 8 and 9 for `size` and `root_address` respectively
        let max_size = Version::SIZE + 8 + 9 + FREE_LIST_MAX_SIZE;
        // Round up to the nearest multiple of MIN_AREA_SIZE
        let remainder = max_size % MIN_AREA_SIZE;
        if remainder == 0 {
            max_size
        } else {
            max_size + MIN_AREA_SIZE - remainder
        }
    };

    fn new() -> Self {
        Self {
            // The store just contains the header at this point
            size: Self::SIZE,
            root_address: None,
            version: Version::new(),
            free_lists: Default::default(),
        }
    }
}

/// A [FreeArea] is stored at the start of the area that contained a node that
/// has been freed.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
struct FreeArea {
    next_free_block: Option<LinearAddress>,
}

/// Reads nodes and the root address from a merkle trie.
pub trait NodeReader {
    /// Returns the node at `addr`.
    fn read_node(&self, addr: LinearAddress) -> Result<Arc<Node>, Error>;
    /// Returns the root of the trie.
    fn root(&self) -> Option<Arc<Node>>;
}

/// Updates a merkle trie.
pub trait NodeWriter: NodeReader {
    /// Deletes the node at `addr`.
    fn delete_node(&mut self, addr: LinearAddress) -> Result<(), Error>;
}

/// A committed revision of a merkle trie.
#[derive(Debug)]
pub struct Committed {
    deleted: Box<[LinearAddress]>,
}

impl ReadInMemoryNode for Committed {
    // A committed revision has no in-memory changes. All its nodes are in storage.
    fn read_in_memory_node(&self, _addr: LinearAddress) -> Option<Arc<Node>> {
        None
    }
}

#[derive(Debug)]
pub enum NodeStoreParent {
    Proposed(Arc<ImmutableProposal>),
    Committed,
}

impl<S: ReadableStorage> From<NodeStore<Committed, S>> for NodeStoreParent {
    fn from(_: NodeStore<Committed, S>) -> Self {
        NodeStoreParent::Committed
    }
}

impl<S: ReadableStorage> From<NodeStore<ImmutableProposal, S>> for NodeStoreParent {
    fn from(val: NodeStore<ImmutableProposal, S>) -> Self {
        NodeStoreParent::Proposed(Arc::new(val.kind))
    }
}

#[derive(Debug)]
/// Contains state for a proposed revision of the trie.
pub struct ImmutableProposal {
    /// Address --> Node for nodes created in this proposal.
    new: HashMap<LinearAddress, Arc<Node>>,
    deleted: Box<[LinearAddress]>,
    /// The parent of this proposal.
    parent: NodeStoreParent,
}

impl ReadInMemoryNode for ImmutableProposal {
    fn read_in_memory_node(&self, addr: LinearAddress) -> Option<Arc<Node>> {
        // Check if the node being requested was created in this proposal.
        if let Some(node) = self.new.get(&addr) {
            return Some(node.clone());
        }

        // It wasn't. Try our parent, and its parent, and so on until we find it or find
        // a committed revision.
        match self.parent {
            NodeStoreParent::Proposed(ref parent) => parent.read_in_memory_node(addr),
            NodeStoreParent::Committed => None,
        }
    }
}

/// Reads a node which hasn't yet been committed to storage.
pub trait ReadInMemoryNode {
    /// Returns the node at `addr` if it is in memory.
    /// Returns None if it isn't.
    fn read_in_memory_node(&self, addr: LinearAddress) -> Option<Arc<Node>>;
}

/// Contains the state of a revision of a merkle trie.
#[derive(Debug)]
pub struct NodeStore<T: ReadInMemoryNode, S: ReadableStorage> {
    // Metadata for this revision.
    header: NodeStoreHeader,
    /// This is one of [Committed], [ImmutableProposal], [ProposedMutable2].
    pub kind: T, // TODO add mut getter and make not pub
    // Persisted storage to read nodes from.
    storage: Arc<S>,
}

/// Contains the state of a proposal that is still being modified.
#[derive(Debug)]
pub struct ProposedMutable2 {
    /// The root of the trie in this proposal.
    pub root: Option<Node>,
    /// Nodes that have been deleted in this proposal.
    pub deleted: Vec<LinearAddress>,
    parent: NodeStoreParent,
}

impl ReadInMemoryNode for NodeStoreParent {
    fn read_in_memory_node(&self, addr: LinearAddress) -> Option<Arc<Node>> {
        match self {
            NodeStoreParent::Proposed(proposed) => proposed.read_in_memory_node(addr),
            NodeStoreParent::Committed => None,
        }
    }
}

impl ReadInMemoryNode for ProposedMutable2 {
    fn read_in_memory_node(&self, addr: LinearAddress) -> Option<Arc<Node>> {
        self.parent.read_in_memory_node(addr)
    }
}

impl<S: ReadableStorage> From<NodeStore<Committed, S>> for NodeStore<ProposedMutable2, S> {
    fn from(val: NodeStore<Committed, S>) -> Self {
        NodeStore {
            header: val.header,
            kind: ProposedMutable2 {
                root: None,
                deleted: Default::default(),
                parent: NodeStoreParent::Committed,
            },
            storage: val.storage,
        }
    }
}

impl<S: ReadableStorage> From<NodeStore<ImmutableProposal, S>> for NodeStore<ProposedMutable2, S> {
    fn from(val: NodeStore<ImmutableProposal, S>) -> Self {
        NodeStore {
            header: val.header,
            kind: ProposedMutable2 {
                root: None,
                deleted: Default::default(),
                parent: NodeStoreParent::Proposed(Arc::new(val.kind)),
            },
            storage: val.storage,
        }
    }
}

/*
    /// Hashes the trie and returns it as its immutable variant.
    pub fn hash(mut self) -> Result<impl NodeReader, MerkleError> {
        let Some(root) = std::mem::take(&mut self.root) else {
            self.nodestore.set_root(None)?;
            return Ok(self.nodestore);
        };

        // TODO use hash
        let (root_addr, _root_hash) = self.hash_helper(root, &mut Path(Default::default()));

        self.nodestore.set_root(Some(root_addr))?;
        Ok(self.nodestore)
    }
*/

impl<S: ReadableStorage> NodeStore<ImmutableProposal, S> {
    /// Hashes `node`, which is at the given `path_prefix`, and its children recursively.
    /// Returns the hashed node and its hash.
    fn hash_helper(&mut self, mut node: Node, path_prefix: &mut Path) -> (LinearAddress, TrieHash) {
        // Allocate addresses and calculate hashes for all new nodes
        match node {
            Node::Branch(ref mut b) => {
                for (nibble, child, child_node) in
                    b.children
                        .iter_mut()
                        .enumerate()
                        .filter_map(|(index, child)| match child {
                            Child::None => None,
                            Child::AddressWithHash(_, _) => None,
                            Child::Node(node) => {
                                let node = std::mem::take(node);
                                Some((index as u8, child, node))
                            }
                        })
                {
                    // Hash this child and update
                    // we extend and truncate path_prefix to reduce memory allocations
                    let original_length = path_prefix.len();
                    path_prefix
                        .0
                        .extend(b.partial_path.0.iter().copied().chain(once(nibble)));

                    let (child_addr, child_hash) = self.hash_helper(child_node, path_prefix);
                    *child = Child::AddressWithHash(child_addr, child_hash);
                    path_prefix.0.truncate(original_length);
                }
            }
            Node::Leaf(_) => {}
        }

        let hash = hash_node(&node, path_prefix);
        let (addr, _) = self.allocate_node(&node).expect("TODO handle error");

        self.kind.new.insert(addr, Arc::new(node));

        (addr, hash)
    }
}

impl<S: ReadableStorage> From<NodeStore<ProposedMutable2, S>> for NodeStore<ImmutableProposal, S> {
    fn from(val: NodeStore<ProposedMutable2, S>) -> Self {
        let NodeStore {
            header,
            kind,
            storage,
        } = val;

        let mut nodestore = NodeStore {
            header,
            kind: ImmutableProposal {
                new: HashMap::new(),
                deleted: kind.deleted.into(),
                parent: kind.parent,
            },
            storage,
        };

        let Some(root) = kind.root else {
            // This trie is now empty.
            nodestore.header.root_address = None;
            return nodestore;
        };

        // Hashes the trie and returns the address of the new root.
        let (root_addr, _) = nodestore.hash_helper(root, &mut Path::new());

        nodestore.header.root_address = Some(root_addr);

        nodestore
    }
}

impl<S: ReadableStorage> NodeReader for NodeStore<Committed, S> {
    fn read_node(&self, addr: LinearAddress) -> Result<Arc<Node>, Error> {
        self.read_node_from_disk(addr)
    }

    fn root(&self) -> Option<Arc<Node>> {
        let Some(root_addr) = self.header.root_address else {
            return None;
        };

        Some(self.read_node(root_addr).expect("TODO handle error"))
    }
}

impl<S: ReadableStorage> NodeReader for NodeStore<ImmutableProposal, S> {
    fn read_node(&self, addr: LinearAddress) -> Result<Arc<Node>, Error> {
        if let Some(node) = self.kind.read_in_memory_node(addr) {
            return Ok(node);
        }

        self.read_node_from_disk(addr)
    }

    fn root(&self) -> Option<Arc<Node>> {
        let Some(root_addr) = self.header.root_address else {
            return None;
        };

        Some(self.read_node(root_addr).expect("TODO handle error"))
    }
}

impl<S: ReadableStorage> NodeReader for NodeStore<ProposedMutable2, S> {
    fn read_node(&self, addr: LinearAddress) -> Result<Arc<Node>, Error> {
        if let Some(node) = self.kind.read_in_memory_node(addr) {
            return Ok(node);
        }

        self.read_node_from_disk(addr)
    }

    fn root(&self) -> Option<Arc<Node>> {
        let Some(root) = &self.kind.root else {
            return None;
        };
        Some(Arc::new((*root).clone()))
    }
}
// TODO replace the logic for finding and marking as deleted all removed nodes
// at hash time
// impl<S: ReadableStorage> NodeWriter for NodeStore<ProposedMutable2, S> {
//     fn delete_node(&mut self, addr: LinearAddress) -> Result<(), Error> {
//         self.deleted.push(addr);
//         Ok(())
//     }
// }

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::linear::memory::MemStore;

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

    // TODO add new tests
    // #[test]
    // fn test_create() {
    //     let memstore = Arc::new(MemStore::new(vec![]));
    //     let mut node_store = NodeStore::new_empty_proposal(memstore);

    //     let leaf = Node::Leaf(LeafNode {
    //         partial_path: Path::from([0, 1, 2]),
    //         value: Box::new([3, 4, 5]),
    //     });

    //     let leaf_addr = node_store.create_node(leaf.clone()).unwrap();
    //     let got_leaf = node_store.kind.new.get(&leaf_addr).unwrap();
    //     assert_eq!(**got_leaf, leaf);
    //     let got_leaf = node_store.read_node(leaf_addr).unwrap();
    //     assert_eq!(*got_leaf, leaf);

    //     // The header should be unchanged in storage
    //     {
    //         let mut header_bytes = node_store.storage.stream_from(0).unwrap();
    //         let header: NodeStoreHeader = bincode::deserialize_from(&mut header_bytes).unwrap();
    //         assert_eq!(header.version, Version::new());
    //         let empty_free_lists: FreeLists = Default::default();
    //         assert_eq!(header.free_lists, empty_free_lists);
    //         assert_eq!(header.root_address, None);
    //     }

    //     // Leaf should go right after the header
    //     assert_eq!(leaf_addr.get(), NodeStoreHeader::SIZE);

    //     // Create another node
    //     let branch = Node::Branch(Box::new(BranchNode {
    //         partial_path: Path::from([6, 7, 8]),
    //         value: Some(vec![9, 10, 11].into_boxed_slice()),
    //         children: Default::default(),
    //     }));

    //     let old_size = node_store.header.size;
    //     let branch_addr = node_store.create_node(branch.clone()).unwrap();
    //     assert!(node_store.header.size > old_size);

    //     // branch should go after leaf
    //     assert!(branch_addr.get() > leaf_addr.get());

    //     // The header should be unchanged in storage
    //     {
    //         let mut header_bytes = node_store.storage.stream_from(0).unwrap();
    //         let header: NodeStoreHeader = bincode::deserialize_from(&mut header_bytes).unwrap();
    //         assert_eq!(header.version, Version::new());
    //         let empty_free_lists: FreeLists = Default::default();
    //         assert_eq!(header.free_lists, empty_free_lists);
    //         assert_eq!(header.root_address, None);
    //     }
    // }

    // #[test]
    // fn test_delete() {
    //     let memstore = Arc::new(MemStore::new(vec![]));
    //     let mut node_store = NodeStore::new_empty_proposal(memstore);

    //     // Create a leaf
    //     let leaf = Node::Leaf(LeafNode {
    //         partial_path: Path::new(),
    //         value: Box::new([1]),
    //     });
    //     let leaf_addr = node_store.create_node(leaf.clone()).unwrap();

    //     // Delete the node
    //     node_store.delete_node(leaf_addr).unwrap();
    //     assert!(node_store.kind.deleted.contains(&leaf_addr));

    //     // Create a new node with the same size
    //     let new_leaf_addr = node_store.create_node(leaf).unwrap();

    //     // The new node shouldn't be at the same address
    //     assert_ne!(new_leaf_addr, leaf_addr);
    // }

    #[test]
    fn test_node_store_new() {
        let memstore = MemStore::new(vec![]);
        let node_store = NodeStore::new_empty_proposal(memstore.into());

        // Check the empty header is written at the start of the ReadableStorage.
        let mut header_bytes = node_store.storage.stream_from(0).unwrap();
        let header: NodeStoreHeader = bincode::deserialize_from(&mut header_bytes).unwrap();
        assert_eq!(header.version, Version::new());
        let empty_free_list: FreeLists = Default::default();
        assert_eq!(header.free_lists, empty_free_list);
    }
}
