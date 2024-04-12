// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{proposed::LayeredReader, LinearStore, ReadLinearStore};

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
    fn stream_from(&self, addr: u64) -> Result<Box<dyn std::io::Read + '_>, std::io::Error> {
        Ok(Box::new(LayeredReader::new(addr, self.into())))
    }

    fn size(&self) -> Result<u64, std::io::Error> {
        todo!()
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::storage::linear::tests::ConstBacked;
    use test_case::test_case;

    #[test_case(&[];"empty parent")]
    #[test_case(&[0];"1 element in parent")]
    #[test_case(&[0,1,2];"multiple elements in parent")]
    fn test_historical_stream_from(parent_state: &'static [u8]) {
        let parent = LinearStore {
            state: ConstBacked::new(parent_state),
        };

        let historical = Historical::from_current(Default::default(), Arc::new(parent));

        let mut stream = historical.stream_from(0).unwrap();
        let mut read_bytes = Vec::new();
        let num_read_bytes = stream.read_to_end(&mut read_bytes).unwrap();
        assert_eq!(num_read_bytes, parent_state.len());
        assert_eq!(read_bytes.as_slice(), parent_state);
    }
}
