// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use std::fmt::{self, Debug};

// TODO: use smallvec
/// Path is part or all of a node's path in the trie.
#[derive(PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Path {
    odd_nibble_length: bool,
    /// Each element is 2 nibbles (or 1 in the case of the final byte
    /// when `odd_nibble_length` is true).
    bytes: Vec<u8>,
}

impl Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        for nib in self.bytes.iter() {
            write!(f, "{:x}", *nib & 0xf)?;
        }
        Ok(())
    }
}

impl std::ops::Deref for Path {
    type Target = [u8]; // TODO danlaine: what type should this be?
    fn deref(&self) -> &[u8] {
        &self.bytes
    }
}

bitflags! {
    // should only ever be the size of a nibble
    struct Flags: u8 {
        const ODD_LEN  = 0b0001;
    }
}

impl Path {
    pub fn _new(bytes: Vec<u8>, odd_nibble_length: bool) -> Self {
        Self {
            bytes,
            odd_nibble_length,
        }
    }

    /// Creates a new Path from a slice of nibbles.
    pub fn from_nibbles(nibbles: &[u8]) -> Self {
        let num_bytes = nibbles.len() / 2 + 1;
        let mut bytes = Vec::with_capacity(num_bytes);
        for i in 0..num_bytes {
            let high = nibbles.get(i * 2).copied().unwrap_or(0);
            let low = nibbles.get(i * 2 + 1).copied().unwrap_or(0);
            bytes.push(high << 4 | low);
        }

        Self {
            bytes,
            odd_nibble_length: nibbles.len() % 2 == 1,
        }
    }
}
