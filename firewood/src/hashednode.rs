// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

use sha3::{Digest, Keccak256};
use std::collections::HashMap;
use std::io::Error;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use crate::trie_hash::TrieHash;
use storage::Node;

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

    pub fn invalidate_hash(&mut self, addr: LinearAddress) {
        self.hashes.insert(addr, None);
    }

    fn hash_internal(&self, node: &Arc<Node>) -> Result<TrieHash, Error> {
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
        Ok(hasher.finalize().into())
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
                let hash = store.hash_internal(&self.0)?;
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
