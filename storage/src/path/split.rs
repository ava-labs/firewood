// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use smallvec::SmallVec;

use super::{PathComponent, TriePath};

/// A trie path that can be (cheaply) split into two sub-paths.
///
/// Implementations are expected to be cheap to split (i.e. no allocations).
pub trait SplitPath: TriePath + Default + Copy {
    /// Splits the path at the given index within the path.
    ///
    /// The returned tuple contains the two sub-paths `(prefix, suffix)`.
    ///
    /// # Panics
    ///
    /// - If `mid > self.len()`.
    fn split_at(self, mid: usize) -> (Self, Self);

    /// Splits the first path component off of this path, returning it along with
    /// the remaining path.
    ///
    /// Returns [`None`] if the path is empty.
    fn split_first(self) -> Option<(PathComponent, Self)>;

    /// Computes the longest common prefix of this path and another path, along
    /// with their respective suffixes.
    fn longest_common_prefix<T: SplitPath>(self, other: T) -> PathCommonPrefix<Self, T> {
        PathCommonPrefix::new(self, other)
    }
}

/// A type that can be converted into a splittable path.
///
/// This trait is analogous to [`IntoIterator`] for [`Iterator`] but for trie
/// paths instead of iterators.
///
/// Like `IntoIterator`, a blanket implementation is provided for all types that
/// already implement [`SplitPath`].
pub trait IntoSplitPath {
    /// The splittable path type derived from this type.
    type Path: SplitPath;

    /// Converts this type into a splittable path.
    #[must_use]
    fn into_split_path(self) -> Self::Path;
}

impl<T: SplitPath> IntoSplitPath for T {
    type Path = T;

    #[inline]
    fn into_split_path(self) -> Self::Path {
        self
    }
}

/// The common prefix of two paths, along with their respective suffixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PathCommonPrefix<A, B> {
    /// The common prefix of the two paths.
    pub common: A,
    /// The suffix of the first path after the common prefix.
    pub a_suffix: A,
    /// The suffix of the second path after the common prefix.
    pub b_suffix: B,
}

impl<A: SplitPath, B: SplitPath> PathCommonPrefix<A, B> {
    /// Computes the common prefix of the two given paths, along with their suffixes.
    pub fn new(a: A, b: B) -> Self {
        let mid = a
            .components()
            .zip(b.components())
            .take_while(|&(a, b)| a == b)
            .count();
        let (common, a_suffix) = a.split_at(mid);
        let (_, b_suffix) = b.split_at(mid);
        Self {
            common,
            a_suffix,
            b_suffix,
        }
    }
}

impl SplitPath for &[PathComponent] {
    fn split_at(self, mid: usize) -> (Self, Self) {
        self.split_at(mid)
    }

    fn split_first(self) -> Option<(PathComponent, Self)> {
        match self.split_first() {
            Some((&first, rest)) => Some((first, rest)),
            None => None,
        }
    }
}

impl<'a, const N: usize> IntoSplitPath for &'a [PathComponent; N] {
    type Path = &'a [PathComponent];

    fn into_split_path(self) -> Self::Path {
        self
    }
}

impl<'a> IntoSplitPath for &'a Vec<PathComponent> {
    type Path = &'a [PathComponent];

    fn into_split_path(self) -> Self::Path {
        self
    }
}

impl<'a, A: smallvec::Array<Item = PathComponent>> IntoSplitPath for &'a SmallVec<A> {
    type Path = &'a [PathComponent];

    fn into_split_path(self) -> Self::Path {
        self
    }
}
