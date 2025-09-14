// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::{BytesIter, JoinedPath, WidenedPath};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct PathNibble(pub(in crate::proofs) u8);

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
        JoinedPath::new(self, other)
    }

    fn eq(&self, other: &(impl Nibbles + ?Sized)) -> bool {
        self.len() == other.len() && self.nibbles_iter().eq(other.nibbles_iter())
    }

    fn by_ref(&self) -> &Self {
        self
    }
}

pub(in crate::proofs) trait SplitNibbles: Nibbles + Sized {
    fn split_at(self, mid: usize) -> (Self, Self);

    fn split_first(self) -> (Option<PathNibble>, Self);
}

/// A view of a path where each byte is widened to represent two nibbles.
#[derive(Clone)]
pub struct CollectedNibbles {
    nibbles: smallvec::SmallVec<[u8; 64]>,
}

impl CollectedNibbles {
    /// Creates an empty collection of nibbles.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            nibbles: smallvec::SmallVec::new(),
        }
    }

    #[must_use]
    pub(crate) fn new(nibbles: impl Nibbles) -> Self {
        Self {
            nibbles: nibbles.nibbles_iter().collect(),
        }
    }

    pub(crate) fn extend(&mut self, nibbles: impl Nibbles) {
        self.nibbles.extend(nibbles.nibbles_iter());
    }

    /// Returns the nibbles as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.nibbles
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
pub(super) fn display_nibbles(
    mut f: impl std::fmt::Write,
    nibbles: impl Iterator<Item = u8>,
) -> std::fmt::Result {
    f.write_str("<")?;
    let mut iter = nibbles;
    if let Some(n) = iter.next() {
        write!(f, "{n:x}")?;
        for n in iter {
            write!(f, "{n:x}")?;
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
            write!(f, "{n:02x}")?;
        }
    }
    f.write_str(">")
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
                Nibbles::eq(self, other)
            }
        }

        impl$(<$($gen)*>)? Eq for $Type {}

        impl<__Other: Nibbles + ?Sized, $($($gen)*)?> PartialOrd<__Other> for $Type {
            fn partial_cmp(&self, other: &__Other) -> Option<::std::cmp::Ordering> {
                Some(self.nibbles_iter().cmp(other.nibbles_iter()))
            }
        }

        impl$(<$($gen)*>)? Ord for $Type {
            fn cmp(&self, other: &Self) -> ::std::cmp::Ordering {
                self.nibbles_iter().cmp(other.nibbles_iter())
            }
        }
    };
    ($Type:ty, $($Other:ty),+ $(,)?) => {
        impl_common_nibbles_traits!($Type);
        impl_common_nibbles_traits!($($Other),+);
    };
}

impl_common_nibbles_traits!(CollectedNibbles, WidenedPath<'_>);

#[cfg(not(feature = "branch_factor_256"))]
impl_common_nibbles_traits!(super::PackedPath<'_>);

impl_common_nibbles_traits!({T: Nibbles, U: Nibbles}, JoinedPath<T, U>);
