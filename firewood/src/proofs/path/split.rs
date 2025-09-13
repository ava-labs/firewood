// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::{Nibbles, SplitNibbles};

pub(in crate::proofs) struct SplitPath<L, R = L> {
    pub common_prefix: L,
    pub lhs_suffix: L,
    pub rhs_suffix: R,
}

impl<L, R> SplitPath<L, R> {
    pub fn new(lhs: L, rhs: R) -> Self
    where
        L: SplitNibbles + Copy,
        R: SplitNibbles + Copy,
    {
        let mid = lhs
            .nibbles_iter()
            .zip(rhs.nibbles_iter())
            .take_while(|&(a, b)| a == b)
            .count();
        let (common_prefix, lhs_suffix) = lhs.split_at(mid);
        let (_, rhs_suffix) = rhs.split_at(mid);
        Self {
            common_prefix,
            lhs_suffix,
            rhs_suffix,
        }
    }
}

impl<L: Nibbles, R: Nibbles> std::fmt::Display for SplitPath<L, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "common: {}, lhs: {}, rhs: {}",
            &self.common_prefix.display(),
            &self.lhs_suffix.display(),
            &self.rhs_suffix.display(),
        )
    }
}
