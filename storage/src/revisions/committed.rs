// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::sync::Arc;

use crate::{revisions::ReadChangedNode, LinearAddress, Node};

/// A type of NodeStore for a committed revision, which has no extra information
/// other than what is in the base
#[derive(Debug)]
pub struct Committed;

impl ReadChangedNode for Committed {
    fn read_changed_node(&self, _addr: LinearAddress) -> Option<Arc<Node>> {
        None
    }
}
