// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use derive_where::derive_where;
use firewood::merkle;
use firewood::v2::api::{self, BoxKeyValueIter};

type KeyValueItem = Result<(merkle::Key, merkle::Value), api::Error>;

/// An opaque wrapper around an Iterator.
#[derive_where(Debug)]
#[derive_where(skip_inner)]
pub struct IteratorHandle<'view> {
    iterator: BoxKeyValueIter<'view>,
}

impl<'view> From<BoxKeyValueIter<'view>> for IteratorHandle<'view> {
    fn from(value: BoxKeyValueIter<'view>) -> Self {
        IteratorHandle { iterator: value }
    }
}

impl Iterator for IteratorHandle<'_> {
    type Item = KeyValueItem;

    fn next(&mut self) -> Option<Self::Item> {
        self.iterator.next()
    }
}

#[derive(Debug)]
pub struct CreateIteratorResult<'db> {
    pub handle: IteratorHandle<'db>,
}
