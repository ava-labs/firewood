// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{merkle::nibbles_to_bytes_iter, trie_hash::TRIE_HASH_LEN};
use enum_as_inner::EnumAsInner;
use serde::{ser::SerializeTuple, Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::fmt::Debug;

mod branch;
mod leaf;
pub(crate) mod path;

pub use branch::BranchNode;
pub use leaf::LeafNode;

use crate::nibbles::Nibbles;

use self::path::Path;

#[derive(PartialEq, Eq, Clone, Debug, EnumAsInner, Serialize, Deserialize)]
pub enum Node {
    Branch(Box<BranchNode>),
    Leaf(LeafNode),
}

/// TODO remove generic on this type and just implement serialize/deserialize once.
/// Contains the fields that we include in a node's hash.
/// If this is a leaf node, `children` is empty and `value` is Some.
/// If this is a branch node, `children` is non-empty.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct EncodedNode {
    pub(crate) partial_path: Path,
    /// If a child is None, it doesn't exist.
    /// If it's Some, it's the value or value hash of the child.
    pub(crate) children: [Option<Vec<u8>>; BranchNode::MAX_CHILDREN],
    pub(crate) value: Option<Vec<u8>>,
}

// TODO danlaine: move node serialization (for persistence and for hashing) somewhere else.
// Note that the serializer passed in should always be the same type as T in EncodedNode<T>.
impl Serialize for EncodedNode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let chd: Vec<(u64, Vec<u8>)> = self
            .children
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.as_ref().map(|c| (i as u64, c)))
            .map(|(i, c)| {
                if c.len() >= TRIE_HASH_LEN {
                    (i, Keccak256::digest(c).to_vec())
                } else {
                    (i, c.to_vec())
                }
            })
            .collect();

        let value = self.value.as_deref();

        let path: Vec<u8> = nibbles_to_bytes_iter(&self.partial_path.encode()).collect();

        let mut s = serializer.serialize_tuple(3)?;

        s.serialize_element(&chd)?;
        s.serialize_element(&value)?;
        s.serialize_element(&path)?;

        s.end()
    }
}

impl<'de> Deserialize<'de> for EncodedNode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let chd: Vec<(u64, Vec<u8>)>;
        let value: Option<Vec<u8>>;
        let path: Vec<u8>;

        (chd, value, path) = Deserialize::deserialize(deserializer)?;

        let path = Path::from_nibbles(Nibbles::new(&path).into_iter());

        let mut children: [Option<Vec<u8>>; BranchNode::MAX_CHILDREN] = Default::default();
        #[allow(clippy::indexing_slicing)]
        for (i, chd) in chd {
            children[i as usize] = Some(chd);
        }

        Ok(Self {
            partial_path: path,
            children,
            value,
        })
    }
}
