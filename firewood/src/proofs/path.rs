// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#[cfg(not(feature = "branch_factor_256"))]
use firewood_storage::NibblesIterator;

pub(crate) trait Nibbles {
    fn nibbles_iter(&self) -> impl Iterator<Item = u8> + Clone + '_;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool;

    fn bytes_iter(&self) -> impl Iterator<Item = u8> + Clone + '_ {
        BytesIter::new(self.nibbles_iter())
    }

    fn collect(&self) -> CollectedNibbles {
        CollectedNibbles::new(self)
    }

    fn display(&self) -> DisplayNibbles<'_, Self> {
        DisplayNibbles(self)
    }

    fn join<O: Nibbles>(self, other: O) -> JoinedPath<Self, O>
    where
        Self: Sized,
    {
        JoinedPath { a: self, b: other }
    }
}

pub(crate) struct DisplayNibbles<'a, P: ?Sized>(&'a P);

impl<P: Nibbles + ?Sized> std::fmt::Debug for DisplayNibbles<'_, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

impl<P: Nibbles + ?Sized> std::fmt::Display for DisplayNibbles<'_, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        display_nibbles(f, self.0.nibbles_iter())
    }
}

#[derive(Clone)]
pub(crate) struct CollectedNibbles {
    nibbles: smallvec::SmallVec<[u8; 32]>,
}

impl CollectedNibbles {
    pub fn new(nibbles: impl Nibbles) -> Self {
        Self {
            nibbles: nibbles.nibbles_iter().collect(),
        }
    }
}

#[cfg(feature = "branch_factor_256")]
pub(super) type PackedPath<'a> = WidenedPath<'a>;

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
#[cfg(not(feature = "branch_factor_256"))]
type NibbleParts<'a> = (Option<u8>, &'a [u8], Option<u8>);

/// Path represents a view over a byte slice as a sequence of nibbles.
///
/// Path also acts as a double-ended cursor over the nibbles, allowing it
/// to be split into sub-paths while retaining the original byte slice.
#[derive(Clone, Copy)]
#[cfg(not(feature = "branch_factor_256"))]
pub(crate) struct PackedPath<'a> {
    bytes: &'a [u8],
    head: usize,
    tail: usize,
}

#[cfg(not(feature = "branch_factor_256"))]
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

pub(super) struct SplitPath<L, R = L> {
    pub common_prefix: L,
    pub lhs_suffix: L,
    pub rhs_suffix: R,
}

impl<L, R> SplitPath<L, R> {
    pub fn new(lhs: L, rhs: R) -> Self
    where
        L: SplitNibbles + Copy,
        R: SplitNibbles + Copy,
    {
        let mid = lhs
            .nibbles_iter()
            .zip(rhs.nibbles_iter())
            .take_while(|&(a, b)| a == b)
            .count();
        let (common_prefix, lhs_suffix) = lhs.split_at(mid);
        let (_, rhs_suffix) = rhs.split_at(mid);
        Self {
            common_prefix,
            lhs_suffix,
            rhs_suffix,
        }
    }
}

impl<L: Nibbles, R: Nibbles> std::fmt::Display for SplitPath<L, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "common: {}, lhs: {}, rhs: {}",
            DisplayNibbles(&self.common_prefix),
            DisplayNibbles(&self.lhs_suffix),
            DisplayNibbles(&self.rhs_suffix),
        )
    }
}

impl<L: Nibbles, R: Nibbles> Nibbles for either::Either<L, R> {
    fn nibbles_iter(&self) -> impl Iterator<Item = u8> + Clone + '_ {
        match self {
            either::Left(l) => either::Left(l.nibbles_iter()),
            either::Right(r) => either::Right(r.nibbles_iter()),
        }
    }

    fn len(&self) -> usize {
        match self {
            either::Left(l) => l.len(),
            either::Right(r) => r.len(),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            either::Left(l) => l.is_empty(),
            either::Right(r) => r.is_empty(),
        }
    }
}

impl<T: Nibbles + ?Sized> Nibbles for &T {
    fn nibbles_iter(&self) -> impl Iterator<Item = u8> + Clone + '_ {
        (**self).nibbles_iter()
    }

    fn len(&self) -> usize {
        (**self).len()
    }

    fn is_empty(&self) -> bool {
        (**self).is_empty()
    }
}

