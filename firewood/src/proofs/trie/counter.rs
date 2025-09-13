// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::BranchNode;

use crate::proofs::path::PathNibble;

pub(super) struct NibbleCounter(u8);

impl NibbleCounter {
    pub const fn new() -> Self {
        Self(0)
    }

    pub fn next(&mut self) -> PathNibble {
        let this = self.0;
        self.0 = self
            .0
            .wrapping_add(1)
            .clamp(0, const { (BranchNode::MAX_CHILDREN - 1) as u8 });
        PathNibble(this)
    }
}
