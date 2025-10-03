// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::ops::{Deref, DerefMut};

use derive_where::derive_where;
use firewood::v2::api::BoxKeyValueIter;

/// An opaque wrapper around a [`BoxKeyValueIter`].
#[derive(Default)]
#[derive_where(Debug)]
#[derive_where(skip_inner)]
pub struct IteratorHandle<'view>(Option<BoxKeyValueIter<'view>>);

impl<'view> From<BoxKeyValueIter<'view>> for IteratorHandle<'view> {
    fn from(value: BoxKeyValueIter<'view>) -> Self {
        IteratorHandle(Some(value))
    }
}

impl<'view> Deref for IteratorHandle<'view> {
    type Target = BoxKeyValueIter<'view>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for IteratorHandle<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Iterator for IteratorHandle<'_> {
    type Item = Result<(merkle::Key, merkle::Value), api::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let out = self.iterator.as_mut()?.next();
        if out.is_none() {
            // iterator exhausted; drop it so the NodeStore can be released
            self.iterator = None;
        }
        out
    }
}

#[derive(Debug, Default)]
pub struct CreateIteratorResult<'db>(pub IteratorHandle<'db>);
