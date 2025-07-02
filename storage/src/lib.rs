// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![warn(missing_debug_implementations, rust_2018_idioms, missing_docs)]
#![deny(unsafe_code)]
#![cfg_attr(
    not(target_pointer_width = "64"),
    forbid(
        clippy::cast_possible_truncation,
        reason = "non-64 bit target likely to cause issues during u64 to usize conversions"
    )
)]

//! # storage implements the storage of a [Node] on top of a `LinearStore`
//!
//! Nodes are stored at a [`LinearAddress`] within a [`ReadableStorage`].
//!
//! The [`NodeStore`] maintains a free list and the [`LinearAddress`] of a root node.
//!
//! A [`NodeStore`] is backed by a [`ReadableStorage`] which is persisted storage.

use std::ops::Range;
use thiserror::Error;

mod checker;
mod hashednode;
mod hashers;
mod linear;
mod node;
mod nodestore;
mod range_set;
mod trie_hash;

/// Logger module for handling logging functionality
pub mod logger;

// re-export these so callers don't need to know where they are
pub use hashednode::{Hashable, Preimage, ValueDigest, hash_node, hash_preimage};
pub use linear::{FileIoError, ReadableStorage, WritableStorage};
pub use node::path::{NibblesIterator, Path};
pub use node::{BranchNode, Child, LeafNode, Node, PathIterItem, branch::HashType};
pub use nodestore::{
    Committed, HashedNodeReader, ImmutableProposal, LinearAddress, MutableProposal, NodeReader,
    NodeStore, Parentable, ReadInMemoryNode, RootReader, TrieReader, UpdateError,
};

pub use linear::filebacked::FileBacked;
pub use linear::memory::MemStore;

pub use trie_hash::TrieHash;

/// A shared node, which is just a triophe Arc of a node
pub type SharedNode = triomphe::Arc<Node>;

/// The strategy for caching nodes that are read
/// from the storage layer. Generally, we only want to
/// cache write operations, but for some read-heavy workloads
/// you can enable caching of branch reads or all reads.
#[derive(Clone, Debug)]
pub enum CacheReadStrategy {
    /// Only cache writes (no reads will be cached)
    WritesOnly,

    /// Cache branch reads (reads that are not leaf nodes)
    BranchReads,

    /// Cache all reads (leaves and branches)
    All,
}

impl std::fmt::Display for CacheReadStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

/// Returns the hash of an empty trie, which is the Keccak256 hash of the RLP encoding of an empty byte array.
///
/// This function is slow, so callers should cache the result
#[cfg(feature = "ethhash")]
#[must_use]
#[expect(
    clippy::missing_panics_doc,
    reason = "Found 1 occurrences after enabling the lint."
)]
pub fn empty_trie_hash() -> TrieHash {
    use sha3::Digest as _;

    sha3::Keccak256::digest(rlp::NULL_RLP)
        .as_slice()
        .try_into()
        .expect("empty trie hash is 32 bytes")
}

/// Errors returned by the checker
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum CheckerError {
    /// The file size is not valid
    #[error("Invalid DB size ({db_size}): {description}")]
    InvalidDBSize {
        /// The size of the db
        db_size: u64,
        /// The description of the error
        description: String,
    },

    /// The address is out of bounds
    #[error("stored area at {start} with size {size} is out of bounds ({bounds:?})")]
    AreaOutOfBounds {
        /// Start of the `StoredArea`
        start: LinearAddress,
        /// Size of the `StoredArea`
        size: u64,
        /// Valid range of addresses
        bounds: Range<LinearAddress>,
    },

    /// Stored areas intersect
    #[error(
        "stored area at {start} with size {size} intersects with other stored areas: {intersection:?}"
    )]
    AreaIntersects {
        /// Start of the `StoredArea`
        start: LinearAddress,
        /// Size of the `StoredArea`
        size: u64,
        /// The intersection
        intersection: Vec<Range<LinearAddress>>,
    },

    /// IO error
    #[error("IO error")]
    IO(#[from] FileIoError),
}
