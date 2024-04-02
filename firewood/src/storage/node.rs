// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

/// The [NodeStore] handles the serialization of nodes and
/// free space management of nodes in the page store. It lays out the format
/// of the [PageStore]. More specifically, it places a [FileIdentifyingMagic]
/// and a [FreeSpaceHeader] at the beginning
use std::fmt::Debug;
use std::io::{Error, ErrorKind, Read, Write};
use std::num::NonZeroU64;
use std::sync::Arc;

use enum_as_inner::EnumAsInner;
use serde::{Deserialize, Serialize};

use super::linear::{LinearStore, ReadOnlyLinearStore, ReadWriteLinearStore};

/// [NodeStore] divides the linear store into blocks of different sizes.
/// [BLOCK_SIZES] is every valid block size.
const BLOCK_SIZES: [u64; 18] = [
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
    1 << 20, // Max block size is 1 MiB
];

const NUM_BLOCK_SIZES: usize = BLOCK_SIZES.len();

/// Either a branch or leaf node
#[derive(PartialEq, Eq, Clone, Debug, EnumAsInner, Deserialize, Serialize)]
enum Node {
    Branch(Branch),
    Leaf(Leaf),
}

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

#[derive(Debug)]
struct NodeStore<T> {
    header: FreeSpaceManagementHeader,
    page_store: LinearStore<T>,
}

impl<T: ReadOnlyLinearStore + std::fmt::Debug> NodeStore<T> {
    /// Read a node from the provided [DiskAddress]
    ///
    /// A node on disk will consist of a header which both identifies the
    /// node type ([Branch] or [Leaf]) followed by the serialized data
    fn read(&self, addr: DiskAddress) -> Result<Arc<Node>, Error> {
        let node_stream = self.page_store.stream_from(addr.get())?;
        let node: Node = bincode::deserialize_from(node_stream)
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
}

impl<T: ReadWriteLinearStore + ReadOnlyLinearStore + std::fmt::Debug> NodeStore<T> {
    /// Allocate space for a [Node] in the [PageStore]
    fn create(&mut self, node: &Node) -> Result<DiskAddress, Error> {
        let serialized =
            bincode::serialize(node).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
        // TODO: search for a free space block we can reuse
        let addr = self.page_store.size()?;
        self.page_store.write(addr, serialized.as_slice())?;
        Ok(addr.try_into().expect("pageStore is never zero size"))
    }

    /// Update a [Node] that was previously at the provided address.
    /// This is complicated by the fact that a node might grow and not be able to fit a the given
    /// address, in which case we return [UpdateError::NodeMoved]
    fn update(&mut self, addr: DiskAddress, node: &Node) -> Result<(), UpdateError> {
        // figure out how large the object at this address is by deserializing and then
        // discarding the object
        let size = self.node_size(addr)?;
        let serialized =
            bincode::serialize(node).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
        if serialized.len() != size {
            // this node is a different size, so allocate a new node
            // TODO: we could do better if the new node is smaller
            let new_address = self.create(node)?;
            self.delete(addr)?;
            Err(UpdateError::NodeMoved(new_address))
        } else {
            self.page_store.write(addr.into(), serialized.as_slice())?;
            Ok(())
        }
    }

    /// Delete a [Node] at a given address
    fn delete(&mut self, addr: DiskAddress) -> Result<(), Error> {
        // figure out how large the object at this address is by deserializing and then
        // discarding the object
        let size = self.node_size(addr)?;

        let free_space_heads_index = size_to_block_index(size).ok_or_else(|| {
            Error::new(
                ErrorKind::InvalidData,
                format!("Node size {} is too large", size),
            )
        })?;

        // The newly freed block points to the current head of the free list.
        let freed_block = FreedBlock {
            size,
            next_free_block: self.header.free_space_heads[free_space_heads_index],
        };

        // The newly freed block is now the head of the free list.
        self.header.free_space_heads[free_space_heads_index] = Some(addr);

        self.page_store
            .write(addr.into(), bytemuck::bytes_of(&freed_block))?;

        // update the free space header
        self.page_store.write(
            std::mem::size_of::<FileIdentifingMagic>() as u64,
            bytemuck::bytes_of(&self.header),
        )?;
        Ok(())
    }

    /// Initialize a new [NodeStore] by writing out the [FileIdentifingMagic] and [FreeSpaceManagementHeader]
    fn init(mut page_store: LinearStore<T>) -> Result<Self, Error> {
        // Write the first 16 bytes of each [PageStore]
        page_store.write(0, bytemuck::bytes_of(&FileIdentifingMagic::new()))?;

        let header = FreeSpaceManagementHeader {
            free_space_heads: [None; NUM_BLOCK_SIZES],
        };
        page_store.write(FileIdentifingMagic::SIZE, bytemuck::bytes_of(&header))?;
        Ok(Self { header, page_store })
    }
}

/// Returns the index in [BLOCK_SIZES] of the smallest block size
/// that can fit the given [size]. Returns None if the size is too large.
/// TODO danlaine: This can probably be optimized.
fn size_to_block_index(size: usize) -> Option<usize> {
    BLOCK_SIZES
        .iter()
        .position(|&block_size| block_size >= size as u64)
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

/// This structure lives at the beginning of the PageStore and is a constant
/// This makes it impossible to update() things at address 0 as a safety feature
/// The header contents can be used by filesystem tooling such as "file" to identify
/// the version of firewood used to create the file.
#[repr(C)]
#[derive(Debug, bytemuck::NoUninit, Clone, Copy)]
struct FileIdentifingMagic {
    magic: [u8; 16],
}

impl FileIdentifingMagic {
    /// The size of [FileIdentifyingMagic] also happens to be the offset
    /// of the the [FreeSpaceManager]
    const SIZE: u64 = std::mem::size_of::<Self>() as u64;

    /// construct a FileIdentifyingMagic from the version of firewood
    fn new() -> Self {
        let mut magic: [u8; Self::SIZE as usize] = Default::default();
        let version = env!("CARGO_PKG_VERSION");
        let _ = magic
            .as_mut_slice()
            .write_all(format!("firewood {}", version).as_bytes());
        Self { magic }
    }
}

/// Immediately following the FileIdentifyingMagic is the FreeSpaceManagementHeader
/// It points to the heads of the free block lists.
#[repr(C)]
#[derive(Debug, bytemuck::NoUninit, Clone, Copy)]
struct FreeSpaceManagementHeader {
    /// Element i is the first free block of size BLOCK_SIZES[i].
    free_space_heads: [Option<DiskAddress>; NUM_BLOCK_SIZES],
}

/// A [FreedBlock] is the object stored where a node used to be when it has been
/// freed
#[repr(C)]
#[derive(Debug, bytemuck::NoUninit, Clone, Copy)]
struct FreedBlock {
    size: usize,
    next_free_block: Option<DiskAddress>,
}
