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

#[cfg(all(test, not(feature = "branch_factor_256")))]
mod tests {
    use super::*;

    use firewood_storage::BranchNode;
    use test_case::test_case;

    #[test_case(BranchNode::empty_children(), &[]; "empty")]
    #[test_case([Some(()), None, None, None, None, None, None, None, None, None, None, None, None, None, None, None], &[0]; "first")]
    #[test_case([None, Some(()), None, None, None, None, None, None, None, None, None, None, None, None, None, None], &[1]; "second")]
    #[test_case([None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, Some(())], &[15]; "last")]
    #[test_case([Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None], &[0, 2, 4, 6, 8, 10, 12, 14]; "evens")]
    #[test_case([None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(())], &[1, 3, 5, 7, 9, 11, 13, 15]; "odds")]
    #[test_case([Some(()); 16], &(0..16).collect::<Vec<_>>(); "all")]
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

#[cfg(all(test, feature = "branch_factor_256"))]
mod tests {
    use super::*;

    use firewood_storage::BranchNode;
    use test_case::test_case;

    #[test_case(BranchNode::empty_children(), &[]; "empty")]
    #[test_case([Some(()), None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None], &[0]; "first")]
    #[test_case([None, Some(()), None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None], &[1]; "second")]
    #[test_case([None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, None, Some(())], &[255]; "last")]
    #[test_case([Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None], &[0, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 30, 32, 34, 36, 38, 40, 42, 44, 46, 48, 50, 52, 54, 56, 58, 60, 62, 64, 66, 68, 70, 72, 74, 76, 78, 80, 82, 84, 86, 88, 90, 92, 94, 96, 98, 100, 102, 104, 106, 108, 110, 112, 114, 116, 118, 120, 122, 124, 126, 128, 130, 132, 134, 136, 138, 140, 142, 144, 146, 148, 150, 152, 154, 156, 158, 160, 162, 164, 166, 168, 170, 172, 174, 176, 178, 180, 182, 184, 186, 188, 190, 192, 194, 196, 198, 200, 202, 204, 206, 208, 210, 212, 214, 216, 218, 220, 222, 224, 226, 228, 230, 232, 234, 236, 238, 240, 242, 244, 246, 248, 250, 252, 254]; "evens")]
    #[test_case([None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(()), None, Some(())], &[1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 21, 23, 25, 27, 29, 31, 33, 35, 37, 39, 41, 43, 45, 47, 49, 51, 53, 55, 57, 59, 61, 63, 65, 67, 69, 71, 73, 75, 77, 79, 81, 83, 85, 87, 89, 91, 93, 95, 97, 99, 101, 103, 105, 107, 109, 111, 113, 115, 117, 119, 121, 123, 125, 127, 129, 131, 133, 135, 137, 139, 141, 143, 145, 147, 149, 151, 153, 155, 157, 159, 161, 163, 165, 167, 169, 171, 173, 175, 177, 179, 181, 183, 185, 187, 189, 191, 193, 195, 197, 199, 201, 203, 205, 207, 209, 211, 213, 215, 217, 219, 221, 223, 225, 227, 229, 231, 233, 235, 237, 239, 241, 243, 245, 247, 249, 251, 253, 255]; "odds")]
    #[test_case([Some(()); 256], &(0..256).collect::<Vec<_>>(); "all")]
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
