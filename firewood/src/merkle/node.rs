// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{
    logger::trace,
    merkle::from_nibbles,
    shale::{CachedStore, DiskAddress, ShaleError, Storable},
};
use bincode::Options;
use bitflags::bitflags;
use bytemuck::{CheckedBitPattern, NoUninit};
use enum_as_inner::EnumAsInner;
use serde::{
    ser::{SerializeSeq, SerializeTuple},
    Deserialize, Serialize,
};
use sha3::{Digest, Keccak256};
use std::{
    fmt::Debug,
    io::{Cursor, Write},
    marker::PhantomData,
    mem::size_of,
};

mod branch;
mod leaf;
mod partial_path;

pub use branch::BranchNode;
pub use leaf::{LeafNode, SIZE as LEAF_NODE_SIZE};
pub use partial_path::PartialPath;

use crate::nibbles::Nibbles;

use super::TRIE_HASH_LEN;

bitflags! {
    // should only ever be the size of a nibble
    struct Flags: u8 {
        const ODD_LEN  = 0b0001;
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Data(pub(super) Vec<u8>);

impl std::ops::Deref for Data {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl From<Vec<u8>> for Data {
    fn from(v: Vec<u8>) -> Self {
        Self(v)
    }
}

impl Data {
    pub fn into_inner(self) -> Vec<u8> {
        self.0
    }
}

#[derive(PartialEq, Eq, Clone, Debug, EnumAsInner)]
pub enum Node {
    Branch(Box<BranchNode>),
    Leaf(LeafNode),
}

impl Node {
    pub fn path_mut(&mut self) -> &mut PartialPath {
        match self {
            Node::Branch(u) => &mut u.path,
            Node::Leaf(node) => &mut node.path,
        }
    }

    pub fn set_path(&mut self, path: PartialPath) {
        match self {
            Node::Branch(u) => u.path = path,
            Node::Leaf(node) => node.path = path,
        }
    }

    pub fn set_data(&mut self, data: Data) {
        match self {
            Node::Branch(u) => u.value = Some(data),
            Node::Leaf(node) => node.data = data,
        }
    }

    pub(super) fn max_branch_node_size() -> u64 {
        Node::Branch(Box::new(BranchNode {
            path: vec![].into(),
            children: [Some(DiskAddress::null()); BranchNode::MAX_CHILDREN],
            value: Some(Data(Vec::new())),
            children_encoded: Default::default(),
        }))
        .serialized_len()
    }
}

impl Storable for Node {
    fn deserialize<T: CachedStore>(offset: usize, mem: &T) -> Result<Self, ShaleError> {
        let meta_raw =
            mem.get_view(offset, Meta::SIZE as u64)
                .ok_or(ShaleError::InvalidCacheView {
                    offset,
                    size: Meta::SIZE as u64,
                })?;
        let meta_raw = meta_raw.as_deref();

        let meta = bytemuck::checked::try_from_bytes::<Meta>(&meta_raw)
            .map_err(|_| ShaleError::InvalidNodeMeta)?;

        let Meta { type_id } = *meta;

        trace!("[{mem:p}] Deserializing node at {offset}");

        let offset = offset + Meta::SIZE;

        let node = match type_id {
            NodeTypeId::Branch => {
                let node = BranchNode::deserialize(offset, mem)?;
                Node::Branch(Box::new(node))
            }
            NodeTypeId::Leaf => {
                let node = LeafNode::deserialize(offset, mem)?;
                Node::Leaf(node)
            }
        };

        Ok(node)
    }

    fn serialized_len(&self) -> u64 {
        Meta::SIZE as u64
            + match &self {
                Node::Branch(n) => n.serialized_len(),
                Node::Leaf(n) => n.serialized_len(),
            }
    }

    fn serialize(&self, to: &mut [u8]) -> Result<(), ShaleError> {
        trace!("[{self:p}] Serializing node");
        let mut cursor = Cursor::new(to);

        let type_id = NodeTypeId::from(self);

        let meta = Meta { type_id };

        cursor.write_all(bytemuck::bytes_of(&meta))?;

        let pos = cursor.position() as usize;

        match &self {
            Node::Branch(n) =>
            {
                #[allow(clippy::indexing_slicing)]
                n.serialize(&mut cursor.get_mut()[pos..])
            }

            Node::Leaf(n) =>
            {
                #[allow(clippy::indexing_slicing)]
                n.serialize(&mut cursor.get_mut()[pos..])
            }
        }
    }
}

#[derive(Clone, Copy, CheckedBitPattern, NoUninit)]
#[repr(C, packed)]
struct Meta {
    type_id: NodeTypeId,
}

impl Meta {
    const SIZE: usize = size_of::<Self>();
}

mod type_id {
    use super::{CheckedBitPattern, NoUninit, Node};
    use crate::shale::ShaleError;

    #[derive(Clone, Copy, CheckedBitPattern, NoUninit)]
    #[repr(u8)]
    pub enum NodeTypeId {
        Branch = 0,
        Leaf = 1,
    }

    impl TryFrom<u8> for NodeTypeId {
        type Error = ShaleError;

        fn try_from(value: u8) -> Result<Self, Self::Error> {
            bytemuck::checked::try_cast::<_, Self>(value).map_err(|_| ShaleError::InvalidNodeType)
        }
    }

    impl From<&Node> for NodeTypeId {
        fn from(node_type: &Node) -> Self {
            match node_type {
                Node::Branch(_) => NodeTypeId::Branch,
                Node::Leaf(_) => NodeTypeId::Leaf,
            }
        }
    }
}

use type_id::NodeTypeId;

pub struct EncodedNode<T> {
    pub(crate) node: EncodedNodeType,
    pub(crate) phantom: PhantomData<T>,
}

impl<T> EncodedNode<T> {
    pub const fn new(node: EncodedNodeType) -> Self {
        Self {
            node,
            phantom: PhantomData,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum EncodedNodeType {
    Leaf(LeafNode),
    Branch {
        path: PartialPath,
        children: Box<[Option<Vec<u8>>; BranchNode::MAX_CHILDREN]>,
        value: Option<Data>,
    },
}

// TODO: probably can merge with `EncodedNodeType`.
#[derive(Debug, Deserialize)]
struct EncodedBranchNode {
    chd: Vec<(u64, Vec<u8>)>,
    data: Option<Vec<u8>>,
    path: Vec<u8>,
}

// Note that the serializer passed in should always be the same type as T in EncodedNode<T>.
impl Serialize for EncodedNode<PlainCodec> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let (chd, data, path) = match &self.node {
            EncodedNodeType::Leaf(n) => {
                let data = Some(&*n.data);
                let chd: Vec<(u64, Vec<u8>)> = Default::default();
                let path: Vec<_> = from_nibbles(&n.path.encode()).collect();
                (chd, data, path)
            }

            EncodedNodeType::Branch {
                path,
                children,
                value,
            } => {
                let chd: Vec<(u64, Vec<u8>)> = children
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

                let data = value.as_deref();

                let path = from_nibbles(&path.encode()).collect();

                (chd, data, path)
            }
        };

        let mut s = serializer.serialize_tuple(3)?;

        s.serialize_element(&chd)?;
        s.serialize_element(&data)?;
        s.serialize_element(&path)?;

        s.end()
    }
}

impl<'de> Deserialize<'de> for EncodedNode<PlainCodec> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let EncodedBranchNode { chd, data, path } = Deserialize::deserialize(deserializer)?;

        let path = PartialPath::from_nibbles(Nibbles::<0>::new(&path).into_iter());

        if chd.is_empty() {
            let data = if let Some(d) = data {
                Data(d)
            } else {
                Data(Vec::new())
            };

            let node = EncodedNodeType::Leaf(LeafNode { path, data });

            Ok(Self::new(node))
        } else {
            let mut children: [Option<Vec<u8>>; BranchNode::MAX_CHILDREN] = Default::default();
            let value = data.map(Data);

            #[allow(clippy::indexing_slicing)]
            for (i, chd) in chd {
                children[i as usize] = Some(chd);
            }

            let node = EncodedNodeType::Branch {
                path,
                children: children.into(),
                value,
            };

            Ok(Self::new(node))
        }
    }
}

// Note that the serializer passed in should always be the same type as T in EncodedNode<T>.
impl Serialize for EncodedNode<Bincode> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match &self.node {
            EncodedNodeType::Leaf(n) => {
                let list = [from_nibbles(&n.path.encode()).collect(), n.data.to_vec()];
                let mut seq = serializer.serialize_seq(Some(list.len()))?;
                for e in list {
                    seq.serialize_element(&e)?;
                }
                seq.end()
            }

            EncodedNodeType::Branch {
                path,
                children,
                value,
            } => {
                let mut list = <[Vec<u8>; BranchNode::MAX_CHILDREN + 2]>::default();
                let children = children
                    .iter()
                    .enumerate()
                    .filter_map(|(i, c)| c.as_ref().map(|c| (i, c)));

                #[allow(clippy::indexing_slicing)]
                for (i, child) in children {
                    list[i] = child.clone();
                }

                if let Some(Data(val)) = &value {
                    list[BranchNode::MAX_CHILDREN] = val.clone();
                }

                let serialized_path = from_nibbles(&path.encode()).collect();
                list[BranchNode::MAX_CHILDREN + 1] = serialized_path;

                let mut seq = serializer.serialize_seq(Some(list.len()))?;

                for e in list {
                    seq.serialize_element(&e)?;
                }

                seq.end()
            }
        }
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
                let Some(data) = items.next() else {
                    return Err(D::Error::custom(
                        "incorrect encoded type for leaf node data",
                    ));
                };
                let path = PartialPath::from_nibbles(Nibbles::<0>::new(&path).into_iter());
                let node = EncodedNodeType::Leaf(LeafNode {
                    path,
                    data: Data(data),
                });
                Ok(Self::new(node))
            }

            BranchNode::MSIZE => {
                let path = items.pop().expect("length was checked above");
                let path = PartialPath::from_nibbles(Nibbles::<0>::new(&path).into_iter());

                let value = items.pop().expect("length was checked above");
                let value = if value.is_empty() {
                    None
                } else {
                    Some(Data(value))
                };

                let mut children: [Option<Vec<u8>>; BranchNode::MAX_CHILDREN] = Default::default();

                for (i, chd) in items.into_iter().enumerate() {
                    #[allow(clippy::indexing_slicing)]
                    (children[i] = Some(chd).filter(|chd| !chd.is_empty()));
                }

                let node = EncodedNodeType::Branch {
                    path,
                    children: children.into(),
                    value,
                };

                Ok(Self {
                    node,
                    phantom: PhantomData,
                })
            }
            size => Err(D::Error::custom(format!("invalid size: {size}"))),
        }
    }
}

