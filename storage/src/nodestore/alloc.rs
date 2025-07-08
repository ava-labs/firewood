// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! # Allocation Module
//!
//! This module handles memory allocation and space management for nodes in the nodestore's
//! linear storage, implementing a malloc-like free space management system.
//!
//! ### Area Sizes
//! Storage is divided into 23 predefined area sizes from 16 bytes to 16MB:
//! - Small sizes (16, 32, 64, 96, 128, 256, 512, 768, 1024 bytes) for common nodes
//! - Power-of-two larger sizes (2KB, 4KB, 8KB, ..., 16MB) for larger data
//!
//! ### Storage Format
//! Each stored area follows this layout:
//! ```text
//! [AreaIndex:1][AreaType:1][NodeData:n]
//! ```
//! - **`AreaIndex`** - Index into `AREA_SIZES` array (1 byte)
//! - **`AreaType`** - 0xFF for free areas, otherwise node type data (1 byte)
//! - **`NodeData`** - Serialized node content

use crate::linear::FileIoError;
use crate::logger::trace;
use bincode::{DefaultOptions, Options as _};
use metrics::counter;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Error, ErrorKind, Read};
use std::num::NonZeroU64;
use std::sync::Arc;

use crate::node::persist::MaybePersistedNode;
use crate::node::{ByteCounter, Node};
use crate::{CacheReadStrategy, ReadableStorage, SharedNode, TrieHash};

use crate::linear::WritableStorage;

/// [`NodeStore`] divides the linear store into blocks of different sizes.
/// [`AREA_SIZES`] is every valid block size.
pub const AREA_SIZES: [u64; 23] = [
    16, // Min block size
    32,
    64,
    96,
    128,
    256,
    512,
    768,
    1024,
    1024 << 1,
    1024 << 2,
    1024 << 3,
    1024 << 4,
    1024 << 5,
    1024 << 6,
    1024 << 7,
    1024 << 8,
    1024 << 9,
    1024 << 10,
    1024 << 11,
    1024 << 12,
    1024 << 13,
    1024 << 14,
];

pub fn serializer() -> impl bincode::Options {
    DefaultOptions::new().with_varint_encoding()
}

pub fn area_size_hash() -> TrieHash {
    let mut hasher = Sha256::new();
    for size in AREA_SIZES {
        hasher.update(size.to_ne_bytes());
    }
    hasher.finalize().into()
}

// TODO: automate this, must stay in sync with above
pub const fn index_name(index: usize) -> &'static str {
    match index {
        0 => "16",
        1 => "32",
        2 => "64",
        3 => "96",
        4 => "128",
        5 => "256",
        6 => "512",
        7 => "768",
        8 => "1024",
        9 => "2048",
        10 => "4096",
        11 => "8192",
        12 => "16384",
        13 => "32768",
        14 => "65536",
        15 => "131072",
        16 => "262144",
        17 => "524288",
        18 => "1048576",
        19 => "2097152",
        20 => "4194304",
        21 => "8388608",
        22 => "16777216",
        _ => "unknown",
    }
}

/// The type of an index into the [`AREA_SIZES`] array
/// This is not usize because we can store this as a single byte
pub type AreaIndex = u8;

pub const NUM_AREA_SIZES: usize = AREA_SIZES.len();
pub const MIN_AREA_SIZE: u64 = AREA_SIZES[0];
pub const MAX_AREA_SIZE: u64 = AREA_SIZES[NUM_AREA_SIZES - 1];

#[inline]
pub fn new_area_index(n: usize) -> AreaIndex {
    n.try_into().expect("Area index out of bounds")
}

/// Returns the index in `BLOCK_SIZES` of the smallest block size >= `n`.
pub fn area_size_to_index(n: u64) -> Result<AreaIndex, Error> {
    if n > MAX_AREA_SIZE {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("Node size {n} is too large"),
        ));
    }

    if n <= MIN_AREA_SIZE {
        return Ok(0);
    }

    AREA_SIZES
        .iter()
        .position(|&size| size >= n)
        .map(new_area_index)
        .ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Node size {n} is too large"),
            )
        })
}

