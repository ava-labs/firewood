// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::{LeafNode, LinearAddress, Node, Path};
use std::fmt::{Debug, Formatter};

/// The type of a hash. For ethereum compatible hashes, this might be a RLP encoded
/// value if it's small enough to fit in less than 32 bytes. For merkledb compatible
/// hashes, it's always a TrieHash.
#[cfg(feature = "ethhash")]
pub type HashType = ethhash::HashOrRlp;

#[cfg(not(feature = "ethhash"))]
/// The type of a hash. For non-ethereum compatible hashes, this is always a TrieHash.
pub type HashType = crate::TrieHash;

pub(crate) trait Serializable {
    fn serialized_bytes(&self) -> Vec<u8>;
    fn from_reader<R: std::io::Read>(reader: R) -> Result<Self, std::io::Error>
    where
        Self: Sized;
}

#[derive(PartialEq, Eq, Clone, Debug)]
#[repr(C)]
/// A child of a branch node.
pub enum Child {
    /// There is a child at this index, but we haven't hashed it
    /// or allocated space in storage for it yet.
    Node(Node),
    /// We know the child's address and hash.
    AddressWithHash(LinearAddress, HashType),
}

#[cfg(feature = "ethhash")]
mod ethhash {
    use serde::{Deserialize, Serialize};
    use sha2::Digest as _;
    use sha3::Keccak256;
    use smallvec::SmallVec;
    use std::{
        fmt::{Display, Formatter},
        io::Read,
        ops::Deref as _,
    };

    use crate::TrieHash;

    use super::Serializable;

    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub enum HashOrRlp {
        Hash(TrieHash),
        // TODO: this slice is never larger than 32 bytes so smallvec is probably not our best container
        // the length is stored in a `usize` but it could be in a `u8` and it will never overflow
        Rlp(SmallVec<[u8; 32]>),
    }

    impl HashOrRlp {
        pub fn as_slice(&self) -> &[u8] {
            self.deref()
        }

        pub(crate) fn into_triehash(self) -> TrieHash {
            self.into()
        }
    }

    impl Serializable for HashOrRlp {
        fn serialized_bytes(&self) -> Vec<u8> {
            match self {
                HashOrRlp::Hash(h) => std::iter::once(0)
                    .chain(h.as_ref().iter().copied())
                    .collect(),
                HashOrRlp::Rlp(r) => {
                    debug_assert!(!r.is_empty());
                    debug_assert!(r.len() < 32);
                    std::iter::once(r.len() as u8)
                        .chain(r.iter().copied())
                        .collect()
                }
            }
        }

        fn from_reader<R: Read>(mut reader: R) -> Result<Self, std::io::Error> {
            let mut bytes = [0; 32];
            reader.read_exact(&mut bytes[0..1])?;
            match bytes[0] {
                0 => {
                    reader.read_exact(&mut bytes)?;
                    Ok(HashOrRlp::Hash(TrieHash::from(bytes)))
                }
                len if len < 32 => {
                    reader.read_exact(&mut bytes[0..len as usize])?;
                    Ok(HashOrRlp::Rlp(SmallVec::from_buf_and_len(
                        bytes,
                        len as usize,
                    )))
                }
                _ => Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid RLP length",
                )),
            }
        }
    }

    impl From<HashOrRlp> for TrieHash {
        fn from(val: HashOrRlp) -> Self {
            match val {
                HashOrRlp::Hash(h) => h,
                HashOrRlp::Rlp(r) => Keccak256::digest(&r).into(),
            }
        }
    }

    impl From<TrieHash> for HashOrRlp {
        fn from(val: TrieHash) -> Self {
            HashOrRlp::Hash(val)
        }
    }

    impl From<[u8; 32]> for HashOrRlp {
        fn from(value: [u8; 32]) -> Self {
            HashOrRlp::Hash(TrieHash::into(value.into()))
        }
    }

    impl AsRef<[u8]> for HashOrRlp {
        fn as_ref(&self) -> &[u8] {
            match self {
                HashOrRlp::Hash(h) => h.as_ref(),
                HashOrRlp::Rlp(r) => r.as_ref(),
            }
        }
    }

    impl std::ops::Deref for HashOrRlp {
        type Target = [u8];
        fn deref(&self) -> &Self::Target {
            match self {
                HashOrRlp::Hash(h) => h,
                HashOrRlp::Rlp(r) => r.deref(),
            }
        }
    }

    impl Display for HashOrRlp {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            match self {
                HashOrRlp::Hash(h) => write!(f, "{}", h),
                HashOrRlp::Rlp(r) => {
                    let width = f.precision().unwrap_or(32);
                    write!(f, "{:.*}", width, hex::encode(r))
                }
            }
        }
    }

    impl Default for HashOrRlp {
        fn default() -> Self {
            HashOrRlp::Hash(TrieHash::default())
        }
    }
}

