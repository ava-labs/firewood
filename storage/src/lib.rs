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

use std::fmt::{Display, Formatter, LowerHex, Result};
use std::ops::Range;

mod checker;
mod hashednode;
mod hashers;
mod iter;
mod linear;
mod node;
mod nodestore;
mod trie_hash;

use crate::nodestore::AreaIndex;

/// Logger module for handling logging functionality
pub mod logger;

#[macro_use]
/// Macros module for defining macros used in the storage module
pub mod macros;
// re-export these so callers don't need to know where they are
pub use checker::{CheckOpt, CheckerReport, FreeListsStats, TrieStats};
pub use hashednode::{Hashable, Preimage, ValueDigest, hash_node, hash_preimage};
pub use linear::{FileIoError, ReadableStorage, WritableStorage};
pub use node::path::{NibblesIterator, Path};
pub use node::{
    BranchNode, Child, Children, LeafNode, Node, PathIterItem,
    branch::{HashType, IntoHashType},
};
pub use nodestore::{
    Committed, HashedNodeReader, ImmutableProposal, LinearAddress, MutableProposal, NodeReader,
    NodeStore, Parentable, RootReader, TrieReader,
};

pub use linear::filebacked::FileBacked;
pub use linear::memory::MemStore;
pub use node::persist::MaybePersistedNode;
pub use trie_hash::{InvalidTrieHashLength, TrieHash};

/// A shared node, which is just a triophe Arc of a node
pub type SharedNode = triomphe::Arc<Node>;

/// The strategy for caching nodes that are read
/// from the storage layer. Generally, we only want to
/// cache write operations, but for some read-heavy workloads
/// you can enable caching of branch reads or all reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CacheReadStrategy {
    /// Only cache writes (no reads will be cached)
    WritesOnly,

    /// Cache branch reads (reads that are not leaf nodes)
    BranchReads,

    /// Cache all reads (leaves and branches)
    All,
}

impl Display for CacheReadStrategy {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{self:?}")
    }
}

/// This enum encapsulates what points to the stored area.
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum StoredAreaParent {
    /// The stored area is a trie node
    TrieNode(TrieNodeParent),
    /// The stored area is a free list
    FreeList(FreeListParent),
}

/// This enum encapsulates what points to the stored area allocated for a trie node.
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum TrieNodeParent {
    /// The stored area is the root of the trie, so the header points to it
    Root,
    /// The stored area is not the root of the trie, so a parent trie node points to it
    Parent(LinearAddress, usize),
}

/// This enum encapsulates what points to the stored area allocated for a free list.
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum FreeListParent {
    /// The stored area is the head of the free list, so the header points to it
    FreeListHead(AreaIndex),
    /// The stored area is not the head of the free list, so a previous free area points to it
    PrevFreeArea(LinearAddress),
}

impl LowerHex for StoredAreaParent {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            StoredAreaParent::TrieNode(trie_parent) => LowerHex::fmt(trie_parent, f),
            StoredAreaParent::FreeList(free_list_parent) => LowerHex::fmt(free_list_parent, f),
        }
    }
}

impl LowerHex for TrieNodeParent {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            TrieNodeParent::Root => f.write_str("Root"),
            TrieNodeParent::Parent(addr, index) => {
                f.write_str("TrieNode@")?;
                LowerHex::fmt(addr, f)?;
                f.write_fmt(format_args!("[{index}]"))
            }
        }
    }
}

impl LowerHex for FreeListParent {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            FreeListParent::FreeListHead(index) => f.write_fmt(format_args!("FreeLists[{index}]")),
            FreeListParent::PrevFreeArea(addr) => {
                f.write_str("FreeArea@")?;
                LowerHex::fmt(addr, f)
            }
        }
    }
}

use derive_where::derive_where;

