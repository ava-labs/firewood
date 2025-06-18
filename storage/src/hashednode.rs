// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Found 1 occurrences after enabling the lint."
)]

use std::{
    iter::{self},
    ops::Deref,
};

use smallvec::SmallVec;

use crate::{BranchNode, HashType, LeafNode, Node, Path};

/// Returns the hash of `node`, which is at the given `path_prefix`.
#[must_use]
pub fn hash_node(node: &Node, path_prefix: &Path) -> HashType {
    match node {
        Node::Branch(node) => {
            // All child hashes should be filled in.
            // TODO danlaine: Enforce this with the type system.
            #[cfg(debug_assertions)]
            debug_assert!(
                node.children
                    .iter()
                    .all(|c| !matches!(c, Some(crate::Child::Node(_)))),
                "branch children: {:?}",
                node.children
            );
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
#[must_use]
pub fn hash_preimage(node: &Node, path_prefix: &Path) -> Box<[u8]> {
    // Key, 3 options, value digest
    let est_len = node.partial_path().len() + path_prefix.len() + 3 + HashType::default().len();
    let mut buf = Vec::with_capacity(est_len);
    match node {
        Node::Branch(node) => {
            NodeAndPrefix {
                node: node.as_ref(),
                prefix: path_prefix,
            }
            .write(&mut buf);
        }
        Node::Leaf(node) => NodeAndPrefix {
            node,
            prefix: path_prefix,
        }
        .write(&mut buf),
    }
    buf.into_boxed_slice()
}

pub trait HasUpdate {
    fn update<T: AsRef<[u8]>>(&mut self, data: T);
}

impl HasUpdate for Vec<u8> {
    fn update<T: AsRef<[u8]>>(&mut self, data: T) {
        self.extend(data.as_ref().iter().copied());
    }
}

// TODO: make it work with any size SmallVec
// impl<T: AsRef<[u8]> + smallvec::Array> HasUpdate for SmallVec<T> {
//     fn update<U: AsRef<[u8]>>(&mut self, data: U) {
//         self.extend(data.as_ref());
//     }
// }

impl HasUpdate for SmallVec<[u8; 32]> {
    fn update<T: AsRef<[u8]>>(&mut self, data: T) {
        self.extend(data.as_ref().iter().copied());
    }
}

#[derive(Clone, Debug)]
/// A `ValueDigest` is either a node's value or the hash of its value.
pub enum ValueDigest<T> {
    /// The node's value.
    Value(T),
    /// TODO this variant will be used when we deserialize a proof node
    /// from a remote Firewood instance. The serialized proof node they
    /// send us may the hash of the value, not the value itself.
    Hash(T),
}

impl<T> Deref for ValueDigest<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        match self {
            ValueDigest::Value(value) => value,
            ValueDigest::Hash(hash) => hash,
        }
    }
}

/// A node in the trie that can be hashed.
pub trait Hashable {
    /// The key of the node where each byte is a nibble.
    fn key(&self) -> impl Iterator<Item = u8> + Clone;
    /// The partial path of this node
    #[cfg(feature = "ethhash")]
    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone;
    /// The node's value or hash.
    fn value_digest(&self) -> Option<ValueDigest<&[u8]>>;
    /// Each element is a child's index and hash.
    /// Yields 0 elements if the node is a leaf.
    fn children(&self) -> impl Iterator<Item = (usize, &HashType)> + Clone;
}

/// A preimage of a hash.
pub trait Preimage {
    /// Returns the hash of this preimage.
    fn to_hash(&self) -> HashType;
    /// Write this hash preimage to `buf`.
    fn write(&self, buf: &mut impl HasUpdate);
}

trait HashableNode {
    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone;
    fn value(&self) -> Option<&[u8]>;
    fn children_iter(&self) -> impl Iterator<Item = (usize, &HashType)> + Clone;
}

impl HashableNode for BranchNode {
    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone {
        self.partial_path.0.iter().copied()
    }

    fn value(&self) -> Option<&[u8]> {
        self.value.as_deref()
    }

    fn children_iter(&self) -> impl Iterator<Item = (usize, &HashType)> + Clone {
        self.children_hashes()
    }
}

impl HashableNode for LeafNode {
    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone {
        self.partial_path.0.iter().copied()
    }

    fn value(&self) -> Option<&[u8]> {
        Some(&self.value)
    }

    fn children_iter(&self) -> impl Iterator<Item = (usize, &HashType)> + Clone {
        iter::empty()
    }
}

struct NodeAndPrefix<'a, N: HashableNode> {
    node: &'a N,
    prefix: &'a Path,
}

impl<'a, N: HashableNode> From<NodeAndPrefix<'a, N>> for HashType {
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

    #[cfg(feature = "ethhash")]
    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone {
        self.node.partial_path()
    }

    fn value_digest(&self) -> Option<ValueDigest<&'a [u8]>> {
        self.node.value().map(ValueDigest::Value)
    }

    fn children(&self) -> impl Iterator<Item = (usize, &HashType)> + Clone {
        self.node.children_iter()
    }
}
