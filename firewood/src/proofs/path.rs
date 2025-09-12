// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::NibblesIterator;

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
pub(super) type NibbleParts<'a> = (Option<u8>, &'a [u8], Option<u8>);

/// Path represents a view over a byte slice as a sequence of nibbles.
///
/// Path also acts as a double-ended cursor over the nibbles, allowing it
/// to be split into sub-paths while retaining the original byte slice.
#[derive(Clone, Copy)]
pub(super) struct PackedPath<'a> {
    bytes: &'a [u8],
    head: usize,
    tail: usize,
}

impl<'a> PackedPath<'a> {
    pub const fn new(key: &'a [u8]) -> Self {
        Self {
            bytes: key,
            head: 0,
            #[cfg(not(feature = "branch_factor_256"))]
            #[expect(clippy::arithmetic_side_effects)]
            tail: key.len() * 2,
            #[cfg(feature = "branch_factor_256")]
            tail: key.len(),
        }
    }

    /// Splits the path into its three parts: first nibble, middle nibbles, and last nibble.
    ///
    /// The first and last nibbles are optional and will be [`None`] if the path
    /// starts and/or ends on a byte boundary.
    #[cfg(not(feature = "branch_factor_256"))]
    pub fn as_parts(&self) -> NibbleParts<'a> {
        if self.is_empty() {
            return (None, &[], None);
        }

        let add_one = !self.head.is_multiple_of(2);
        let sub_one = !self.tail.is_multiple_of(2);

        let mid1 = self.head.midpoint(usize::from(add_one));
        #[expect(clippy::arithmetic_side_effects)]
        let mid2 = (self.tail - usize::from(sub_one)) / 2;

        let (head, tail) = self.bytes.split_at(mid1);
        #[expect(clippy::arithmetic_side_effects)]
        let (middle, tail) = tail.split_at(mid2 - mid1);

        let first = if add_one {
            Some(head.last().copied().unwrap_or(0) & 0x0F)
        } else {
            None
        };

        let last = if sub_one {
            Some(tail.first().copied().unwrap_or(0) >> 4)
        } else {
            None
        };

        (first, middle, last)
    }

    #[cfg(feature = "branch_factor_256")]
    pub fn as_parts(&self) -> NibbleParts<'a> {
        if self.is_empty() {
            (None, &[], None)
        } else {
            #[expect(clippy::indexing_slicing)]
            (None, &self.bytes[self.head..self.tail], None)
        }
    }
}

impl std::fmt::Debug for PackedPath<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.alternate() {
            f.debug_struct("Path")
                .field("bytes", &self.bytes)
                .field("offset", &self.head)
                .field("length", &self.tail)
                .field("nibbles", &format_args!("{self}"))
                .finish()
        } else {
            write!(f, "{self}")
        }
    }
}

impl std::fmt::Display for PackedPath<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        display_nibbles(f, self.nibbles_iter())
    }
}

/// Represents a view over a `Path` where each nibble has already been widened
/// into a full byte.
///
/// Also acts as a double-ended cursor over the nibbles, but we do not need to
/// handle any partial bytes since each nibble is already widened into a byte.
#[derive(Clone, Copy)]
pub(super) struct WidenedPath<'a> {
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

impl std::fmt::Debug for WidenedPath<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if f.alternate() {
            f.debug_struct("WidenedPath")
                .field("bytes", &self.bytes)
                .field("offset", &self.head)
                .field("length", &self.tail)
                .field("nibbles", &format_args!("{self}"))
                .finish()
        } else {
            write!(f, "{self}")
        }
    }
}

impl std::fmt::Display for WidenedPath<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        display_nibbles(f, self.nibbles_iter())
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum RangeProofPath<'a> {
    Packed(PackedPath<'a>),
    Widened(WidenedPath<'a>),
}

impl std::fmt::Display for RangeProofPath<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RangeProofPath::Packed(p) => write!(f, "{p}"),
            RangeProofPath::Widened(w) => write!(f, "{w}"),
        }
    }
}

pub(super) struct SplitKey<L, R = L> {
    pub common_prefix: L,
    pub lhs_suffix: L,
    pub rhs_suffix: R,
}

impl<L, R> SplitKey<L, R> {
    pub fn new<'a>(lhs: L, rhs: R) -> Self
    where
        L: Nibbles<'a> + Copy,
        R: Nibbles<'a> + Copy,
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

#[derive(Debug, Clone, Copy)]
pub(super) struct WithPrefix<T>(T);

impl WithPrefix<PackedPath<'_>> {
    #[cfg(not(feature = "branch_factor_256"))]
    pub fn to_packed_key(self) -> Box<[u8]> {
        #![expect(clippy::indexing_slicing)]
        let mut out = Vec::with_capacity(self.0.tail.div_ceil(2));
        out.copy_from_slice(&self.0.bytes[..self.0.tail.div_ceil(2)]);
        if let Some(last) = out.last_mut()
            && self.0.tail % 2 != 0
        {
            *last &= 0xF0;
        }
        out.into_boxed_slice()
    }

