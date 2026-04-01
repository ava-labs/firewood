// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::{Children, PathComponent, U4};

/// Compact u16 bitmap for tracking which of a branch node's 16 children
/// are present.
///
/// Used in proof serialization to track which children are present in a node,
/// enabling efficient bitmap-based encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ChildMask(u16);

impl ChildMask {
    /// Returns `true` if the bit at `nibble` is set.
    pub(crate) const fn is_set(self, nibble: U4) -> bool {
        self.0 & 1u16.wrapping_shl(nibble.as_u8() as u32) != 0
    }

    /// Create a new `ChildMask` from the given children array, setting a bit
    /// for each child that is `Some`.
    pub(crate) fn from_children<T>(children: &Children<Option<T>>) -> Self {
        Self(children.iter_present().fold(0, |bits, (i, _)| {
            bits | 1u16.wrapping_shl(u32::from(i.as_u8()))
        }))
    }

    /// Iterates over the indices of all set bits, yielding `PathComponent`s.
    ///
    /// Uses bit-scanning (`trailing_zeros` + clear-lowest-bit) so the cost is
    /// proportional to the number of set bits, not the total width.
    pub(crate) fn iter_indices(self) -> impl Iterator<Item = PathComponent> {
        PathComponent::ALL
            .into_iter()
            .filter(move |&i| self.is_set(i.0))
    }

    /// Serialize as little-endian bytes (for proof wire format).
    pub(crate) const fn to_le_bytes(self) -> [u8; 2] {
        self.0.to_le_bytes()
    }

    /// Deserialize from little-endian bytes (for proof wire format).
    pub(crate) const fn from_le_bytes(bytes: [u8; 2]) -> Self {
        Self(u16::from_le_bytes(bytes))
    }
}

impl std::ops::BitOr for ChildMask {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for ChildMask {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn u4(v: u8) -> U4 {
        U4::new_masked(v)
    }

    #[test]
    fn child_mask_default_is_empty() {
        let mask = ChildMask::default();

        for i in 0..16u8 {
            assert!(!mask.is_set(u4(i)), "default should have no bits set");
        }
    }

    #[test]
    fn child_mask_from_children_empty() {
        let children: Children<Option<()>> = Children::new();
        let mask = ChildMask::from_children(&children);
        assert_eq!(mask.0, 0);
    }

    #[test]
    fn child_mask_from_children_some() {
        let mut children: Children<Option<()>> = Children::new();
        children[PathComponent::ALL[0]] = Some(());
        children[PathComponent::ALL[5]] = Some(());
        children[PathComponent::ALL[15]] = Some(());
        let mask = ChildMask::from_children(&children);
        assert!(mask.is_set(u4(0)));
        assert!(!mask.is_set(u4(1)));
        assert!(mask.is_set(u4(5)));
        assert!(mask.is_set(u4(15)));
    }

    #[test]
    fn child_mask_from_children_all() {
        let children: Children<Option<()>> = Children::from_fn(|_| Some(()));
        let mask = ChildMask::from_children(&children);
        // All 16 bits should be set
        for i in 0..16u8 {
            assert!(mask.is_set(u4(i)), "from_children(all) should set bit {i}");
        }
    }

    #[test]
    fn child_mask_iter_indices() {
        let mut children: Children<Option<()>> = Children::new();
        children[PathComponent::ALL[1]] = Some(());
        children[PathComponent::ALL[7]] = Some(());
        children[PathComponent::ALL[14]] = Some(());
        let mask = ChildMask::from_children(&children);
        let indices: Vec<u8> = mask.iter_indices().map(PathComponent::as_u8).collect();
        assert_eq!(indices, vec![1, 7, 14]);
    }

    #[test]
    fn child_mask_iter_indices_empty() {
        let indices: Vec<u8> = ChildMask::default()
            .iter_indices()
            .map(PathComponent::as_u8)
            .collect();
        assert!(indices.is_empty());
    }

    #[test]
    fn child_mask_le_bytes_roundtrip() {
        let mut children: Children<Option<()>> = Children::new();
        children[PathComponent::ALL[0]] = Some(());
        children[PathComponent::ALL[8]] = Some(());
        children[PathComponent::ALL[15]] = Some(());
        let mask = ChildMask::from_children(&children);
        let bytes = mask.to_le_bytes();
        assert_eq!(ChildMask::from_le_bytes(bytes), mask);
    }

    #[test]
    fn child_mask_le_bytes_encoding() {
        // bit 0 in low byte, bit 9 in high byte
        let mut children: Children<Option<()>> = Children::new();
        children[PathComponent::ALL[0]] = Some(());
        children[PathComponent::ALL[9]] = Some(());
        let mask = ChildMask::from_children(&children);
        assert_eq!(mask.to_le_bytes(), [0x01, 0x02]);
    }

    #[test]
    fn child_mask_bitor_assign() {
        let mut children_a: Children<Option<()>> = Children::new();
        children_a[PathComponent::ALL[1]] = Some(());
        let mut mask = ChildMask::from_children(&children_a);

        let mut children_b: Children<Option<()>> = Children::new();
        children_b[PathComponent::ALL[14]] = Some(());
        mask |= ChildMask::from_children(&children_b);

        assert!(mask.is_set(u4(1)));
        assert!(mask.is_set(u4(14)));
        assert!(!mask.is_set(u4(0)));
    }
}
