// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.
use bytes::{Bytes, BytesMut};
use std::{
    cell::RefCell,
    ops::{Deref, DerefMut},
};

use shale::{CachedStore, CachedView};

pub type SpaceID = u8;

/// Purely volatile, dynamically allocated vector-based implementation for [CachedStore]. This is similar to
/// PlainMem. The only difference is, when [write] dynamically allocate more space if original space is
/// not enough.
#[derive(Debug)]
pub struct DynamicBytes {
    space: RefCell<BytesMut>,
    id: SpaceID,
}

impl DynamicBytes {
    pub fn new(size: u64, id: SpaceID) -> Self {
        let space = RefCell::new(BytesMut::with_capacity(size as usize));
        Self { space, id }
    }
}

impl CachedStore for DynamicBytes {
    fn get_view(
        &self,
        offset: u64,
        length: u64,
    ) -> Option<Box<dyn CachedView<DerefReturn = Vec<u8>>>> {
        let offset = offset as usize;
        let length = length as usize;
        let size = offset + length;
        // Increase the size if the request range exceeds the current limit.
        if size > self.space.borrow().len() {
            self.space.borrow_mut().resize(size, 0);
        }
        Some(Box::new(DynamicBytesView {
            mem: Bytes::copy_from_slice(&self.space.borrow()[offset..size]),
        }))
    }

    fn get_shared(&self) -> Option<Box<dyn DerefMut<Target = dyn CachedStore>>> {
        Some(Box::new(DynamicMemShared(Self {
            space: self.space.clone(),
            id: self.id,
        })))
    }

    fn write(&mut self, offset: u64, change: &[u8]) {
        let offset = offset as usize;
        let length = change.len();
        let size = offset + length;
        // Increase the size if the request range exceeds the current limit.
        if size > self.space.borrow().len() {
            self.space.borrow_mut().resize(size, 0);
        }
        self.space.borrow_mut()[offset..offset + length].copy_from_slice(change)
    }

    fn id(&self) -> SpaceID {
        self.id
    }
}

#[derive(Debug)]
struct DynamicBytesView {
    mem: Bytes,
}

struct DynamicMemShared(DynamicBytes);

impl Deref for DynamicBytesView {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.mem.as_ref()
    }
}

impl Deref for DynamicMemShared {
    type Target = dyn CachedStore;
    fn deref(&self) -> &(dyn CachedStore + 'static) {
        &self.0
    }
}

impl DerefMut for DynamicMemShared {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl CachedView for DynamicBytesView {
    type DerefReturn = Vec<u8>;

    fn as_deref(&self) -> Self::DerefReturn {
        self.mem.as_ref().to_vec()
    }
}
#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn test_dynamic_bytes() {
        let mut db = DynamicBytes::new(4, 0);
        db.write(0, &[1, 2, 3, 4]);
        assert_eq!(db.get_view(0, 4).unwrap().as_deref(), vec![1, 2, 3, 4]);
        assert_eq!(db.space.borrow().len(), 4);

        // grow it
        db.write(4, &[5, 6, 7, 8]);
        assert_eq!(db.get_view(4, 4).unwrap().as_deref(), vec![5, 6, 7, 8]);
        assert_eq!(db.space.borrow().len(), 8);

        // zero fill grow
        db.write(9, &[10]);
        assert_eq!(db.get_view(8, 2).unwrap().as_deref(), vec![0, 10]);
    }
}
