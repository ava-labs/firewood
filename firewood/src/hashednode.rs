// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::Error;
use std::iter::{self, once};

use storage::ReadLinearStore;
use storage::{Child, TrieHash};
use storage::{LinearAddress, NodeStore};
use storage::{Node, Path, ProposedImmutable};

use integer_encoding::VarInt;

const MAX_VARINT_SIZE: usize = 10;
const BITS_PER_NIBBLE: u64 = 4;

#[derive(Debug, Default)]
pub enum Root {
    #[default]
    None,
    AddrWithHash(LinearAddress, TrieHash),
    Node(Node),
}

impl From<Node> for Root {
    fn from(node: Node) -> Self {
        Root::Node(node)
    }
}

impl From<Option<Node>> for Root {
    fn from(node: Option<Node>) -> Self {
        match node {
            Some(node) => Root::Node(node),
            None => Root::None,
        }
    }
}

// /// A [HashedNodeStore] keeps track of nodes as they change when they are backed by a LinearStore.
// /// This defers the writes of those nodes until all the changes are made to the trie as part of a
// /// batch. It also supports a `freeze` method which consumes a [WriteLinearStore] backed [HashedNodeStore],
// /// makes sure the hashes are filled in, and flushes the nodes out before returning a [ReadLinearStore]
// /// backed HashedNodeStore for future operations. `freeze` is called when the batch is completed and is
// /// used to obtain the merkle trie for the proposal.
// #[derive(Debug)]
// pub struct HashedNodeStore<T: ReadLinearStore> {
//     nodestore: NodeStore<T>,
//     deleted: HashSet<LinearAddress>,
//     pub(super) root: Root,
// }

// impl<T: ReadLinearStore> HashedNodeStore<T> {
//     /// Returns a new empty HashedNodeStore that uses `linearstore` as the backing store.
//     pub fn new(nodestore: NodeStore<T>) -> Result<Self, Error> {
//         Ok(HashedNodeStore {
//             nodestore,
//             deleted: HashSet::new(),
//             root: Root::None,
//         })
//     }
// }

// impl<T: ReadLinearStore> HashedNodeStore<T> {
//     pub fn read_node(&self, addr: LinearAddress) -> Result<Node, Error> {
//         if self.deleted.contains(&addr) {
//             return Err(Error::new(std::io::ErrorKind::NotFound, "Node not found"));
//         }
//         self.nodestore.read_node(addr)
//     }

//     pub const fn root(&self) -> &Root {
//         &self.root
//     }
// }

// impl<T: ReadLinearStore> HashedNodeStore<T> {
//     // Hashes `node`, which is at the given `path_prefix`, and its children recursively,
//     // then writes the `node` to `self.nodestore`. Returns its hash and address.
//     fn hash(
//         &mut self,
//         mut node: Node,
//         path_prefix: &mut Path,
//     ) -> Result<(TrieHash, LinearAddress), Error> {
//         match node {
//             Node::Branch(ref mut b) => {
//                 for (nibble, child) in b.children.iter_mut().enumerate() {
//                     // Take child from b.children:
//                     let Child::Node(child_node) = std::mem::replace(child, Child::None) else {
//                         // There is no child or we already know its hash.
//                         continue;
//                     };

//                     // Hash this child and update
//                     let original_length = path_prefix.len();
//                     path_prefix
//                         .0
//                         .extend(b.partial_path.0.iter().copied().chain(once(nibble as u8)));

//                     let (child_hash, child_addr) = self.hash(child_node, path_prefix)?;

//                     *child = Child::AddressWithHash(child_addr, child_hash);
//                     path_prefix.0.truncate(original_length);
//                 }
//             }
//             Node::Leaf(_) => {}
//         }

//         let hash = hash_node(&node, path_prefix);
//         match self.nodestore.create_node(node) {
//             Ok(addr) => Ok((hash, addr)),
//             Err(e) => Err(e),
//         }
//     }

