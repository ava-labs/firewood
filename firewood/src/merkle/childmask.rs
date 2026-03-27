// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::U4;

/// Bitmask indicating which of a branch node's 16 children are "outside" the
/// proven range and should use the proof's original hashes instead of being
/// recomputed from the proving trie.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct ChildMask(u16);

impl ChildMask {
    /// A mask with all children marked as outside.
    pub(super) const ALL: Self = Self(u16::MAX);

    /// Marks all children with index less than `nibble` as outside.
    pub(super) const fn mark_left_outside(mut self, nibble: U4) -> Self {
        self.0 |= 1u16.wrapping_shl(nibble.as_u8() as u32).wrapping_sub(1);
        self
    }

    /// Marks all children with index greater than `nibble` as outside.
    pub(super) const fn mark_right_outside(mut self, nibble: U4) -> Self {
        self.0 |= u16::MAX.wrapping_shl(nibble.as_u8() as u32).wrapping_shl(1);
        self
    }

    /// Marks a single child as outside.
    pub(super) const fn set_outside(mut self, nibble: U4) -> Self {
        self.0 |= 1u16.wrapping_shl(nibble.as_u8() as u32);
        self
    }

    /// Returns true if the child at `nibble` is outside the proven range.
    pub(super) const fn is_outside(self, nibble: U4) -> bool {
        self.0 & 1u16.wrapping_shl(nibble.as_u8() as u32) != 0
    }

    /// Merges another mask into this one (union of outside children).
    pub(super) const fn merge(mut self, other: ChildMask) -> Self {
        self.0 |= other.0;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn u4(v: u8) -> U4 {
        U4::new_masked(v)
    }

    #[test]
    fn child_mask_mark_left_outside() {
        // mark_left_outside(n) should set bits 0..n (children < n)
        let expected: [(U4, u16); 16] = [
            (u4(0), 0b0000_0000_0000_0000),
            (u4(1), 0b0000_0000_0000_0001),
            (u4(2), 0b0000_0000_0000_0011),
            (u4(3), 0b0000_0000_0000_0111),
            (u4(4), 0b0000_0000_0000_1111),
            (u4(5), 0b0000_0000_0001_1111),
            (u4(6), 0b0000_0000_0011_1111),
            (u4(7), 0b0000_0000_0111_1111),
            (u4(8), 0b0000_0000_1111_1111),
            (u4(9), 0b0000_0001_1111_1111),
            (u4(10), 0b0000_0011_1111_1111),
            (u4(11), 0b0000_0111_1111_1111),
            (u4(12), 0b0000_1111_1111_1111),
            (u4(13), 0b0001_1111_1111_1111),
            (u4(14), 0b0011_1111_1111_1111),
            (u4(15), 0b0111_1111_1111_1111),
        ];
        for (nibble, bits) in expected {
            let mask = ChildMask::default().mark_left_outside(nibble);
            assert_eq!(mask.0, bits, "mark_left_outside({nibble})");
        }
    }

    #[test]
    fn child_mask_mark_right_outside() {
        // mark_right_outside(n) should set bits (n+1)..16 (children > n)
        let expected: [(U4, u16); 16] = [
            (u4(0), 0b1111_1111_1111_1110),
            (u4(1), 0b1111_1111_1111_1100),
            (u4(2), 0b1111_1111_1111_1000),
            (u4(3), 0b1111_1111_1111_0000),
            (u4(4), 0b1111_1111_1110_0000),
            (u4(5), 0b1111_1111_1100_0000),
            (u4(6), 0b1111_1111_1000_0000),
            (u4(7), 0b1111_1111_0000_0000),
            (u4(8), 0b1111_1110_0000_0000),
            (u4(9), 0b1111_1100_0000_0000),
            (u4(10), 0b1111_1000_0000_0000),
            (u4(11), 0b1111_0000_0000_0000),
            (u4(12), 0b1110_0000_0000_0000),
            (u4(13), 0b1100_0000_0000_0000),
            (u4(14), 0b1000_0000_0000_0000),
            (u4(15), 0b0000_0000_0000_0000),
        ];
        for (nibble, bits) in expected {
            let mask = ChildMask::default().mark_right_outside(nibble);
            assert_eq!(mask.0, bits, "mark_right_outside({nibble})");
        }
    }

    #[test]
    fn child_mask_is_outside() {
        // Mark children < 3 and children > 12 as outside
        let mask = ChildMask::default()
            .mark_left_outside(u4(3))
            .mark_right_outside(u4(12));

        for i in 0..16u8 {
            let nibble = u4(i);
            let expected = !(3..=12).contains(&i);
            assert_eq!(mask.is_outside(nibble), expected, "is_outside({nibble})");
        }
    }

    #[test]
    fn child_mask_merge() {
        let merged = ChildMask::default()
            .mark_left_outside(u4(5)) // bits 0..5
            .merge(ChildMask::default().mark_right_outside(u4(10))); // bits 11..16

        for i in 0..16u8 {
            let nibble = u4(i);
            let expected = !(5..=10).contains(&i);
            assert_eq!(
                merged.is_outside(nibble),
                expected,
                "merged is_outside({nibble})"
            );
        }
    }

    #[test]
    fn child_mask_default_is_empty() {
        let mask = ChildMask::default();
        for i in 0..16u8 {
            assert!(!mask.is_outside(u4(i)), "default should have no bits set");
        }
    }

    #[test]
    fn child_mask_all_is_full() {
        let mask = ChildMask::ALL;
        for i in 0..16u8 {
            assert!(mask.is_outside(u4(i)), "ALL should have all bits set");
        }
    }
}
