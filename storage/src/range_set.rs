// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![warn(clippy::pedantic)]

use btree_slab::BTreeMap;
use btree_slab::generic::map::BTreeExt;
use btree_slab::generic::node::Address;
use std::cmp::Ordering;
use std::ops::Range;

use crate::nodestore::STORAGE_AREA_START;
use crate::{CheckerError, LinearAddress};

pub struct RangeSet<T> {
    index: BTreeMap<T, T>, // start --> end
}

struct CoalescingRanges<'a, T> {
    prev_consecutive_range: Option<Range<&'a T>>,
    intersecting_ranges: Vec<Range<&'a T>>,
    next_consecutive_range: Option<Range<&'a T>>,
}

#[allow(dead_code)]
impl<T: Clone + Ord + std::fmt::Debug> RangeSet<T> {
    pub fn new() -> Self {
        Self {
            index: BTreeMap::new(),
        }
    }

    // returns the address of the previous and next item in the tree - we need to return next as well since prev may be None
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

    // returns disjoint ranges that will coalesce with the given range in ascending order
    fn get_coalescing_ranges(&self, range: &Range<T>) -> CoalescingRanges<'_, T> {
        let (prev_addr, mut next_addr) = self.find_prev_and_next_addr(&range.start);
        let mut prev_consecutive_range = None;
        let mut intersecting_ranges = Vec::new();
        let mut next_consecutive_range = None;

        // check if the previous range will coalesce with the given range
        if let Some(prev_addr) = prev_addr {
            let prev_range = self.index.item(prev_addr).expect("prev item should exist");
            match prev_range.value().cmp(&range.start) {
                Ordering::Less => {} // the previous range is does not intersect or is consecutive with the given range - it will not be coalesced
                Ordering::Equal => {
                    // the previous range does not intersect with the given range but is consecutive
                    prev_consecutive_range = Some(prev_range.key()..prev_range.value());
                }
                Ordering::Greater => {
                    // the previous range either intersects or is consecutive with the given range
                    intersecting_ranges.push(prev_range.key()..prev_range.value());
                }
            }
        }

        // check if the given range intersects with the next range
        while let Some(curr_addr) = next_addr {
            let curr_range = self.index.item(curr_addr).expect("next item should exist");
            match range.end.cmp(curr_range.key()) {
                Ordering::Less => break, // the current range is after the given range - we are done
                Ordering::Equal => {
                    // the current range does not intersect with the given range but is consecutive
                    next_consecutive_range = Some(curr_range.key()..curr_range.value());
                    break;
                }
                Ordering::Greater => {
                    // the current range intersects or is consecutive with the given range
                    intersecting_ranges.push(curr_range.key()..curr_range.value());
                }
            }
            next_addr = self.index.next_item_address(curr_addr);
        }

