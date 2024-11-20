// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use enum_as_inner::EnumAsInner;
use integer_encoding::VarIntWriter as _;
use serde::{Deserialize, Serialize};
use smallvec::{smallvec, SmallVec};
use std::io::Read;
use std::{fmt::Debug, sync::Arc};

mod branch;
mod leaf;
pub mod path;

pub use branch::BranchNode;
pub use branch::Child;
pub use leaf::LeafNode;

use crate::Path;

/// A node, either a Branch or Leaf

// TODO: explain why Branch is boxed but Leaf is not
#[derive(PartialEq, Eq, Clone, Debug, EnumAsInner, Serialize, Deserialize)]
pub enum Node {
    /// This node is a [BranchNode]
    Branch(Box<BranchNode>),
    /// This node is a [LeafNode]
    Leaf(LeafNode),
}

impl Default for Node {
    fn default() -> Self {
        Node::Leaf(LeafNode {
            partial_path: Path::new(),
            value: SmallVec::default(),
        })
    }
}

impl From<BranchNode> for Node {
    fn from(branch: BranchNode) -> Self {
        Node::Branch(Box::new(branch))
    }
}

impl From<LeafNode> for Node {
    fn from(leaf: LeafNode) -> Self {
        Node::Leaf(leaf)
    }
}

impl Node {
    /// Returns the partial path of the node.
    pub fn partial_path(&self) -> &Path {
        match self {
            Node::Branch(b) => &b.partial_path,
            Node::Leaf(l) => &l.partial_path,
        }
    }

    /// Updates the partial path of the node to `partial_path`.
    pub fn update_partial_path(&mut self, partial_path: Path) {
        match self {
            Node::Branch(b) => b.partial_path = partial_path,
            Node::Leaf(l) => l.partial_path = partial_path,
        }
    }

    /// Updates the value of the node to `value`.
    pub fn update_value(&mut self, value: Box<[u8]>) {
        match self {
            Node::Branch(b) => b.value = Some(value),
            Node::Leaf(l) => l.value = SmallVec::from(&value[..]),
        }
    }

    /// Returns a new `Arc<Node>` which is the same as `self` but with the given `partial_path`.
    pub fn new_with_partial_path(self: &Node, partial_path: Path) -> Node {
        match self {
            Node::Branch(b) => Node::Branch(Box::new(BranchNode {
                partial_path,
                value: b.value.clone(),
                children: b.children.clone(),
            })),
            Node::Leaf(l) => Node::Leaf(LeafNode {
                partial_path,
                value: l.value.clone(),
            }),
        }
    }

    /// Returns Some(value) inside the node, or None if the node is a branch
    /// with no value.
    pub fn value(&self) -> Option<&[u8]> {
        match self {
            Node::Branch(b) => b.value.as_deref(),
            Node::Leaf(l) => Some(&l.value),
        }
    }