    #[cfg(feature = "branch_factor_256")]
    pub fn to_packed_key(self) -> Box<[u8]> {
        #![expect(clippy::indexing_slicing)]
        self.0.bytes[..self.0.tail].to_vec().into_boxed_slice()
    }
}

impl<'a> WithPrefix<WidenedPath<'a>> {
    pub fn nibbles_iter(self) -> impl Iterator<Item = u8> + use<'a> {
        WidenedPath {
            bytes: self.0.bytes,
            head: 0,
            tail: self.0.tail,
        }
        .nibbles_iter()
    }

    #[cfg(not(feature = "branch_factor_256"))]
    pub fn to_packed_key(self) -> Box<[u8]> {
        #![expect(clippy::indexing_slicing)]
        firewood_storage::padded_packed_path(self.0.bytes[..self.0.tail].iter().copied())
    }

    #[cfg(feature = "branch_factor_256")]
    pub fn to_packed_key(self) -> Box<[u8]> {
        #![expect(clippy::indexing_slicing)]
        self.0.bytes[..self.0.tail].to_vec().into_boxed_slice()
    }
}

impl<'a> From<PackedPath<'a>> for RangeProofPath<'a> {
    fn from(p: PackedPath<'a>) -> Self {
        Self::Packed(p)
    }
}

impl<'a> From<WidenedPath<'a>> for RangeProofPath<'a> {
    fn from(w: WidenedPath<'a>) -> Self {
        Self::Widened(w)
    }
}

pub(super) trait Nibbles<'a>: Sized {
    fn nibbles_iter(self) -> impl Iterator<Item = u8> + Clone + 'a;

    fn split_at(self, mid: usize) -> (Self, Self);

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool;

    fn split_first(self) -> (Option<u8>, Self);

    fn with_prefix(self) -> WithPrefix<Self> {
        WithPrefix(self)
    }
}

impl<'a> Nibbles<'a> for PackedPath<'a> {
    fn nibbles_iter(self) -> impl Iterator<Item = u8> + Clone + 'a {
        let (first, middle, last) = self.as_parts();
        first
            .into_iter()
            .chain(NibblesIterator::new(middle))
            .chain(last)
    }

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

    fn len(&self) -> usize {
        self.tail.saturating_sub(self.head)
    }

    fn is_empty(&self) -> bool {
        self.tail <= self.head
    }

    fn split_first(self) -> (Option<u8>, Self) {
        if self.is_empty() {
            (None, self)
        } else {
            let first = self.nibbles_iter().next();
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

impl<'a> Nibbles<'a> for WidenedPath<'a> {
    fn nibbles_iter(self) -> impl Iterator<Item = u8> + Clone + 'a {
        #![expect(clippy::indexing_slicing)]
        self.bytes[self.head..self.tail].iter().copied()
    }

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

    fn len(&self) -> usize {
        self.tail.saturating_sub(self.head)
    }

    fn is_empty(&self) -> bool {
        self.tail <= self.head
    }

    fn split_first(self) -> (Option<u8>, Self) {
        if self.is_empty() {
            (None, self)
        } else {
            let first = self.nibbles_iter().next();
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

impl<'a> Nibbles<'a> for RangeProofPath<'a> {
    fn nibbles_iter(self) -> impl Iterator<Item = u8> + Clone + 'a {
        match self {
            RangeProofPath::Packed(p) => EitherIter::A(p.nibbles_iter()),
            RangeProofPath::Widened(w) => EitherIter::B(w.nibbles_iter()),
        }
    }

    fn split_at(self, mid: usize) -> (Self, Self) {
        match self {
            RangeProofPath::Packed(p) => {
                let (l, r) = p.split_at(mid);
                (RangeProofPath::Packed(l), RangeProofPath::Packed(r))
            }
            RangeProofPath::Widened(w) => {
                let (l, r) = w.split_at(mid);
                (RangeProofPath::Widened(l), RangeProofPath::Widened(r))
            }
        }
    }

    fn len(&self) -> usize {
        match self {
            RangeProofPath::Packed(p) => p.len(),
            RangeProofPath::Widened(w) => w.len(),
        }
    }

    fn is_empty(&self) -> bool {
        match self {
            RangeProofPath::Packed(p) => p.is_empty(),
            RangeProofPath::Widened(w) => w.is_empty(),
        }
    }

    fn split_first(self) -> (Option<u8>, Self) {
        match self {
            RangeProofPath::Packed(p) => {
                let (first, rest) = p.split_first();
                (first, RangeProofPath::Packed(rest))
            }
            RangeProofPath::Widened(w) => {
                let (first, rest) = w.split_first();
                (first, RangeProofPath::Widened(rest))
            }
        }
    }
}

impl<'a, T: Nibbles<'a> + Copy> PartialEq<T> for PackedPath<'a> {
    fn eq(&self, other: &T) -> bool {
        self.len() == other.len() && self.nibbles_iter().eq(other.nibbles_iter())
    }
}

impl Eq for PackedPath<'_> {}

impl<'a, T: Nibbles<'a> + Copy> PartialEq<T> for WidenedPath<'a> {
    fn eq(&self, other: &T) -> bool {
        self.len() == other.len() && self.nibbles_iter().eq(other.nibbles_iter())
    }
}

