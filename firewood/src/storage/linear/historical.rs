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
    pub(crate) parent_old: BTreeMap<u64, Box<[u8]>>,
    pub(crate) parent: Arc<LinearStore<P>>,
}

impl<P: ReadLinearStore> Historical<P> {
    pub(crate) fn from_current(
        parent_old: BTreeMap<u64, Box<[u8]>>,
        parent: Arc<LinearStore<P>>,
    ) -> Self {
        Self { parent_old, parent }
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

    #[test_case(&[0,1,2,3],&[(0,&[4,5,6])],&[4,5,6,3];"read diff, parent")]
    #[test_case(&[0,1,2,3],&[(0,&[4,5,6,7])],&[4,5,6,7];"read diff only")]
    #[test_case(&[0,1,2,3],&[(1,&[4,5,6])],&[0,4,5,6]; "read parent, diff")]
    #[test_case(&[0,1,2,3,4,5],&[(1,&[6,7]),(4,&[8])],&[0,6,7,3,8,5]; "read parent, diff, parent, diff, parent")]
    #[test_case(&[0,1,2,3,4,5],&[(0,&[6,7]),(3,&[8,9])],&[6,7,2,8,9,5]; "read diff, parent, diff, parent")]
    fn test_historical_stream_from(
        parent_state: &'static [u8],
        diffs: &[(u64, &[u8])],
        expected: &'static [u8],
    ) {
        let parent = LinearStore {
            state: ConstBacked::new(parent_state),
        };

        let mut diffs_map = BTreeMap::<u64, Box<[u8]>>::new();
        for (addr, data) in diffs {
            diffs_map.insert(*addr, data.to_vec().into_boxed_slice());
        }

        let historical = Historical::from_current(diffs_map, Arc::new(parent));

        for i in 0..expected.len() {
            let mut stream = historical.stream_from(i as u64).unwrap();
            let mut read_bytes = Vec::new();
            stream.read_to_end(&mut read_bytes).unwrap();
            assert_eq!(read_bytes.as_slice(), &expected[i..]);
        }
    }
}