/// Objects cannot be stored at the zero address, so a [`LinearAddress`] is guaranteed not
/// to be zero. This reserved zero can be used as a [None] value for some use cases. In particular,
/// branches can use `Option<LinearAddress>` which is the same size as a [`LinearAddress`]
pub type LinearAddress = NonZeroU64;

/// Each [`StoredArea`] contains an [Area] which is either a [Node] or a [`FreeArea`].
#[repr(u8)]
#[derive(PartialEq, Eq, Clone, Debug, Deserialize, Serialize)]
pub enum Area<T, U> {
    Node(T),
    Free(U) = 255, // this is magic: no node starts with a byte of 255
}

/// Every item stored in the [`NodeStore`]'s `ReadableStorage`  after the
/// `NodeStoreHeader` is a [`StoredArea`].
///
/// As an overview of what this looks like stored, we get something like this:
///  - Byte 0: The index of the area size
///  - Byte 1: 0x255 if free, otherwise the low-order bit indicates Branch or Leaf
///  - Bytes 2..n: The actual data
#[derive(PartialEq, Eq, Clone, Debug, Deserialize, Serialize)]
pub struct StoredArea<T> {
    /// Index in [`AREA_SIZES`] of this area's size
    area_size_index: AreaIndex,
    area: T,
}

impl<T> StoredArea<T> {
    /// Create a new `StoredArea`
    pub const fn new(area_size_index: AreaIndex, area: T) -> Self {
        Self {
            area_size_index,
            area,
        }
    }

    /// Destructure the `StoredArea` into its components
    pub fn into_parts(self) -> (AreaIndex, T) {
        (self.area_size_index, self.area)
    }
}

pub type FreeLists = [Option<LinearAddress>; NUM_AREA_SIZES];

/// A [`FreeArea`] is stored at the start of the area that contained a node that
/// has been freed.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub struct FreeArea {
    next_free_block: Option<LinearAddress>,
}

impl FreeArea {
    /// Create a new `FreeArea`
    pub const fn new(next_free_block: Option<LinearAddress>) -> Self {
        Self { next_free_block }
    }

    /// Get the next free block address
    pub const fn next_free_block(self) -> Option<LinearAddress> {
        self.next_free_block
    }
}

impl FreeArea {
    pub fn from_storage<S: ReadableStorage>(
        storage: &S,
        address: LinearAddress,
    ) -> Result<(Self, AreaIndex), FileIoError> {
        let free_area_addr = address.get();
        let stored_area_stream = storage.stream_from(free_area_addr)?;
        let stored_area: StoredArea<Area<Node, FreeArea>> = serializer()
            .deserialize_from(stored_area_stream)
            .map_err(|e| {
                storage.file_io_error(
                    Error::new(ErrorKind::InvalidData, e),
                    free_area_addr,
                    Some("FreeArea::from_storage".to_string()),
                )
            })?;
        let (stored_area_index, area) = stored_area.into_parts();
        let Area::Free(free_area) = area else {
            return Err(storage.file_io_error(
                Error::new(ErrorKind::InvalidData, "Attempted to read a non-free area"),
                free_area_addr,
                Some("FreeArea::from_storage".to_string()),
            ));
        };

        Ok((free_area, stored_area_index as AreaIndex))
    }
}

// Re-export the NodeStore types we need
use super::{Committed, ImmutableProposal, NodeStore, ReadInMemoryNode};

impl<T: ReadInMemoryNode, S: ReadableStorage> NodeStore<T, S> {
    /// Returns (index, `area_size`) for the stored area at `addr`.
    /// `index` is the index of `area_size` in the array of valid block sizes.
    ///
    /// # Errors
    ///
    /// Returns a [`FileIoError`] if the area cannot be read.
    pub fn area_index_and_size(
        &self,
        addr: LinearAddress,
    ) -> Result<(AreaIndex, u64), FileIoError> {
        let mut area_stream = self.storage.stream_from(addr.get())?;

        let index: AreaIndex = serializer()
            .deserialize_from(&mut area_stream)
            .map_err(|e| {
                self.storage.file_io_error(
                    Error::new(ErrorKind::InvalidData, e),
                    addr.get(),
                    Some("deserialize".to_string()),
                )
            })?;

        let size = *AREA_SIZES
            .get(index as usize)
            .ok_or(self.storage.file_io_error(
                Error::other(format!("Invalid area size index {index}")),
                addr.get(),
                None,
            ))?;

        Ok((index, size))
    }

