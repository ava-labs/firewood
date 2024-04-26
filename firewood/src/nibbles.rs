// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::{iter::FusedIterator, ops::Index};

static NIBBLES: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

/// Nibbles is a newtype that contains only a reference to a [u8], and produces
/// nibbles. Nibbles can be indexed using nib\[x\] or you can get an iterator
/// with `into_iter()`
///
/// # Examples
///
/// ```
/// # use firewood::nibbles;
/// # fn main() {
/// let nib = nibbles::Nibbles::new(&[0x56, 0x78]);
/// assert_eq!(nib.into_iter().collect::<Vec<_>>(), [0x5, 0x6, 0x7, 0x8]);
///
/// // nibbles can be efficiently advanced without rendering the
/// // intermediate values
/// assert_eq!(nib.into_iter().skip(3).collect::<Vec<_>>(), [0x8]);
///
/// // nibbles can also be indexed
///
/// assert_eq!(nib[1], 0x6);
///
/// // or reversed
/// assert_eq!(nib.into_iter().rev().next(), Some(0x8));
/// # }
/// ```
#[derive(Debug, Copy, Clone)]
pub struct Nibbles<'a>(&'a [u8]);

impl<'a> Index<usize> for Nibbles<'a> {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        if index % 2 == 0 {
            #[allow(clippy::indexing_slicing)]
            &NIBBLES[(self.0[(index) / 2] >> 4) as usize]
        } else {
            #[allow(clippy::indexing_slicing)]
            &NIBBLES[(self.0[(index) / 2] & 0xf) as usize]
        }
    }
}

impl<'a> IntoIterator for Nibbles<'a> {
    type Item = u8;
    type IntoIter = NibblesIterator<'a>;

    #[must_use]
    fn into_iter(self) -> Self::IntoIter {
        NibblesIterator {
            data: self,
            head: 0,
            tail: self.len(),
        }
    }
}

impl<'a> Nibbles<'a> {
    #[must_use]
    pub const fn len(&self) -> usize {
        2 * self.0.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub const fn new(inner: &'a [u8]) -> Self {
        Nibbles(inner)
    }
}

/// An iterator returned by [Nibbles::into_iter]
/// See their documentation for details.
#[derive(Clone, Debug)]
pub struct NibblesIterator<'a> {
    data: Nibbles<'a>,
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
        #[allow(clippy::indexing_slicing)]
        let result = Some(self.data[self.head]);
        self.head += 1;
        result
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
}

impl<'a> DoubleEndedIterator for NibblesIterator<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.is_empty() {
            return None;
        }
        self.tail -= 1;
        #[allow(clippy::indexing_slicing)]
        Some(self.data[self.tail])
    }

    fn nth_back(&mut self, n: usize) -> Option<Self::Item> {
        self.tail -= std::cmp::min(n, self.tail - self.head);
        self.next_back()
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
mod test {
    use super::Nibbles;
    static TEST_BYTES: [u8; 4] = [0xdeu8, 0xad, 0xbe, 0xef];

    #[test]
    fn happy_regular_nibbles() {
        let nib = Nibbles(&TEST_BYTES);
        let expected = [0xdu8, 0xe, 0xa, 0xd, 0xb, 0xe, 0xe, 0xf];
        for v in expected.into_iter().enumerate() {
            assert_eq!(nib[v.0], v.1, "{v:?}");
        }
    }

    #[test]
    #[should_panic]
    fn out_of_bounds_panics() {
        let nib = Nibbles(&TEST_BYTES);
        let _ = nib[8];
    }

    #[test]
    fn last_nibble() {
        let nib = Nibbles(&TEST_BYTES);
        assert_eq!(nib[7], 0xf);
    }

    #[test]
    fn size_hint() {
        let nib = Nibbles(&TEST_BYTES);
        let mut nib_iter = nib.into_iter();
        assert_eq!((8, Some(8)), nib_iter.size_hint());
        let _ = nib_iter.next();
        assert_eq!((7, Some(7)), nib_iter.size_hint());
    }

    #[test]
    fn backwards() {
        let nib = Nibbles(&TEST_BYTES);
        let nib_iter = nib.into_iter().rev();
        let expected = [0xf, 0xe, 0xe, 0xb, 0xd, 0xa, 0xe, 0xd];

        assert!(nib_iter.eq(expected));
    }

    #[test]
    fn empty() {
        let nib = Nibbles(&[]);
        assert!(nib.is_empty());
        let it = nib.into_iter();
        assert!(it.is_empty());
        assert_eq!(it.size_hint().0, 0);
    }

    #[test]
    fn not_empty_because_of_data() {
        let nib = Nibbles(&[1]);
        assert!(!nib.is_empty());
        let mut it = nib.into_iter();
        assert!(!it.is_empty());
        assert_eq!(it.size_hint(), (2, Some(2)));
        assert_eq!(it.next(), Some(0));
        assert!(!it.is_empty());
        assert_eq!(it.size_hint(), (1, Some(1)));
        assert_eq!(it.next(), Some(1));
        assert!(it.is_empty());
        assert_eq!(it.size_hint(), (0, Some(0)));
    }
}
