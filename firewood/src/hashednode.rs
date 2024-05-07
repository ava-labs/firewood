// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use sha3::{Digest, Keccak256};
use std::collections::HashMap;
use std::io::Error;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use storage::TrieHash;
use storage::{Node, ProposedImmutable};

use storage::{LinearAddress, NodeStore};
use storage::{ReadLinearStore, WriteLinearStore};

#[derive(Debug)]
pub struct HashedNode(Arc<Node>);

#[derive(Debug)]
pub struct HashedNodeStore<T: ReadLinearStore> {
    nodestore: NodeStore<T>,
    modified: HashMap<LinearAddress, Node>,
}

impl<T: WriteLinearStore> HashedNodeStore<T> {
    pub fn initialize(linearstore: T) -> Result<Self, Error> {
        let nodestore = NodeStore::initialize(linearstore)?;
        Ok(HashedNodeStore {
            nodestore,
            modified: Default::default(),
        })
    }
}

impl<T: ReadLinearStore> From<NodeStore<T>> for HashedNodeStore<T> {
    fn from(nodestore: NodeStore<T>) -> Self {
        HashedNodeStore {
            nodestore,
            modified: Default::default(),
        }
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
    pub fn freeze(self) -> HashedNodeStore<ProposedImmutable> {
        for _node in self.modified {
            todo!("write the nodes to the reserved space");
        }
        let frozen_nodestore = self.nodestore.freeze();
        HashedNodeStore {
            nodestore: frozen_nodestore,
            modified: Default::default(),
        }
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
    // TODO: This is broken
    pub fn hash<T: ReadLinearStore>(
        &self,
        _addr: LinearAddress,
        store: &mut HashedNodeStore<T>,
    ) -> Result<TrieHash, Error> {
        Ok(store.hash_internal(&self.0))
    }
}

impl Deref for HashedNode {
    type Target = Arc<Node>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