    /// Read a [Node] from the provided [`LinearAddress`].
    /// `addr` is the address of a `StoredArea` in the `ReadableStorage`.
    ///
    /// # Errors
    ///
    /// Returns a [`FileIoError`] if the node cannot be read.
    pub fn read_node_from_disk(
        &self,
        addr: LinearAddress,
        mode: &'static str,
    ) -> Result<SharedNode, FileIoError> {
        if let Some(node) = self.storage.read_cached_node(addr, mode) {
            return Ok(node);
        }

        debug_assert!(addr.get() % 8 == 0);

        // saturating because there is no way we can be reading at u64::MAX
        // and this will fail very soon afterwards
        let actual_addr = addr.get().saturating_add(1); // skip the length byte

        let _span = fastrace::local::LocalSpan::enter_with_local_parent("read_and_deserialize");

        let area_stream = self.storage.stream_from(actual_addr)?;
        let node: SharedNode = Node::from_reader(area_stream)
            .map_err(|e| {
                self.storage
                    .file_io_error(e, actual_addr, Some("read_node_from_disk".to_string()))
            })?
            .into();
        match self.storage.cache_read_strategy() {
            CacheReadStrategy::All => {
                self.storage.cache_node(addr, node.clone());
            }
            CacheReadStrategy::BranchReads => {
                if !node.is_leaf() {
                    self.storage.cache_node(addr, node.clone());
                }
            }
            CacheReadStrategy::WritesOnly => {}
        }
        Ok(node)
    }

    /// Read a [Node] from the provided [`LinearAddress`] and size.
    /// This is an uncached read, primarily used by check utilities
    ///
    /// # Errors
    ///
    /// Returns a [`FileIoError`] if the node cannot be read.
    pub fn uncached_read_node_and_size(
        &self,
        addr: LinearAddress,
    ) -> Result<(SharedNode, u8), FileIoError> {
        let mut area_stream = self.storage.stream_from(addr.get())?;
        let mut size = [0u8];
        area_stream.read_exact(&mut size).map_err(|e| {
            self.storage.file_io_error(
                e,
                addr.get(),
                Some("uncached_read_node_and_size".to_string()),
            )
        })?;
        self.storage.stream_from(addr.get().saturating_add(1))?;
        let node: SharedNode = Node::from_reader(area_stream)
            .map_err(|e| {
                self.storage.file_io_error(
                    e,
                    addr.get(),
                    Some("uncached_read_node_and_size".to_string()),
                )
            })?
            .into();
        Ok((node, size[0]))
    }

    /// Get the size of an area index (used by the checker)
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds for the `AREA_SIZES` array.
    #[must_use]
    pub const fn size_from_area_index(index: AreaIndex) -> u64 {
        #[expect(clippy::indexing_slicing)]
        AREA_SIZES[index as usize]
    }
}

impl<S: ReadableStorage> NodeStore<Arc<ImmutableProposal>, S> {
    /// Attempts to allocate `n` bytes from the free lists.
    /// If successful returns the address of the newly allocated area
    /// and the index of the free list that was used.
    /// If there are no free areas big enough for `n` bytes, returns None.
    /// TODO Consider splitting the area if we return a larger area than requested.
    #[expect(clippy::indexing_slicing)]
    fn allocate_from_freed(
        &mut self,
        n: u64,
    ) -> Result<Option<(LinearAddress, AreaIndex)>, FileIoError> {
        // Find the smallest free list that can fit this size.
        let index_wanted = area_size_to_index(n).map_err(|e| {
            self.storage
                .file_io_error(e, 0, Some("allocate_from_freed".to_string()))
        })?;

        if let Some((index, free_stored_area_addr)) = self
            .header
            .free_lists_mut()
            .iter_mut()
            .enumerate()
            .skip(index_wanted as usize)
            .find(|item| item.1.is_some())
        {
            let address = free_stored_area_addr
                .take()
                .expect("impossible due to find earlier");
            // Get the first free block of sufficient size.
            if let Some(free_head) = self.storage.free_list_cache(address) {
                trace!("free_head@{address}(cached): {free_head:?} size:{index}");
                *free_stored_area_addr = free_head;
            } else {
                let (free_head, read_index) =
                    FreeArea::from_storage(self.storage.as_ref(), address)?;
                debug_assert_eq!(read_index as usize, index);

                // Update the free list to point to the next free block.
                *free_stored_area_addr = free_head.next_free_block;
            }

            counter!("firewood.space.reused", "index" => index_name(index))
                .increment(AREA_SIZES[index]);
            counter!("firewood.space.wasted", "index" => index_name(index))
                .increment(AREA_SIZES[index].saturating_sub(n));

            // Return the address of the newly allocated block.
            trace!("Allocating from free list: addr: {address:?}, size: {index}");
            return Ok(Some((address, index as AreaIndex)));
        }

        trace!("No free blocks of sufficient size {index_wanted} found");
        counter!("firewood.space.from_end", "index" => index_name(index_wanted as usize))
            .increment(AREA_SIZES[index_wanted as usize]);
        Ok(None)
    }

