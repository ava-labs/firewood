// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::shale::{disk_address::DiskAddress, CachedStore, ShaleError, ShaleStore, Storable};
use bincode::{Error, Options};
use bitflags::bitflags;
use enum_as_inner::EnumAsInner;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::{
    fmt::Debug,
    io::{Cursor, Write},
    mem::size_of,
    sync::{
        atomic::{AtomicBool, Ordering},
        OnceLock,
    },
};

mod branch;
mod extension;
mod leaf;
mod partial_path;

pub use branch::BranchNode;
pub use extension::ExtNode;
pub use leaf::{LeafNode, SIZE as LEAF_NODE_SIZE};
pub use partial_path::PartialPath;

use crate::nibbles::Nibbles;

use super::{TrieHash, TRIE_HASH_LEN};

bitflags! {
    // should only ever be the size of a nibble
    struct Flags: u8 {
        const TERMINAL = 0b0010;
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

#[derive(Serialize, Deserialize, Debug)]
enum Encoded<T> {
    Raw(T),
    Data(T),
}

impl Default for Encoded<Vec<u8>> {
    fn default() -> Self {
        // This is the default serialized empty vector
        Encoded::Data(vec![0])
    }
}

impl<T: DeserializeOwned + AsRef<[u8]>> Encoded<T> {
    pub fn decode(self) -> Result<T, bincode::Error> {
        match self {
            Encoded::Raw(raw) => Ok(raw),
            Encoded::Data(data) => bincode::DefaultOptions::new().deserialize(data.as_ref()),
        }
    }
}

#[derive(PartialEq, Eq, Clone, Debug, EnumAsInner)]
pub enum NodeType {
    Branch(Box<BranchNode>),
    Leaf(LeafNode),
    Extension(ExtNode),
}

impl NodeType {
    pub fn decode(buf: &[u8]) -> Result<NodeType, Error> {
        let items: Vec<Encoded<Vec<u8>>> = bincode::DefaultOptions::new().deserialize(buf)?;

        match items.len() {
            LEAF_NODE_SIZE => {
                let mut items = items.into_iter();

                let decoded_key: Vec<u8> = items.next().unwrap().decode()?;

                let decoded_key_nibbles = Nibbles::<0>::new(&decoded_key);

                let (cur_key_path, term) =
                    PartialPath::from_nibbles(decoded_key_nibbles.into_iter());

                let cur_key = cur_key_path.into_inner();
                let data: Vec<u8> = items.next().unwrap().decode()?;

                if term {
                    Ok(NodeType::Leaf(LeafNode::new(cur_key, data)))
                } else {
                    Ok(NodeType::Extension(ExtNode {
                        path: PartialPath(cur_key),
                        child: DiskAddress::null(),
                        child_encoded: Some(data),
                    }))
                }
            }
            // TODO: add path
            BranchNode::MSIZE => Ok(NodeType::Branch(BranchNode::decode(buf)?.into())),
            size => Err(Box::new(bincode::ErrorKind::Custom(format!(
                "invalid size: {size}"
            )))),
        }
    }

    pub fn encode<S: ShaleStore<Node>>(&self, store: &S) -> Vec<u8> {
        match &self {
            NodeType::Leaf(n) => n.encode(),
            NodeType::Extension(n) => n.encode(store),
            NodeType::Branch(n) => n.encode(store),
        }
    }

    pub fn path_mut(&mut self) -> &mut PartialPath {
        match self {
            NodeType::Branch(u) => &mut u.path,
            NodeType::Leaf(node) => &mut node.path,
            NodeType::Extension(node) => &mut node.path,
        }
    }
}

#[derive(Debug)]
pub struct Node {
    pub(super) root_hash: OnceLock<TrieHash>,
    is_encoded_longer_than_hash_len: OnceLock<bool>,
    encoded: OnceLock<Vec<u8>>,
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
            root_hash,
            is_encoded_longer_than_hash_len,
            encoded,
            lazy_dirty: _,
            inner,
        } = self;
        *root_hash == other.root_hash
            && *is_encoded_longer_than_hash_len == other.is_encoded_longer_than_hash_len
            && *encoded == other.encoded
            && is_dirty == other.is_dirty()
            && *inner == other.inner
    }
}

