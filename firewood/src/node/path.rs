// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use std::{
    fmt::{self, Debug}, io::{Cursor, Read}, iter::once
};

use crate::nibbles::NibblesIterator;

// TODO: use smallvec
/// Path is part or all of a node's path in the trie.
/// Each element is a nibble.
#[derive(PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Path(pub Vec<u8>);

impl Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        for nib in self.0.iter() {
            write!(f, "{:x}", *nib & 0xf)?;
        }
        Ok(())
    }
}

impl std::ops::Deref for Path {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl From<Vec<u8>> for Path {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

bitflags! {
    // should only ever be the size of a nibble
    struct Flags: u8 {
        const ODD_LEN  = 0b0001;
    }
}

impl Path {
    pub fn iter_encoded(&self) -> impl Iterator<Item = u8> + '_ {
        let mut flags = Flags::empty();

        let has_odd_len = self.0.len() & 1 == 1;

        let extra_byte = if has_odd_len {
            flags.insert(Flags::ODD_LEN);

            None
        } else {
            Some(0)
        };

        once(flags.bits())
            .chain(extra_byte)
            .chain(self.0.iter().copied())
    }
    pub(crate) fn encode(&self) -> Vec<u8> {
            self.iter_encoded()
            .collect()
    }

    /// Returns the decoded path.
    pub fn from_nibbles<const N: usize>(nibbles: NibblesIterator<'_, N>) -> Self {
        Self::from_iter(nibbles)
    }

    /// Assumes all bytes are nibbles, prefer to use `from_nibbles` instead.
    fn from_iter<Iter: Iterator<Item = u8>>(mut iter: Iter) -> Self {
        let flags = Flags::from_bits_retain(iter.next().unwrap_or_default());

        if !flags.contains(Flags::ODD_LEN) {
            let _ = iter.next();
        }

        Self(iter.collect())
    }
}
