// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.
#![warn(missing_debug_implementations, rust_2018_idioms, missing_docs)]
#![forbid(unsafe_code)]

//! # storage implements the storage of a [Node] on top of a LinearStore
//!
//! Nodes are stored at a [LinearAddress] within a [NodeStore]
//!
//! The [NodeStore] maintains a the [LinearAddress] of the root node
//! for this revision.
//! The [NodeStore] maintains a the [LinearAddress] of the root node
//! for this revision.
//!
//! A [NodeStore] has a generic argument which indicates what kind of
//! revision the nodestore represents. Possible options are:
//!  - [Committed] for a committed revision
//!  - [ProposedImmutable] for a revision which proposed and has all the k/v inserts completed
//!  - [ProposedMutable] for a revision being created as a proposal
//!
//! The [ProposedMutable] revision is likely to have some incomplete hash values. These
//! get filled in when converting the [NodeStore] to a [NodeStore<ProposedImmutable>].
//!
//! Proposed [NodeStore]s contain some extra data, notably an updated freelist and a
//! list of the new nodes yet to be written to disk.

mod node;
mod nodestore;
mod revisions;
mod trie_hash;

// re-export these so callers don't need to know where they are
pub use node::{
    path::NibblesIterator, path::Path, BranchNode, Child, LeafNode, Node, PathIterItem,
};
pub use nodestore::{LinearAddress, NodeStore, UpdateError};
pub use revisions::{
    filestore::FileStore, memstore::MemStore, Committed, ProposedImmutable, ProposedMutable,
};
pub use revisions::{NodeStoreParent, ReadChangedNode, ReadableStorage, WritableStorage};

pub use trie_hash::TrieHash;
