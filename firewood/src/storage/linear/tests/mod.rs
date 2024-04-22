// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::io::{Cursor, Error, Read};
use std::sync::Arc;

use super::current::Current;
use super::proposed::{Immutable, Mutable, Proposed};
use super::{LinearStore, ReadLinearStore, WriteLinearStore};
use test_case::test_case;

#[derive(Debug)]
pub(crate) struct ConstBacked {
    data: &'static [u8],
}

impl ConstBacked {
    pub(crate) const DATA: &'static [u8] = b"random data";

    pub(crate) const fn new(data: &'static [u8]) -> Self {
        Self { data }
    }
}

impl From<ConstBacked> for Arc<LinearStore<ConstBacked>> {
    fn from(state: ConstBacked) -> Self {
        Arc::new(LinearStore { state })
    }
}

impl From<ConstBacked> for Proposed<ConstBacked, Mutable> {
    fn from(value: ConstBacked) -> Self {
        Proposed::new(value.into())
    }
}

impl From<ConstBacked> for Proposed<ConstBacked, Immutable> {
    fn from(value: ConstBacked) -> Self {
        Proposed::new(value.into())
    }
}

impl ReadLinearStore for ConstBacked {
    fn stream_from(&self, addr: u64) -> Result<Box<dyn Read>, std::io::Error> {
        Ok(Box::new(Cursor::new(
            self.data.get(addr as usize..).unwrap_or(&[]),
        )))
    }

    fn size(&self) -> Result<u64, Error> {
        Ok(self.data.len() as u64)
    }
}

#[test]
fn reparent() {
    let base = Arc::new(LinearStore {
        state: ConstBacked::new(ConstBacked::DATA),
    });
    let current = Arc::new(LinearStore {
        state: Current::new(base),
    });
    let _proposal = Arc::new(LinearStore {
        state: Proposed::<_, Mutable>::new(current),
    });

    // TODO:
    // proposal becomes Arc<LinearStore<Current<ConstBacked>>>
    // current becomes Arc<LinearStore<ConstBacked>>
}

#[derive(Debug)]
pub struct InMemReadWriteLinearStore {
    bytes: Vec<u8>,
}

impl InMemReadWriteLinearStore {
    pub const fn new() -> Self {
        Self { bytes: vec![] }
    }
}

impl WriteLinearStore for InMemReadWriteLinearStore {
    fn write(&mut self, offset: u64, object: &[u8]) -> Result<usize, std::io::Error> {
        let offset = offset as usize;
        if offset + object.len() > self.bytes.len() {
            self.bytes.resize(offset + object.len(), 0);
        }
        self.bytes[offset..offset + object.len()].copy_from_slice(object);
        Ok(object.len())
    }
}

impl ReadLinearStore for InMemReadWriteLinearStore {
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
    let mut store = InMemReadWriteLinearStore { bytes: vec![] };
    assert_eq!(store.size().unwrap(), 0);

    for write in writes {
        store.write(write.0, write.1).unwrap();
    }

    let mut reader = store.stream_from(expected.0).unwrap();
    let mut read_bytes = vec![];
    reader.read_to_end(&mut read_bytes).unwrap();
    assert_eq!(read_bytes, expected.1);
}
