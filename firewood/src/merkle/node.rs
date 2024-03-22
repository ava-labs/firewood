// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{
    logger::trace,
    merkle::nibbles_to_bytes_iter,
    shale::{disk_address::DiskAddress, CachedStore, ShaleError, Storable},
};
use bincode::Options;
use bitflags::bitflags;
use bytemuck::{CheckedBitPattern, NoUninit, Pod, Zeroable};
use enum_as_inner::EnumAsInner;
use serde::{
    de::Visitor,
    ser::{SerializeSeq, SerializeTuple},
    Deserialize, Serialize,
};
use std::{
    fmt::Debug,
    io::{Cursor, Write},
    marker::PhantomData,
    mem::size_of,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
    vec,
};

mod branch;
mod leaf;
mod path;

pub use branch::BranchNode;
pub use leaf::{LeafNode, SIZE as LEAF_NODE_SIZE};
pub use path::Path;

use crate::nibbles::Nibbles;

use super::TRIE_HASH_LEN;

bitflags! {
    // should only ever be the size of a nibble
    struct Flags: u8 {
        const ODD_LEN  = 0b0001;
    }
}

const NIBBLES_PER_BYTE: usize = 2;
const BITS_PER_NIBBLE: u64 = 4;
const BITS_PER_BYTE: u64 = 8;

// TODO danlaine: This exists to prevent someone from forcing a very large
// memory allocation during PathWithBitsPrefix deserialization.
// Instead of specifying a max size, we should change how we
// serialize/deserialize paths during node hashing so that on deserialize, when
// we read the path bit length, we can verify that the &[u8] we're decoding
// has the expected number of bits.
pub const MAX_PATH_BYTE_LEN: u64 = 2 * 1024 * 1024; // 2 MiB
const MAX_PATH_BIT_LEN: u64 = MAX_PATH_BYTE_LEN * BITS_PER_BYTE;

#[derive(PartialEq, Eq, Clone, Debug, EnumAsInner)]
pub enum NodeType {
    Branch(Box<BranchNode>),
    Leaf(LeafNode),
}

impl NodeType {
    pub fn path_mut(&mut self) -> &mut Path {
        match self {
            NodeType::Branch(u) => &mut u.partial_path,
            NodeType::Leaf(node) => &mut node.partial_path,
        }
    }

    pub fn set_path(&mut self, path: Path) {
        match self {
            NodeType::Branch(u) => u.partial_path = path,
            NodeType::Leaf(node) => node.partial_path = path,
        }
    }

    pub fn set_value(&mut self, value: Vec<u8>) {
        match self {
            NodeType::Branch(u) => u.value = Some(value),
            NodeType::Leaf(node) => node.value = value,
        }
    }
}

#[derive(Debug)]
pub struct Node {
    encoded: OnceLock<Vec<u8>>,
    is_encoded_longer_than_hash_len: OnceLock<bool>,
    // lazy_dirty is an atomicbool, but only writers ever set it
    // Therefore, we can always use Relaxed ordering. It's atomic
    // just to ensure Sync + Send.
    lazy_dirty: AtomicBool,
    pub(super) inner: NodeType,
}

impl Eq for Node {}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        let is_dirty = self.is_dirty();

        let Node {
            encoded,
            is_encoded_longer_than_hash_len: _,
            lazy_dirty: _,
            inner,
        } = self;
        *encoded == other.encoded && is_dirty == other.is_dirty() && *inner == other.inner
    }
}

impl Clone for Node {
    fn clone(&self) -> Self {
        Self {
            is_encoded_longer_than_hash_len: self.is_encoded_longer_than_hash_len.clone(),
            encoded: self.encoded.clone(),
            lazy_dirty: AtomicBool::new(self.is_dirty()),
            inner: self.inner.clone(),
        }
    }
}

impl From<NodeType> for Node {
    fn from(inner: NodeType) -> Self {
        let mut s = Self {
            encoded: OnceLock::new(),
            is_encoded_longer_than_hash_len: OnceLock::new(),
            inner,
            lazy_dirty: AtomicBool::new(false),
        };
        s.rehash();
        s
    }
}

bitflags! {
    #[derive(Debug, Default, Clone, Copy, Pod, Zeroable)]
    #[repr(transparent)]
    struct NodeAttributes: u8 {
        const ENCODED_LENGTH_IS_KNOWN           = 0b001;
        const ENCODED_IS_LONG = 0b010;
    }
}