pub trait BinarySerde {
    type SerializeError: serde::ser::Error;
    type DeserializeError: serde::de::Error;

    fn new() -> Self;

    fn serialize<T: Serialize>(t: &T) -> Result<Vec<u8>, Self::SerializeError>
    where
        Self: Sized,
    {
        Self::new().serialize_impl(t)
    }

    fn deserialize<'de, T: Deserialize<'de>>(bytes: &'de [u8]) -> Result<T, Self::DeserializeError>
    where
        Self: Sized,
    {
        Self::new().deserialize_impl(bytes)
    }

    fn serialize_impl<T: Serialize>(&self, t: &T) -> Result<Vec<u8>, Self::SerializeError>;
    fn deserialize_impl<'de, T: Deserialize<'de>>(
        &self,
        bytes: &'de [u8],
    ) -> Result<T, Self::DeserializeError>;
}

#[derive(Default)]
pub struct Bincode(pub bincode::DefaultOptions);

impl Debug for Bincode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "[bincode::DefaultOptions]")
    }
}

impl BinarySerde for Bincode {
    type SerializeError = bincode::Error;
    type DeserializeError = Self::SerializeError;

    fn new() -> Self {
        Self(bincode::DefaultOptions::new())
    }

    fn serialize_impl<T: Serialize>(&self, t: &T) -> Result<Vec<u8>, Self::SerializeError> {
        self.0.serialize(t)
    }

