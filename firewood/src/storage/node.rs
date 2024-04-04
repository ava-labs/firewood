// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

/// The [NodeStore] handles the serialization of nodes and
/// free space management of nodes in the page store. It lays out the format
/// of the [PageStore]. More specifically, it places a [FileIdentifyingMagic]
/// and a [FreeSpaceHeader] at the beginning
use std::io::{Error, ErrorKind, Read, Write};
use std::num::NonZeroU64;
use std::sync::Arc;

use enum_as_inner::EnumAsInner;
use serde::{Deserialize, Serialize};

use super::linear::{LinearStore, ReadLinearStore, WriteLinearStore};

/// [NodeStore] divides the linear store into blocks of different sizes.
/// [AREA_SIZES] is every valid block size.
const AREA_SIZES: [u64; 21] = [
    1 << 3, // Min block size is 8
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

const NUM_AREA_SIZES: usize = AREA_SIZES.len();
const MIN_AREA_SIZE: u64 = AREA_SIZES[0];
const MAX_AREA_SIZE: u64 = AREA_SIZES[NUM_AREA_SIZES - 1];

/// Number of children in a branch
const BRANCH_CHILDREN: usize = 16;

type Path = Box<[u8]>;
type DiskAddress = NonZeroU64;

#[derive(PartialEq, Eq, Clone, Debug, Deserialize, Serialize)]
struct Branch {
    path: Path,
    value: Option<Box<[u8]>>,
    children: [Option<DiskAddress>; BRANCH_CHILDREN],
}

#[derive(PartialEq, Eq, Clone, Debug, Deserialize, Serialize)]
struct Leaf {
    path: Path,
    value: Box<[u8]>,
}

/// A branch, a leaf, or a freed area.
#[repr(u8)]
#[derive(PartialEq, Eq, Clone, Debug, EnumAsInner, Deserialize, Serialize)]
enum Area {
    Branch(Branch) = 1,
    Leaf(Leaf) = 2,
    Free(FreedArea) = 3,
}

#[derive(PartialEq, Eq, Clone, Debug, Serialize)]
struct StoredArea<'a> {
    /// Size of this area is at this index in [AREA_SIZES]
    area_sizes_index: u8,
    area: &'a Area,
}

#[derive(Debug)]
struct NodeStore<T: ReadLinearStore> {
    header: NodeStoreHeader,
    page_store: LinearStore<T>,
}

impl<T: ReadLinearStore> NodeStore<T> {
    /// Read a node from the provided [DiskAddress]
    ///
    /// A node on disk will consist of a header which both identifies the
    /// node type ([Branch] or [Leaf]) followed by the serialized data
    fn read(&self, addr: DiskAddress) -> Result<Arc<Area>, Error> {
        let node_stream = self.page_store.stream_from(addr.get())?;
        let node: Area = bincode::deserialize_from(node_stream)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
        Ok(Arc::new(node))
    }

    /// Determine the size of the existing node at the given address
    /// TODO: Let's write the length as a constant 4 bytes at the beginning of the block
    /// and skip forward when stream deserializing like this
    fn node_size(&self, addr: DiskAddress) -> Result<usize, Error> {
        let node_stream = self.page_store.stream_from(addr.get())?;
        let mut reader = ReaderWrapperWithSize::new(Box::new(node_stream));
        bincode::deserialize_from(&mut reader)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

        Ok(reader.bytes_read)
    }

    /// Returns (index, size) for the [StoredArea] at `addr`
    /// where `index` is the index of the area size in [AREA_SIZES]
    /// and `size` is the size of the area.
    fn stored_area_size(&self, addr: DiskAddress) -> Result<(u8, u64), Error> {
        let node_stream = self.page_store.stream_from(addr.get())?;
        let mut reader = ReaderWrapperWithSize::new(Box::new(node_stream));
        let index: u8 = bincode::deserialize_from(&mut reader)
            .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
        if index as usize >= NUM_AREA_SIZES {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("Invalid area size index {}", index),
            ));
        }

        Ok((index, AREA_SIZES[index as usize]))
    }
}

impl<T: WriteLinearStore + ReadLinearStore> NodeStore<T> {
    /// Attempts to allocate `n` bytes from the free lists.
    /// If successful return the address of the newly allocated area
    /// and the index of the free list that was used.
    /// If there are no free areas big enough for `n` bytes, returns None.
    fn allocate_from_freed(&mut self, n: u64) -> Result<Option<(DiskAddress, u8)>, Error> {
        // Add 1 to account for the discriminant byte that we will write
        // to the beginning of the area.
        let n = n + 1;

        // Find the smallest free list that can fit this size.
        let index = size_to_free_lists_index(n)?;

        for index in index as usize..=NUM_AREA_SIZES {
            // Get the first free block of sufficient size.
            let free_head_addr = self.header.free_lists[index];
            if let Some(free_head_addr) = free_head_addr {
                // Update the free list head.
                let free_head_stream = self.page_store.stream_from(free_head_addr.get())?;
                let free_head: FreedArea = bincode::deserialize_from(free_head_stream)
                    .map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

                // Update the free list to point to the next free block.
                self.header.free_lists[index] = free_head.next_free_block;

                // self.page_store.write(offset, object);

                // Return the address of the newly allocated block.
                return Ok(Some((free_head_addr, index as u8)));
            }
            // No free blocks in this list, try the next size up.
        }

        Ok(None)
    }

