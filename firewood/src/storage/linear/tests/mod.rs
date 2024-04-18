// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::io::{Cursor, Error, Read};

use super::ReadLinearStore;

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