    fn deserialize_impl<'de, T: Deserialize<'de>>(
        &self,
        bytes: &'de [u8],
    ) -> Result<T, Self::DeserializeError> {
        self.0.deserialize(bytes)
    }
}

#[derive(Default)]
pub struct PlainCodec(pub bincode::DefaultOptions);

impl Debug for PlainCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        write!(f, "PlainCodec")
    }
}

impl BinarySerde for PlainCodec {
    type SerializeError = bincode::Error;
    type DeserializeError = Self::SerializeError;

    fn new() -> Self {
        Self(bincode::DefaultOptions::new())
    }

    fn serialize_impl<T: Serialize>(&self, t: &T) -> Result<Vec<u8>, Self::SerializeError> {
        // Serializes the object directly into a Writer without include the length.
        let mut writer = Vec::new();
        self.0.serialize_into(&mut writer, t)?;
        Ok(writer)
    }

    fn deserialize_impl<'de, T: Deserialize<'de>>(
        &self,
        bytes: &'de [u8],
    ) -> Result<T, Self::DeserializeError> {
        self.0.deserialize(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bincode::Error;
    use test_case::test_case;

    #[test_case(&[])]
    #[test_case(&[0x00])]
    #[test_case(&[0x0F])]
    #[test_case(&[0x00, 0x00])]
    #[test_case(&[0x01, 0x02])]
    #[test_case(&[0x00,0x0F])]
    #[test_case(&[0x0F,0x0F])]
    #[test_case(&[0x0F,0x01,0x0F])]
    fn encoded_branch_node_bincode_serialize(path_nibbles: &[u8]) -> Result<(), Error> {
        let node = EncodedNode::<Bincode>::new(EncodedNodeType::Branch {
            path: PartialPath(path_nibbles.to_vec()),
            children: Default::default(),
            value: Some(Data(vec![1, 2, 3, 4])),
        });

        let node_bytes = Bincode::serialize(&node)?;

        let deserialized_node: EncodedNode<Bincode> = Bincode::deserialize(&node_bytes)?;

        assert_eq!(&node.node, &deserialized_node.node);

        Ok(())
    }

    struct Nil;

    macro_rules! impl_nil_for {
        // match a comma separated list of types
        ($($t:ty),* $(,)?) => {
            $(
                impl From<Nil> for Option<$t> {
                    fn from(_val: Nil) -> Self {
                        None
                    }
                }
            )*
        };
    }

    impl_nil_for!([u8; 32], Vec<u8>, usize, u8, bool);
}
