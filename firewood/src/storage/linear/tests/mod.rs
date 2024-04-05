// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::io::{Cursor, Error, Read};
use std::sync::Arc;

use super::current::Current;
use super::proposed::{Immutable, Mutable, Proposed};
use super::{LinearStore, ReadLinearStore};

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
            Self::DATA
                .get(addr as usize..)
                .expect("invalid offset in test"),
        )))
    }

    fn size(&self) -> Result<u64, Error> {
        Ok(Self::DATA.len() as u64)
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
