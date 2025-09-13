// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::NibblesIterator;

use super::{Nibbles, PathNibble, SplitNibbles};

/// (first nibble, middle nibbles, last nibble)
///
/// The options on either end are to handle the case where the path starts or
/// in the middle of a byte, splitting it into an incomplete byte.
///
/// In both cases, the nibble will be shifted to the lower 4 bits of the byte.
///
/// If `branch_factor_256` is enabled, both options will always be None.
///
/// The middle nibbles slice will always be whole bytes and if `branch_factor_256`
/// is not enabled, represent 2 nibbles per byte (in big-endian order).
///
/// This represents all the parts in a sub-slice of a Path.
type NibbleParts<'a> = (Option<u8>, &'a [u8], Option<u8>);

/// Path represents a view over a byte slice as a sequence of nibbles.
///
/// Path also acts as a double-ended cursor over the nibbles, allowing it
/// to be split into sub-paths while retaining the original byte slice.
#[derive(Clone, Copy)]
pub(crate) struct PackedPath<'a> {
    bytes: &'a [u8],
    head: usize,
    tail: usize,
}

impl<'a> PackedPath<'a> {
    pub const fn new(key: &'a [u8]) -> Self {
        Self {
            bytes: key,
            head: 0,
            #[expect(clippy::arithmetic_side_effects)]
            tail: key.len() * 2,
        }
    }

    /// Splits the path into its three parts: first nibble, middle nibbles, and last nibble.
    ///
    /// The first and last nibbles are optional and will be [`None`] if the path
    /// starts and/or ends on a byte boundary.
    fn as_parts(&self) -> NibbleParts<'a> {
        #![expect(clippy::arithmetic_side_effects, clippy::indexing_slicing)]

        if self.is_empty() {
            return (None, &[], None);
        }

        let (leading_nibble, mid1) = if self.head.is_multiple_of(2) {
            (None, self.head)
        } else {
            let byte = self.bytes[self.head / 2];
            (Some(byte & 0xf), self.head + 1)
        };

        let (trailing_nibble, mid2) = if self.tail.is_multiple_of(2) {
            (None, self.tail)
        } else {
            let byte = self.bytes[self.tail / 2];
            (Some(byte >> 4), self.tail - 1)
        };

        let middle = if mid2 > mid1 {
            &self.bytes[mid1 / 2..mid2 / 2]
        } else {
            &[]
        };

        (leading_nibble, middle, trailing_nibble)
    }
}

impl Nibbles for PackedPath<'_> {
    fn nibbles_iter(&self) -> impl Iterator<Item = u8> + Clone + '_ {
        let (first, middle, last) = self.as_parts();
        first
            .into_iter()
            .chain(NibblesIterator::new(middle))
            .chain(last)
    }

    fn len(&self) -> usize {
        self.tail.saturating_sub(self.head)
    }

    fn is_empty(&self) -> bool {
        self.tail <= self.head
    }
}
impl SplitNibbles for PackedPath<'_> {
    fn split_at(self, mid: usize) -> (Self, Self) {
        let len = self.len();
        assert!(mid <= len, "split index out of bounds");
        #[expect(clippy::arithmetic_side_effects)]
        let mid = self.head + mid;
        let left = Self {
            bytes: self.bytes,
            head: self.head,
            tail: mid,
        };
        let right = Self {
            bytes: self.bytes,
            head: mid,
            tail: self.tail,
        };
        (left, right)
    }

    fn split_first(self) -> (Option<PathNibble>, Self) {
        if self.is_empty() {
            (None, self)
        } else {
            let first = self.nibbles_iter().next().map(PathNibble);
            let rest = Self {
                bytes: self.bytes,
                #[expect(clippy::arithmetic_side_effects)]
                head: self.head + 1,
                tail: self.tail,
            };
            (first, rest)
        }
    }
}