/// Errors returned by the checker
#[derive(thiserror::Error, Debug)]
#[derive_where(PartialEq, Eq)]
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

    /// Hash mismatch for a node
    #[error(
        "Hash mismatch for node {path:?} at address {address}: parent stored {parent_stored_hash}, computed {computed_hash}"
    )]
    HashMismatch {
        /// The path of the node
        path: Path,
        /// The address of the node
        address: LinearAddress,
        /// The parent of the node
        parent: TrieNodeParent,
        /// The hash value stored in the parent node
        parent_stored_hash: HashType,
        /// The hash value computed for the node
        computed_hash: HashType,
    },

    /// The address is out of bounds
    #[error(
        "stored area at {start:#x} with size {size} (parent: {parent:#x}) is out of bounds ({bounds:#x?})"
    )]
    AreaOutOfBounds {
        /// Start of the `StoredArea`
        start: LinearAddress,
        /// Size of the `StoredArea`
        size: u64,
        /// Valid range of addresses
        bounds: Range<LinearAddress>,
        /// The parent of the `StoredArea`
        parent: StoredAreaParent,
    },

    /// Stored areas intersect
    #[error(
        "stored area at {start:#x} with size {size} (parent: {parent:#x}) intersects with other stored areas: {intersection:#x?}"
    )]
    AreaIntersects {
        /// Start of the `StoredArea`
        start: LinearAddress,
        /// Size of the `StoredArea`
        size: u64,
        /// The intersection
        intersection: Vec<Range<LinearAddress>>,
        /// The parent of the `StoredArea`
        parent: StoredAreaParent,
    },

    /// Node is larger than the area it is stored in
    #[error(
        "stored area at {area_start:#x} with size {area_size} (parent: {parent:#x}) stores a node of size {node_bytes}"
    )]
    NodeLargerThanArea {
        /// Address of the area
        area_start: LinearAddress,
        /// Size of the area
        area_size: u64,
        /// Size of the node
        node_bytes: u64,
        /// The parent of the area
        parent: TrieNodeParent,
    },

    /// Freelist area size does not match
    #[error(
        "Free area {address:#x} of size {size} (parent: {parent:#x}) is found in free list {actual_free_list} but it should be in freelist {expected_free_list}"
    )]
    FreelistAreaSizeMismatch {
        /// Address of the free area
        address: LinearAddress,
        /// Actual size of the free area
        size: u64,
        /// Free list on which the area is stored
        actual_free_list: AreaIndex,
        /// Expected size of the area
        expected_free_list: AreaIndex,
        /// The parent of the free area
        parent: FreeListParent,
    },

    /// The start address of a stored area is not a multiple of 16
    #[error(
        "The start address of a stored area (parent: {parent:#x}) is not a multiple of {}: {address:#x}",
        nodestore::alloc::LinearAddress::MIN_AREA_SIZE
    )]
    AreaMisaligned {
        /// The start address of the stored area
        address: LinearAddress,
        /// The parent of the `StoredArea`
        parent: StoredAreaParent,
    },

    /// Found leaked areas
    #[error("Found leaked areas: {0}")]
    #[derive_where(skip_inner)]
    AreaLeaks(checker::LinearAddressRangeSet),

    /// The root is not persisted
    #[error("The checker can only check persisted nodestores")]
    UnpersistedRoot,

    #[error(
        "The node {key:#x} at {address:#x} (parent: {parent:#x}) has a value but its path is not 32 or 64 bytes long"
    )]
    /// A value is found corresponding to an invalid key.
    /// With ethhash, keys must be 32 or 64 bytes long.
    /// Without ethhash, keys cannot contain half-bytes (i.e., odd number of nibbles).
    InvalidKey {
        /// The key found, or equivalently the path of the node that stores the value
        key: Path,
        /// Address of the node
        address: LinearAddress,
        /// Parent of the node
        parent: TrieNodeParent,
    },

    /// IO error
    #[error("IO error")]
    #[derive_where(skip_inner)]
    IO {
        /// The error
        error: FileIoError,
        /// parent of the area
        parent: Option<StoredAreaParent>,
    },
}

impl From<CheckerError> for Vec<CheckerError> {
    fn from(error: CheckerError) -> Self {
        vec![error]
    }
}

#[cfg(any(test, feature = "test_utils"))]
mod test_utils {
    use std::cell::RefCell;
    use std::rc::Rc;

