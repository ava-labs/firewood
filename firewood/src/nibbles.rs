// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::iter::FusedIterator;

static NIBBLES: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

/// Iterates over the nibbles in `data`. That is, each byte in `data` is
/// converted to two nibbles.
#[derive(Clone, Debug)]
pub struct NibblesIterator<'a> {
    data: &'a [u8],
    head: usize,
    tail: usize,
}

impl<'a> FusedIterator for NibblesIterator<'a> {}

impl<'a> Iterator for NibblesIterator<'a> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if self.is_empty() {
            return None;
        }
        let result = if self.head % 2 == 0 {
            #[allow(clippy::indexing_slicing)]
            NIBBLES[(self.data[self.head / 2] >> 4) as usize]
        } else {
            #[allow(clippy::indexing_slicing)]
            NIBBLES[(self.data[self.head / 2] & 0xf) as usize]
        };
        self.head += 1;
        Some(result)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.tail - self.head;
        (remaining, Some(remaining))
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.head += std::cmp::min(n, self.tail - self.head);
        self.next()
    }
}

impl<'a> NibblesIterator<'a> {
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.head == self.tail
    }

    pub fn new(data: &'a [u8]) -> Self {
        NibblesIterator {
            data,
            head: 0,
            tail: 2 * data.len(),
        }
    }
}

impl<'a> DoubleEndedIterator for NibblesIterator<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.is_empty() {
            return None;
        }

        let result = if self.tail % 2 == 0 {
            #[allow(clippy::indexing_slicing)]
            NIBBLES[(self.data[self.tail / 2 - 1] & 0xf) as usize]
        } else {
            #[allow(clippy::indexing_slicing)]
            NIBBLES[(self.data[self.tail / 2] >> 4) as usize]
        };
        self.tail -= 1;

        Some(result)
    }

    fn nth_back(&mut self, n: usize) -> Option<Self::Item> {
        self.tail -= std::cmp::min(n, self.tail - self.head);
        self.next_back()
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod test {
    use super::NibblesIterator;

    static TEST_BYTES: [u8; 4] = [0xde, 0xad, 0xbe, 0xef];

    #[test]
    fn happy_regular_nibbles() {
        let iter = NibblesIterator::new(&TEST_BYTES);
        let expected = [0xd, 0xe, 0xa, 0xd, 0xb, 0xe, 0xe, 0xf];

        assert!(iter.eq(expected));
    }

    #[test]
    fn size_hint() {
        let mut iter = NibblesIterator::new(&TEST_BYTES);
        assert_eq!((8, Some(8)), iter.size_hint());
        let _ = iter.next();
        assert_eq!((7, Some(7)), iter.size_hint());
    }

    #[test]
    fn backwards() {
        let iter = NibblesIterator::new(&TEST_BYTES).rev();
        let expected = [0xf, 0xe, 0xe, 0xb, 0xd, 0xa, 0xe, 0xd];
        assert!(iter.eq(expected));
    }

    #[test]
    fn neth_back() {
        let mut iter = NibblesIterator::new(&TEST_BYTES);
        assert_eq!(iter.nth_back(0), Some(0xf));
        assert_eq!(iter.nth_back(0), Some(0xe));
        assert_eq!(iter.nth_back(1), Some(0xb));
        assert_eq!(iter.nth_back(2), Some(0xe));
    }

    #[test]
    fn empty() {
        let nib = NibblesIterator::new(&[]);
        assert!(nib.is_empty());
        let it = nib.into_iter();
        assert!(it.is_empty());
        assert_eq!(it.size_hint().0, 0);
    }

    #[test]
    fn not_empty_because_of_data() {
        let mut iter = NibblesIterator::new(&[1]);
        assert!(!iter.is_empty());
        assert!(!iter.is_empty());
        assert_eq!(iter.size_hint(), (2, Some(2)));
        assert_eq!(iter.next(), Some(0));
        assert!(!iter.is_empty());
        assert_eq!(iter.size_hint(), (1, Some(1)));
        assert_eq!(iter.next(), Some(1));
        assert!(iter.is_empty());
        assert_eq!(iter.size_hint(), (0, Some(0)));
    }
}
