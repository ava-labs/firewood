// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use btree_slab::BTreeMap;
use btree_slab::generic::map::BTreeExt;
use btree_slab::generic::node::Address;
use std::ops::Range;

use crate::nodestore::NodeStore;
use crate::{CheckerError, LinearAddress};

pub(super) struct RangeSet<T> {
    index: BTreeMap<T, T>, // start --> end
}

impl<T: Clone + Ord + std::fmt::Debug> RangeSet<T> {
    fn new() -> Self {
        Self {
            index: BTreeMap::new(),
        }
    }

    fn find_prev_and_next_addr(&self, key: &T) -> (Option<Address>, Option<Address>) {
        // btree_slab::BTreeMap::address_of returns:
        // - Err(nowhere) if the tree is empty
        // - Some(addr) where addr is the address of the key if it is present
        // - Err(addr) where addr is the address following the nearest key smaller than the given key if the key is not present
        // e.g.: If the key is 10 and the tree has 5 @ address 1 and 15 @ address 2, it will return Err(2) - we should return (Some(1), Some(2))
        let my_addr = self.index.address_of(key);
        match my_addr {
            Ok(addr) => (Some(addr), self.index.next_item_address(addr)), // if the key is present, merge with this range
            Err(addr) => {
                // tree is empty - this check is necessary since item(addr) will unwrap on none if the address is nowhere
                if addr.is_nowhere() {
                    return (None, None);
                }

                let prev_addr = self.index.previous_item_address(addr);
                let next_addr = match self.index.item(addr) {
                    Some(_) => Some(addr),
                    None => self.index.next_item_address(addr),
                };
                (prev_addr, next_addr)
            }
        }
    }

    fn intersects(&self, range: Range<T>) -> bool {
        let (prev_addr, next_addr) = self.find_prev_and_next_addr(&range.start);

        // check if the given range intersects with the previous range
        if let Some(prev_addr) = prev_addr {
            let prev_range = self.index.item(prev_addr).expect("prev item should exist");
            // if the end of previous range and the start of the current range are equal, they are not intersecting since end is exclusive
            if *prev_range.value() > range.start {
                return true;
            }
        }

        // check if the given range intersects with the next range
        if let Some(next_addr) = next_addr {
            let next_range = self.index.item(next_addr).expect("next item should exist");
            // if the end of current range and the start of the next range are equal, they are not intersecting since end is exclusive
            if range.end > *next_range.key() {
                return true;
            }
        }

        false
    }

    fn insert_range(&mut self, range: Range<T>) {
        let (prev_addr, mut next_addr) = self.find_prev_and_next_addr(&range.start);
        // this will be the final range after merging
        let (range_start, mut range_end) = match prev_addr {
            Some(addr) => {
                let prev_item = self.index.item(addr).expect("prev item should exist");
                if *prev_item.value() < range.start {
                    // previous range does not intersect with the new range
                    (range.start, range.end)
                } else if *prev_item.value() < range.end {
                    // previous range intersects with the new range partially
                    (prev_item.key().clone(), range.end)
                } else {
                    // previous range contains the new range
                    (prev_item.key().clone(), prev_item.value().clone())
                }
            }
            // this range starts before all ranges in the set
            None => (range.start, range.end),
        };

        // try merge with the next range
        let mut remove_keys = Vec::new();
        while let Some(curr_addr) = next_addr {
            let curr_item = self.index.item(curr_addr).expect("curr item should exist");
            if range_end < *curr_item.key() {
                // the next range does not intersect with the new range - we are done
                break;
            }
            // the next range intersects with the new range - remove the next range and update the end of the new range
            remove_keys.push(curr_item.key().clone());
            if range_end < *curr_item.value() {
                range_end = curr_item.value().clone();
            }
            // get the next range
            next_addr = self.index.next_item_address(curr_addr);
        }

        for key in remove_keys {
            self.index.remove(&key);
        }
        self.index.insert(range_start, range_end);
    }

    fn complement(&self, min: &T, max: &T) -> Self {
        let mut complement_tree = BTreeMap::new();
        // first range will start from min
        let mut start = min;
        for (cur_start, cur_end) in self.index.iter() {
            // insert the range from the previous end to the current start
            if cur_start > start {
                complement_tree.insert(start.clone(), cur_start.clone());
            }
            start = cur_end;
        }

        // insert the last range before the max
        if start < max {
            complement_tree.insert(start.clone(), max.clone());
        }

        Self {
            index: complement_tree,
        }
    }
}

