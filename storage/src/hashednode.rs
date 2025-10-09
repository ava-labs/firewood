// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{BranchNode, Children, HashType, LeafNode, Node, Path};
use smallvec::SmallVec;

/// Returns the hash of `node`, which is at the given `path_prefix`.
#[must_use]
pub fn hash_node(node: &Node, path_prefix: &Path) -> HashType {
    match node {
        Node::Branch(node) => {
            // All child hashes should be filled in.
            // TODO danlaine: Enforce this with the type system.
            debug_assert!(
                node.children
                    .iter()
                    .all(|(_, c)| !matches!(c, Some(crate::Child::Node(_)))),
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
    #[expect(clippy::arithmetic_side_effects)]
    let est_len = node.partial_path().len() + path_prefix.len() + 3 + HashType::empty().len();
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

#[derive(Clone, Debug, PartialEq, Eq)]
/// A `ValueDigest` is either a node's value or the hash of its value.
pub enum ValueDigest<T> {
    /// The node's value.
    Value(T),
    #[cfg(not(feature = "ethhash"))]
    /// For MerkleDB hashing, the digest is the hash of the value if it is 32
    /// bytes or longer.
    Hash(HashType),
}

impl<T: AsRef<[u8]>> ValueDigest<T> {
    /// Verifies that the value or hash matches the expected value.
    pub fn verify(&self, expected: impl AsRef<[u8]>) -> bool {
        match self {
            Self::Value(got_value) => {
                // This proof proves that `key` maps to `got_value`.
                got_value.as_ref() == expected.as_ref()
            }
            #[cfg(not(feature = "ethhash"))]
            Self::Hash(got_hash) => {
                use sha2::{Digest, Sha256};
                // This proof proves that `key` maps to a value
                // whose hash is `got_hash`.
                *got_hash == HashType::from(Sha256::digest(expected.as_ref()))
            }
        }
    }

    /// Returns a `ValueDigest` that borrows from this one.
    pub fn as_ref(&self) -> ValueDigest<&[u8]> {
        match self {
            Self::Value(v) => ValueDigest::Value(v.as_ref()),
            #[cfg(not(feature = "ethhash"))]
            Self::Hash(h) => ValueDigest::Hash(h.clone()),
        }
    }

    /// Convert the value to a hash if it is not already a hash.
    ///
    /// If the value is less than 32 bytes, it will be passed through as is
    /// instead of hashing.
    ///
    /// If etherum hashing is enabled, this will always return the value as is.
    pub fn make_hash(&self) -> ValueDigest<&[u8]> {
        match self.as_ref() {
            #[cfg(not(feature = "ethhash"))]
            ValueDigest::Value(v) if v.len() >= 32 => {
                use sha2::{Digest, Sha256};
                ValueDigest::Hash(HashType::from(Sha256::digest(v)))
            }

            ValueDigest::Value(v) => ValueDigest::Value(v),

            #[cfg(not(feature = "ethhash"))]
            ValueDigest::Hash(v) => ValueDigest::Hash(v),
        }
    }
}

impl<T: AsRef<[u8]>> AsRef<[u8]> for ValueDigest<T> {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Value(v) => v.as_ref(),
            #[cfg(not(feature = "ethhash"))]
            Self::Hash(h) => h.as_ref(),
        }
    }
}

/// A node in the trie that can be hashed.
pub trait Hashable: std::fmt::Debug {
    /// The full path of this node's parent where each byte is a nibble.
    fn parent_prefix_path(&self) -> impl Iterator<Item = u8> + Clone;
    /// The partial path of this node where each byte is a nibble.
    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone;
    /// The node's value or hash.
    fn value_digest(&self) -> Option<ValueDigest<&[u8]>>;
    /// Each element is a child's index and hash.
    /// Yields 0 elements if the node is a leaf.
    fn children(&self) -> Children<Option<HashType>>;

    /// The full path of this node including the parent's prefix where each byte is a nibble.
    fn full_path(&self) -> impl Iterator<Item = u8> + Clone {
        self.parent_prefix_path().chain(self.partial_path())
    }
}

/// A preimage of a hash.
pub trait Preimage: std::fmt::Debug {
    /// Returns the hash of this preimage.
    fn to_hash(&self) -> HashType;
    /// Write this hash preimage to `buf`.
    fn write(&self, buf: &mut impl HasUpdate);
}

trait HashableNode: std::fmt::Debug {
    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone;
    fn value(&self) -> Option<&[u8]>;
    fn child_hashes(&self) -> Children<Option<HashType>>;
}

impl HashableNode for BranchNode {
    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone {
        self.partial_path.0.iter().copied()
    }

    fn value(&self) -> Option<&[u8]> {
        self.value.as_deref()
    }

    fn child_hashes(&self) -> Children<Option<HashType>> {
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

    fn child_hashes(&self) -> Children<Option<HashType>> {
        Children::new()
    }
}

#[derive(Debug)]
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
    fn parent_prefix_path(&self) -> impl Iterator<Item = u8> + Clone {
        self.prefix.0.iter().copied()
    }

    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone {
        self.node.partial_path()
    }

    fn value_digest(&self) -> Option<ValueDigest<&'a [u8]>> {
        self.node.value().map(ValueDigest::Value)
    }

    fn children(&self) -> Children<Option<HashType>> {
        self.node.child_hashes()
    }
}
