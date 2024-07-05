// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Error;
use std::iter::once;
use std::sync::{Arc, OnceLock};

use storage::{BranchNode, LeafNode, TrieHash};
use storage::{LinearAddress, NodeStore};
use storage::{Node, Path, ProposedImmutable};
use storage::{ReadLinearStore, WriteLinearStore};

use integer_encoding::VarInt;

const MAX_VARINT_SIZE: usize = 10;
const BITS_PER_NIBBLE: u64 = 4;

use crate::merkle::MerkleError;
use storage::PathIterItem;

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

    // recursively hash this node
    fn hash(&self, node_addr: LinearAddress, path_prefix: &mut Path) -> Result<TrieHash, Error> {
        let node = self.read_node(node_addr)?;

        match &*node {
            Node::Branch(b) => {
                // We found a branch, so find all the children that need hashing
                let mut children_hashes: [Option<TrieHash>; BranchNode::MAX_CHILDREN] =
                    Default::default();

                #[allow(clippy::indexing_slicing)]
                for (index, child) in b.children.iter().enumerate() {
                    let Some(child_addr) = child else {
                        // There is no child or we already know its hash.
                        continue;
                    };

                    // Hash this child.
                    let child_addr = *child_addr;
                    let original_length = path_prefix.len();
                    path_prefix
                        .0
                        .extend(b.partial_path.0.iter().copied().chain(once(index as u8)));

                    children_hashes[index] = Some(self.hash(child_addr, path_prefix)?);
                    path_prefix.0.truncate(original_length);
                }

                let hash = hash_branch(b, path_prefix, children_hashes);

                Ok(hash)
            }
            Node::Leaf(l) => Ok(hash_leaf(l, path_prefix)),
        }
    }

    pub fn root_hash(&self) -> Result<TrieHash, Error> {
        debug_assert!(self.modified.is_empty());
        let Some(root_addr) = self.nodestore.root_address() else {
            return Ok(TrieHash::default());
        };

        // TODO danlaine: Uncomment this once get_or_try_init is stable
        // return self
        //     .root_hash
        //     .get_or_try_init(|| {
        //         let node = self.read_node(addr)?;
        //         Ok(hash_node(&node, &Path(Default::default())))
        //     })
        //     .cloned();

        Ok(self
            .root_hash
            // TODO danlaine: avoid unwrap() below
            .get_or_init(|| {
                self.hash(root_addr, &mut Path(Default::default()))
                    .expect("TODO danlaine remove this unwrap")
            })
            .clone())
    }

    pub const fn root_address(&self) -> Option<LinearAddress> {
        self.nodestore.root_address()
    }
}

impl<T: WriteLinearStore> HashedNodeStore<T> {
    // Writes all of the nodes in `self.modified` to the linear store
    // and clears `self.modified`.
    fn flush(&mut self) -> Result<(), Error> {
        for (node_addr, (node, _)) in self.modified.iter() {
            self.nodestore.update_in_place(*node_addr, node)?;
        }
        self.modified.clear();
        Ok(())
    }