impl Eq for WidenedPath<'_> {}

impl<'a, T: Nibbles<'a> + Copy> PartialEq<T> for RangeProofPath<'a> {
    fn eq(&self, other: &T) -> bool {
        self.len() == other.len() && self.nibbles_iter().eq(other.nibbles_iter())
    }
}

impl Eq for RangeProofPath<'_> {}

#[derive(Debug, Clone)]
enum EitherIter<A, B> {
    A(A),
    B(B),
}

impl<A, B, T> Iterator for EitherIter<A, B>
where
    A: Iterator<Item = T>,
    B: Iterator<Item = T>,
{
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::A(a) => a.next(),
            Self::B(b) => b.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::A(a) => a.size_hint(),
            Self::B(b) => b.size_hint(),
        }
    }

    fn count(self) -> usize {
        match self {
            Self::A(a) => a.count(),
            Self::B(b) => b.count(),
        }
    }

    fn last(self) -> Option<Self::Item> {
        match self {
            Self::A(a) => a.last(),
            Self::B(b) => b.last(),
        }
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        match self {
            Self::A(a) => a.nth(n),
            Self::B(b) => b.nth(n),
        }
    }

    fn for_each<F: FnMut(Self::Item)>(self, f: F) {
        match self {
            Self::A(a) => a.for_each(f),
            Self::B(b) => b.for_each(f),
        }
    }

    fn fold<V, F: FnMut(V, Self::Item) -> V>(self, init: V, f: F) -> V {
        match self {
            Self::A(a) => a.fold(init, f),
            Self::B(b) => b.fold(init, f),
        }
    }
}

impl<A, B, T> DoubleEndedIterator for EitherIter<A, B>
where
    A: DoubleEndedIterator<Item = T>,
    B: DoubleEndedIterator<Item = T>,
{
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Self::A(a) => a.next_back(),
            Self::B(b) => b.next_back(),
        }
    }

    fn rfold<V, F: FnMut(V, Self::Item) -> V>(self, init: V, f: F) -> V {
        match self {
            Self::A(a) => a.rfold(init, f),
            Self::B(b) => b.rfold(init, f),
        }
    }
}

impl<A, B, T> ExactSizeIterator for EitherIter<A, B>
where
    A: ExactSizeIterator<Item = T>,
    B: ExactSizeIterator<Item = T>,
{
    fn len(&self) -> usize {
        match self {
            Self::A(a) => a.len(),
            Self::B(b) => b.len(),
        }
    }
}

impl<A, B, T> std::iter::FusedIterator for EitherIter<A, B>
where
    A: std::iter::FusedIterator<Item = T>,
    B: std::iter::FusedIterator<Item = T>,
{
}

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

    #[test_case(b"", b"", SplitKey { common_prefix: path!(""), lhs_suffix: path!(""), rhs_suffix: path!("") }; "both empty")]
    #[test_case(b"a", b"a", SplitKey { common_prefix: path!("a"), lhs_suffix: path!("a", 1), rhs_suffix: path!("a", 1) }; "both same single byte")]
    #[test_case(b"a", b"b", SplitKey {
        common_prefix: PackedPath { bytes: b"a", head: 0, tail: HALF },
        lhs_suffix: PackedPath { bytes: b"a", head: HALF, tail: MULT },
        rhs_suffix: PackedPath { bytes: b"b", head: HALF, tail: MULT },
    }; "both different single byte")]
    #[test_case(b"abc", b"abc", SplitKey { common_prefix: path!("abc"), lhs_suffix: path!("abc", 3), rhs_suffix: path!("abc", 3) }; "both same multiple bytes")]
    #[test_case(b"abc", b"abd", SplitKey {
        common_prefix: PackedPath { bytes: b"abc", head: 0, tail: 2 * MULT + HALF },
        lhs_suffix: PackedPath { bytes: b"abc", head: 2 * MULT + HALF, tail: 3 * MULT },
        rhs_suffix: PackedPath { bytes: b"abd", head: 2 * MULT + HALF, tail: 3 * MULT },
    }; "same prefix, different last byte")]
    #[test_case(b"abc", b"ab", SplitKey { common_prefix: path!("abc", 0, 2), lhs_suffix: path!("abc", 2), rhs_suffix: path!("ab", 2) }; "lhs longer than rhs")]
    #[test_case(b"ab", b"abc", SplitKey { common_prefix: path!("ab"), lhs_suffix: path!("ab", 2), rhs_suffix: path!("abc", 2) }; "rhs longer than lhs")]
    #[test_case(b"abc", b"xyz", SplitKey { common_prefix: path!("abc", 0, 0), lhs_suffix: path!("abc"), rhs_suffix: path!("xyz") }; "no common prefix")]
    fn test_split_key(a: &[u8], b: &[u8], split: SplitKey<PackedPath<'_>>) {
        let actual = SplitKey::new(PackedPath::new(a), PackedPath::new(b));
        assert_eq!(actual.common_prefix, split.common_prefix);
        assert_eq!(actual.lhs_suffix, split.lhs_suffix);
        assert_eq!(actual.rhs_suffix, split.rhs_suffix);
    }
}
