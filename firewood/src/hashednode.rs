// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use sha3::{Digest, Keccak256};
use std::collections::HashMap;
use std::io::Error;
use std::sync::{Arc, OnceLock};

use storage::{Node, ProposedImmutable};
use storage::{TrieHash, UpdateError};

use storage::{LinearAddress, NodeStore};
use storage::{ReadLinearStore, WriteLinearStore};

/// A [HashedNodeStore] keeps track of nodes as they change when they are backed by a LinearStore.
/// This defers the writes of those nodes until all the changes are made to the trie as part of a
/// batch. It also supports a `freeze` method which consumes a [WriteLinearStore] backed [HashedNodeStore],
/// makes sure the hashes are filled in, and flushes the nodes out before returning a [ReadLinearStore]
/// backed HashedNodeStore for future operations. `freeze` is called when the batch is completed and is
/// used to obtain the merkle trie for the proposal.
#[derive(Debug)]
pub struct HashedNodeStore<T: ReadLinearStore> {
    nodestore: NodeStore<T>,
    modified: HashMap<LinearAddress, (Arc<Node>, u8)>,
    root_hash: OnceLock<TrieHash>,
}

impl<T: WriteLinearStore> HashedNodeStore<T> {
    pub fn initialize(linearstore: T) -> Result<Self, Error> {
        let nodestore = NodeStore::initialize(linearstore)?;
        Ok(HashedNodeStore {
            nodestore,
            modified: Default::default(),
            root_hash: Default::default(),
        })
    }
}

impl<T: ReadLinearStore> From<NodeStore<T>> for HashedNodeStore<T> {
    fn from(nodestore: NodeStore<T>) -> Self {
        HashedNodeStore {
            nodestore,
            modified: Default::default(),
            root_hash: Default::default(),
        }
    }
}

impl<T: ReadLinearStore> HashedNodeStore<T> {
    pub fn read_node(&self, addr: LinearAddress) -> Result<Arc<Node>, Error> {
        if let Some(modified_node) = self.modified.get(&addr) {
            Ok(modified_node.0.clone())
        } else {
            Ok(self.nodestore.read_node(addr)?)
        }
    }

    pub fn root_hash(&self) -> Result<TrieHash, Error> {
        if let Some(addr) = self.nodestore.root_address() {
            #[cfg(nightly)]
            let result = self
                .root_hash
                .get_or_try_init(|| {
                    let node = self.read_node(addr)?;
                    Ok(self.hash_internal(&node))
                })
                .cloned();
            #[cfg(not(nightly))]
            let result = Ok(self
                .root_hash
                .get_or_init(|| {
                    let node = self
                        .read_node(addr)
                        .expect("TODO: use get_or_try_init once it's available");
                    self.hash_internal(&node)
                })
                .clone());
            result
        } else {
            Ok(TrieHash::default())
        }
    }

    pub const fn root_address(&self) -> Option<LinearAddress> {
        self.nodestore.root_address()
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
            root_hash: Default::default(),
        }
    }
    pub fn create_node(&mut self, node: Node) -> Result<LinearAddress, Error> {
        let (addr, size) = self.nodestore.allocate_node(&node)?;
        self.modified.insert(addr, (Arc::new(node), size));
        Ok(addr)
    }

    pub fn update_node(
        &mut self,
        old_address: LinearAddress,
        node: Node,
    ) -> Result<(), UpdateError> {
        let old_size = if let Some((_, old_size)) = self.modified.get(&old_address) {
            *old_size
        } else {
            self.nodestore.node_size(old_address)?
        };
        // the node was already modified, see if it still fits
        if !self.nodestore.still_fits(old_size, &node)? {
            self.modified.remove(&old_address);
            self.nodestore.delete_node(old_address)?;
            return Err(UpdateError::NodeMoved(self.create_node(node)?));
        }
        self.modified
            .insert(old_address, (Arc::new(node), old_size));

        Ok(())
    }

    pub fn set_root(&mut self, root_addr: LinearAddress) -> Result<(), Error> {
        self.nodestore.set_root(root_addr)
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