impl Node {
    pub(super) fn max_branch_node_size() -> u64 {
        let max_size: OnceLock<u64> = OnceLock::new();
        *max_size.get_or_init(|| {
            Self {
                encoded: OnceLock::new(),
                is_encoded_longer_than_hash_len: OnceLock::new(),
                inner: NodeType::Branch(
                    BranchNode {
                        partial_path: vec![].into(),
                        children: [Some(DiskAddress::null()); BranchNode::MAX_CHILDREN],
                        value: Some(Vec::new()),
                        children_encoded: Default::default(),
                    }
                    .into(),
                ),
                lazy_dirty: AtomicBool::new(false),
            }
            .serialized_len()
        })
    }

    pub(super) fn rehash(&mut self) {
        self.encoded = OnceLock::new();
        self.is_encoded_longer_than_hash_len = OnceLock::new();
    }

    pub fn from_branch<B: Into<Box<BranchNode>>>(node: B) -> Self {
        Self::from(NodeType::Branch(node.into()))
    }

    pub fn from_leaf(leaf: LeafNode) -> Self {
        Self::from(NodeType::Leaf(leaf))
    }

    pub const fn inner(&self) -> &NodeType {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut NodeType {
        &mut self.inner
    }

    pub(super) fn new_from_hash(
        encoded: Option<Vec<u8>>,
        is_encoded_longer_than_hash_len: Option<bool>,
        inner: NodeType,
    ) -> Self {
        Self {
            encoded: match encoded.filter(|encoded| !encoded.is_empty()) {
                Some(e) => OnceLock::from(e),
                None => OnceLock::new(),
            },
            is_encoded_longer_than_hash_len: match is_encoded_longer_than_hash_len {
                Some(v) => OnceLock::from(v),
                None => OnceLock::new(),
            },
            inner,
            lazy_dirty: AtomicBool::new(false),
        }
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.lazy_dirty.load(Ordering::Relaxed)
    }

    // TODO use or remove
    // pub(super) fn set_dirty(&self, is_dirty: bool) {
    //     self.lazy_dirty.store(is_dirty, Ordering::Relaxed)
    // }

    pub(crate) fn as_branch_mut(&mut self) -> &mut Box<BranchNode> {
        self.inner_mut()
            .as_branch_mut()
            .expect("must be a branch node")
    }
}

#[derive(Clone, Copy, CheckedBitPattern, NoUninit)]
#[repr(C, packed)]
struct Meta {
    attrs: NodeAttributes,
    encoded_len: u64,
    encoded: [u8; TRIE_HASH_LEN],
    type_id: NodeTypeId,
}

impl Meta {
    const SIZE: usize = size_of::<Self>();
}

mod type_id {
    use super::{CheckedBitPattern, NoUninit, NodeType};
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

    impl From<&NodeType> for NodeTypeId {
        fn from(node_type: &NodeType) -> Self {
            match node_type {
                NodeType::Branch(_) => NodeTypeId::Branch,
                NodeType::Leaf(_) => NodeTypeId::Leaf,
            }
        }
    }
}

use type_id::NodeTypeId;

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

        let Meta {
            attrs,
            encoded_len,
            encoded,
            type_id,
        } = *meta;

        trace!("[{mem:p}] Deserializing node at {offset}");

        let offset = offset + Meta::SIZE;

        let encoded = if encoded_len > 0 {
            Some(encoded.iter().take(encoded_len as usize).copied().collect())
        } else {
            None
        };

        let is_encoded_longer_than_hash_len =
            if attrs.contains(NodeAttributes::ENCODED_LENGTH_IS_KNOWN) {
                attrs.contains(NodeAttributes::ENCODED_IS_LONG).into()
            } else {
                None
            };