    #[expect(
        clippy::disallowed_types,
        reason = "we are implementing the alternative"
    )]
    use rand::rngs::StdRng;
    #[expect(
        clippy::disallowed_types,
        reason = "we are implementing the alternative"
    )]
    use rand::{RngCore, SeedableRng, TryRngCore};

    #[derive(Debug, Clone)]
    #[must_use]
    /// A seeded random number generator for testing purposes.
    #[expect(
        clippy::disallowed_types,
        reason = "we are implementing the alternative"
    )]
    pub struct SeededRng(Rc<RefCell<StdRng>>);

    impl SeededRng {
        const ENV: &str = "FIREWOOD_TEST_SEED";

        /// Creates a new `SeededRng` with the given seed.
        pub fn new(seed: u64) -> Self {
            #![expect(
                clippy::disallowed_types,
                reason = "we are implementing the alternative"
            )]
            Self(Rc::new(RefCell::new(StdRng::seed_from_u64(seed))))
        }

        /// Creates a new `SeededRng` from an `Option<u64>`.
        ///
        /// If the provided value is `None`, [`SeededRng::from_env_or_random`] will
        /// be used to check the environment variable or otherwise generate a random seed.
        pub fn from_option(seed: Option<u64>) -> Self {
            seed.map_or_else(Self::from_env_or_random, Self::new)
        }

        /// Creates a new `SeededRng` from an environment variable set seed.
        ///
        /// # Returns
        ///
        /// None if the environment variable is not set, otherwise a [`SeededRng`]
        /// with the seed initialized.
        ///
        /// # Panics
        ///
        /// Panics if the environment variable is present but contains invalid UTF-8
        /// or does not parse as a valid `u64`.
        #[track_caller]
        #[must_use]
        pub fn from_env() -> Option<Self> {
            let s = std::env::var_os(Self::ENV)?
                .into_string()
                .unwrap_or_else(|_| panic!("{} must be a valid UTF-8 string", Self::ENV));
            Some(Self::new(s.parse().unwrap_or_else(|_| {
                panic!("{} must be a valid u64", Self::ENV)
            })))
        }

        /// Creates a new `SeededRng` with a random seed generated by the OS.
        pub fn from_random() -> Self {
            let seed = rand::rngs::OsRng.unwrap_err().next_u64();
            eprintln!(
                "Seed {seed}: to rerun with this data, export {}={seed}",
                Self::ENV
            );
            Self::new(seed)
        }

        /// Creates a new `SeededRng` from an environment variable (if set), otherwise
        /// a random seed.
        #[track_caller]
        pub fn from_env_or_random() -> Self {
            Self::from_env().unwrap_or_else(Self::from_random)
        }

        /// Creates a new `SeededRng` seeded by this Rng.
        pub fn seeded_fork(&self) -> Self {
            Self::new(self.next_u64())
        }

        /// Convenience method to generate a new u32 from the Rng with a reference
        /// and not a mutable reference.
        #[must_use]
        pub fn next_u32(&self) -> u32 {
            self.0.borrow_mut().next_u32()
        }

        /// Convenience method to generate a new u64 from the Rng with a reference
        /// and not a mutable reference.
        #[must_use]
        pub fn next_u64(&self) -> u64 {
            self.0.borrow_mut().next_u64()
        }

        /// Convenience method to fill a byte slice with random bytes from the Rng
        /// with a reference and not a mutable reference.
        pub fn fill_bytes(&self, dst: &mut [u8]) {
            self.0.borrow_mut().fill_bytes(dst);
        }

        /// Convenience method for [`rand::Rng::random`] but with a reference and
        /// instead of a mutable reference.
        #[must_use]
        #[inline]
        pub fn random<T>(&self) -> T
        where
            rand::distr::StandardUniform: rand::distr::Distribution<T>,
        {
            rand::Rng::random(&mut &*self)
        }

        /// Convenience method for [`rand::Rng::random_range`] but with a reference
        /// and instead of a mutable reference.
        #[track_caller]
        pub fn random_range<T, R>(&self, range: R) -> T
        where
            T: rand::distr::uniform::SampleUniform,
            R: rand::distr::uniform::SampleRange<T>,
        {
            rand::Rng::random_range(&mut &*self, range)
        }
    }

    impl rand::RngCore for SeededRng {
        #[inline]
        fn next_u32(&mut self) -> u32 {
            SeededRng::next_u32(self)
        }

        #[inline]
        fn next_u64(&mut self) -> u64 {
            SeededRng::next_u64(self)
        }

        #[inline]
        fn fill_bytes(&mut self, dst: &mut [u8]) {
            SeededRng::fill_bytes(self, dst);
        }
    }

    impl rand::RngCore for &SeededRng {
        #[inline]
        fn next_u32(&mut self) -> u32 {
            SeededRng::next_u32(self)
        }

        #[inline]
        fn next_u64(&mut self) -> u64 {
            SeededRng::next_u64(self)
        }

        #[inline]
        fn fill_bytes(&mut self, dst: &mut [u8]) {
            SeededRng::fill_bytes(self, dst);
        }
    }
}

#[cfg(any(test, feature = "test_utils"))]
pub use self::test_utils::SeededRng;
