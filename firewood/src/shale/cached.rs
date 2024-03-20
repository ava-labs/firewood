// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::shale::{CachedView, LinearStore, SendSyncDerefMut, SpaceId};
use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
    sync::{Arc, RwLock},
};

use super::ShaleError;

// Purely volatile, dynamically allocated vector-based implementation for
// [CachedStore]. Allocates more space on `write` if original size isn't enough.
#[derive(Debug)]
pub struct InMemLinearStore {
    space: Arc<RwLock<Vec<u8>>>,
    id: SpaceId,
}

impl InMemLinearStore {
    pub fn new(size: u64, id: SpaceId) -> Self {
        let space = Arc::new(RwLock::new(vec![0; size as usize]));
        Self { space, id }
    }
}

impl LinearStore for InMemLinearStore {
    fn get_view(
        &self,
        offset: usize,
        length: u64,
    ) -> Option<Box<dyn CachedView<DerefReturn = Vec<u8>>>> {
        let length = length as usize;
        let size = offset + length;
        #[allow(clippy::unwrap_used)]
        let mut space = self.space.write().unwrap();

        // Increase the size if the request range exceeds the current limit.
        if size > space.len() {
            space.resize(size, 0);
        }

        Some(Box::new(InMemLinearStoreView {
            offset,
            length,
            mem: Self {
                space: self.space.clone(),
                id: self.id,
            },
        }))
    }

    fn get_shared(&self) -> Box<dyn SendSyncDerefMut<Target = dyn LinearStore>> {
        Box::new(InMemLinearStoreShared(Self {
            space: self.space.clone(),
            id: self.id,
        }))
    }

    fn write(&mut self, offset: usize, change: &[u8]) -> Result<(), ShaleError> {
        let length = change.len();
        let size = offset + length;

        #[allow(clippy::unwrap_used)]
        let mut space = self.space.write().unwrap();

        // Increase the size if the request range exceeds the current limit.
        if size > space.len() {
            space.resize(size, 0);
        }
        #[allow(clippy::indexing_slicing)]
        space[offset..offset + length].copy_from_slice(change);

        Ok(())
    }

    fn id(&self) -> SpaceId {
        self.id
    }

    fn is_writeable(&self) -> bool {
        true
    }
}

/// A range within an in-memory linear byte store.
#[derive(Debug)]
struct InMemLinearStoreView {
    /// The start of the range.
    offset: usize,
    /// The length of the range.
    length: usize,
    /// The underlying store.
    mem: InMemLinearStore,
}

struct InMemLinearStoreShared(InMemLinearStore);

impl Deref for InMemLinearStoreShared {
    type Target = dyn LinearStore;
    fn deref(&self) -> &(dyn LinearStore + 'static) {
        &self.0
    }
}

impl DerefMut for InMemLinearStoreShared {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl CachedView for InMemLinearStoreView {
    type DerefReturn = Vec<u8>;

    fn as_deref(&self) -> Self::DerefReturn {
        #[allow(clippy::indexing_slicing, clippy::unwrap_used)]
        self.mem.space.read().unwrap()[self.offset..self.offset + self.length].to_vec()
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_dynamic_mem() {
        let mut view = InMemLinearStoreShared(InMemLinearStore::new(2, 0));
        let mem = &mut *view;
        mem.write(0, &[1, 2]).unwrap();
        mem.write(0, &[3, 4]).unwrap();
        assert_eq!(mem.get_view(0, 2).unwrap().as_deref(), [3, 4]);
        mem.get_shared().write(0, &[5, 6]).unwrap();

        // capacity is increased
        mem.write(5, &[0; 10]).unwrap();

        // get a view larger than recent growth
        assert_eq!(mem.get_view(3, 20).unwrap().as_deref(), [0; 20]);
    }
}
