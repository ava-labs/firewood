// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use btree_range_map::{AnyRange, AsRange, RangeSet};
use std::num::NonZeroU64;
use std::ops::Bound;

use crate::LinearAddress;

pub(super) struct LinearAddressRangeSet {
    range_set: RangeSet<u64>,
}

impl LinearAddressRangeSet {
    pub(super) fn new() -> Self {
        Self {
            range_set: RangeSet::new(),
        }
    }

    pub(super) fn insert(&mut self, addr: LinearAddress, size: u64) {
        self.range_set.insert(addr.get()..addr.get() + size);
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
