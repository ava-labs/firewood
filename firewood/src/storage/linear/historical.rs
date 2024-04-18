// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{
    layered::{Layer, LayeredReader},
    ReadLinearStore,
};

/// A linear store used for historical revisions
///
/// A [Historical] [LinearStore] supports read operations only
#[derive(Debug)]
pub(crate) struct Historical {
    /// (offset, value) for every area of this LinearStore modified in
    /// the revision after this one (i.e. `parent`).
    /// That is, what each area `was` in this revision.
    /// For example, if the first 3 bytes of this revision are `[0,1,2]` and the
    /// first 3 bytes of the next revision are `[4,5,6]` then this map contains
    /// `[(0, [0,1,2])]`.
    was: BTreeMap<u64, Box<[u8]>>,
    /// The state of the revision after this one.
    parent: Arc<dyn ReadLinearStore>,
    size: u64,
}

impl Historical {
    pub(super) fn new(
        was: BTreeMap<u64, Box<[u8]>>,
        parent: Arc<dyn ReadLinearStore>,
        size: u64,
    ) -> Self {
        Self { was, parent, size }
    }

    pub(super) fn stream_from(
        &self,
        addr: u64,
    ) -> Result<Box<dyn std::io::Read + '_>, std::io::Error> {
        Ok(Box::new(LayeredReader::new(
            addr,
            Layer::new(self.parent.clone(), &self.was),
        )))
    }

    pub(super) const fn size(&self) -> Result<u64, std::io::Error> {
        Ok(self.size)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::storage::linear::filebacked::tests::new_temp_filebacked;
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
        let parent = new_temp_filebacked(parent_state);

        let mut was = BTreeMap::<u64, Box<[u8]>>::new();
        for (addr, data) in diffs {
            was.insert(*addr, data.to_vec().into_boxed_slice());
        }

        let historical = Historical::new(was, Arc::new(parent), expected.len() as u64);

        for i in 0..expected.len() {
            let mut stream = historical.stream_from(i as u64).unwrap();
            let mut read_bytes = Vec::new();
            stream.read_to_end(&mut read_bytes).unwrap();
            assert_eq!(read_bytes.as_slice(), &expected[i..]);
        }
    }
}
