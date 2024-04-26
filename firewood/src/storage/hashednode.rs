// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

use sha3::{Digest, Keccak256};
use std::io::Error;
use std::ops::Deref;
use std::sync::{Arc, OnceLock};

use crate::merkle::TrieHash;
use crate::node::Node;

use super::linear::{ReadLinearStore, WriteLinearStore};
use super::node::{LinearAddress, NodeStore};

#[derive(Debug)]
struct HashedNode {
    node: Arc<Node>,
    hash: OnceLock<TrieHash>,
}

struct HashedNodeStore<T: ReadLinearStore> {
    nodestore: NodeStore<T>,
}

impl<T: WriteLinearStore> HashedNodeStore<T> {
    pub fn initialize(linearstore: T) -> Result<Self, Error> {
        let nodestore = NodeStore::initialize(linearstore)?;
        Ok(HashedNodeStore { nodestore })
    }
}

impl<T: ReadLinearStore> HashedNodeStore<T> {
    pub fn read_node(&self, addr: LinearAddress) -> Result<Arc<HashedNode>, Error> {
        // TODO: implement caching, currently just passing through
        let node = self.nodestore.read_node(addr)?;
        Ok(Arc::new(HashedNode {
            node,
            hash: Default::default(),
        }))
    }

    pub fn read_hashed_node(&self, addr: LinearAddress) -> Result<Arc<HashedNode>, Error> {
        let node = self.nodestore.read_node(addr)?;
        let hash = OnceLock::new();
        hash.set(self.hash_internal(&node))
            .expect("cannot be empty");
        Ok(Arc::new(HashedNode { node, hash }))
    }
    fn hash_internal(&self, node: &Arc<Node>) -> TrieHash {
        let mut hasher = Keccak256::new();
        match **node {
            Node::Branch(ref _branch) => {
                todo!()
            }
            Node::Leaf(ref leaf) => {
                // TODO: can we use the stack here and call update less?
                for byte in leaf.partial_path.iter_encoded() {
                    hasher.update([byte]);
                }
                hasher.update(&leaf.value);
            }
        }
        TrieHash(hasher.finalize().into())
    }
}

// TODO: This should be a concrete implementation that invalidates hashes for items
// at the same address
impl<T: WriteLinearStore> Deref for HashedNodeStore<T> {
    type Target = NodeStore<T>;

    fn deref(&self) -> &Self::Target {
        &self.nodestore
    }
}

impl HashedNode {
    pub fn hash<T: ReadLinearStore>(&self, store: HashedNodeStore<T>) -> TrieHash {
        // see if we already have the hashed value for this node
        *self.hash.get_or_init(|| store.hash_internal(&self.node))
    }
}
