// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::PathComponent;

/// The maximum number of children in a [`Children`] array and also the arity
/// of a branch node.
///
/// This cannot be defined on the `Children` struct because it is generic, and
/// Rust no longer allows using associated constants of generic types within
/// generic other contexts. This means that while we can define the const in the
/// impl block for `Children<T>`, we cannot use `Children::<T>::MAX_CHILDREN` in
/// contexts where `T` is generic, which doesn't make it very useful.
pub const MAX_CHILDREN: usize = if cfg!(feature = "branch_factor_256") {
    256
} else {
    16
};

/// Type alias for an iterator over the slots of a branch node's children
/// with its corresponding [`PathComponent`].
pub type ChildrenSlots<T> = std::iter::Zip<
    std::array::IntoIter<PathComponent, MAX_CHILDREN>,
    std::array::IntoIter<T, MAX_CHILDREN>,
>;

/// The type of iterator returned by [`Children::iter_present`].
pub type IterPresentRef<'a, T> = std::iter::FilterMap<
    ChildrenSlots<&'a Option<T>>,
    fn((PathComponent, &'a Option<T>)) -> Option<(PathComponent, &'a T)>,
>;

/// The type of iterator returned by [`Children::iter_present`].
pub type IterPresentMut<'a, T> = std::iter::FilterMap<
    ChildrenSlots<&'a mut Option<T>>,
    fn((PathComponent, &'a mut Option<T>)) -> Option<(PathComponent, &'a mut T)>,
>;

/// Type alias for a collection of children in a branch node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Children<T>([T; MAX_CHILDREN]);

impl<T> Children<T> {
    /// Creates a new [`Children`] by calling `f` for each possible
    /// [`PathComponent`].
    #[must_use]
    pub fn from_fn(f: impl FnMut(PathComponent) -> T) -> Self {
        Self(PathComponent::ALL.map(f))
    }

    /// Borrows each element and returns a [`Children`] wrapping the references.
    #[must_use]
    pub fn each_ref(&self) -> Children<&T> {
        Children(self.0.each_ref())
    }

    /// Borrows each element mutably and returns a [`Children`] wrapping the
    /// mutable references.
    #[must_use]
    pub fn each_mut(&mut self) -> Children<&mut T> {
        Children(self.0.each_mut())
    }

    /// Returns a reference to the element at `index`.
    ///
    /// This is infallible because `index` is guaranteed to be in-bounds.
    #[must_use]
    pub const fn get(&self, index: PathComponent) -> &T {
        #![expect(clippy::indexing_slicing)]
        &self.0[index.as_usize()]
    }

    /// Returns a mutable reference to the element at `index`.
    ///
    /// This is infallible because `index` is guaranteed to be in-bounds.
    #[must_use]
    pub const fn get_mut(&mut self, index: PathComponent) -> &mut T {
        #![expect(clippy::indexing_slicing)]
        &mut self.0[index.as_usize()]
    }

    /// Replaces the element at `index` with `value`, returning the previous
    /// value.
    ///
    /// This is infallible because `index` is guaranteed to be in-bounds.
    pub const fn replace(&mut self, index: PathComponent, value: T) -> T {
        #![expect(clippy::indexing_slicing)]
        std::mem::replace(&mut self.0[index.as_usize()], value)
    }

    /// Maps each element to another value using `f`, returning a new
    /// [`Children`] containing the results.
    #[must_use]
    pub fn map<O>(self, f: impl FnMut(T) -> O) -> Children<O> {
        Children(self.0.map(f))
    }

    /// Returns an iterator over each element with its corresponding
    /// [`PathComponent`].
    pub fn iter(&self) -> ChildrenSlots<&T> {
        self.into_iter()
    }

    /// Returns a mutable iterator over each element with its corresponding
    /// [`PathComponent`].
    pub fn iter_mut(&mut self) -> ChildrenSlots<&mut T> {
        self.into_iter()
    }
}

impl<T> Children<Option<T>> {
    /// Creates a new [`Children`] with all elements set to `None`.
    #[must_use]
    pub const fn new() -> Self {
        Self([const { None }; MAX_CHILDREN])
    }

    /// Returns the number of [`Some`] elements in this collection.
    #[must_use]
    pub fn count(&self) -> usize {
        self.0.iter().filter(|c| c.is_some()).count()
    }

    /// Sets the element at `index` to `None`, returning the previous value.
    #[must_use]
    pub const fn take(&mut self, index: PathComponent) -> Option<T> {
        self.replace(index, None)
    }

    /// Returns an iterator over each [`Some`] element with its corresponding
    /// [`PathComponent`].
    pub fn iter_present(&self) -> IterPresentRef<'_, T> {
        self.into_iter()
            .filter_map(|(pc, opt)| opt.as_ref().map(|v| (pc, v)))
    }
}

impl<T> Default for Children<Option<T>> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, T> Children<&'a Option<T>> {
    /// Returns the number of [`Some`] elements in this collection.
    #[must_use]
    pub fn count(&self) -> usize {
        self.0.iter().filter(|c| c.is_some()).count()
    }

    /// Returns an iterator over each [`Some`] element with its corresponding
    /// [`PathComponent`].
    pub fn iter_present(self) -> IterPresentRef<'a, T> {
        self.into_iter()
            .filter_map(|(pc, opt)| opt.as_ref().map(|v| (pc, v)))
    }
}

impl<'a, T> Children<&'a mut Option<T>> {
    /// Returns the number of [`Some`] elements in this collection.
    #[must_use]
    pub fn count(&self) -> usize {
        self.0.iter().filter(|c| c.is_some()).count()
    }

    /// Returns an iterator over each [`Some`] element with its corresponding
    /// [`PathComponent`].
    pub fn iter_present(self) -> IterPresentMut<'a, T> {
        self.into_iter()
            .filter_map(|(pc, opt)| opt.as_mut().map(|v| (pc, v)))
    }
}

impl<T> std::ops::Index<PathComponent> for Children<T> {
    type Output = T;

    fn index(&self, index: PathComponent) -> &Self::Output {
        self.get(index)
    }
}

impl<T> std::ops::IndexMut<PathComponent> for Children<T> {
    fn index_mut(&mut self, index: PathComponent) -> &mut Self::Output {
        self.get_mut(index)
    }
}

impl<T> IntoIterator for Children<T> {
    type Item = (PathComponent, T);
    type IntoIter = ChildrenSlots<T>;

    fn into_iter(self) -> Self::IntoIter {
        PathComponent::ALL.into_iter().zip(self.0)
    }
}

impl<'a, T: 'a> IntoIterator for &'a Children<T> {
    type Item = (PathComponent, &'a T);
    type IntoIter = ChildrenSlots<&'a T>;

    fn into_iter(self) -> Self::IntoIter {
        self.each_ref().into_iter()
    }
}

impl<'a, T: 'a> IntoIterator for &'a mut Children<T> {
    type Item = (PathComponent, &'a mut T);
    type IntoIter = ChildrenSlots<&'a mut T>;

    fn into_iter(self) -> Self::IntoIter {
        self.each_mut().into_iter()
    }
}
