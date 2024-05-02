// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use enum_as_inner::EnumAsInner;
use serde::{Deserialize, Serialize};
use std::{fmt::Debug, sync::Arc};

mod branch;
mod leaf;
pub mod path;

pub use branch::BranchNode;
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

impl Node {
    /// Returns a new Arc<Node> which is the same as `self` but with the given `partial_path`.
    pub fn new_with_partial_path(self: Arc<Node>, partial_path: Path) -> Arc<Node> {
        match self.as_ref() {
            Node::Branch(b) => Arc::new(Node::Branch(Box::new(BranchNode {
                partial_path,
                value: b.value.clone(),
                children: b.children.clone(),
            }))),
            Node::Leaf(l) => Arc::new(Node::Leaf(LeafNode {
                partial_path,
                value: l.value.clone(),
            })),
        }
    }
}