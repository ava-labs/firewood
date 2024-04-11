// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::historical::Historical;
use super::{LinearStore, ReadLinearStore};

/// A linear store used for the current revision. The difference betwen this
/// and a historical revision is that it has a mutable parent. It is reparented
/// when another proposal commits
///
/// A [Current] [LinearStore] supports read operations only
#[derive(Debug)]
pub(crate) struct Current<P: ReadLinearStore> {
    old: BTreeMap<u64, Box<[u8]>>,
    parent: Mutex<Arc<LinearStore<P>>>,
}

impl<P: ReadLinearStore> Current<P> {
    pub(crate) fn new(parent: Arc<LinearStore<P>>) -> Self {
        Self {
            parent: Mutex::new(parent),
            old: Default::default(),
        }
    }

    pub(crate) fn reparent(self, parent: Arc<LinearStore<P>>) -> Historical<P> {
        Historical::from_current(self.old, parent)
    }
}

impl<P: ReadLinearStore> ReadLinearStore for Current<P> {
    fn stream_from(
        &self,
        addr: u64,
    ) -> Result<Box<dyn std::io::prelude::Read + '_>, std::io::Error> {
        let parent = self.parent.lock().expect("poisoned lock").clone();
        Ok(Box::new(CurrentStream { parent, addr }))
    }

    fn size(&self) -> Result<u64, std::io::Error> {
        todo!()
    }
}

struct CurrentStream<P: ReadLinearStore> {
    parent: Arc<LinearStore<P>>,
    addr: u64,
}

impl<P: ReadLinearStore> std::io::prelude::Read for CurrentStream<P> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.parent.stream_from(self.addr)?.read(buf)
    }
}