impl Clone for Node {
    fn clone(&self) -> Self {
        Self {
            root_hash: self.root_hash.clone(),
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
            root_hash: OnceLock::new(),
            is_encoded_longer_than_hash_len: OnceLock::new(),
            encoded: OnceLock::new(),
            inner,
            lazy_dirty: AtomicBool::new(false),
        };
        s.rehash();
        s
    }
}

bitflags! {
    struct NodeAttributes: u8 {
        const ROOT_HASH_VALID      = 0b001;
        const IS_ENCODED_BIG_VALID = 0b010;
        const LONG                 = 0b100;
    }
}

impl Node {
    pub(super) fn max_branch_node_size() -> u64 {
        let max_size: OnceLock<u64> = OnceLock::new();
        *max_size.get_or_init(|| {
            Self {
                root_hash: OnceLock::new(),
                is_encoded_longer_than_hash_len: OnceLock::new(),
                encoded: OnceLock::new(),
                inner: NodeType::Branch(
                    BranchNode {
                        path: vec![].into(),
                        children: [Some(DiskAddress::null()); BranchNode::MAX_CHILDREN],
                        value: Some(Data(Vec::new())),
                        children_encoded: Default::default(),
                    }
                    .into(),
                ),
                lazy_dirty: AtomicBool::new(false),
            }
            .serialized_len()
        })
    }

    pub(super) fn get_encoded<S: ShaleStore<Node>>(&self, store: &S) -> &[u8] {
        self.encoded.get_or_init(|| self.inner.encode::<S>(store))
    }

    pub(super) fn get_root_hash<S: ShaleStore<Node>>(&self, store: &S) -> &TrieHash {
        self.root_hash.get_or_init(|| {
            self.set_dirty(true);
            TrieHash(Keccak256::digest(self.get_encoded::<S>(store)).into())
        })
    }

    fn is_encoded_longer_than_hash_len<S: ShaleStore<Node>>(&self, store: &S) -> bool {
        *self.is_encoded_longer_than_hash_len.get_or_init(|| {
            self.set_dirty(true);
            self.get_encoded(store).len() >= TRIE_HASH_LEN
        })
    }

    pub(super) fn rehash(&mut self) {
        self.encoded = OnceLock::new();
        self.is_encoded_longer_than_hash_len = OnceLock::new();
        self.root_hash = OnceLock::new();
    }

    pub fn from_branch<B: Into<Box<BranchNode>>>(node: B) -> Self {
        Self::from(NodeType::Branch(node.into()))
    }

    pub fn from_leaf(path: PartialPath, data: Data) -> Self {
        Self::from(NodeType::Leaf(LeafNode { path, data }))
    }

    pub fn inner(&self) -> &NodeType {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut NodeType {
        &mut self.inner
    }

    pub(super) fn new_from_hash(
        root_hash: Option<TrieHash>,
        encoded_longer_than_hash_len: Option<bool>,
        inner: NodeType,
    ) -> Self {
        Self {
            root_hash: match root_hash {
                Some(h) => OnceLock::from(h),
                None => OnceLock::new(),
            },
            is_encoded_longer_than_hash_len: match encoded_longer_than_hash_len {
                Some(b) => OnceLock::from(b),
                None => OnceLock::new(),
            },
            encoded: OnceLock::new(),
            inner,
            lazy_dirty: AtomicBool::new(false),
        }
    }

    pub(super) fn is_dirty(&self) -> bool {
        self.lazy_dirty.load(Ordering::Relaxed)
    }

    pub(super) fn set_dirty(&self, is_dirty: bool) {
        self.lazy_dirty.store(is_dirty, Ordering::Relaxed)
    }
}

#[repr(C)]
struct Meta {
    root_hash: [u8; TRIE_HASH_LEN],
    attrs: NodeAttributes,
    is_encoded_longer_than_hash_len: Option<bool>,
}

impl Meta {
    const SIZE: usize = size_of::<Self>();
}

mod type_id {
    use crate::shale::ShaleError;

