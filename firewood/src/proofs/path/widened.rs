// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::{Nibbles, PathNibble, SplitNibbles};

/// Represents a view over a `Path` where each nibble has already been widened
/// into a full byte.
///
/// Also acts as a double-ended cursor over the nibbles, but we do not need to
/// handle any partial bytes since each nibble is already widened into a byte.
#[derive(Clone, Copy)]
pub(crate) struct WidenedPath<'a> {
    bytes: &'a [u8],
    head: usize,
    tail: usize,
}

impl<'a> WidenedPath<'a> {
    pub const fn new(key: &'a [u8]) -> Self {
        Self {
            bytes: key,
            head: 0,
            tail: key.len(),
        }
    }
}

impl Nibbles for WidenedPath<'_> {
    fn nibbles_iter(&self) -> impl Iterator<Item = u8> + Clone + '_ {
        #![expect(clippy::indexing_slicing)]
        self.bytes[self.head..self.tail].iter().copied()
    }

    fn len(&self) -> usize {
        self.tail.saturating_sub(self.head)
    }

    fn is_empty(&self) -> bool {
        self.tail <= self.head
    }
}

impl SplitNibbles for WidenedPath<'_> {
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
