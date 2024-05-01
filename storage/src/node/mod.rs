// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use enum_as_inner::EnumAsInner;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

mod branch;
mod leaf;
pub mod path;

pub use branch::BranchNode;
pub use leaf::LeafNode;

/// A node, either a Branch or Leaf

// TODO: explain why Branch is boxed but Leaf is not
#[derive(PartialEq, Eq, Clone, Debug, EnumAsInner, Serialize, Deserialize)]
pub enum Node {
    /// This node is a [BranchNode]
    Branch(Box<BranchNode>),
    /// This node is a [LeafNode]
    Leaf(LeafNode),
}
