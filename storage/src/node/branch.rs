// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use serde::{Deserialize, Serialize};

use crate::{LeafNode, LinearAddress, Path, TrieHash};
use std::fmt::{Debug, Error as FmtError, Formatter};

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
    pub children: [Option<(LinearAddress, Option<TrieHash>)>; Self::MAX_CHILDREN],
}

impl Debug for BranchNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), FmtError> {
        write!(f, "[Branch")?;
        write!(f, r#" path="{:?}""#, self.partial_path)?;

        for (i, c) in self.children.iter().enumerate() {
            match c {
                None => {}
                Some((address, hash)) => write!(
                    f,
                    "(index: {i:?}), address={:?}, hash={:?})",
                    address,
                    hash.as_ref().map(hex::encode),
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

    /// Returns the address of the child at the given index, and the hash,
    /// which is None if we don't know the hash.
    /// None if there is no child at that index.
    /// Panics if `child_index` >= [BranchNode::MAX_CHILDREN].
    pub fn child(&self, child_index: u8) -> Option<(&LinearAddress, Option<&TrieHash>)> {
        self.children
            .get(child_index as usize)
            .expect("child_index is in bounds")
            .as_ref()
            .map(|(address, hash)| (address, hash.as_ref()))
    }

    /// Update the child at `child_index` to be `new_child_addr`.
    /// If `new_child_addr` is None, the child is removed.
    pub fn update_child(&mut self, child_index: u8, new_child_addr: Option<LinearAddress>) {
        let child = self
            .children
            .get_mut(child_index as usize)
            .expect("child_index is in bounds");

        match new_child_addr {
            None => {
                *child = None;
            }
            Some(new_child_addr) => {
                *child = Some((new_child_addr, None));
            }
        }
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
