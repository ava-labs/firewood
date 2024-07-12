// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use serde::{Deserialize, Serialize};

use crate::{LeafNode, LinearAddress, Node, Path, TrieHash};
use std::fmt::{Debug, Error as FmtError, Formatter};

#[derive(PartialEq, Eq, Clone, Serialize, Deserialize, Default, Debug)]
/// A child of a branch node.
pub enum Child {
    #[default]
    /// There is no child at this index.
    None,
    Node(Node),
    /// We know the child's address and hash.
    AddressWithHash(LinearAddress, TrieHash),
}

#[derive(PartialEq, Eq, Clone, Serialize, Deserialize)]
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
    pub children: [Child; Self::MAX_CHILDREN],
}

impl Debug for BranchNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), FmtError> {
        write!(f, "[Branch")?;
        write!(f, r#" path="{:?}""#, self.partial_path)?;

        for (i, c) in self.children.iter().enumerate() {
            match c {
                Child::None => {}
                Child::Node(_) => {} //TODO
                Child::AddressWithHash(addr, hash) => write!(
                    f,
                    "(index: {i:?}), address={addr:?}, hash={:?})",
                    hex::encode(hash),
                )?,
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
    pub const MAX_CHILDREN: usize = 16;

    /// Returns the address of the child at the given index.
    /// Panics if `child_index` >= [BranchNode::MAX_CHILDREN].
    pub fn child(&self, child_index: u8) -> &Child {
        self.children
            .get(child_index as usize)
            .expect("child_index is in bounds")
    }

    /// Update the child at `child_index` to be `new_child_addr`.
    /// If `new_child_addr` is None, the child is removed.
    pub fn update_child(&mut self, child_index: u8, new_child: Child) {
        let child = self
            .children
            .get_mut(child_index as usize)
            .expect("child_index is in bounds");

        *child = new_child;
    }

    /// Returns (index, hash) for each child that has a hash set.
    pub fn children_iter(&self) -> impl Iterator<Item = (usize, &TrieHash)> + Clone {
        self.children.iter().enumerate().filter_map(
            // TODO danlaine: can we avoid indexing?
            #[allow(clippy::indexing_slicing)]
            |(i, child)| match child {
                Child::None => None,
                Child::Node(_) => unreachable!("TODO make unreachable"),
                Child::AddressWithHash(_, hash) => Some((i, hash)),
            },
        )
    }
}

impl From<&LeafNode> for BranchNode {
    fn from(leaf: &LeafNode) -> Self {
        BranchNode {
            partial_path: leaf.partial_path.clone(),
            value: Some(leaf.value.clone()),
            children: Default::default(),
        }
    }
}