impl IntoIterator for RangeSet<LinearAddress> {
    type Item = Range<LinearAddress>;
    type IntoIter = std::iter::Map<
        <BTreeMap<LinearAddress, LinearAddress> as IntoIterator>::IntoIter,
        fn((LinearAddress, LinearAddress)) -> Self::Item,
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.index
            .into_iter()
            .map(|(start, end)| Range { start, end })
    }
}

pub(super) struct LinearAddressRangeSet {
    range_set: RangeSet<LinearAddress>,
    max_addr: LinearAddress,
}

impl LinearAddressRangeSet {
    pub(super) fn new(db_size: u64) -> Result<Self, CheckerError> {
        let max_addr = LinearAddress::new(db_size).ok_or(CheckerError::InvalidDBSize(db_size))?;
        if max_addr < NodeStore::STORAGE_AREA_START {
            return Err(CheckerError::InvalidDBSize(db_size));
        }

        Ok(Self {
            range_set: RangeSet::new(),
            max_addr,
        })
    }

    pub(super) fn insert_area(
        &mut self,
        addr: LinearAddress,
        size: u64,
    ) -> Result<(), CheckerError> {
        let start = addr;
        let end = start
            .checked_add(size)
            .ok_or(CheckerError::AreaOutOfBounds { start, size })?; // This can only happen due to overflow
        if addr < NodeStore::STORAGE_AREA_START || end > self.max_addr {
            return Err(CheckerError::AreaOutOfBounds { start: addr, size });
        }

        if self.range_set.intersects(start..end) {
            return Err(CheckerError::AreaIntersects { start: addr, size });
        }
        self.range_set.insert_range(start..end);
        Ok(())
    }

    #[expect(clippy::unwrap_used)]
    pub(super) fn complement(&self) -> Self {
        let complement_set = self
            .range_set
            .complement(&NodeStore::STORAGE_AREA_START, &self.max_addr);

        Self {
            range_set: complement_set,
            max_addr: self.max_addr,
        }
    }
}

impl IntoIterator for LinearAddressRangeSet {
    type Item = Range<LinearAddress>;
    type IntoIter = <RangeSet<LinearAddress> as IntoIterator>::IntoIter;

    #[expect(clippy::unwrap_used)]
    fn into_iter(self) -> Self::IntoIter {
        self.range_set.into_iter()
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

        assert!(
            matches!(error, CheckerError::AreaIntersects { start, size } if start == start2_addr && size == size2)
        );

        // try inserting in opposite order
        let mut visited2 = LinearAddressRangeSet::new(0x1000).unwrap();
        visited2
            .insert_area(start2_addr, size2)
            .expect("the given area should be within bounds");

        let error = visited2
            .insert_area(start1_addr, size1)
            .expect_err("the given area should intersect with the first area");

        assert!(
            matches!(error, CheckerError::AreaIntersects { start, size } if start == start1_addr && size == size1)
        );
    }

    #[test]
    fn test_complement() {
        let start1 = 3000;
        let size1 = 1024;
        let start2 = 4096;
        let size2 = 1024;
        let db_size = 0x2000;

        let db_begin = NodeStore::STORAGE_AREA_START;
        let start1_addr = LinearAddress::new(start1).unwrap();
        let end1_addr = LinearAddress::new(start1 + size1).unwrap();
        let start2_addr = LinearAddress::new(start2).unwrap();
        let end2_addr = LinearAddress::new(start2 + size2).unwrap();
        let db_end = LinearAddress::new(db_size).unwrap();

        let mut visited = LinearAddressRangeSet::new(db_size).unwrap();
        visited.insert_area(start1_addr, size1).unwrap();
        visited.insert_area(start2_addr, size2).unwrap();

        let complement = visited.complement().into_iter().collect::<Vec<_>>();
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
        let complement = visited.complement().into_iter().collect::<Vec<_>>();
        assert_eq!(complement, vec![]);
    }

    #[test]
    fn test_complement_with_empty() {
        let db_size = NodeStore::STORAGE_AREA_START;
        let visited = LinearAddressRangeSet::new(db_size.get()).unwrap();
        let complement = visited.complement().into_iter().collect::<Vec<_>>();
        assert_eq!(complement, vec![]);
    }
}