    pub fn freeze(mut self) -> Result<HashedNodeStore<ProposedImmutable>, Error> {
        self.flush()?;
        assert!(self.modified.is_empty());

        let root_hash = if let Some(root_address) = self.root_address() {
            let hash = self.hash(root_address, &mut Path(Default::default()))?;
            let once_hash = OnceLock::new();
            let _ = once_hash.set(hash);
            once_hash
        } else {
            Default::default()
        };

        let frozen_nodestore = self.nodestore.freeze();
        Ok(HashedNodeStore {
            nodestore: frozen_nodestore,
            modified: Default::default(),
            root_hash,
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
    fn fix_ancestors<'a, A: DoubleEndedIterator<Item = &'a PathIterItem>>(
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
        let parent_branch = parent
            .node
            .as_branch()
            .expect("parent of a node is a branch");

        // The index of the updated node in `parent`'s children array.
        let child_index = parent.next_nibble.expect("must have a nibble address");

        if old_addr == new_addr {
            // We already invalidated the moved node's hash, which means we must
            // have already invalidated the parent's hash in its parent, and so
            // on, up to the root.
            // The updated node didn't move, so we don't need to update
            // `parent`'s pointer to the updated node.
            // We're done fixing the ancestors.
            return Ok(());
        }

        let mut updated_parent = parent_branch.clone();

        updated_parent.update_child(child_index, Some(new_addr));

        let updated_parent = Node::Branch(updated_parent);
        self.update_node(ancestors, old_parent_address, updated_parent)?;

        Ok(())
    }

    /// Updates the node at `old_address` to be `node`. The node may move.
    /// `ancestors` contains the nodes from the root up to an including `node`'s
    /// parent. Returns the new address of `node`, which may be the same as `old_address`.
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

pub fn hash_branch<'a>(
    node: &'a BranchNode,
    path_prefix: &'a Path,
    children: [Option<TrieHash>; BranchNode::MAX_CHILDREN],
) -> TrieHash {
    NodeAndPrefix {
        node,
        prefix: path_prefix,
        children,
    }
    .into()
}

pub fn hash_leaf<'a>(node: &'a LeafNode, path_prefix: &'a Path) -> TrieHash {
    NodeAndPrefix {
        node,
        prefix: path_prefix,
        children: Default::default(),
    }
    .into()
}

// TODO danlaine: Uncomment this function, which will be needed
// for proof generation / verification.
/// Returns the serialized representation of `node` used as the pre-image
/// when hashing the node. The node is at the given `path_prefix`.
// pub fn hash_preimage<'a, C: Iterator<Item = (usize, &'a TrieHash)> + Clone>(
//     node: &Node,
//     children: C,
//     path_prefix: &Path,
// ) -> Box<[u8]> {
//     // Key, 3 options, value digest
//     let est_len = node.partial_path().len() + path_prefix.len() + 3 + TrieHash::default().len();
//     let mut buf = Vec::with_capacity(est_len);
//     match node {
//         Node::Branch(node) => {
//             NodeAndPrefix {
//                 node,
//                 prefix: path_prefix,
//                 children,
//             }
//             .write(&mut buf);
//         }
//         Node::Leaf(node) => {
//             NodeAndPrefix {
//                 node,
//                 prefix: path_prefix,
//                 children: iter::empty(),
//             }
//             .write(&mut buf);
//         }
//     }
//     buf.into_boxed_slice()
// }

pub(super) trait HasUpdate {
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

pub(super) enum ValueDigest<'a> {
    /// A node's value.
    Value(&'a [u8]),
    /// The hash of a a node's value.
    /// TODO danlaine: Use this variant when implement ProofNode
    _Hash(Box<[u8]>),
}

trait Hashable {
    /// The key of the node where each byte is a nibble.
    fn key(&self) -> impl Iterator<Item = u8> + Clone;
    /// The node's value or hash.
    fn value_digest(&self) -> Option<ValueDigest>;
    /// Each element is a child's index and hash.
    /// Yields 0 elements if the node is a leaf.
    fn children(&self) -> impl Iterator<Item = (usize, &TrieHash)> + Clone;
}

pub(super) trait Preimage {
    /// Returns the hash of this preimage.
    fn to_hash(self) -> TrieHash;
    /// Write this hash preimage to `buf`.
    fn write(self, buf: &mut impl HasUpdate);
}

// Implement Preimage for all types that implement Hashable
impl<T> Preimage for T
where
    T: Hashable,
{
    fn to_hash(self) -> TrieHash {
        let mut hasher = Sha256::new();
        self.write(&mut hasher);
        hasher.finalize().into()
    }

    fn write(self, buf: &mut impl HasUpdate) {
        let children = self.children();

        let num_children = children.clone().count() as u64;
        add_varint_to_buf(buf, num_children);

        for (index, hash) in children {
            add_varint_to_buf(buf, index as u64);
            buf.update(hash);
        }

        // Add value digest (if any) to hash pre-image
        add_value_digest_to_buf(buf, self.value_digest());

        // Add key length (in bits) to hash pre-image
        let mut key = self.key();
        // let mut key = key.as_ref().iter();
        let key_bit_len = BITS_PER_NIBBLE * key.clone().count() as u64;
        add_varint_to_buf(buf, key_bit_len);

        // Add key to hash pre-image
        while let Some(high_nibble) = key.next() {
            let low_nibble = key.next().unwrap_or(0);
            let byte = (high_nibble << 4) | low_nibble;
            buf.update([byte]);
        }
    }
}

trait HashableNode {
    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone;
    fn value(&self) -> Option<&[u8]>;
}

impl<'a> HashableNode for &'a storage::BranchNode {
    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone {
        self.partial_path.0.iter().copied()
    }

    fn value(&self) -> Option<&[u8]> {
        self.value.as_deref()
    }
}

impl<'a> HashableNode for &'a storage::LeafNode {
    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone {
        self.partial_path.0.iter().copied()
    }

    fn value(&self) -> Option<&[u8]> {
        Some(&self.value)
    }
}

struct NodeAndPrefix<'a, N: HashableNode> {
    node: N,
    prefix: &'a Path,
    children: [Option<TrieHash>; BranchNode::MAX_CHILDREN],
}

impl<'a, N: HashableNode> From<NodeAndPrefix<'a, N>> for TrieHash {
    fn from(node: NodeAndPrefix<'a, N>) -> Self {
        node.to_hash()
    }
}

impl<'a, N: HashableNode> Hashable for NodeAndPrefix<'a, N> {
    fn key(&self) -> impl Iterator<Item = u8> + Clone {
        self.prefix
            .0
            .iter()
            .copied()
            .chain(self.node.partial_path())
    }

    fn value_digest(&self) -> Option<ValueDigest> {
        self.node.value().map(ValueDigest::Value)
    }

    fn children(&self) -> impl Iterator<Item = (usize, &TrieHash)> + Clone {
        self.children
            .iter()
            .enumerate()
            .filter_map(|(index, hash)| hash.as_ref().map(|hash| (index, hash)))
    }
}

fn add_value_digest_to_buf<H: HasUpdate>(buf: &mut H, value_digest: Option<ValueDigest>) {
    let Some(value_digest) = value_digest else {
        let value_exists: u8 = 0;
        buf.update([value_exists]);
        return;
    };

    let value_exists: u8 = 1;
    buf.update([value_exists]);

    match value_digest {
        ValueDigest::Value(value) if value.as_ref().len() >= 32 => {
            let hash = Sha256::digest(value);
            add_len_and_value_to_buf(buf, hash.as_ref());
        }
        ValueDigest::Value(value) => {
            add_len_and_value_to_buf(buf, value);
        }
        ValueDigest::_Hash(hash) => {
            add_len_and_value_to_buf(buf, hash.as_ref());
        }
    }
}

#[inline]
/// Writes the length of `value` and `value` to `buf`.
fn add_len_and_value_to_buf<H: HasUpdate>(buf: &mut H, value: &[u8]) {
    let value_len = value.len();
    buf.update([value_len as u8]);
    buf.update(value);
}

#[inline]
/// Encodes `value` as a varint and writes it to `buf`.
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