#[derive(PartialEq, Eq, Clone)]
/// A branch node
pub struct BranchNode {
    /// The partial path for this branch
    pub partial_path: Path,

    /// The value of the data for this branch, if any
    pub value: Option<Box<[u8]>>,

    /// The children of this branch.
    /// Element i is the child at index i, or None if there is no child at that index.
    /// Each element is (child_hash, child_address).
    /// child_address is None if we don't know the child's hash.
    pub children: [Option<Child>; Self::MAX_CHILDREN],
}

impl Serialize for BranchNode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("BranchNode", 3)?;
        state.serialize_field("partial_path", &self.partial_path)?;
        state.serialize_field("value", &self.value)?;

        let children: SmallVec<[(_, _, _); Self::MAX_CHILDREN]> = self
            .children
            .iter()
            .enumerate()
            .filter_map(|(offset, child)| match child {
                None => None,
                Some(Child::Node(_)) => {
                    panic!("serializing in-memory node for disk storage")
                }
                Some(Child::AddressWithHash(addr, hash)) => Some((offset as u8, *addr, hash)),
            })
            .collect();

        state.serialize_field("children", &children)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for BranchNode {
    fn deserialize<D>(deserializer: D) -> Result<BranchNode, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct SerializedBranchNode {
            partial_path: Path,
            value: Option<Box<[u8]>>,
            children: SmallVec<[(u8, LinearAddress, HashType); BranchNode::MAX_CHILDREN]>,
        }

        let s: SerializedBranchNode = Deserialize::deserialize(deserializer)?;

        let mut children: [Option<Child>; BranchNode::MAX_CHILDREN] =
            [const { None }; BranchNode::MAX_CHILDREN];
        for (offset, addr, hash) in s.children.iter() {
            children[*offset as usize] = Some(Child::AddressWithHash(*addr, hash.clone()));
        }

        Ok(BranchNode {
            partial_path: s.partial_path,
            value: s.value,
            children,
        })
    }
}

impl Debug for BranchNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "[BranchNode")?;
        write!(f, r#" path="{:?}""#, self.partial_path)?;

        for (i, c) in self.children.iter().enumerate() {
            match c {
                None => {}
                Some(Child::Node(_)) => {} //TODO
                Some(Child::AddressWithHash(addr, hash)) => {
                    write!(f, "({i:?}: address={addr:?} hash={})", hash)?
                }
            }
        }

        write!(
            f,
            " v={}]",
            match &self.value {
                Some(v) => hex::encode(&**v),
                None => "nil".to_string(),
            }
        )
    }
}

impl BranchNode {
    /// The maximum number of children in a [BranchNode]
    #[cfg(feature = "branch_factor_256")]
    pub const MAX_CHILDREN: usize = 256;

    /// The maximum number of children in a [BranchNode]
    #[cfg(not(feature = "branch_factor_256"))]
    pub const MAX_CHILDREN: usize = 16;

    /// Returns the address of the child at the given index.
    /// Panics if `child_index` >= [BranchNode::MAX_CHILDREN].
    pub fn child(&self, child_index: u8) -> &Option<Child> {
        self.children
            .get(child_index as usize)
            .expect("child_index is in bounds")
    }

    /// Update the child at `child_index` to be `new_child_addr`.
    /// If `new_child_addr` is None, the child is removed.
    pub fn update_child(&mut self, child_index: u8, new_child: Option<Child>) {
        let child = self
            .children
            .get_mut(child_index as usize)
            .expect("child_index is in bounds");

        *child = new_child;
    }

    /// Returns (index, hash) for each child that has a hash set.
    pub fn children_iter(&self) -> impl Iterator<Item = (usize, &HashType)> + Clone {
        self.children
            .iter()
            .enumerate()
            .filter_map(|(i, child)| match child {
                None => None,
                Some(Child::Node(_)) => unreachable!("TODO make unreachable"),
                Some(Child::AddressWithHash(_, hash)) => Some((i, hash)),
            })
    }
}

impl From<&LeafNode> for BranchNode {
    fn from(leaf: &LeafNode) -> Self {
        BranchNode {
            partial_path: leaf.partial_path.clone(),
            value: Some(Box::from(&leaf.value[..])),
            children: [const { None }; BranchNode::MAX_CHILDREN],
        }
    }
}
