// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use derive_where::derive_where;
use firewood::merkle;
use firewood::v2::api::{self, ArcDynDbView, BoxKeyValueIter};
use std::iter::FusedIterator;

type KeyValueItem = (merkle::Key, merkle::Value);

/// An opaque wrapper around a [`BoxKeyValueIter`] and a reference
/// to the [`ArcDynDbView`] backing it, preventing the view from
/// being dropped while iteration is in progress.
#[derive(Default)]
#[derive_where(Debug)]
#[derive_where(skip_inner)]
pub struct IteratorHandle<'view> {
    // order is important to ensure the view is dropped after the iterator
    iter: Option<BoxKeyValueIter<'view>>,
    view: Option<ArcDynDbView>,
}

impl Iterator for IteratorHandle<'_> {
    type Item = Result<KeyValueItem, api::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let out = self.iter.as_mut()?.next();
        if out.is_none() {
            // iterator exhausted; drop it so the NodeStore can be released
            self.iter = None;
            self.view = None;
        }
        out.map(|res| res.map_err(api::Error::from))
    }
}

impl FusedIterator for IteratorHandle<'_> {}

#[expect(clippy::missing_errors_doc)]
impl<'view> IteratorHandle<'view> {
    pub fn iter_next_n(&mut self, n: usize) -> Result<Vec<KeyValueItem>, api::Error> {
        self.by_ref().take(n).collect()
    }

    pub fn new(view: ArcDynDbView, iter: BoxKeyValueIter<'view>) -> Self {
        IteratorHandle {
            iter: Some(iter),
            view: Some(view),
        }
    }
}

#[derive(Debug, Default)]
pub struct CreateIteratorResult<'db>(pub IteratorHandle<'db>);
