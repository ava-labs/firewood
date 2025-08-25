// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood::merkle;
use firewood::v2::api::{self};
use std::fmt::{Debug, Formatter};

// This wrapper doesn't do anything special now, but we need to figure out how to handle
// proposal commits, drops, revision cleaning
// TODO(amin): figure this out

type KeyValueItem = Result<(merkle::Key, merkle::Value), api::Error>;

/// An opaque wrapper around an Iterator.
pub struct IteratorHandle<'db> {
    pub iterator: Box<dyn Iterator<Item = KeyValueItem> + 'db>,
}

impl Debug for IteratorHandle<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IteratorHandle")
            .finish()
    }
}

impl IteratorHandle<'_> {
    pub fn iter_next(&mut self) -> Option<KeyValueItem> {
        self.iterator.next()
    }
}

#[derive(Debug)]
pub struct CreateIteratorResult<'db> {
    pub handle: IteratorHandle<'db>,
}
