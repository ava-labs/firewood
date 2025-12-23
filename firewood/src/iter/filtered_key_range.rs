// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::v2::api::{KeyType, KeyValuePair};

pub trait FilteredKeyRangeExt: Iterator<Item: KeyValuePair> + Sized {
    /// Returns a new iterator that will emit key-value pairs up to and
    /// including `last_key`.
    fn stop_after_key<K: KeyType>(self, last_key: Option<K>) -> FilteredKeyRangeIter<Self, K> {
        FilteredKeyRangeIter::new(self, last_key)
    }
}

impl<I: Iterator<Item: KeyValuePair>> FilteredKeyRangeExt for I {}

/// An iterator over key-value pairs that stops after a specified final key.
#[derive(Debug)]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub enum FilteredKeyRangeIter<I, K> {
    Unfiltered { iter: I },
    Filtered { iter: I, last_key: K },
    Exhausted,
}

impl<I: Iterator<Item = T>, T: KeyValuePair, K: KeyType> FilteredKeyRangeIter<I, K> {
    /// Creates a new [`FilteredKeyRangeIter`] that will iterate over `iter`
    /// stopping early if `last_key` is `Some` and a key greater than it is
    /// encountered.
    pub fn new(iter: I, last_key: Option<K>) -> Self {
        match last_key {
            Some(k) => FilteredKeyRangeIter::Filtered { iter, last_key: k },
            None => FilteredKeyRangeIter::Unfiltered { iter },
        }
    }
}

impl<I: Iterator<Item = T>, T: KeyValuePair, K: KeyType> Iterator for FilteredKeyRangeIter<I, K> {
    type Item = Result<(T::Key, T::Value), T::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            FilteredKeyRangeIter::Unfiltered { iter } => iter.next().map(T::try_into_tuple),
            FilteredKeyRangeIter::Filtered { iter, last_key } => {
                match iter.next().map(T::try_into_tuple) {
                    Some(Ok((key, value))) if key.as_ref() <= last_key.as_ref() => {
                        Some(Ok((key, value)))
                    }
                    Some(Err(e)) => Some(Err(e)),
                    _ => {
                        *self = FilteredKeyRangeIter::Exhausted;
                        None
                    }
                }
            }
            FilteredKeyRangeIter::Exhausted => None,
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            FilteredKeyRangeIter::Unfiltered { iter } => iter.size_hint(),
            FilteredKeyRangeIter::Filtered { iter, .. } => {
                let (_, upper) = iter.size_hint();
                (0, upper)
            }
            FilteredKeyRangeIter::Exhausted => (0, Some(0)),
        }
    }
}
