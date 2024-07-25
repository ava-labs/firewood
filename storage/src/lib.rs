// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.
#![warn(missing_debug_implementations, rust_2018_idioms, missing_docs)]
#![forbid(unsafe_code)]

//! # storage implements the storage of a [Node] on top of a LinearStore
//!
//! Nodes are stored at a [LinearAddress] within a [ReadableStorage].
//!
//! The [NodeStore] maintains a free list and the [LinearAddress] of a root node.
//!
//! A [NodeStore] is backed by a [ReadableStorage] which is persisted storage.

mod hashednode;
mod linear;
mod node;
mod nodestore;
mod trie_hash;

// re-export these so callers don't need to know where they are
pub use hashednode::{hash_node, hash_preimage};
pub use linear::{ReadableStorage, WritableStorage};
pub use node::{
    path::NibblesIterator, path::Path, BranchNode, Child, LeafNode, Node, PathIterItem,
};
pub use nodestore::{
    ImmutableProposal, LinearAddress, NodeReader, NodeStore, NodeWriter, ProposedMutable2,
    ReadInMemoryNode, UpdateError,
};

pub use linear::{filebacked::FileBacked, memory::MemStore};

pub use trie_hash::TrieHash;