        match type_id {
            NodeTypeId::Branch => {
                let inner = NodeType::Branch(Box::new(BranchNode::deserialize(offset, mem)?));

                Ok(Self::new_from_hash(
                    encoded,
                    is_encoded_longer_than_hash_len,
                    inner,
                ))
            }

            NodeTypeId::Leaf => {
                let inner = NodeType::Leaf(LeafNode::deserialize(offset, mem)?);
                let node = Self::new_from_hash(encoded, is_encoded_longer_than_hash_len, inner);

                Ok(node)
            }
        }
    }

    fn serialized_len(&self) -> u64 {
        Meta::SIZE as u64
            + match &self.inner {
                NodeType::Branch(n) => n.serialized_len(),
                NodeType::Leaf(n) => n.serialized_len(),
            }
    }

    fn serialize(&self, to: &mut [u8]) -> Result<(), ShaleError> {
        trace!("[{self:p}] Serializing node");
        let mut cursor = Cursor::new(to);

        let encoded = self
            .encoded
            .get()
            .filter(|encoded| encoded.len() < TRIE_HASH_LEN);

        let encoded_len = encoded.map(Vec::len).unwrap_or(0) as u64;

        let mut attrs = NodeAttributes::empty();
        if let Some(&is_encoded_longer_than_hash_len) = self.is_encoded_longer_than_hash_len.get() {
            attrs.insert(if is_encoded_longer_than_hash_len {
                NodeAttributes::ENCODED_IS_LONG
            } else {
                NodeAttributes::ENCODED_LENGTH_IS_KNOWN
            });
        }

        let encoded = std::array::from_fn({
            let mut encoded = encoded.into_iter().flatten().copied();
            move |_| encoded.next().unwrap_or(0)
        });

        let type_id = NodeTypeId::from(&self.inner);

        let meta = Meta {
            attrs,
            encoded_len,
            encoded,
            type_id,
        };

        cursor.write_all(bytemuck::bytes_of(&meta))?;

        match &self.inner {
            NodeType::Branch(n) => {
                let pos = cursor.position() as usize;

                #[allow(clippy::indexing_slicing)]
                n.serialize(&mut cursor.get_mut()[pos..])
            }

            NodeType::Leaf(n) => {
                let pos = cursor.position() as usize;

                #[allow(clippy::indexing_slicing)]
                n.serialize(&mut cursor.get_mut()[pos..])
            }
        }
    }
}

/// Contains the fields that we include in a node's hash.
/// If this is a leaf node, `children` is empty and `value` is Some.
/// If this is a branch node, `children` is non-empty.
#[derive(Debug, Eq)]
pub struct EncodedNode<T> {
    pub(crate) path: Path,
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
            path,
            children,
            value,
            phantom: _,
        } = self;
        path == &other.path && children == &other.children && value == &other.value
    }
}

/// A path and its length in bits.
#[derive(Debug)]
struct PathWithBitsPrefix(Path);

impl PathWithBitsPrefix {
    fn bit_len(&self) -> u64 {
        self.0.len() as u64 * BITS_PER_NIBBLE
    }
}

impl<'de> Deserialize<'de> for PathWithBitsPrefix {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct PathVisitor;

        impl<'de> Visitor<'de> for PathVisitor {
            type Value = PathWithBitsPrefix;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a bit-length prefixed path of nibbles")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let path_bits_len = seq
                    .size_hint()
                    .map(|x| x as u64)
                    .ok_or(serde::de::Error::custom("missing path length"))?;

                if path_bits_len == 0 {
                    return Ok(PathWithBitsPrefix(Path(vec![])));
                }

                if path_bits_len % BITS_PER_NIBBLE != 0 {
                    return Err(serde::de::Error::custom(
                        "path length is not a multiple of 4",
                    ));
                }

                if path_bits_len > MAX_PATH_BIT_LEN {
                    return Err(serde::de::Error::custom(format!(
                        "path bit length > {MAX_PATH_BIT_LEN}"
                    )));
                }

                // Add BITS_PER_NIBBLE so that if there are an odd number of nibbles,
                // we need to read the final byte.
                // e.g. if `path_bits_len` is 12 we need to read 2 bytes
                // If we didn't add BITS_PER_NIBBLE `path_bytes_len` would be 1.
                // Note `path_bytes_len` >= 1 because we assert `path_bits_len` > 0.
                let path_bytes_len = (path_bits_len + BITS_PER_NIBBLE) / BITS_PER_BYTE;

                let mut path = Vec::with_capacity(NIBBLES_PER_BYTE * path_bytes_len as usize);

