// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

use sha3::{Digest, Keccak256};
use std::collections::HashMap;
use std::io::Error;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use storage::Node;
use storage::TrieHash;

use storage::{LinearAddress, NodeStore};
use storage::{ReadLinearStore, WriteLinearStore};

#[derive(Debug)]
pub struct HashedNode(Arc<Node>);

#[derive(Debug)]
pub struct HashedNodeStore<T: ReadLinearStore> {
    nodestore: NodeStore<T>,
    hashes: HashMap<LinearAddress, Option<TrieHash>>,
}

impl<T: WriteLinearStore> HashedNodeStore<T> {
    pub fn initialize(linearstore: T) -> Result<Self, Error> {
        let nodestore = NodeStore::initialize(linearstore)?;
        Ok(HashedNodeStore {
            nodestore,
            hashes: Default::default(),
        })
    }
}

impl<T: ReadLinearStore> HashedNodeStore<T> {
    pub fn read_node(&self, addr: LinearAddress) -> Result<HashedNode, Error> {
        let node = self.nodestore.read_node(addr)?;
        Ok(HashedNode(node))
    }

    pub fn root_hash(&self, addr: LinearAddress) -> Result<TrieHash, Error> {
        let node = self.read_node(addr)?;
        Ok(self.hash_internal(&node))
    }
}

// TODO: This should be a concrete implementation that invalidates hashes for items
// at the same address
impl<T: ReadLinearStore> Deref for HashedNodeStore<T> {
    type Target = NodeStore<T>;

    fn deref(&self) -> &Self::Target {
        &self.nodestore
    }
}

impl<T: ReadLinearStore> DerefMut for HashedNodeStore<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.nodestore
    }
}

impl<T: WriteLinearStore> HashedNodeStore<T> {
    pub fn invalidate_hash(&mut self, addr: LinearAddress) {
        self.hashes.insert(addr, None);
    }
}

impl<T: ReadLinearStore> HashedNodeStore<T> {
    // hash a node
    // assumes all the children of a branch have their hashes filled in
    // TODO: We should include the full partial path up to this node in the hash
    // this is because the hash will not change for leaves when they move

    fn hash_internal(&self, node: &Arc<Node>) -> TrieHash {
        let mut hasher = Keccak256::new();
        match **node {
            Node::Branch(ref branch) => {
                hasher.update(&branch.partial_path.0);
                if let Some(value) = &branch.value {
                    hasher.update(value);
                }
                hasher.update(
                    bincode::serialize(&branch.child_hashes).expect("serialization of constant"),
                );
            }
            Node::Leaf(ref leaf) => {
                // TODO: This should be the full path to this node, not the partial path
                hasher.update(&leaf.partial_path.0);
                hasher.update(&leaf.value);
            }
        }
        hasher.finalize().into()
    }
}

impl HashedNode {
    pub fn hash<T: ReadLinearStore>(
        &self,
        addr: LinearAddress,
        store: &mut HashedNodeStore<T>,
    ) -> Result<TrieHash, Error> {
        // see if we already have the hashed value for this node
        let value = store.hashes.get_mut(&addr);
        match value {
            None | Some(None) => {
                let hash = store.hash_internal(&self.0);
                store.hashes.insert(addr, Some(hash.clone()));
                Ok(hash)
            }
            Some(Some(hash)) => Ok(hash.clone()),
        }
    }
}

impl Deref for HashedNode {
    type Target = Arc<Node>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
