// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::{
    layered::{Layer, LayeredReader},
    ImmutableLinearStore,
};

/// A linear store used for historical revisions
///
/// A [Historical] [LinearStore] supports read operations only
#[derive(Debug)]
pub(crate) struct Historical {
    /// (offset, value) for every area of this LinearStore that is modified in
    /// the revision after this one (i.e. `parent`).
    /// For example, if the first 3 bytes of this revision are [0,1,2] and the
    /// first 3 bytes of the next revision are [4,5,6] then this map would
    /// contain [(0, [0,1,2])].
    changed_in_parent: BTreeMap<u64, Box<[u8]>>,
    /// The state of the revision after this one.
    parent: Arc<ImmutableLinearStore>,
    size: u64,
}

impl Historical {
    pub(super) fn new(
        changed_in_parent: BTreeMap<u64, Box<[u8]>>,
        parent: Arc<ImmutableLinearStore>,
        size: u64,
    ) -> Self {
        Self {
            changed_in_parent,
            parent,
            size,
        }
    }

    pub(super) fn stream_from(
        &self,
        addr: u64,
    ) -> Result<Box<dyn std::io::Read + '_>, std::io::Error> {
        Ok(Box::new(LayeredReader::new(
            addr,
            Layer {
                parent: self.parent.clone(),
                diffs: &self.changed_in_parent,
            },
        )))
    }

    pub(super) fn size(&self) -> Result<u64, std::io::Error> {
        Ok(self.size)
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use crate::storage::linear::tests::ConstBacked;
//     use test_case::test_case;

//     #[test_case(&[0,1,2,3],&[(0,&[4,5,6])],&[4,5,6,3];"read diff, parent")]
//     #[test_case(&[0,1,2,3],&[(0,&[4,5,6,7])],&[4,5,6,7];"read diff only")]
//     #[test_case(&[0,1,2,3],&[(1,&[4,5,6])],&[0,4,5,6]; "read parent, diff")]
//     #[test_case(&[0,1,2,3,4,5],&[(1,&[6,7]),(4,&[8])],&[0,6,7,3,8,5]; "read parent, diff, parent, diff, parent")]
//     #[test_case(&[0,1,2,3,4,5],&[(0,&[6,7]),(3,&[8,9])],&[6,7,2,8,9,5]; "read diff, parent, diff, parent")]
//     fn test_historical_stream_from(
//         parent_state: &'static [u8],
//         diffs: &[(u64, &[u8])],
//         expected: &'static [u8],
//     ) {
//         let parent = ImmutableLinearStore:: {
//             state: ConstBacked::new(parent_state),
//         };

//         let mut changed_in_parent = BTreeMap::<u64, Box<[u8]>>::new();
//         for (addr, data) in diffs {
//             changed_in_parent.insert(*addr, data.to_vec().into_boxed_slice());
//         }

//         let historical =
//             Historical::new(changed_in_parent, Arc::new(parent), expected.len() as u64);

//         for i in 0..expected.len() {
//             let mut stream = historical.stream_from(i as u64).unwrap();
//             let mut read_bytes = Vec::new();
//             stream.read_to_end(&mut read_bytes).unwrap();
//             assert_eq!(read_bytes.as_slice(), &expected[i..]);
//         }
//     }
// }
