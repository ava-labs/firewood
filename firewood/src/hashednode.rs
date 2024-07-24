// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use sha2::{Digest, Sha256};
use std::iter::{self};

use storage::LinearAddress;
use storage::{Child, TrieHash};
use storage::{Node, Path};

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

pub fn hash_node(node: &Node, path_prefix: &Path) -> TrieHash {
    match node {
        Node::Branch(node) => {
            // All child hashes should be filled in.
            // TODO danlaine: Enforce this with the type system.
            debug_assert!(node
                .children
                .iter()
                .all(|c| !matches!(c, Some(Child::Node(_)))));
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
