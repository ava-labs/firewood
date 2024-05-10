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

    /// The children of this branch
    pub children: [Option<LinearAddress>; Self::MAX_CHILDREN],

    /// The hashes for each child
    /// TODO: Serialize only non-zero ones. We know how many from the 'children' array
    /// and can reconstruct it without storing blanks for each non-existent child
    pub child_hashes: [TrieHash; Self::MAX_CHILDREN],
}

impl Debug for BranchNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), FmtError> {
        write!(f, "[Branch")?;
        write!(f, r#" path="{:?}""#, self.partial_path)?;

        for (i, c) in self.children.iter().enumerate() {
            if let Some(c) = c {
                write!(f, " ({i:x} {c:?})")?;
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

    /// Obtain a mutable reference to a child address within a branch
    /// This convenience method takes a u8 as the nibble offset
    /// Panics if `child_index` >= [BranchNode::MAX_CHILDREN].
    pub fn child_mut(&mut self, child_index: u8) -> &mut Option<LinearAddress> {
        self.children
            .get_mut(child_index as usize)
            .expect("child_index is in bounds")
    }

    /// Returns the address of the child at the given index.
    /// None if there is no child at that index.
    /// Panics if `child_index` >= [BranchNode::MAX_CHILDREN].
    pub fn child(&self, child_index: u8) -> Option<&LinearAddress> {
        self.children
            .get(child_index as usize)
            .expect("child_index is in bounds")
            .as_ref()
    }

    /// consume a branch node, finding the old child address and setting it
    /// to a new one. Also invalidates the hash for it.
    pub fn update_child_address(
        &self,
        old_child_addr: LinearAddress,
        new_child_addr: Option<LinearAddress>,
    ) -> BranchNode {
        let mut new_children = self.children;
        let (index, child_ref) = new_children
            .iter_mut()
            .enumerate()
            .find(|(_, &mut child_addr)| child_addr == Some(old_child_addr))
            .expect("child was not in the parent");
        *child_ref = new_child_addr;

        let mut new_child_hashes = self.child_hashes.clone();
        *new_child_hashes
            .get_mut(index)
            .expect("arrays are same size, so offset into one must match the other") =
            Default::default();
        BranchNode {
            partial_path: self.partial_path.clone(),
            value: self.value.clone(),
            children: new_children,
            child_hashes: new_child_hashes,
        }
    }

    /// Update the child address of a branch node and invalidate the hash
    pub fn update_child(&mut self, child_index: u8, new_child_addr: Option<LinearAddress>) {
        *self.child_mut(child_index) = new_child_addr;
        self.invalidate_child_hash(child_index);
    }

    pub(crate) fn invalidate_child_hash(&mut self, child_index: u8) {
        *self
            .child_hashes
            .get_mut(child_index as usize)
            .expect("nibble") = Default::default();
    }
}

impl From<&LeafNode> for BranchNode {
    fn from(leaf: &LeafNode) -> Self {
        BranchNode {
            partial_path: leaf.partial_path.clone(),
            value: Some(leaf.value.clone()),
            children: Default::default(),
            child_hashes: Default::default(),
        }
    }
}
