// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::hash::Hash;
use std::io::Error;
use std::iter::{once, Chain};
use std::slice::Iter;
use std::sync::{Arc, OnceLock};

use storage::{BranchNode, TrieHash};
use storage::{LinearAddress, NodeStore};
use storage::{Node, Path, ProposedImmutable};
use storage::{ReadLinearStore, WriteLinearStore};

use integer_encoding::VarInt;

const MAX_VARINT_SIZE: usize = 10;
const BITS_PER_NIBBLE: u64 = 4;

use crate::merkle::MerkleError;
use crate::stream::PathIterItem;

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
        if let Some((modified_node, _)) = self.modified.get(&addr) {
            Ok(modified_node.clone())
        } else {
            Ok(self.nodestore.read_node(addr)?)
        }
    }

    // Returns a actual node instead of just an arc to it. This node is then
    // owned by the caller.
    // If the node was previously modified, then it will consume the modified Arc
    // and return the inner value. This assumes no other references to that Arc
    // exist at this time (that is, no iterators are active on the HashedNodeStore)
    // If the node was not modified, a clone of the node is returned
    fn take_node(&mut self, addr: LinearAddress) -> Result<Node, Error> {
        if let Some((modified_node, _)) = self.modified.remove(&addr) {
            Ok(Arc::into_inner(modified_node).expect("no other references to this node can exist"))
        } else {
            let node = self.nodestore.read_node(addr)?;
            Ok((*node).clone())
        }
    }

    pub fn root_hash(&self) -> Result<TrieHash, Error> {
        debug_assert!(self.modified.is_empty());
        if let Some(addr) = self.nodestore.root_address() {
            #[cfg(nightly)]
            let result = self
                .root_hash
                .get_or_try_init(|| {
                    let node = self.read_node(addr)?;
                    Ok(hash_node(&node, &Path(Default::default())))
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
                    hash_node(&node, &Path(Default::default()))
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
                    .filter_map(|(nibble, (&addr, &mut ref mut hash))| {
                        if *hash == TrieHash::default() {
                            addr.map(|addr| (nibble, (addr, hash)))
                        } else {
                            None
                        }
                    })
                {
                    // we found a child that needs hashing, so hash the child and assign it to the right spot in the child_hashes
                    // array
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
                if modified {
                    self.nodestore.update_in_place(node_addr, node)?;
                    self.modified.remove(&node_addr);
                }
                Ok(hash_node(node, path_prefix))
            }
            Node::Leaf(_) => Ok(hash_node(node, path_prefix)),
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

    pub fn delete_node(&mut self, addr: LinearAddress) -> Result<(), Error> {
        self.nodestore.delete_node(addr)
    }

    /// Fixes the trie after a node is updated.
    /// When a node is updated, it might move to a different LinearAddress.
    /// If it does, we need to update its parent so that it points to the
    /// new LinearAddress where its child lives.
    /// Updating a node changes its hash, so we need to mark in its parent
    /// that the previously computed hash for the updated node (if present)
    /// is no longer valid.
    /// `old_addr` and `new_addr` are the address of the updated node before
    /// and after update, respectively. These may be the same, meaning the
    /// node didn't move during update.
    /// `ancestors` returns the ancestors of the updated node, from the root
    /// up to and including the updated node's parent.
    pub fn fix_ancestors<'a, A: DoubleEndedIterator<Item = &'a PathIterItem>>(
        &mut self,
        mut ancestors: A,
        old_addr: LinearAddress,
        new_addr: LinearAddress,
    ) -> Result<(), MerkleError> {
        let Some(parent) = ancestors.next_back() else {
            self.set_root(Some(new_addr))?;
            return Ok(());
        };

        let old_parent_address = parent.addr;

        // The parent of the updated node.
        let parent = parent
            .node
            .as_branch()
            .expect("parent of a node is a branch");

        // The index of the updated node in `parent`'s children array.
        let child_index = parent
            .children
            .iter()
            .enumerate()
            .find(|(_, child_addr)| **child_addr == Some(old_addr))
            .map(|(child_index, _)| child_index)
            .expect("parent has node as child");

        // True iff the moved node's hash was already marked as invalid
        // in `parent` because we never computed it or we invalidated it in
        // a previous traversal
        let child_hash_already_invalidated = parent
            .child_hashes
            .get(child_index)
            .expect("index is a nibble")
            .is_empty();

        if child_hash_already_invalidated && old_addr == new_addr {
            // We already invalidated the moved node's hash, which means we must
            // have already invalidated the parent's hash in its parent, and so
            // on, up to the root.
            // The updated node didn't move, so we don't need to update
            // `parent`'s pointer to the updated node.
            // We're done fixing the ancestors.
            return Ok(());
        }

        let mut updated_parent = parent.clone();

        updated_parent.update_child_address(child_index, new_addr);

        let updated_parent = Node::Branch(updated_parent);
        self.update_node(ancestors, old_parent_address, updated_parent)?;

        Ok(())
    }

    /// Updates the node at `old_address` to be `node`. The node will be moved if
    /// it doesn't fit in its current location. `ancestors` contains the nodes
    /// from the root up to an including `node`'s parent. Returns the new address
    /// of `node`, which may be the same as `old_address`.
    pub fn update_node<'a, A: DoubleEndedIterator<Item = &'a PathIterItem>>(
        &mut self,
        ancestors: A,
        old_address: LinearAddress,
        node: Node,
    ) -> Result<LinearAddress, MerkleError> {
        let old_node_size_index =
            if let Some((_, old_node_size_index)) = self.modified.get(&old_address) {
                *old_node_size_index
            } else {
                self.nodestore.node_size(old_address)?
            };

        // If the node was already modified, see if it still fits
        let new_address = if !self.nodestore.still_fits(old_node_size_index, &node)? {
            self.modified.remove(&old_address);
            self.nodestore.delete_node(old_address)?;
            let (new_address, new_node_size_index) = self.nodestore.allocate_node(&node)?;
            self.modified
                .insert(new_address, (Arc::new(node), new_node_size_index));
            new_address
        } else {
            self.modified
                .insert(old_address, (Arc::new(node), old_node_size_index));
            old_address
        };

        self.fix_ancestors(ancestors, old_address, new_address)?;

        Ok(new_address)
    }

    pub fn set_root(&mut self, root_addr: Option<LinearAddress>) -> Result<(), Error> {
        self.nodestore.set_root(root_addr)
    }
}

trait HasUpdate {
    fn update<T: AsRef<[u8]>>(&mut self, data: T);
}

impl HasUpdate for Sha256 {
    fn update<T: AsRef<[u8]>>(&mut self, data: T) {
        sha2::Digest::update(self, data)
    }
}

impl HasUpdate for Vec<u8> {
    fn update<T: AsRef<[u8]>>(&mut self, data: T) {
        self.extend(data.as_ref());
    }
}

/// Contains the fields that are serialized to hash a node.
pub struct HashPreimage<'a, K: Iterator<Item = &'a u8> + Clone, V: AsRef<[u8]>> {
    key: K,
    value_digest: Option<V>,
    children: Option<&'a [TrieHash; BranchNode::MAX_CHILDREN]>,
}

impl<'a, K: Iterator<Item = &'a u8> + Clone, V: AsRef<[u8]>> HashPreimage<'a, K, V> {
    /// Calls `buf.Update` for each byte in the pre-image of `node`.
    fn write<H: HasUpdate>(mut self, buf: &mut H) {
        if let Some(children) = self.children {
            let children_iter = children
                .iter()
                .enumerate()
                .filter(|(_, hash)| **hash != Default::default());

            let num_children = children_iter.clone().count() as u64;
            add_varint_to_buf(buf, num_children);

            for (index, hash) in children_iter {
                add_varint_to_buf(buf, index as u64);
                buf.update(hash);
            }
        } else {
            let num_children: u64 = 0;
            add_varint_to_buf(buf, num_children);
        }

        // Add value digest (if any) to hash pre-image
        add_value_digest_to_buf(buf, self.value_digest);

        // Add key length (in bits) to hash pre-image
        let key_bit_len = BITS_PER_NIBBLE * self.key.clone().count() as u64;
        add_varint_to_buf(buf, key_bit_len);

        // Add key to hash pre-image
        while let Some(high_nibble) = self.key.next() {
            let low_nibble = self.key.next().unwrap_or(&0);
            let byte = (high_nibble << 4) | low_nibble;
            buf.update([byte]);
        }
    }
}

/// Returns the SHA-256 hash of the `preimage`.
pub fn hash<'a, K: Iterator<Item = &'a u8> + Clone, V: AsRef<[u8]>>(
    preimage: HashPreimage<'a, K, V>,
) -> TrieHash {
    let mut hasher: Sha256 = Sha256::new();
    preimage.write(&mut hasher);
    hasher.finalize().into()
}

/// Returns the hash of `node` which is at the given `path_prefix`.
fn write_hash_preimage<'a, H: HasUpdate>(
    node: &'a Node,
    path_prefix: &'a Path,
) -> HashPreimage<'a, _, _> {
    let key = path_prefix.iter().chain(node.partial_path().iter());

    let children = match *node {
        Node::Branch(ref branch) => Some(&branch.child_hashes),
        Node::Leaf(_) => None,
    };

    let value = match *node {
        Node::Branch(ref branch) => branch.value.as_ref().map(|v| v.as_ref()),
        Node::Leaf(ref leaf) => Some(leaf.value.as_ref()),
    };

    match value {
        Some(value) if value.len() >= 32 => {
            let hash = Sha256::digest(value);
            HashPreimage {
                key,
                value_digest: Some(hash),
                children,
            }
        }
        _ => HashPreimage {
            key,
            value_digest: value,
            children,
        },
    }
}