                for _ in 0..path_bytes_len {
                    let byte = seq.next_element::<u8>()?.ok_or(serde::de::Error::custom(
                        "not enough bytes for path desierialization",
                    ))?;

                    // Convert the byte to 2 nibbles
                    path.push(byte >> 4);
                    path.push(byte & 0b_0000_1111);
                }

                if path_bits_len % BITS_PER_BYTE != 0 {
                    // The last byte only contained one nibble.
                    let padding_nibble = path.pop().expect("path_bytes_len > 0");
                    if padding_nibble != 0 {
                        return Err(serde::de::Error::custom("path padding nibble is not 0"));
                    }
                }

                Ok(PathWithBitsPrefix(Path(path)))
            }
        }

        deserializer.deserialize_seq(PathVisitor)
    }
}

impl Serialize for PathWithBitsPrefix {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let path_length_bits = self.bit_len();

        let mut serializer = serializer.serialize_seq(Some(path_length_bits as usize))?;

        let path_bytes = nibbles_to_bytes_iter(self.0.as_ref());

        for byt in path_bytes {
            serializer.serialize_element(&byt)?;
        }

        serializer.end()
    }
}

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
            .filter_map(|(i, c)| c.as_ref().map(|c| (i as u64, c.to_vec())))
            .collect();

        let value = self.value.as_deref();

        let path = PathWithBitsPrefix(self.path.clone());

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
        let path: PathWithBitsPrefix;

        (chd, value, path) = Deserialize::deserialize(deserializer)?;

        let mut children: [Option<Vec<u8>>; BranchNode::MAX_CHILDREN] = Default::default();
        #[allow(clippy::indexing_slicing)]
        for (i, chd) in chd {
            children[i as usize] = Some(chd);
        }

        Ok(Self {
            path: path.0,
            children,
            value,
            phantom: PhantomData,
        })
    }
}