    #[expect(clippy::indexing_slicing)]
    fn allocate_from_end(&mut self, n: u64) -> Result<(LinearAddress, AreaIndex), FileIoError> {
        let index = area_size_to_index(n).map_err(|e| {
            self.storage
                .file_io_error(e, 0, Some("allocate_from_end".to_string()))
        })?;
        let area_size = AREA_SIZES[index as usize];
        let addr = LinearAddress::new(self.header.size()).expect("node store size can't be 0");
        self.header
            .set_size(self.header.size().saturating_add(area_size));
        debug_assert!(addr.get() % 8 == 0);
        trace!("Allocating from end: addr: {addr:?}, size: {index}");
        Ok((addr, index))
    }

    /// Returns the length of the serialized area for a node.
    #[must_use]
    pub fn stored_len(node: &Node) -> u64 {
        let mut bytecounter = ByteCounter::new();
        node.as_bytes(0, &mut bytecounter);
        bytecounter.count()
    }

    /// Returns an address that can be used to store the given `node` and updates
    /// `self.header` to reflect the allocation. Doesn't actually write the node to storage.
    /// Also returns the index of the free list the node was allocated from.
    ///
    /// # Errors
    ///
    /// Returns a [`FileIoError`] if the node cannot be allocated.
    pub fn allocate_node(
        &mut self,
        node: &Node,
    ) -> Result<(LinearAddress, AreaIndex), FileIoError> {
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
}

impl<S: WritableStorage> NodeStore<Committed, S> {
    /// Deletes the [Node] at the given address, updating the next pointer at
    /// the given addr, and changing the header of this committed nodestore to
    /// have the address on the freelist
    ///
    /// # Errors
    ///
    /// Returns a [`FileIoError`] if the node cannot be deleted.
    #[expect(clippy::indexing_slicing)]
    pub fn delete_node(&mut self, node: MaybePersistedNode) -> Result<(), FileIoError> {
        let Some(addr) = node.as_linear_address() else {
            return Ok(());
        };
        debug_assert!(addr.get() % 8 == 0);

        let (area_size_index, _) = self.area_index_and_size(addr)?;
        trace!("Deleting node at {addr:?} of size {area_size_index}");
        counter!("firewood.delete_node", "index" => index_name(area_size_index as usize))
            .increment(1);
        counter!("firewood.space.freed", "index" => index_name(area_size_index as usize))
            .increment(AREA_SIZES[area_size_index as usize]);

        // The area that contained the node is now free.
        let area: Area<Node, FreeArea> = Area::Free(FreeArea::new(
            self.header.free_lists()[area_size_index as usize],
        ));

        let stored_area = StoredArea::new(area_size_index, area);

        let stored_area_bytes = serializer().serialize(&stored_area).map_err(|e| {
            self.storage.file_io_error(
                Error::new(ErrorKind::InvalidData, e),
                addr.get(),
                Some("delete_node".to_string()),
            )
        })?;

        self.storage.write(addr.into(), &stored_area_bytes)?;

        self.storage
            .add_to_free_list_cache(addr, self.header.free_lists()[area_size_index as usize]);

        // The newly freed block is now the head of the free list.
        self.header.free_lists_mut()[area_size_index as usize] = Some(addr);

        Ok(())
    }
}
