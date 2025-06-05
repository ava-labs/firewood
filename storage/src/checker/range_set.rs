// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use btree_range_map::{AnyRange, AsRange, RangeSet};
use std::num::NonZeroU64;
use std::ops::Bound;

use crate::nodestore::NodeStoreHeader;
use crate::{CheckerError, LinearAddress};

pub(super) struct LinearAddressRangeSet {
    range_set: RangeSet<u64>,
    size: u64,
}

impl LinearAddressRangeSet {
    pub(super) fn new(file_size: u64) -> Result<Self, CheckerError> {
        if file_size < NodeStoreHeader::SIZE {
            return Err(CheckerError::InvalidFileSize(file_size));
        }

        Ok(Self {
            range_set: RangeSet::new(),
            size: file_size,
        })
    }

    pub(super) fn insert_area(
        &mut self,
        addr: LinearAddress,
        size: u64,
    ) -> Result<(), CheckerError> {
        let start = addr.get();
        // end can be larger than the file size: only the node in the stored area is written to disk
        if start < NodeStoreHeader::SIZE || start > self.size {
            return Err(CheckerError::AreaOutOfBounds { start: addr, size });
        }
        if self.range_set.intersects(start..start + size) {
            return Err(CheckerError::AreaIntersects { start: addr, size });
        }
        self.range_set.insert(start..start + size);
        Ok(())
    }
}

impl IntoIterator for LinearAddressRangeSet {
    type Item = (LinearAddress, u64);
    type IntoIter = std::iter::Map<
        <RangeSet<u64> as IntoIterator>::IntoIter,
        fn(AnyRange<u64>) -> (LinearAddress, u64),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.range_set.into_iter().map(|range| {
            // TODO: this is a hack to get the start and end of the range
            let Bound::Included(start) = range.start() else {
                panic!("start of range is not included, this should not happen");
            };
            let Bound::Excluded(end) = range.end() else {
                panic!("end of range is not excluded, this should not happen");
            };
            let start_addr =
                NonZeroU64::new(*start).expect("found zero address, this should not happen");
            (start_addr, end - start)
        })
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {

    use super::*;

    #[test]
    fn test_empty() {
        let visited = LinearAddressRangeSet::new(0x1000).expect("this will not panic");
        assert_eq!(visited.into_iter().collect::<Vec<_>>(), vec![]);
    }

    #[test]
    fn test_insert_area() {
        let mut visited = LinearAddressRangeSet::new(0x1000).expect("this will not panic");
        let start = LinearAddress::new(2048).expect("this will not panic");
        let size = 1024;

        visited
            .insert_area(start, size)
            .expect("the given area should be within bounds");

        assert_eq!(visited.into_iter().collect::<Vec<_>>(), vec![(start, size)]);
    }

    #[test]
    fn test_consecutive_areas_merge() {
        let mut visited = LinearAddressRangeSet::new(0x1000).expect("this will not panic");
        let start1 = 2048;
        let size1 = 1024;
        let start2 = start1 + size1;
        let size2 = 1024;

        let start1_addr = LinearAddress::new(start1).expect("this will not panic");
        let start2_addr = LinearAddress::new(start2).expect("this will not panic");

        visited
            .insert_area(start1_addr, size1)
            .expect("the given area should be within bounds");

        visited
            .insert_area(start2_addr, size2)
            .expect("the given area should be within bounds");

        assert_eq!(
            visited.into_iter().collect::<Vec<_>>(),
            vec![(start1_addr, size1 + size2)]
        );
    }

    #[test]
    fn test_intersecting_areas_will_fail() {
        let start1 = 2048;
        let size1 = 1024;
        let start2 = start1 + size1 - 1;
        let size2 = 1024;

        let start1_addr = LinearAddress::new(start1).expect("this will not panic");
        let start2_addr = LinearAddress::new(start2).expect("this will not panic");

        let mut visited = LinearAddressRangeSet::new(0x1000).expect("this will not panic");
        visited
            .insert_area(start1_addr, size1)
            .expect("the given area should be within bounds");

        let error = visited
            .insert_area(start2_addr, size2)
            .expect_err("the given area should intersect with the first area");

        let CheckerError::AreaIntersects {
            start: err_start2_addr,
            size: err_size2,
        } = error
        else {
            panic!("the error should be an AreaIntersects error");
        };

        assert_eq!(err_start2_addr, start2_addr);
        assert_eq!(err_size2, size2);

        // try inserting in opposite order
        let mut visited2 = LinearAddressRangeSet::new(0x1000).expect("this will not panic");
        visited2
            .insert_area(start2_addr, size2)
            .expect("the given area should be within bounds");

        let error = visited2
            .insert_area(start1_addr, size1)
            .expect_err("the given area should intersect with the first area");

        let CheckerError::AreaIntersects {
            start: err_start1_addr,
            size: err_size1,
        } = error
        else {
            panic!("the error should be an AreaIntersects error");
        };

        assert_eq!(err_start1_addr, start1_addr);
        assert_eq!(err_size1, size1);
    }
}