    /// Allocate space for a [Node] in the [LinearStore]
    fn create(&mut self, node: &Area) -> Result<DiskAddress, Error> {
        let serialized =
            bincode::serialize(node).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

        let node_size = serialized.len() as u64;

        // Attempt to allocate from the free list.
        // If we can't allocate from the free list, allocate past the existing
        // of the store.
        let (addr, index) = match self.allocate_from_freed(node_size)? {
            Some((addr, index)) => (addr.get(), index),
            None => (
                self.page_store.size()?,
                size_to_free_lists_index(node_size)? as u8,
            ),
        };

        let stored_area = StoredArea {
            area_sizes_index: index,
            area: node,
        };

        let stored_area_bytes =
            bincode::serialize(&stored_area).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

        self.page_store.write(addr, stored_area_bytes.as_slice())?;
        Ok(addr.try_into().expect("LinearStore is never zero size"))
    }

    /// Update a [Node] that was previously at the provided address.
    /// This is complicated by the fact that a node might grow and not be able to fit a the given
    /// address, in which case we return [UpdateError::NodeMoved]
    fn update(&mut self, addr: DiskAddress, node: &Area) -> Result<(), UpdateError> {
        // figure out how large the object at this address is by deserializing and then
        // discarding the object
        let (_, old_stored_area_size) = self.stored_area_size(addr)?;

        let new_node_bytes =
            bincode::serialize(node).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

        let new_stored_area_size = new_node_bytes.len() as u64 + 1; // +1 for the size index byte

        if new_stored_area_size <= old_stored_area_size {
            // the new node fits in the old node's area
            self.page_store
                .write(addr.into(), new_node_bytes.as_slice())?;
            return Ok(());
        }

        // the new node is larger than the old node, so we need to allocate a new area
        let new_node_addr = self.create(node)?;
        self.delete(addr)?;
        Err(UpdateError::NodeMoved(new_node_addr))
    }

    /// Delete a [Node] at a given address
    fn delete(&mut self, addr: DiskAddress) -> Result<(), Error> {
        let (area_size_index, _) = self.stored_area_size(addr)?;

        // The area that contained the node is now free.
        let freed_area = FreedArea {
            next_free_block: self.header.free_lists[area_size_index as usize],
        };

        let freed_area = Area::Free(freed_area);

        let stored_area = StoredArea {
            area_sizes_index: area_size_index,
            area: &freed_area,
        };

        // The newly freed block is now the head of the free list.
        self.header.free_lists[area_size_index as usize] = Some(addr);

        let stored_area_bytes =
            bincode::serialize(&stored_area).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;

        self.page_store.write(addr.into(), &stored_area_bytes)?;

        self.write_free_lists_header()?;
        Ok(())
    }

    fn write_free_lists_header(&mut self) -> Result<(), Error> {
        let header_bytes = bincode::serialize(&self.header.free_lists).map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to serialize free lists: {}", e),
            )
        })?;
        self.page_store.write(
            std::mem::size_of::<VersionHeader>() as u64,
            header_bytes.as_slice(),
        )?;
        Ok(())
    }

    /// Initialize a new [NodeStore] by writing out the [NodeStoreHeader].
    fn init(mut page_store: LinearStore<T>) -> Result<Self, Error> {
        let header = NodeStoreHeader {
            free_lists: [None; NUM_AREA_SIZES],
            version_header: VersionHeader::new(),
        };
        let header_bytes = bincode::serialize(&header).map_err(|e| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Failed to serialize header: {}", e),
            )
        })?;
        page_store.write(0, header_bytes.as_slice())?;
        Ok(Self { header, page_store })
    }
}

/// Returns the index in [BLOCK_SIZES] of the smallest block size
/// that can fit the given [size].
fn size_to_free_lists_index(size: u64) -> Result<u8, Error> {
    if size > MAX_AREA_SIZE {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("Node size {} is too large", size),
        ));
    }

    if size < MIN_AREA_SIZE {
        return Ok(0);
    }

    Ok(size.ilog2() as u8 - 2)
}

#[derive(Debug)]
enum UpdateError {
    Io(Error),
    NodeMoved(DiskAddress),
}

impl From<Error> for UpdateError {
    fn from(value: Error) -> Self {
        UpdateError::Io(value)
    }
}

#[derive(Debug)]
struct ReaderWrapperWithSize<T> {
    reader: T,
    bytes_read: usize,
}

impl<T> ReaderWrapperWithSize<T> {
    const fn new(reader: T) -> Self {
        Self {
            reader,
            bytes_read: 0,
        }
    }
}

impl<T: Read> Read for ReaderWrapperWithSize<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read_result = self.reader.read(buf)?;
        self.bytes_read += read_result;
        Ok(read_result)
    }
}

/// Can be used by filesystem tooling such as "file" to identify
/// the version of firewood used to create this [NodeStore] file.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
struct VersionHeader {
    bytes: [u8; 16],
}

impl VersionHeader {
    const SIZE: u64 = std::mem::size_of::<Self>() as u64;

    /// construct a [VersionHeader] from the firewood version
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

/// Persisted metadata for a [NodeStore].
/// The [NodeStoreHeader] is at the start of the [LinearStore].
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
struct NodeStoreHeader {
    /// Identifies the version of firewood used to create this [NodeStore].
    version_header: VersionHeader,
    /// Element i is the pointer to the first free block of size BLOCK_SIZES[i].
    free_lists: [Option<DiskAddress>; NUM_AREA_SIZES],
}

/// A [FreedArea] is stored at the start of the area that contained a node that
/// has been freed.
#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
struct FreedArea {
    next_free_block: Option<DiskAddress>,
}
