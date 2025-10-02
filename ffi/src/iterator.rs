// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use derive_where::derive_where;
use firewood::merkle;
use firewood::v2::api::{self, BoxKeyValueIter};

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

#[expect(clippy::missing_errors_doc)]
impl IteratorHandle<'_> {
    pub fn iter_next_n(&mut self, n: usize) -> Result<Vec<KeyValueItem>, api::Error> {
        let mut items = Vec::<KeyValueItem>::new();
        for _ in 0..n {
            let item = self.next();
            if let Some(item) = item {
                items.push(item?);
            } else {
                break;
            }
        }
        Ok(items)
    }
}

#[derive(Debug, Default)]
pub struct CreateIteratorResult<'db> {
    pub handle: IteratorHandle<'db>,
}