/// TODO comment
fn hash_node(node: &Node, path_prefix: &Path) -> TrieHash {
    let mut hasher: Sha256 = Sha256::new();
    let preimage = write_hash_preimage(node, path_prefix);
    hasher.finalize().into()
}

/// Returns the serialized representation of `node` used as the pre-image
/// when hashing the node. The node is at the given `path_prefix`.
// pub fn hash_preimage(node: &Node, path_prefix: &Path) -> Box<[u8]> {
//     let mut buf = vec![];
//     HashPreimage::from(node, path_prefix).write(&mut buf);
//     buf.into_boxed_slice()
// }

fn add_value_digest_to_buf<H: HasUpdate, T: AsRef<[u8]>>(buf: &mut H, value_digest: Option<T>) {
    let Some(value_digest) = value_digest else {
        let value_exists: u8 = 0;
        buf.update([value_exists]);
        return;
    };

    let value_exists: u8 = 1;
    buf.update([value_exists]);
    add_len_and_value_to_buf(buf, value_digest.as_ref());
}

#[inline]
/// Writes the length of `value` and `value` to `hasher`.
fn add_len_and_value_to_buf<H: HasUpdate>(buf: &mut H, value: &[u8]) {
    let value_len = value.len();
    buf.update([value_len as u8]);
    buf.update(value);
}

#[inline]
/// Encodes `value` as a varint and writes it to `hasher`.
fn add_varint_to_buf<H: HasUpdate>(buf: &mut H, value: u64) {
    let mut buf_arr = [0u8; MAX_VARINT_SIZE];
    let len = value.encode_var(&mut buf_arr);
    buf.update(
        buf_arr
            .get(..len)
            .expect("length is always less than MAX_VARINT_SIZE"),
    );
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
        hns.set_root(Some(addr)).unwrap();

        let frozen = hns.freeze().unwrap();
        assert_ne!(frozen.root_hash().unwrap(), Default::default());
    }
}
