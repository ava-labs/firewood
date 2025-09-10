// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::Children;

pub(super) mod magic {
    pub const PROOF_HEADER: &[u8; 8] = b"fwdproof";

    pub const PROOF_VERSION: u8 = 0;

    #[cfg(not(feature = "ethhash"))]
    pub const HASH_MODE: u8 = 0;
    #[cfg(feature = "ethhash")]
    pub const HASH_MODE: u8 = 1;

    pub const fn hash_mode_name(v: u8) -> &'static str {
        match v {
            0 => "sha256",
            1 => "keccak256",
            _ => "unknown",
        }
    }

    #[cfg(not(feature = "branch_factor_256"))]
    pub const BRANCH_FACTOR: u8 = 16;
    #[cfg(feature = "branch_factor_256")]
    pub const BRANCH_FACTOR: u8 = 0; // 256 wrapped to 0

    pub const fn widen_branch_factor(v: u8) -> u16 {
        match v {
            0 => 256,
            _ => v as u16,
        }
    }
}

/// The type of serialized proof.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProofType {
    /// A proof for a single key/value pair.
    ///
    /// A proof is a sequence of nodes from the root to a specific node.
    /// Each node in the path includes the hash of its child nodes, allowing
    /// for verification of the integrity of the path.
    ///
    /// A single proof includes the full key and value (if present) of the target
    /// node.
    Single = 0,
    /// A range proof for all key/value pairs over a specific key range.
    ///
    /// A range proof includes a key proof for the beginning and end of the
    /// range, as well as all key/value pairs in the range.
    Range = 1,
    /// A change proof for all key/value pairs that changed between two
    /// versions of the tree.
    ///
    /// A change proof includes a key proof for the beginning and end of the
    /// changed range, as well as all key/value pairs that changed.
    Change = 2,
}

impl ProofType {
    /// Parse a byte into a [`ProofType`].
    #[must_use]
    pub const fn new(v: u8) -> Option<Self> {
        match v {
            0 => Some(ProofType::Single),
            1 => Some(ProofType::Range),
            2 => Some(ProofType::Change),
            _ => None,
        }
    }

    /// Human readable name for the [`ProofType`]
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            ProofType::Single => "single",
            ProofType::Range => "range",
            ProofType::Change => "change",
        }
    }
}

/// A fixed-size header at the beginning of every serialized proof.
///
/// # Format
///
/// - 8 bytes: A magic value to identify the file type. This is `b"fwdproof"`.
/// - 1 byte: The version of the proof format. Currently `0`.
/// - 1 byte: The hash mode used in the proof. Currently `0` for sha256, `1` for
///   keccak256.
/// - 1 byte: The branching factor of the trie. Currently `16` or `0` for `256`.
/// - 1 byte: The type of proof. See [`ProofType`].
/// - 20 bytes: Reserved for future use and to pad the header to 32 bytes. Ignored
///   when reading, and set to zero when writing.
#[derive(Debug, Clone, Copy, bytemuck_derive::Pod, bytemuck_derive::Zeroable)]
#[repr(C)]
pub struct Header {
    pub(super) magic: [u8; 8],
    pub(super) version: u8,
    pub(super) hash_mode: u8,
    pub(super) branch_factor: u8,
    pub(super) proof_type: u8,
    pub(super) _reserved: [u8; 20],
}

const _: () = {
    assert!(size_of::<Header>() == 32);
};

impl Header {
    /// Construct a new header for the given proof type.
    #[must_use]
    pub(crate) const fn new(proof_type: ProofType) -> Self {
        Self {
            magic: *magic::PROOF_HEADER,
            version: magic::PROOF_VERSION,
            hash_mode: magic::HASH_MODE,
            branch_factor: magic::BRANCH_FACTOR,
            proof_type: proof_type as u8,
            _reserved: [0; 20],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, bytemuck_derive::Pod, bytemuck_derive::Zeroable)]
#[repr(C)]
/// A bitmap indicating which children are present in a node.
pub(super) struct ChildrenMap([u8; ChildrenMap::SIZE]);

impl ChildrenMap {
    const SIZE: usize = firewood_storage::BranchNode::MAX_CHILDREN / 8;

    /// Create a new `ChildrenMap` from the given children array.
    pub fn new<T>(children: &Children<T>) -> Self {
        let mut map = [0_u8; Self::SIZE];

        for (i, child) in children.iter().enumerate() {
            if child.is_some() {
                let (idx, bit) = (i / 8, i % 8);
                #[expect(clippy::indexing_slicing)]
                {
                    map[idx] |= 1 << bit;
                }
            }
        }

        Self(map)
    }

    #[cfg(test)]
    pub fn len(self) -> usize {
        self.0.iter().map(|b| b.count_ones() as usize).sum()
    }

    pub fn iter_indices(self) -> impl Iterator<Item = usize> {
        (0..firewood_storage::BranchNode::MAX_CHILDREN).filter(
            #[expect(clippy::indexing_slicing)]
            move |i| self.0[i / 8] & (1 << (i % 8)) != 0,
        )
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
        write!(f, "{a:0128b}{b:0128b}")
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
    #[test_case(
        {
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
