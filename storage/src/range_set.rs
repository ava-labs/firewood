// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use btree_range_map::{AnyRange, RangeSet};
use std::ops::{Bound, Range};

use crate::nodestore::NodeStore;
use crate::{CheckerError, LinearAddress};

pub(super) struct LinearAddressRangeSet {
    range_set: RangeSet<u64>,
    size: u64,
}

impl LinearAddressRangeSet {
    pub(super) fn new(db_size: u64) -> Result<Self, CheckerError> {
        if db_size < NodeStore::HEADER_SIZE {
            return Err(CheckerError::InvalidDBSize(db_size));
        }

        Ok(Self {
            range_set: RangeSet::new(),
            size: db_size,
        })
    }

    pub(super) fn insert_area(
        &mut self,
        addr: LinearAddress,
        size: u64,
    ) -> Result<(), CheckerError> {
        let start = addr.get();
        // end can be larger than the file size: only the node in the stored area is written to disk
        if start < NodeStore::HEADER_SIZE || start + size > self.size {
            return Err(CheckerError::AreaOutOfBounds { start: addr, size });
        }
        if self.range_set.intersects(start..start + size) {
            return Err(CheckerError::AreaIntersects { start: addr, size });
        }
        self.range_set.insert(start..start + size);
        Ok(())
    }

    #[expect(clippy::unwrap_used)]
    pub(super) fn complement(self) -> Vec<Range<LinearAddress>> {
        let mut complement = Vec::new();

        let mut cur_start = LinearAddress::new(NodeStore::HEADER_SIZE).unwrap();
        let db_end = LinearAddress::new(self.size).unwrap();

        for Range { start, end } in self.into_iter() {
            // skip empty ranges - this may happen for the first range
            debug_assert!(cur_start <= start);
            if cur_start < start {
                complement.push(cur_start..start);
            }
            cur_start = end;
        }

        if cur_start != db_end {
            complement.push(cur_start..db_end);
        }

        complement
    }
}

impl IntoIterator for LinearAddressRangeSet {
    type Item = Range<LinearAddress>;
    type IntoIter =
        std::iter::Map<<RangeSet<u64> as IntoIterator>::IntoIter, fn(AnyRange<u64>) -> Self::Item>;

    #[expect(clippy::unwrap_used)]
    fn into_iter(self) -> Self::IntoIter {
        self.range_set.into_iter().map(|AnyRange { start, end }| {
            let Bound::Included(start) = start else {
                panic!("start of range is not included, this should not happen");
            };
            let Bound::Excluded(end) = end else {
                panic!("end of range is not excluded, this should not happen");
            };
            Range {
                start: LinearAddress::new(start).unwrap(),
                end: LinearAddress::new(end).unwrap(),
            }
        })
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {

    use super::*;

    #[test]
    fn test_empty() {
        let visited = LinearAddressRangeSet::new(0x1000).unwrap();
        let visited_ranges = visited
            .into_iter()
            .map(|range| (range.start, range.end))
            .collect::<Vec<_>>();
        assert_eq!(visited_ranges, vec![]);
    }

    #[test]
    fn test_insert_area() {
        let start = 2048;
        let size = 1024;

        let start_addr = LinearAddress::new(start).unwrap();
        let end_addr = LinearAddress::new(start + size).unwrap();

        let mut visited = LinearAddressRangeSet::new(0x1000).unwrap();
        visited
            .insert_area(start_addr, size)
            .expect("the given area should be within bounds");

        let visited_ranges = visited
            .into_iter()
            .map(|range| (range.start, range.end))
            .collect::<Vec<_>>();
        assert_eq!(visited_ranges, vec![(start_addr, end_addr)]);
    }

    #[test]
    fn test_consecutive_areas_merge() {
        let start1 = 2048;
        let size1 = 1024;
        let start2 = start1 + size1;
        let size2 = 1024;

        let start1_addr = LinearAddress::new(start1).unwrap();
        let start2_addr = LinearAddress::new(start2).unwrap();
        let end2_addr = LinearAddress::new(start2 + size2).unwrap();

        let mut visited = LinearAddressRangeSet::new(0x1000).unwrap();
        visited
            .insert_area(start1_addr, size1)
            .expect("the given area should be within bounds");

        visited
            .insert_area(start2_addr, size2)
            .expect("the given area should be within bounds");

        let visited_ranges = visited
            .into_iter()
            .map(|range| (range.start, range.end))
            .collect::<Vec<_>>();
        assert_eq!(visited_ranges, vec![(start1_addr, end2_addr),]);
    }

    #[test]
    fn test_intersecting_areas_will_fail() {
        let start1 = 2048;
        let size1 = 1024;
        let start2 = start1 + size1 - 1;
        let size2 = 1024;

        let start1_addr = LinearAddress::new(start1).unwrap();
        let start2_addr = LinearAddress::new(start2).unwrap();

        let mut visited = LinearAddressRangeSet::new(0x1000).unwrap();
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
        let mut visited2 = LinearAddressRangeSet::new(0x1000).unwrap();
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

    #[test]
    fn test_complement() {
        let start1 = 3000;
        let size1 = 1024;
        let start2 = 4096;
        let size2 = 1024;
        let db_size = 0x2000;

        let db_begin = LinearAddress::new(NodeStore::HEADER_SIZE).unwrap();
        let start1_addr = LinearAddress::new(start1).unwrap();
        let end1_addr = LinearAddress::new(start1 + size1).unwrap();
        let start2_addr = LinearAddress::new(start2).unwrap();
        let end2_addr = LinearAddress::new(start2 + size2).unwrap();
        let db_end = LinearAddress::new(db_size).unwrap();

        let mut visited = LinearAddressRangeSet::new(db_size).unwrap();
        visited.insert_area(start1_addr, size1).unwrap();
        visited.insert_area(start2_addr, size2).unwrap();

        let complement = visited.complement();
        assert_eq!(
            complement,
            vec![
                db_begin..start1_addr,
                end1_addr..start2_addr,
                end2_addr..db_end,
            ]
        );
    }

    #[test]
    fn test_complement_with_full_range() {
        let db_size = 0x1000;
        let start = 2048;
        let size = db_size - start;

        let mut visited = LinearAddressRangeSet::new(db_size).unwrap();
        visited
            .insert_area(LinearAddress::new(start).unwrap(), size)
            .unwrap();
        let complement = visited.complement();
        assert_eq!(complement, vec![]);
    }
}