//     pub fn freeze(mut self) -> Result<HashedNodeStore<ProposedImmutable>, Error> {
//         for addr in self.deleted.iter() {
//             self.nodestore.delete_node(*addr)?;
//         }
//         match self.root() {
//             Root::None => {
//                 self.nodestore.set_root(None)?;
//                 Ok(HashedNodeStore {
//                     nodestore: self.nodestore.freeze(),
//                     deleted: self.deleted,
//                     root: Root::None, // TODO do this better. We have `root` above.
//                 })
//             }
//             Root::AddrWithHash(addr, hash) => {
//                 let hash = hash.clone(); // Todo can we avoid this clone?
//                 let addr = *addr;
//                 self.nodestore.set_root(Some(addr))?;
//                 Ok(HashedNodeStore {
//                     nodestore: self.nodestore.freeze(),
//                     deleted: self.deleted,
//                     root: Root::AddrWithHash(addr, hash),
//                 })
//             }
//             Root::Node(node) => {
//                 // TODO avoid clone

//                 let (hash, addr) = self.hash(node.clone(), &mut Path(Default::default()))?;
//                 self.nodestore.set_root(Some(addr))?;
//                 Ok(HashedNodeStore {
//                     nodestore: self.nodestore.freeze(),
//                     deleted: self.deleted,
//                     root: Root::AddrWithHash(addr, hash),
//                 })
//             }
//         }
//     }

//     pub fn delete_node(&mut self, addr: LinearAddress) -> Result<(), Error> {
//         self.deleted.insert(addr);
//         Ok(())
//         // self.nodestore.delete_node(addr)
//     }

//     pub fn set_root(&mut self, root: Root) -> Result<(), Error> {
//         self.root = root;
//         Ok(())
//     }
// }

pub fn hash_node(node: &Node, path_prefix: &Path) -> TrieHash {
    match node {
        Node::Branch(node) => {
            // All child hashes should be filled in.
            // TODO danlaine: Enforce this with the type system.
            debug_assert!(node.children.iter().all(|c| !matches!(c, Child::Node(..))));
            NodeAndPrefix {
                node: node.as_ref(),
                prefix: path_prefix,
            }
            .into()
        }
        Node::Leaf(node) => NodeAndPrefix {
            node,
            prefix: path_prefix,
        }
        .into(),
    }
}

/// Returns the serialized representation of `node` used as the pre-image
/// when hashing the node. The node is at the given `path_prefix`.
pub fn hash_preimage(node: &Node, path_prefix: &Path) -> Box<[u8]> {
    // Key, 3 options, value digest
    let est_len = node.partial_path().len() + path_prefix.len() + 3 + TrieHash::default().len();
    let mut buf = Vec::with_capacity(est_len);
    match node {
        Node::Branch(node) => {
            NodeAndPrefix {
                node: node.as_ref(),
                prefix: path_prefix,
            }
            .write(&mut buf);
        }
        Node::Leaf(node) => {
            NodeAndPrefix {
                node,
                prefix: path_prefix,
            }
            .write(&mut buf);
        }
    }
    buf.into_boxed_slice()
}

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
    fn children_iter(&self) -> impl Iterator<Item = (usize, &TrieHash)> + Clone;
}

impl HashableNode for storage::BranchNode {
    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone {
        self.partial_path.0.iter().copied()
    }

    fn value(&self) -> Option<&[u8]> {
        self.value.as_deref()
    }

    fn children_iter(&self) -> impl Iterator<Item = (usize, &TrieHash)> + Clone {
        self.children_iter()
    }
}

impl HashableNode for storage::LeafNode {
    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone {
        self.partial_path.0.iter().copied()
    }

    fn value(&self) -> Option<&[u8]> {
        Some(&self.value)
    }

    fn children_iter(&self) -> impl Iterator<Item = (usize, &TrieHash)> + Clone {
        iter::empty()
    }
}

struct NodeAndPrefix<'a, N: HashableNode> {
    node: &'a N,
    prefix: &'a Path,
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
        self.node.children_iter()
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
        let mut hns = HashedNodeStore::new(memstore).unwrap();
        let node = Node::Leaf(storage::LeafNode {
            partial_path: Path(Default::default()),
            value: Box::new(*b"abc"),
        });
        // let addr = hns.create_node(node).unwrap();
        hns.set_root(Root::Node(node)).unwrap();

        let frozen = hns.freeze().unwrap();
        assert!(!matches!(frozen.root(), Root::None));
    }
}
