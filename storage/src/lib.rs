// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.
#![warn(missing_debug_implementations, rust_2018_idioms, missing_docs)]
#![forbid(unsafe_code)]

//! # storage implements the storage of a [Node] on top of a LinearStore
//! 
//! Nodes are stored at a [LinearAddress] within a [NodeStore]
//! 
//! The [NodeStore] maintains a free list and the [LinearAddress] of a root node.
//! 
//! A [NodeStore] is typically backed by a [ReadLinearStore] which is immutable.
//! However, [NodeStore] can also be backed by a [WriteLinearStore]. These
//! support writes.

mod linear;
mod nodestore;
mod node;

// re-export these so callers don't need to know where they are
pub use node::{Node, LeafNode, BranchNode, path::Path};
pub use linear::{ReadLinearStore, WriteLinearStore, LinearStoreParent};
pub use nodestore::{LinearAddress, NodeStore, UpdateError};

pub use linear::{filebacked::FileBacked, historical::Historical, memory::MemStore, proposed::Proposed};
pub use linear::proposed::{ProposedImmutable, ProposedMutable};
