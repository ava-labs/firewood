// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use derive_where::derive_where;
use firewood::merkle;
use firewood::v2::api::{self, BoxKeyValueIter};
use std::iter::FusedIterator;

type KeyValueItem = (merkle::Key, merkle::Value);

/// An opaque wrapper around an Iterator.
#[derive(Default)]
#[derive_where(Debug)]
#[derive_where(skip_inner(Debug))]
pub struct IteratorHandle<'view> {
    iterator: Option<BoxKeyValueIter<'view>>,
}

impl<'view> From<BoxKeyValueIter<'view>> for IteratorHandle<'view> {
    fn from(value: BoxKeyValueIter<'view>) -> Self {
        IteratorHandle {
            iterator: Some(value),
        }
    }
}

impl Iterator for IteratorHandle<'_> {
    type Item = Result<KeyValueItem, api::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let out = self.iterator.as_mut()?.next();
        if out.is_none() {
            // iterator exhausted; drop it so the NodeStore can be released
            self.iterator.take();
        }
        out
    }
}

impl FusedIterator for IteratorHandle<'_> {}

#[expect(clippy::missing_errors_doc)]
impl IteratorHandle<'_> {
    pub fn iter_next_n(&mut self, n: usize) -> Result<Vec<KeyValueItem>, api::Error> {
        // Take up to n items from this iterator; if the iterator is
        // exhausted before n items are produced, we return fewer.
        // The iterator is fused; once exhausted, subsequent calls will
        // continue to yield no items.
        self.by_ref().take(n).collect()
    }
}

#[derive(Debug, Default)]
pub struct CreateIteratorResult<'db> {
    pub handle: IteratorHandle<'db>,
}