        CoalescingRanges {
            prev_consecutive_range,
            intersecting_ranges,
            next_consecutive_range,
        }
    }

    fn insert_range_helper(
        &mut self,
        range: Range<T>,
        allow_intersecting: bool,
    ) -> Result<(), Vec<Range<T>>> {
        // if the range is empty, do nothing
        if range.is_empty() {
            return Ok(());
        }

        let CoalescingRanges {
            prev_consecutive_range,
            intersecting_ranges,
            next_consecutive_range,
        } = self.get_coalescing_ranges(&range);

        // if the insert needs to be disjoint but we found intersecting ranges, return error with the intersection
        if !allow_intersecting && !intersecting_ranges.is_empty() {
            let intersections = intersecting_ranges
                .into_iter()
                .map(|intersecting_range| {
                    let start =
                        std::cmp::max(range.start.clone(), intersecting_range.start.clone());
                    let end = std::cmp::min(range.end.clone(), intersecting_range.end.clone());
                    start..end
                })
                .collect();
            return Err(intersections);
        }

        // find the new start and end after the coalescing
        let coalesced_start = match (&prev_consecutive_range, intersecting_ranges.first()) {
            (Some(prev_range), _) => prev_range.start.clone(),
            (None, Some(first_intersecting_range)) => {
                std::cmp::min(range.start, first_intersecting_range.start.clone())
            }
            (None, None) => range.start,
        };
        let coalesced_end = match (&next_consecutive_range, intersecting_ranges.last()) {
            (Some(next_range), _) => next_range.end.clone(),
            (None, Some(last_intersecting_range)) => {
                std::cmp::max(range.end, last_intersecting_range.end.clone())
            }
            (None, None) => range.end,
        };

        // remove the coalescing ranges
        let remove_ranges_iter = prev_consecutive_range
            .into_iter()
            .chain(intersecting_ranges)
            .chain(next_consecutive_range);
        let remove_keys = remove_ranges_iter
            .map(|range| range.start.clone())
            .collect::<Vec<_>>();
        for key in remove_keys {
            self.index.remove(&key);
        }

        // insert the new range after coalescing
        self.index.insert(coalesced_start, coalesced_end);
        Ok(())
    }

    // insert the range
    pub fn insert_range(&mut self, range: Range<T>) {
        self.insert_range_helper(range, true)
            .expect("insert range should always success if we allow intersecting area insert");
    }

    // insert the given range if the range is disjoint, otherwise return the intersection
    pub fn insert_disjoint_range(&mut self, range: Range<T>) -> Result<(), Vec<Range<T>>> {
        self.insert_range_helper(range, false)
    }

    pub fn complement(&self, min: &T, max: &T) -> Self {
        let mut complement_tree = BTreeMap::new();
        // first range will start from min
        let mut start = min;
        for (cur_start, cur_end) in &self.index {
            // insert the range from the previous end to the current start
            if cur_start > start {
                if cur_start >= max {
                    // we have reached the max - we are done
                    break;
                }
                complement_tree.insert(start.clone(), cur_start.clone());
            }
            start = std::cmp::max(start, cur_end); // in case the entire range is smaller than min
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

impl<T> IntoIterator for RangeSet<T> {
    type Item = Range<T>;
    type IntoIter =
        std::iter::Map<<BTreeMap<T, T> as IntoIterator>::IntoIter, fn((T, T)) -> Self::Item>;

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
        let max_addr = LinearAddress::new(db_size).ok_or(CheckerError::InvalidDBSize {
            db_size,
            description: String::from("db size cannot be 0"),
        })?;
        if max_addr < STORAGE_AREA_START {
            return Err(CheckerError::InvalidDBSize {
                db_size,
                description: format!(
                    "db size should not be smaller than the header size ({STORAGE_AREA_START})"
                ),
            });
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
            .ok_or(CheckerError::AreaOutOfBounds {
                start,
                size,
                bounds: STORAGE_AREA_START..self.max_addr,
            })?; // This can only happen due to overflow
        if addr < STORAGE_AREA_START || end > self.max_addr {
            return Err(CheckerError::AreaOutOfBounds {
                start: addr,
                size,
                bounds: STORAGE_AREA_START..self.max_addr,
            });
        }

        if let Err(intersection) = self.range_set.insert_disjoint_range(start..end) {
            return Err(CheckerError::AreaIntersects {
                start: addr,
                size,
                intersection,
            });
        }
        Ok(())
    }

    pub(super) fn complement(&self) -> Self {
        let complement_set = self
            .range_set
            .complement(&STORAGE_AREA_START, &self.max_addr);

        Self {
            range_set: complement_set,
            max_addr: self.max_addr,
        }
    }
}

impl IntoIterator for LinearAddressRangeSet {
    type Item = Range<LinearAddress>;
    type IntoIter = <RangeSet<LinearAddress> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.range_set.into_iter()
    }
}

#[cfg(test)]
mod test_range_set {
    use super::*;

    #[test]
    fn test_create() {
        let range_set: RangeSet<u64> = RangeSet::new();
        assert_eq!(range_set.into_iter().collect::<Vec<_>>(), vec![]);
    }

    #[test]
    fn test_insert_range() {
        let mut range_set = RangeSet::new();
        range_set.insert_range(0..10);
        range_set.insert_range(20..30);
        assert_eq!(
            range_set.into_iter().collect::<Vec<_>>(),
            vec![0..10, 20..30]
        );
    }

    #[test]
    #[allow(clippy::reversed_empty_ranges)]
    fn test_insert_empty_range() {
        let mut range_set = RangeSet::new();
        range_set.insert_range(0..0);
        range_set.insert_range(10..10);
        range_set.insert_range(20..10);
        range_set.insert_range(30..0);
        assert_eq!(range_set.into_iter().collect::<Vec<_>>(), vec![]);
    }

    #[test]
    fn test_insert_range_coalescing() {
        // coalesce ranges that are disjoint
        let mut range_set = RangeSet::new();
        range_set.insert_range(0..10);
        range_set.insert_range(20..30);
        range_set.insert_range(10..20);
        assert_eq!(range_set.into_iter().collect::<Vec<_>>(), vec![0..30]);

        // coalesce ranges that are partially overlapping
        let mut range_set = RangeSet::new();
        range_set.insert_range(0..10);
        range_set.insert_range(20..30);
        range_set.insert_range(5..25);
        assert_eq!(range_set.into_iter().collect::<Vec<_>>(), vec![0..30]);

        // coalesce multiple ranges
        let mut range_set = RangeSet::new();
        range_set.insert_range(5..10);
        range_set.insert_range(15..20);
        range_set.insert_range(25..30);
        range_set.insert_range(0..25);
        assert_eq!(range_set.into_iter().collect::<Vec<_>>(), vec![0..30]);
    }

    #[test]
    fn test_insert_disjoint_range() {
        // insert disjoint ranges
        let mut range_set = RangeSet::new();
        assert!(range_set.insert_disjoint_range(0..10).is_ok());
        assert!(range_set.insert_disjoint_range(20..30).is_ok());
        assert!(range_set.insert_disjoint_range(12..18).is_ok());
        assert_eq!(
            range_set.into_iter().collect::<Vec<_>>(),
            vec![0..10, 12..18, 20..30]
        );

        // insert consecutive ranges
        let mut range_set = RangeSet::new();
        assert!(range_set.insert_disjoint_range(0..10).is_ok());
        assert!(range_set.insert_disjoint_range(20..30).is_ok());
        assert!(range_set.insert_disjoint_range(10..20).is_ok());
        assert_eq!(range_set.into_iter().collect::<Vec<_>>(), vec![0..30]);

        // insert intersecting ranges
        let mut range_set = RangeSet::new();
        assert!(range_set.insert_disjoint_range(0..10).is_ok());
        assert!(range_set.insert_disjoint_range(20..30).is_ok());
        assert!(matches!(
            range_set.insert_disjoint_range(5..25),
            Err(intersections) if intersections == vec![5..10, 20..25]
        ));
        assert_eq!(
            range_set.into_iter().collect::<Vec<_>>(),
            vec![0..10, 20..30]
        );

        // insert with completely overlapping range
        let mut range_set = RangeSet::new();
        assert!(range_set.insert_disjoint_range(0..10).is_ok());
        assert!(range_set.insert_disjoint_range(20..30).is_ok());
        assert!(range_set.insert_disjoint_range(40..50).is_ok());
        assert!(matches!(
            range_set.insert_disjoint_range(5..45),
            Err(intersections) if intersections == vec![5..10, 20..30, 40..45]
        ));
        assert_eq!(
            range_set.into_iter().collect::<Vec<_>>(),
            vec![0..10, 20..30, 40..50]
        );
    }

    #[test]
    fn test_complement_with_empty() {
        let range_set = RangeSet::new();
        assert_eq!(
            range_set
                .complement(&0, &40)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![0..40]
        );
    }

    #[test]
    fn test_complement_with_different_bounds() {
        let mut range_set = RangeSet::new();
        range_set.insert_range(0..10);
        range_set.insert_range(20..30);
        range_set.insert_range(40..50);

        // all ranges within bound
        assert_eq!(
            range_set
                .complement(&0, &60)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![10..20, 30..40, 50..60]
        );

        // some ranges entirely out of bound
        assert_eq!(
            range_set
                .complement(&15, &35)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![15..20, 30..35]
        );

        // some ranges are partially out of bound
        assert_eq!(
            range_set
                .complement(&5, &45)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![10..20, 30..40]
        );

        // test all ranges out of bound
        assert_eq!(
            range_set
                .complement(&12, &18)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![12..18]
        );

        // test complement empty
        assert_eq!(
            range_set
                .complement(&20, &30)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![]
        );

        // test boundaries at range start and end
        assert_eq!(
            range_set
                .complement(&0, &30)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![10..20]
        );

        // test with equal bounds
        assert_eq!(
            range_set
                .complement(&35, &35)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![]
        );

        // test with reversed bounds
        assert_eq!(
            range_set
                .complement(&30, &0)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![]
        );
    }

    #[test]
    fn test_many_small_ranges() {
        let mut range_set = RangeSet::new();
        // Insert many small ranges that will coalesce
        for i in 0..1000 {
            range_set.insert_range(i * 10..(i * 10 + 5));
        }
        // Should result in 1000 separate single-element ranges
        assert_eq!(range_set.into_iter().count(), 1000);
    }

    #[test]
    fn test_large_range_coalescing() {
        let mut range_set = RangeSet::new();
        // Insert ranges that will eventually all coalesce into one
        for i in 0..1000 {
            range_set.insert_range(i * 10..(i * 10 + 5));
        }
        // Fill the gaps
        range_set.insert_range(0..10000);
        assert_eq!(range_set.into_iter().collect::<Vec<_>>(), vec![0..10000]);
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod test_linear_address_range_set {

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
        let end1_addr = LinearAddress::new(start1 + size1).unwrap();
        let start2_addr = LinearAddress::new(start2).unwrap();

        let mut visited = LinearAddressRangeSet::new(0x1000).unwrap();
        visited
            .insert_area(start1_addr, size1)
            .expect("the given area should be within bounds");

        let error = visited
            .insert_area(start2_addr, size2)
            .expect_err("the given area should intersect with the first area");

        assert!(
            matches!(error, CheckerError::AreaIntersects { start, size, intersection } if start == start2_addr && size == size2 && intersection == vec![start2_addr..end1_addr])
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
            matches!(error, CheckerError::AreaIntersects { start, size, intersection } if start == start1_addr && size == size1 && intersection == vec![start2_addr..end1_addr])
        );
    }

    #[test]
    fn test_complement() {
        let start1 = 3000;
        let size1 = 1024;
        let start2 = 4096;
        let size2 = 1024;
        let db_size = 0x2000;

        let db_begin = STORAGE_AREA_START;
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
        let db_size = STORAGE_AREA_START;
        let visited = LinearAddressRangeSet::new(db_size.get()).unwrap();
        let complement = visited.complement().into_iter().collect::<Vec<_>>();
        assert_eq!(complement, vec![]);
    }
}
