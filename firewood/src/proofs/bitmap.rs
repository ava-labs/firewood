// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::Children;

#[derive(Clone, Copy, PartialEq, Eq, bytemuck_derive::Pod, bytemuck_derive::Zeroable)]
#[repr(C)]
/// A bitmap indicating which children are present in a node.
pub(super) struct ChildrenMap([u8; ChildrenMap::SIZE]);

impl ChildrenMap {
    const SIZE: usize = firewood_storage::BranchNode::MAX_CHILDREN / 8;

    pub const fn empty() -> Self {
        Self([0; Self::SIZE])
    }

    pub const fn get(self, index: usize) -> bool {
        #![expect(clippy::indexing_slicing)]
        self.0[index / 8] & (1 << (index % 8)) != 0
    }

    pub const fn set(&mut self, index: usize) {
        #![expect(clippy::indexing_slicing)]
        self.0[index / 8] |= 1 << (index % 8);
    }

    #[expect(unused)]
    pub const fn unset(&mut self, index: usize) {
        #![expect(clippy::indexing_slicing)]
        self.0[index / 8] &= !(1 << (index % 8));
    }

    #[expect(unused)]
    pub fn first(self) -> Option<usize> {
        self.iter_indices().next()
    }

    #[expect(unused)]
    pub fn last(self) -> Option<usize> {
        self.iter_indices().last()
    }

    /// Create a new `ChildrenMap` from the given children array.
    pub fn new<T>(children: &Children<T>) -> Self {
        let mut map = Self::empty();

        for (i, child) in children.iter().enumerate() {
            if child.is_some() {
                map.set(i);
            }
        }

        map
    }

    #[cfg(test)]
    pub fn len(self) -> usize {
        self.0.iter().map(|b| b.count_ones() as usize).sum()
    }

    pub fn iter_indices(self) -> impl Iterator<Item = usize> {
        (0..firewood_storage::BranchNode::MAX_CHILDREN).filter(move |&i| self.get(i))
    }
}

impl std::fmt::Display for ChildrenMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.alternate() {
            f.debug_list().entries(self.iter_indices()).finish()
        } else {
            write!(f, "{self:b}")
        }
    }
}

#[cfg(not(feature = "branch_factor_256"))]
impl std::fmt::Binary for ChildrenMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016b}", u16::from_le_bytes(self.0))
    }
}

#[cfg(feature = "branch_factor_256")]
impl std::fmt::Binary for ChildrenMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [a, b] = bytemuck::cast::<_, [[u8; 16]; 2]>(self.0);
        let a = u128::from_le_bytes(a);
        let b = u128::from_le_bytes(b);
        // NOTE: print `b` before `a` so that the bits are in the expected order.
        //
        // Bytes are displayed in big-endian order (most to least significant),
        // but stored in little-endian order (least to most significant), so the
        // swap is necessary to display the bits in the expected order.
        write!(f, "{b:0128b}{a:0128b}")
    }
}

impl std::fmt::Debug for ChildrenMap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use firewood_storage::BranchNode;
    use test_case::test_case;

    #[test_case(BranchNode::empty_children(), &[]; "empty")]
    #[test_case({
        let mut children = BranchNode::empty_children();
        children[0] = Some(());
        children
    }, &[0]; "first")]
    #[test_case({
        let mut children = BranchNode::empty_children();
        children[1] = Some(());
        children
    }, &[1]; "second")]
    #[test_case({
        let mut children = BranchNode::empty_children();
        children[BranchNode::MAX_CHILDREN - 1] = Some(());
        children
    }, &[BranchNode::MAX_CHILDREN - 1]; "last")]
    #[test_case({
        let mut children = BranchNode::empty_children();
        for slot in children.iter_mut().step_by(2) {
            *slot = Some(());
        }
        children
    }, &(0..BranchNode::MAX_CHILDREN).step_by(2).collect::<Vec<_>>(); "evens")]
    #[test_case({
        let mut children = BranchNode::empty_children();
        for slot in children.iter_mut().skip(1).step_by(2) {
            *slot = Some(());
        }
        children
    }, &(1..BranchNode::MAX_CHILDREN).step_by(2).collect::<Vec<_>>(); "odds")]
    #[test_case([Some(()); BranchNode::MAX_CHILDREN], &(0..BranchNode::MAX_CHILDREN).collect::<Vec<_>>(); "all")]
    fn test_children_map(children: Children<()>, indicies: &[usize]) {
        let map = ChildrenMap::new(&children);
        assert_eq!(map.len(), indicies.len());

        assert!(
            indicies.iter().copied().eq(map.iter_indices()),
            "expected {:?}, got {:?}",
            indicies,
            map.iter_indices().collect::<Vec<_>>()
        );
    }
}
