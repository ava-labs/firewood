// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::BranchNode;

use crate::proofs::path::PathNibble;

/// A simple wrapping counter for nibbles.
///
/// This is used for iterating over the possible child indices of a branch node
/// from `0..MAX_CHILDREN` when mapping over the array.
///
/// The array `map` method doesn't provide the index, so this is used to track
/// the number of times `next` has been called achieving the same effect.
pub(super) struct NibbleCounter(u8);

impl NibbleCounter {
    /// Creates a new `NibbleCounter` starting at zero.
    pub const fn new() -> Self {
        Self(0)
    }

    /// Returns the next `PathNibble` in the sequence, wrapping at
    /// [`BranchNode::MAX_CHILDREN`].
    pub fn next(&mut self) -> PathNibble {
        let this = self.0;
        self.0 = self
            .0
            .wrapping_add(1)
            .clamp(0, const { (BranchNode::MAX_CHILDREN - 1) as u8 });
        PathNibble(this)
    }
}
