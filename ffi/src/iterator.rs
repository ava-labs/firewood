// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use derive_where::derive_where;
use firewood::merkle;
use firewood::v2::api::{self};

type KeyValueItem = Result<(merkle::Key, merkle::Value), api::Error>;

/// An opaque wrapper around an Iterator.
#[derive(Default)]
#[derive_where(Debug)]
#[derive_where(skip_inner(Debug))]
pub struct IteratorHandle<'db> {
    iterator: Option<Box<dyn Iterator<Item = KeyValueItem> + 'db>>,
}

impl From<Box<dyn Iterator<Item = KeyValueItem>>> for IteratorHandle<'_> {
    fn from(value: Box<dyn Iterator<Item = KeyValueItem>>) -> Self {
        IteratorHandle {
            iterator: Some(value),
        }
    }
}

impl Iterator for IteratorHandle<'_> {
    type Item = KeyValueItem;

    fn next(&mut self) -> Option<Self::Item> {
        let out = self.iterator.as_mut()?.next();
        if out.is_none() {
            // iterator exhausted; drop it so the NodeStore can be released
            self.iterator.take();
        }
        out
    }
}

#[derive(Default)]
pub struct CreateIteratorResult<'db> {
    pub handle: IteratorHandle<'db>,
}