// Note that the serializer passed in should always be the same type as T in EncodedNode<T>.
// TODO danlaine: Consider implementing our own serializer for hashing nodes so that we don't
// need to make assumptions about how values are serialized by the [serde::Serializer].
// Namely, we should implement it specifically so it encodes nodes the same way as merkledb.
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
            list[i] = child.to_vec();
        }

        if let Some(val) = &self.value {
            list[BranchNode::MAX_CHILDREN].clone_from(val);
        }

        let serialized_path = nibbles_to_bytes_iter(&self.path.encode()).collect();
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
                    path,
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
                    path,
                    children,
                    value,
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::shale::cached::InMemLinearStore;
    use bincode::Error;
    use std::iter::repeat;
    use test_case::{test_case, test_matrix};

    #[test_matrix(
        [Nil, vec![], vec![0x01], (0..TRIE_HASH_LEN as u8).collect::<Vec<_>>(), (0..33).collect::<Vec<_>>()],
        [Nil, false, true]
    )]
    fn cached_node_data(
        encoded: impl Into<Option<Vec<u8>>>,
        is_encoded_longer_than_hash_len: impl Into<Option<bool>>,
    ) {
        let leaf = NodeType::Leaf(LeafNode::new(Path(vec![1, 2, 3]), vec![4, 5]));
        let branch = NodeType::Branch(Box::new(BranchNode {
            partial_path: vec![].into(),
            children: [Some(DiskAddress::from(1)); BranchNode::MAX_CHILDREN],
            value: Some(vec![1, 2, 3]),
            children_encoded: std::array::from_fn(|_| Some(vec![1])),
        }));

        let encoded = encoded.into();
        let is_encoded_longer_than_hash_len = is_encoded_longer_than_hash_len.into();

        let node = Node::new_from_hash(encoded.clone(), is_encoded_longer_than_hash_len, leaf);

        check_node_encoding(node);

        let node = Node::new_from_hash(encoded.clone(), is_encoded_longer_than_hash_len, branch);

        check_node_encoding(node);
    }

    #[test_matrix(
        (0..0, 0..15, 0..16, 0..31, 0..32),
        [0..0, 0..16, 0..32]
    )]
    fn leaf_node<Iter: Iterator<Item = u8>>(path: Iter, value: Iter) {
        let node = Node::from_leaf(LeafNode::new(
            Path(path.map(|x| x & 0xf).collect()),
            value.collect::<Vec<u8>>(),
        ));

        check_node_encoding(node);
    }

    #[test_case(&[])]
    #[test_case(&[0x00])]
    #[test_case(&[0x0F])]
    #[test_case(&[0x00, 0x00])]
    #[test_case(&[0x01, 0x02])]
    #[test_case(&[0x00,0x0F])]
    #[test_case(&[0x0F,0x0F])]
    #[test_case(&[0x0F,0x01,0x0F])]
    fn encoded_branch_node_bincode_serialize(path_nibbles: &[u8]) -> Result<(), Error> {
        let node = EncodedNode::<Bincode> {
            path: Path(path_nibbles.to_vec()),
            children: Default::default(),
            value: Some(vec![1, 2, 3, 4]),
            phantom: PhantomData,
        };

        let node_bytes = Bincode::serialize(&node)?;

        let deserialized_node: EncodedNode<Bincode> = Bincode::deserialize(&node_bytes)?;

        assert_eq!(&node, &deserialized_node);

        Ok(())
    }

    #[test_matrix(
        [&[], &[0xf], &[0xf, 0xf]],
        [vec![], vec![1,0,0,0,0,0,0,1], vec![1,0,0,0,0,0,0,0,0,0,0,0,0,0,0,1], repeat(1).take(16).collect()],
        [Nil, 0, 15],
        [
            std::array::from_fn(|_| None),
            std::array::from_fn(|_| Some(vec![1])),
            [Some(vec![1]), None, None, None, None, None, None, None, None, None, None, None, None, None, None, Some(vec![1])],
            std::array::from_fn(|_| Some(vec![1; 32])),
            std::array::from_fn(|_| Some(vec![1; 33]))
        ]
    )]
    fn branch_encoding(
        path: &[u8],
        children: Vec<usize>,
        value: impl Into<Option<u8>>,
        children_encoded: [Option<Vec<u8>>; BranchNode::MAX_CHILDREN],
    ) {
        let partial_path = Path(path.iter().copied().map(|x| x & 0xf).collect());

        let mut children = children.into_iter().map(|x| {
            if x == 0 {
                None
            } else {
                Some(DiskAddress::from(x))
            }
        });

        let children = std::array::from_fn(|_| children.next().flatten());

        let value = value
            .into()
            .map(|x| std::iter::repeat(x).take(x as usize).collect());

        let node = Node::from_branch(BranchNode {
            partial_path,
            children,
            value,
            children_encoded,
        });

        check_node_encoding(node);
    }

    fn check_node_encoding(node: Node) {
        let serialized_len = node.serialized_len();

        let mut bytes = vec![0; serialized_len as usize];
        node.serialize(&mut bytes).expect("node should serialize");

        let mut mem = InMemLinearStore::new(serialized_len, 0);
        mem.write(0, &bytes).expect("write should succed");

        let mut hydrated_node = Node::deserialize(0, &mem).expect("node should deserialize");

        let encoded = node
            .encoded
            .get()
            .filter(|encoded| encoded.len() >= TRIE_HASH_LEN);

        match encoded {
            // long-encoded won't be serialized
            Some(encoded) if hydrated_node.encoded.get().is_none() => {
                hydrated_node.encoded = OnceLock::from(encoded.clone());
            }
            _ => (),
        }

        assert_eq!(node, hydrated_node);
    }

    #[test_case(&[],&[0])]
    #[test_case(&[0],&[4,0])]
    #[test_case(&[0x01],&[4,0x10])]
    #[test_case(&[0x0F],&[4,0xF0])]
    #[test_case(&[0x00, 0x00],&[8,0x00])]
    #[test_case(&[0x01, 0x02],&[8,0x12])]
    #[test_case(&[0x00,0x0F],&[8,0x0F])]
    #[test_case(&[0x0F,0x0F],&[8,0xFF])]
    #[test_case(&[0x0F,0x01,0x0F],&[12,0xF1,0xF0])]
    fn test_encode_path_with_bits_prefix(path_nibbles: &[u8], expected_bytes: &[u8]) {
        let path = PathWithBitsPrefix(Path(path_nibbles.to_vec()));

        let serialized_path = PlainCodec::serialize(&path).unwrap();
        assert_eq!(serialized_path, expected_bytes);

        let deserialized_path =
            PlainCodec::deserialize::<PathWithBitsPrefix>(&serialized_path).unwrap();
        assert_eq!(deserialized_path.0, path.0);
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