impl<T: Nibbles> Nibbles for Option<T> {
    fn nibbles_iter(&self) -> impl Iterator<Item = u8> + Clone + '_ {
        self.as_ref().into_iter().flat_map(T::nibbles_iter)
    }

    fn len(&self) -> usize {
        self.as_ref().map_or(0, T::len)
    }

    fn is_empty(&self) -> bool {
        self.as_ref().is_none_or(T::is_empty)
    }
}

pub(super) trait SplitNibbles: Nibbles + Sized {
    fn split_at(self, mid: usize) -> (Self, Self);

    fn split_first(self) -> (Option<PathNibble>, Self);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct PathNibble(pub(super) u8);

impl Nibbles for PathNibble {
    fn nibbles_iter(&self) -> impl Iterator<Item = u8> + Clone + '_ {
        std::iter::once(self.0)
    }

    fn len(&self) -> usize {
        1
    }

    fn is_empty(&self) -> bool {
        false
    }
}

impl Nibbles for CollectedNibbles {
    fn nibbles_iter(&self) -> impl Iterator<Item = u8> + Clone + '_ {
        self.nibbles.iter().copied()
    }

    fn len(&self) -> usize {
        self.nibbles.len()
    }

    fn is_empty(&self) -> bool {
        self.nibbles.is_empty()
    }
}

#[cfg(not(feature = "branch_factor_256"))]
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

#[cfg(not(feature = "branch_factor_256"))]
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

#[derive(Clone, Copy)]
pub(crate) struct JoinedPath<A, B> {
    a: A,
    b: B,
}

impl<T: Nibbles, U: Nibbles> Nibbles for JoinedPath<T, U> {
    fn nibbles_iter(&self) -> impl Iterator<Item = u8> + Clone + '_ {
        self.a.nibbles_iter().chain(self.b.nibbles_iter())
    }

    fn len(&self) -> usize {
        self.a.len().saturating_add(self.b.len())
    }

    fn is_empty(&self) -> bool {
        self.a.is_empty() && self.b.is_empty()
    }
}

#[derive(Clone)]
pub(crate) struct BytesIter<I> {
    iter: I,
}

impl<I: Iterator<Item = u8>> BytesIter<I> {
    pub const fn new(iter: I) -> Self {
        Self { iter }
    }
}

#[cfg(not(feature = "branch_factor_256"))]
impl<I: Iterator<Item = u8>> Iterator for BytesIter<I> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        let high = self.iter.next()?;
        let low = self.iter.next().unwrap_or(0);
        Some((high << 4) | (low & 0x0F))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let (low, high) = self.iter.size_hint();
        let low = low.div_ceil(2);
        let high = high.map(|h| h.div_ceil(2));
        (low, high)
    }
}

#[cfg(feature = "branch_factor_256")]
impl<I: Iterator<Item = u8>> Iterator for BytesIter<I> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

/// $Type is the type to implement the traits for an is expected to already
/// implement [`Nibbles`]
macro_rules! impl_common_nibbles_traits {
    ($({$($gen:tt)*},)? $Type:ty) => {
        impl$(<$($gen)*>)? ::std::fmt::Debug for $Type {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                display_nibbles(f, self.nibbles_iter())
            }
        }

        impl$(<$($gen)*>)? ::std::fmt::Display for $Type {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                display_nibbles(f, self.nibbles_iter())
            }
        }

        impl<__Other: Nibbles + ?Sized, $($($gen)*)?> PartialEq<__Other> for $Type {
            fn eq(&self, other: &__Other) -> bool {
                self.len() == other.len() && self.nibbles_iter().eq(other.nibbles_iter())
            }
        }

        impl$(<$($gen)*>)? Eq for $Type {}
    };
    ($Type:ty, $($Other:ty),+ $(,)?) => {
        impl_common_nibbles_traits!($Type);
        impl_common_nibbles_traits!($($Other),+);
    };
}

