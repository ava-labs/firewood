// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{LinearStoreParent, Proposed};

use super::{proposed::Immutable, ReadLinearStore, WriteLinearStore};
use std::{
    io::{Cursor, Read},
    sync::Arc,
};

#[derive(Debug, Clone, Default)]
/// An in-memory impelementation of [WriteLinearStore]
pub struct MemStore {
    bytes: Vec<u8>,
}

impl MemStore {
    /// Create a new, empty [MemStore]
    pub const fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}

impl WriteLinearStore for MemStore {
    fn write(&mut self, offset: u64, object: &[u8]) -> Result<usize, std::io::Error> {
        let offset = offset as usize;
        if offset + object.len() > self.bytes.len() {
            self.bytes.resize(offset + object.len(), 0);
        }
        self.bytes[offset..offset + object.len()].copy_from_slice(object);
        Ok(object.len())
    }

    fn freeze(self) -> Proposed<Immutable> {
        let parent = LinearStoreParent::MemBacked(Arc::new(self));
        Proposed::new(parent).freeze()
    }
}

impl ReadLinearStore for MemStore {
    fn stream_from(&self, addr: u64) -> Result<Box<dyn Read>, std::io::Error> {
        let bytes = self
            .bytes
            .get(addr as usize..)
            .unwrap_or_default()
            .to_owned();

        Ok(Box::new(Cursor::new(bytes)))
    }

    fn size(&self) -> Result<u64, std::io::Error> {
        Ok(self.bytes.len() as u64)
    }
}

#[allow(clippy::unwrap_used)]
#[cfg(test)]
mod test {
    use super::*;
    use test_case::test_case;

    #[test_case(&[(0,&[1, 2, 3])],(0,&[1, 2, 3]); "write to empty store")]
    #[test_case(&[(0,&[1, 2, 3])],(1,&[2, 3]); "read from middle of store")]
    #[test_case(&[(0,&[1, 2, 3])],(2,&[3]); "read from end of store")]
    #[test_case(&[(0,&[1, 2, 3])],(3,&[]); "read past end of store")]
    #[test_case(&[(0,&[1, 2, 3]),(3,&[4,5,6])],(0,&[1, 2, 3,4,5,6]); "write to end of store")]
    #[test_case(&[(0,&[1, 2, 3]),(0,&[4])],(0,&[4,2,3]); "overwrite start of store")]
    #[test_case(&[(0,&[1, 2, 3]),(1,&[4])],(0,&[1,4,3]); "overwrite middle of store")]
    #[test_case(&[(0,&[1, 2, 3]),(2,&[4])],(0,&[1,2,4]); "overwrite end of store")]
    #[test_case(&[(0,&[1, 2, 3]),(2,&[4,5])],(0,&[1,2,4,5]); "overwrite/extend end of store")]
    fn test_in_mem_write_linear_store(writes: &[(u64, &[u8])], expected: (u64, &[u8])) {
        let mut store = MemStore { bytes: vec![] };
        assert_eq!(store.size().unwrap(), 0);

        for write in writes {
            store.write(write.0, write.1).unwrap();
        }

        let mut reader = store.stream_from(expected.0).unwrap();
        let mut read_bytes = vec![];
        reader.read_to_end(&mut read_bytes).unwrap();
        assert_eq!(read_bytes, expected.1);
    }
}
