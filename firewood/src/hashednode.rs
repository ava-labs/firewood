// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use sha3::{Digest, Keccak256};
use std::collections::HashMap;
use std::io::Error;
use std::iter::once;
use std::sync::{Arc, OnceLock};

use storage::{Node, Path, ProposedImmutable};
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

    fn take_node(&mut self, addr: LinearAddress) -> Result<Node, Error> {
        if let Some((modified_node, _)) = self.modified.remove(&addr) {
            Ok(Arc::into_inner(modified_node).expect("no other references to this node can exist"))
        } else {
            let node = self.nodestore.read_node(addr)?;
            Ok((*node).clone())
        }
    }

    pub fn root_hash(&self) -> Result<TrieHash, Error> {
        if let Some(addr) = self.nodestore.root_address() {
            #[cfg(nightly)]
            let result = self
                .root_hash
                .get_or_try_init(|| {
                    let node = self.read_node(addr)?;
                    Ok(self.hash_internal(&node, &Path(Default::default())))
                })
                .cloned();
            #[cfg(not(nightly))]
            let result = Ok(self
                .root_hash
                .get_or_init(|| {
                    let node = self
                        .nodestore
                        .read_node(addr)
                        .expect("TODO: use get_or_try_init once it's available");
                    self.hash_internal(&node, &Path(Default::default()))
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
    // recursively hash this node
    fn hash(
        &mut self,
        node_addr: LinearAddress,
        node: &mut Node,
        path_prefix: &mut Path,
    ) -> Result<TrieHash, Error> {
        match node {
            Node::Branch(b) => {
                // We found a branch, so find all the children that ned hashing
                let mut modified = false;
                for (nibble, (child_addr, &mut ref mut child_hash)) in b
                    .children
                    .iter()
                    .zip(b.child_hashes.iter_mut())
                    .enumerate()
                    .filter(|(_, (&addr, &mut ref hash))| {
                        addr.is_some() && *hash == TrieHash::default()
                    })
                {
                    // we found a child that needs hashing, so hash the child and assign it to the right spot in the child_hashes
                    // array
                    let child_addr =
                        child_addr.expect("guaranteed to be Some; fixme with fancier rust");
                    let mut child = self.take_node(child_addr)?;
                    let original_length = path_prefix.len();
                    path_prefix
                        .0
                        .extend(b.partial_path.0.iter().copied().chain(once(nibble as u8)));
                    *child_hash = self.hash(child_addr, &mut child, path_prefix)?;
                    path_prefix.0.truncate(original_length);
                    modified = true;
                    self.nodestore.update_in_place(child_addr, &child)?;
                }
                let hash = self.hash_internal(node, path_prefix);
                if modified {
                    self.nodestore.update_in_place(node_addr, node)?;
                    self.modified.remove(&node_addr);
                }
                Ok(hash)
            }
            Node::Leaf(_) => Ok(self.hash_internal(node, path_prefix)),
        }
    }
    pub fn freeze(mut self) -> Result<HashedNodeStore<ProposedImmutable>, Error> {
        // fill in all remaining hashes, including the root hash
        if let Some(root_address) = self.root_address() {
            let mut root_node = self.take_node(root_address)?;
            let hash = self.hash(root_address, &mut root_node, &mut Path(Default::default()))?;
            let _ = self.root_hash.set(hash);
            self.nodestore.update_in_place(root_address, &root_node)?;
        }
        assert!(self.modified.is_empty());
        let frozen_nodestore = self.nodestore.freeze();
        Ok(HashedNodeStore {
            nodestore: frozen_nodestore,
            modified: Default::default(),
            root_hash: Default::default(),
        })
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
    fn hash_internal(&self, node: &Node, path_prefix: &Path) -> TrieHash {
        let mut hasher = Keccak256::new();
        match *node {
            Node::Branch(ref branch) => {
                // collect the full key
                let key: Box<_> = path_prefix
                    .bytes_iter()
                    .chain(branch.partial_path.bytes_iter())
                    .collect();
                hasher.update(key);

                // collect the value
                if let Some(value) = &branch.value {
                    hasher.update(value);
                }

                // collect the active child hashes (only the active ones)
                for hash in branch
                    .children
                    .iter()
                    .zip(branch.child_hashes.iter())
                    .filter_map(|(addr, hash)| addr.map(|_| hash))
                {
                    debug_assert_ne!(**hash, Default::default());
                    hasher.update(**hash)
                }
            }
            Node::Leaf(ref leaf) => {
                // collect and hash the key
                let key: Box<_> = path_prefix
                    .bytes_iter()
                    .chain(leaf.partial_path.bytes_iter())
                    .collect();
                hasher.update(key);

                // collect and hash the value
                hasher.update(&leaf.value);
            }
        }
        hasher.finalize().into()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use storage::{MemStore, Path};

    use super::*;

    #[test]
    fn freeze_test() {
        let memstore = MemStore::new(vec![]);
        let mut hns = HashedNodeStore::initialize(memstore).unwrap();
        let node = Node::Leaf(storage::LeafNode {
            partial_path: Path(Default::default()),
            value: Box::new(*b"abc"),
        });
        let addr = hns.create_node(node).unwrap();
        hns.set_root(addr).unwrap();

        let frozen = hns.freeze().unwrap();
        assert_ne!(frozen.root_hash().unwrap(), Default::default());
    }
}