impl_common_nibbles_traits!(CollectedNibbles, WidenedPath<'_>);

#[cfg(not(feature = "branch_factor_256"))]
impl_common_nibbles_traits!(PackedPath<'_>);

impl_common_nibbles_traits!({T: Nibbles, U: Nibbles}, JoinedPath<T, U>);

#[cfg(not(feature = "branch_factor_256"))]
pub(super) fn display_nibbles(
    mut f: impl std::fmt::Write,
    nibbles: impl Iterator<Item = u8>,
) -> std::fmt::Result {
    f.write_str("<")?;
    let mut iter = nibbles;
    if let Some(n) = iter.next() {
        write!(f, "{n:x}")?;
        for n in iter {
            write!(f, " {n:x}")?;
        }
    }
    f.write_str(">")
}

#[cfg(feature = "branch_factor_256")]
pub(super) fn display_nibbles(
    mut f: impl std::fmt::Write,
    nibbles: impl Iterator<Item = u8>,
) -> std::fmt::Result {
    f.write_str("<")?;
    let mut iter = nibbles;
    if let Some(n) = iter.next() {
        write!(f, "{n:x}")?;
        for n in iter {
            write!(f, " {n:02x}")?;
        }
    }
    f.write_str(">")
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_case::test_case;

    const HALF: usize = if cfg!(feature = "branch_factor_256") {
        0
    } else {
        1
    };

    const MULT: usize = 1 + HALF;

    macro_rules! path {
        ($key:literal) => {
            path!($key, 0)
        };
        ($key:literal, $head:expr) => {
            path!($key, $head, $key.len())
        };
        ($key:literal, $head:expr, $tail:expr) => {
            const {
                PackedPath {
                    bytes: $key.as_bytes(),
                    head: $head * MULT,
                    tail: $tail * MULT,
                }
            }
        };
    }

    #[test_case(b"", b"", SplitPath { common_prefix: path!(""), lhs_suffix: path!(""), rhs_suffix: path!("") }; "both empty")]
    #[test_case(b"a", b"a", SplitPath { common_prefix: path!("a"), lhs_suffix: path!("a", 1), rhs_suffix: path!("a", 1) }; "both same single byte")]
    #[test_case(b"a", b"b", SplitPath {
        common_prefix: PackedPath { bytes: b"a", head: 0, tail: HALF },
        lhs_suffix: PackedPath { bytes: b"a", head: HALF, tail: MULT },
        rhs_suffix: PackedPath { bytes: b"b", head: HALF, tail: MULT },
    }; "both different single byte")]
    #[test_case(b"abc", b"abc", SplitPath { common_prefix: path!("abc"), lhs_suffix: path!("abc", 3), rhs_suffix: path!("abc", 3) }; "both same multiple bytes")]
    #[test_case(b"abc", b"abd", SplitPath {
        common_prefix: PackedPath { bytes: b"abc", head: 0, tail: 2 * MULT + HALF },
        lhs_suffix: PackedPath { bytes: b"abc", head: 2 * MULT + HALF, tail: 3 * MULT },
        rhs_suffix: PackedPath { bytes: b"abd", head: 2 * MULT + HALF, tail: 3 * MULT },
    }; "same prefix, different last byte")]
    #[test_case(b"abc", b"ab", SplitPath { common_prefix: path!("abc", 0, 2), lhs_suffix: path!("abc", 2), rhs_suffix: path!("ab", 2) }; "lhs longer than rhs")]
    #[test_case(b"ab", b"abc", SplitPath { common_prefix: path!("ab"), lhs_suffix: path!("ab", 2), rhs_suffix: path!("abc", 2) }; "rhs longer than lhs")]
    #[test_case(b"abc", b"xyz", SplitPath { common_prefix: path!("abc", 0, 0), lhs_suffix: path!("abc"), rhs_suffix: path!("xyz") }; "no common prefix")]
    fn test_split_key(a: &[u8], b: &[u8], split: SplitPath<PackedPath<'_>>) {
        let actual = SplitPath::new(PackedPath::new(a), PackedPath::new(b));
        assert_eq!(actual.common_prefix, split.common_prefix);
        assert_eq!(actual.lhs_suffix, split.lhs_suffix);
        assert_eq!(actual.rhs_suffix, split.rhs_suffix);
    }
}