    const BRANCH: u8 = 0;
    const LEAF: u8 = 1;
    const EXTENSION: u8 = 2;

    #[repr(u8)]
    pub enum NodeTypeId {
        Branch = BRANCH,
        Leaf = LEAF,
        Extension = EXTENSION,
    }

    impl TryFrom<u8> for NodeTypeId {
        type Error = ShaleError;

        fn try_from(value: u8) -> Result<Self, Self::Error> {
            match value {
                BRANCH => Ok(Self::Branch),
                LEAF => Ok(Self::Leaf),
                EXTENSION => Ok(Self::Extension),
                _ => Err(ShaleError::InvalidNodeType),
            }
        }
    }
}

use type_id::NodeTypeId;

impl Storable for Node {
    fn deserialize<T: CachedStore>(mut offset: usize, mem: &T) -> Result<Self, ShaleError> {
        let meta_raw =
            mem.get_view(offset, Meta::SIZE as u64)
                .ok_or(ShaleError::InvalidCacheView {
                    offset,
                    size: Meta::SIZE as u64,
                })?;

        offset += Meta::SIZE;

        let attrs = NodeAttributes::from_bits_retain(meta_raw.as_deref()[TRIE_HASH_LEN]);

        let root_hash = if attrs.contains(NodeAttributes::ROOT_HASH_VALID) {
            Some(TrieHash(
                meta_raw.as_deref()[..TRIE_HASH_LEN]
                    .try_into()
                    .expect("invalid slice"),
            ))
        } else {
            None
        };

        let is_encoded_longer_than_hash_len =
            if attrs.contains(NodeAttributes::IS_ENCODED_BIG_VALID) {
                Some(attrs.contains(NodeAttributes::LONG))
            } else {
                None
            };

        match meta_raw.as_deref()[TRIE_HASH_LEN + 1].try_into()? {
            NodeTypeId::Branch => {
                let inner = NodeType::Branch(Box::new(BranchNode::deserialize(offset, mem)?));

                Ok(Self::new_from_hash(
                    root_hash,
                    is_encoded_longer_than_hash_len,
                    inner,
                ))
            }

            NodeTypeId::Extension => {
                let inner = NodeType::Extension(ExtNode::deserialize(offset, mem)?);
                let node = Self::new_from_hash(root_hash, is_encoded_longer_than_hash_len, inner);

                Ok(node)
            }

            NodeTypeId::Leaf => {
                let inner = NodeType::Leaf(LeafNode::deserialize(offset, mem)?);
                let node = Self::new_from_hash(root_hash, is_encoded_longer_than_hash_len, inner);

                Ok(node)
            }
        }
    }

    fn serialized_len(&self) -> u64 {
        Meta::SIZE as u64
            + match &self.inner {
                NodeType::Branch(n) => {
                    // TODO: add path
                    n.serialized_len()
                }
                NodeType::Extension(n) => n.serialized_len(),
                NodeType::Leaf(n) => n.serialized_len(),
            }
    }

