// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood::merkle;
use firewood::v2::api;
use std::fmt::{Debug, Formatter};

type KeyValueItem = (merkle::Key, merkle::Value);

/// An opaque wrapper around an Iterator.
#[derive(Default)]
pub struct IteratorHandle<'db> {
    pub iterator: Option<Box<dyn Iterator<Item = Result<KeyValueItem, api::Error>> + 'db>>,
}

impl Debug for IteratorHandle<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IteratorHandle").finish()
    }
}

#[expect(clippy::missing_errors_doc)]
impl IteratorHandle<'_> {
    pub fn iter_next(&mut self) -> Option<Result<KeyValueItem, api::Error>> {
        if let Some(iterator) = self.iterator.as_mut() {
            iterator.next()
        } else {
            None
        }
    }

    pub fn iter_next_n(&mut self, n: usize) -> Result<Vec<KeyValueItem>, api::Error> {
        let mut items = Vec::<KeyValueItem>::new();
        for _ in 0..n {
            let item = self.iter_next();
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