    /// Given a [Node], returns a set of bytes to write to storage
    /// The format is as follows:
    ///
    /// For a branch:
    ///  - Byte 0:
    ///   - Bit 0: always 0
    ///   - Bit 1: indicates if the branch has a value
    ///   - Bits 2-5: the number of children (unless branch_factor_256, which stores it in the next byte)
    ///   - Bits 6-7: 0: empty partial_path, 1: 1 nibble, 2: 2 nibbles, 3: length is encoded in the next byte
    ///     (for branch_factor_256, bits 2-7 are used for partial_path length, up to 63 nibbles)
    ///
    /// The remaining bytes are in the following order:
    ///   - The partial path, possibly preceeded by the length if it is longer than 3 nibbles (varint encoded)
    ///   - The number of children, if the branch factor is 256
    ///   - The children. If the number of children == [Self::MAX_CHILDREN], then the children are just
    ///     addresses with hashes. Otherwise, they are offset, address, hash tuples.
    ///
    /// For a leaf:
    ///  - Byte 0:
    ///    - Bit 0: always 1
    ///    - Bits 1-7: the length of the partial path. If the partial path is longer than 127 nibbles, this is set to
    ///      127 and the length is encoded in the next byte.
    ///
    /// The remaining bytes are in the following order:
    ///    - The partial path, possibly preceeded by the length if it is longer than 127 nibbles (varint encoded)
    ///    - The value, always preceeded by the length, varint encoded
    pub fn serialize_for_storage(&self) -> Box<[u8]> {
        match self {
            #[cfg(not(feature = "branch_factor_256"))]
            Node::Branch(b) => {
                let child_iter = b
                    .children
                    .iter()
                    .enumerate()
                    .filter_map(|(offset, child)| child.as_ref().map(|c| (offset, c)));
                let childcount = child_iter.clone().count();

                // encode the first byte
                let first_byte = (b.value.is_some() as u8) << 1
                    | (childcount as u8) << 2
                    | if b.partial_path.0.len() < 3 {
                        (b.partial_path.0.len() as u8) << 6
                    } else {
                        3 << 6
                    };

                // create an output stack item, which can overflow to memory for very large branch nodes
                const OPTIMIZE_BRANCHES_FOR_SIZE: usize = 1024;
                let mut encoded: SmallVec<[_; OPTIMIZE_BRANCHES_FOR_SIZE]> = smallvec![first_byte];

                // encode the partial path, including the length if it didn't fit above
                if b.partial_path.0.len() >= 3 {
                    encoded
                        .write_varint(b.partial_path.len())
                        .expect("writing to vec should succeed");
                }
                encoded.extend_from_slice(&b.partial_path.0);

                // encode the value. For tries that have the same length keys, this is always empty
                if let Some(v) = &b.value {
                    encoded
                        .write_varint(v.len())
                        .expect("writing to vec should succeed");
                    encoded.extend_from_slice(v);
                }

                // encode the children
                if childcount == BranchNode::MAX_CHILDREN {
                    for (_, child) in child_iter {
                        if let Child::AddressWithHash(address, hash) = child {
                            encoded.extend_from_slice(&address.get().to_ne_bytes());
                            encoded.extend_from_slice(hash);
                        } else {
                            panic!("attempt to serialize to persist a branch with a child that is not an AddressWithHash");
                        }
                    }
                } else {
                    for (position, child) in child_iter {
                        encoded
                            .write_varint(position)
                            .expect("writing to vec should succeed");
                        if let Child::AddressWithHash(address, hash) = child {
                            encoded.extend_from_slice(&address.get().to_ne_bytes());
                            encoded.extend_from_slice(hash);
                        } else {
                            panic!("attempt to serialize to persist a branch with a child that is not an AddressWithHash");
                        }
                    }
                }
                encoded.into_boxed_slice()
            }
            Node::Leaf(l) => {
                let first_byte = if l.partial_path.0.len() < 127 {
                    ((l.partial_path.0.len() as u8) << 1) | 1
                } else {
                    255
                };

                const OPTIMIZE_LEAVES_FOR_SIZE: usize = 128;
                let mut encoded: SmallVec<[_; OPTIMIZE_LEAVES_FOR_SIZE]> = smallvec![first_byte];

                // encode the partial path, including the length if it didn't fit above
                if l.partial_path.0.len() >= 127 {
                    encoded
                        .write_varint(l.partial_path.len())
                        .expect("write to array should succeed");
                }
                encoded.extend_from_slice(l.partial_path.0.as_ref());

                encoded.into_boxed_slice()
            }
        }
    }

    /// Given a reader, return a [Node] from those bytes
    pub fn deserialize_from_reader(_serialized: impl Read) -> Self {
        todo!()
    }
}

/// A path iterator item, which has the key nibbles up to this point,
/// a node, the address of the node, and the nibble that points to the
/// next child down the list
#[derive(Debug)]
pub struct PathIterItem {
    /// The key of the node at `address` as nibbles.
    pub key_nibbles: Box<[u8]>,
    /// A reference to the node
    pub node: Arc<Node>,
    /// The next item returned by the iterator is a child of `node`.
    /// Specifically, it's the child at index `next_nibble` in `node`'s
    /// children array.
    /// None if `node` is the last node in the path.
    pub next_nibble: Option<u8>,
}
