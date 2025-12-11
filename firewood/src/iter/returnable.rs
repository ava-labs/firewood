// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

pub(crate) trait ReturnableIteratorExt: Iterator + Sized {
    /// Wraps this iterator in a [`ReturnableIterator`].
    fn returnable(self) -> ReturnableIterator<Self> {
        ReturnableIterator::new(self)
    }
}

impl<I: Iterator> ReturnableIteratorExt for I {}

/// Similar to a peekable iterator. In addition to being able to peek at the
/// next item without consuming it, it also allows "returning" an item back to
/// the iterator to be yielded on the next call to [`next()`].
///
/// [`next()`]: Iterator::next
pub(crate) struct ReturnableIterator<I: Iterator> {
    iter: I,
    next: Option<I::Item>,
}

impl<I: Iterator> ReturnableIterator<I> {
    pub(crate) const fn new(iter: I) -> Self {
        Self { iter, next: None }
    }

    /// Peeks at the next item without consuming it. The next call to
    /// [`next()`] will still return this item.
    ///
    /// [`next()`]: Iterator::next
    pub(crate) fn peek(&mut self) -> Option<&mut I::Item> {
        if self.next.is_none() {
            self.next = self.iter.next();
        }

        self.next.as_mut()
    }

    /// Puts an item back to be returned on the next call to [`next()`]. This
    /// makes it easy to "un-read" a single item from the iterator without
    /// needing to implement complex buffering logic.
    ///
    /// NOTE: This will replace and return any item that was already in the
    /// return slot.
    ///
    /// [`next()`]: Iterator::next
    pub(crate) const fn return_item(&mut self, head: I::Item) -> Option<I::Item> {
        self.next.replace(head)
    }
}

impl<I: Iterator> Iterator for ReturnableIterator<I> {
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.next.take().or_else(|| self.iter.next())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let (lower, upper) = self.iter.size_hint();
        let head_count = usize::from(self.next.is_some());
        (
            lower.saturating_add(head_count),
            upper.and_then(|u| u.checked_add(head_count)),
        )
    }
}
