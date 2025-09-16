// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#[derive(Clone)]
pub(crate) struct BytesIter<I> {
    iter: I,
}

impl<I: Iterator<Item = u8>> BytesIter<I> {
    pub const fn new(iter: I) -> Self {
        Self { iter }
    }
}

#[cfg(not(feature = "branch_factor_256"))]
impl<I: Iterator<Item = u8>> Iterator for BytesIter<I> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        let high = self.iter.next()?;
        let low = self.iter.next().unwrap_or(0);
        Some((high << 4) | (low & 0x0F))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let (low, high) = self.iter.size_hint();
        let low = low.div_ceil(2);
        let high = high.map(|h| h.div_ceil(2));
        (low, high)
    }
}

#[cfg(feature = "branch_factor_256")]
impl<I: Iterator<Item = u8>> Iterator for BytesIter<I> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}
