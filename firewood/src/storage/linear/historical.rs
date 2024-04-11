// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{LinearStore, ReadLinearStore};

/// A linear store used for historical revisions
///
/// A [Historical] [LinearStore] supports read operations only
#[derive(Debug)]
pub(crate) struct Historical<P: ReadLinearStore> {
    pub(crate) old: BTreeMap<u64, Box<[u8]>>,
    pub(crate) parent: Arc<LinearStore<P>>,
}

impl<P: ReadLinearStore> Historical<P> {
    pub(crate) fn from_current(old: BTreeMap<u64, Box<[u8]>>, parent: Arc<LinearStore<P>>) -> Self {
        Self { old, parent }
    }
}

impl<P: ReadLinearStore> ReadLinearStore for Historical<P> {
    fn stream_from(&self, _addr: u64) -> Result<Box<dyn std::io::Read + '_>, std::io::Error> {
        todo!()
    }

    fn size(&self) -> Result<u64, std::io::Error> {
        todo!()
    }
}