    fn serialize(&self, to: &mut [u8]) -> Result<(), ShaleError> {
        let mut cur = Cursor::new(to);

        let mut attrs = match self.root_hash.get() {
            Some(h) => {
                cur.write_all(&h.0)?;
                NodeAttributes::ROOT_HASH_VALID
            }
            None => {
                cur.write_all(&[0; 32])?;
                NodeAttributes::empty()
            }
        };

        if let Some(&b) = self.is_encoded_longer_than_hash_len.get() {
            attrs.insert(if b {
                NodeAttributes::LONG
            } else {
                NodeAttributes::IS_ENCODED_BIG_VALID
            });
        }

        cur.write_all(&[attrs.bits()]).unwrap();

        match &self.inner {
            NodeType::Branch(n) => {
                // TODO: add path
                cur.write_all(&[type_id::NodeTypeId::Branch as u8])?;

                let pos = cur.position() as usize;

                n.serialize(&mut cur.get_mut()[pos..])
            }

            NodeType::Extension(n) => {
                cur.write_all(&[type_id::NodeTypeId::Extension as u8])?;

                let pos = cur.position() as usize;

                n.serialize(&mut cur.get_mut()[pos..])
            }

            NodeType::Leaf(n) => {
                cur.write_all(&[type_id::NodeTypeId::Leaf as u8])?;

                let pos = cur.position() as usize;

                n.serialize(&mut cur.get_mut()[pos..])
            }
        }
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::array::from_fn;

    use super::*;
    use crate::shale::cached::PlainMem;
    use test_case::test_case;

    pub fn leaf(path: Vec<u8>, data: Vec<u8>) -> Node {
        Node::from_leaf(PartialPath(path), Data(data))
    }

    pub fn branch<const N: usize, const M: usize, T, U>(
        path: &[u8],
        repeated_disk_address: usize,
        value: T,
        repeated_encoded_child: U,
    ) -> Node
    where
        T: Into<Option<&'static [u8; N]>>,
        U: Into<Option<&'static [u8; M]>>,
    {
        let value = value.into().map(|val| val.to_vec()).map(Data);
        let repeated_encoded_child = repeated_encoded_child.into().map(|val| val.to_vec());

        let children: [Option<DiskAddress>; BranchNode::MAX_CHILDREN] = from_fn(|i| {
            if i < BranchNode::MAX_CHILDREN / 2 {
                DiskAddress::from(repeated_disk_address).into()
            } else {
                None
            }
        });

        let children_encoded = repeated_encoded_child
            .map(|child| {
                from_fn(|i| {
                    if i < BranchNode::MAX_CHILDREN / 2 {
                        child.clone().into()
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default();

        Node::from_branch(BranchNode {
            path: PartialPath(path.to_vec()),
            children,
            value,
            children_encoded,
        })
    }

    pub fn extension(
        path: Vec<u8>,
        child_address: DiskAddress,
        child_encoded: Option<Vec<u8>>,
    ) -> Node {
        Node::from(NodeType::Extension(ExtNode {
            path: PartialPath(path),
            child: child_address,
            child_encoded,
        }))
    }

    struct Nil;

    impl From<Nil> for Option<&'static [u8; 0]> {
        fn from(_val: Nil) -> Self {
            None
        }
    }

    #[test_case(leaf(vec![0x01, 0x02, 0x03], vec![0x04, 0x05]); "leaf_node")]
    #[test_case(extension(vec![0x01, 0x02, 0x03], DiskAddress::from(0x42), None); "extension with child address")]
    #[test_case(extension(vec![0x01, 0x02, 0x03], DiskAddress::null(), vec![0x01, 0x02, 0x03].into()) ; "extension without child address")]
    #[test_case(branch(&[], 0x0a, Nil, Nil); "branch with nothing")]
    #[test_case(branch(&[1], 0x0a, Nil, Nil); "branch with path")]
    #[test_case(branch(&[], 0x0a, b"value", Nil); "branch with data ")]
    #[test_case(branch(&[], 0x0a, Nil, &[1, 2, 3]); "branch with children")]
    #[test_case(branch(&[1], 0x0a, b"value", Nil); "branch with path and data")]
    #[test_case(branch(&[1], 0x0a, Nil, &[1, 2, 3]); "branch with path and children")]
    #[test_case(branch(&[], 0x0a, b"value", &[1, 2, 3]); "branch with data and children")]
    #[test_case(branch(&[1], 0x0a, b"value", &[1, 2, 3]); "branch with path data and children")]
    fn encoding(node: Node) {
        let mut bytes = vec![0; node.serialized_len() as usize];

        node.serialize(&mut bytes).unwrap();

        let mut mem = PlainMem::new(node.serialized_len(), 0x00);
        mem.write(0, &bytes);

        let hydrated_node = Node::deserialize(0, &mem).unwrap();

        assert_eq!(node, hydrated_node);
    }
}
