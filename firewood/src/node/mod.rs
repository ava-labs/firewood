// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::merkle::codec::Bincode;
use crate::merkle::codec::PlainCodec;
use crate::merkle::nibbles_to_bytes_iter;
use crate::merkle::TRIE_HASH_LEN;
use enum_as_inner::EnumAsInner;
use serde::{
    ser::{SerializeSeq, SerializeTuple},
    Deserialize, Serialize,
};
use sha3::{Digest, Keccak256};
use std::{fmt::Debug, marker::PhantomData};

mod branch;
mod leaf;
mod path;

pub use branch::BranchNode;
pub use leaf::{LeafNode, SIZE as LEAF_NODE_SIZE};

use crate::nibbles::Nibbles;

use self::path::Path;

#[derive(PartialEq, Eq, Clone, Debug, EnumAsInner)]
pub enum Node {
    Branch(Box<BranchNode>),
    Leaf(LeafNode),
}

/// TODO remove generic on this type and just implement serialize/deserialize once.
/// Contains the fields that we include in a node's hash.
/// If this is a leaf node, `children` is empty and `value` is Some.
/// If this is a branch node, `children` is non-empty.
#[derive(Debug, Eq)]
pub struct EncodedNode<T> {
    pub(crate) partial_path: Path,
    /// If a child is None, it doesn't exist.
    /// If it's Some, it's the value or value hash of the child.
    pub(crate) children: [Option<Vec<u8>>; BranchNode::MAX_CHILDREN],
    pub(crate) value: Option<Vec<u8>>,
    pub(crate) phantom: PhantomData<T>,
}

// driving this adds an unnecessary bound, T: PartialEq
// PhantomData<T> is PartialEq for all T
impl<T> PartialEq for EncodedNode<T> {
    fn eq(&self, other: &Self) -> bool {
        let Self {
            partial_path,
            children,
            value,
            phantom: _,
        } = self;
        partial_path == &other.partial_path && children == &other.children && value == &other.value
    }
}

// TODO danlaine: move node serialization (for persistence and for hashing) somewhere else.
// Note that the serializer passed in should always be the same type as T in EncodedNode<T>.
impl Serialize for EncodedNode<PlainCodec> {
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

impl<'de> Deserialize<'de> for EncodedNode<PlainCodec> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let chd: Vec<(u64, Vec<u8>)>;
        let value: Option<Vec<u8>>;
        let path: Vec<u8>;

        (chd, value, path) = Deserialize::deserialize(deserializer)?;

        let path = Path::from_nibbles(Nibbles::<0>::new(&path).into_iter());

        let mut children: [Option<Vec<u8>>; BranchNode::MAX_CHILDREN] = Default::default();
        #[allow(clippy::indexing_slicing)]
        for (i, chd) in chd {
            children[i as usize] = Some(chd);
        }

        Ok(Self {
            partial_path: path,
            children,
            value,
            phantom: PhantomData,
        })
    }
}

// Note that the serializer passed in should always be the same type as T in EncodedNode<T>.
impl Serialize for EncodedNode<Bincode> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut list = <[Vec<u8>; BranchNode::MAX_CHILDREN + 2]>::default();
        let children = self
            .children
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.as_ref().map(|c| (i, c)));

        #[allow(clippy::indexing_slicing)]
        for (i, child) in children {
            if child.len() >= TRIE_HASH_LEN {
                let serialized_hash = Keccak256::digest(child).to_vec();
                list[i] = serialized_hash;
            } else {
                list[i] = child.to_vec();
            }
        }

        if let Some(val) = &self.value {
            list[BranchNode::MAX_CHILDREN].clone_from(val);
        }

        let serialized_path = nibbles_to_bytes_iter(&self.partial_path.encode()).collect();
        list[BranchNode::MAX_CHILDREN + 1] = serialized_path;

        let mut seq = serializer.serialize_seq(Some(list.len()))?;

        for e in list {
            seq.serialize_element(&e)?;
        }

        seq.end()
    }
}

impl<'de> Deserialize<'de> for EncodedNode<Bincode> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        let mut items: Vec<Vec<u8>> = Deserialize::deserialize(deserializer)?;
        let len = items.len();

        match len {
            LEAF_NODE_SIZE => {
                let mut items = items.into_iter();
                let Some(path) = items.next() else {
                    return Err(D::Error::custom(
                        "incorrect encoded type for leaf node path",
                    ));
                };
                let Some(value) = items.next() else {
                    return Err(D::Error::custom(
                        "incorrect encoded type for leaf node value",
                    ));
                };
                let path = Path::from_nibbles(Nibbles::<0>::new(&path).into_iter());
                let children: [Option<Vec<u8>>; BranchNode::MAX_CHILDREN] = Default::default();
                Ok(Self {
                    partial_path: path,
                    children,
                    value: Some(value),
                    phantom: PhantomData,
                })
            }

            BranchNode::MSIZE => {
                let path = items.pop().expect("length was checked above");
                let path = Path::from_nibbles(Nibbles::<0>::new(&path).into_iter());

                let value = items.pop().expect("length was checked above");
                let value = if value.is_empty() { None } else { Some(value) };

                let mut children: [Option<Vec<u8>>; BranchNode::MAX_CHILDREN] = Default::default();

                for (i, chd) in items.into_iter().enumerate() {
                    #[allow(clippy::indexing_slicing)]
                    (children[i] = Some(chd).filter(|chd| !chd.is_empty()));
                }

                Ok(Self {
                    partial_path: path,
                    children,
                    value,
                    phantom: PhantomData,
                })
            }
            size => Err(D::Error::custom(format!("invalid size: {size}"))),
        }
    }
}
