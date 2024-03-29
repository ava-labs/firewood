// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::Flags;
use crate::nibbles::NibblesIterator;
use std::{
    fmt::{self, Debug},
    iter::once,
};

// TODO: use smallvec
/// Path is part or all of a node's path in the trie.
/// Each element is a nibble.
#[derive(PartialEq, Eq, Clone)]
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

impl Path {
    pub fn into_inner(self) -> Vec<u8> {
        self.0
    }

    pub(crate) fn encode(&self) -> Vec<u8> {
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
            .collect()
    }

    // TODO: remove all non `Nibbles` usages and delete this function.
    // I also think `Path` could probably borrow instead of own data.
    //
    /// Returns the decoded path.
    pub fn decode(raw: &[u8]) -> Self {
        Self::from_iter(raw.iter().copied())
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

    pub(super) fn serialized_len(&self) -> u64 {
        let len = self.0.len();

        // if len is even the prefix takes an extra byte
        // otherwise is combined with the first nibble
        let len = if len & 1 == 1 {
            (len + 1) / 2
        } else {
            len / 2 + 1
        };

        len as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test_case(&[1, 2, 3, 4])]
    #[test_case(&[1, 2, 3])]
    #[test_case(&[0, 1, 2])]
    #[test_case(&[1, 2])]
    #[test_case(&[1])]
    fn test_encoding(steps: &[u8]) {
        let path = Path(steps.to_vec());
        let encoded = path.encode();

        assert_eq!(encoded.len(), path.serialized_len() as usize * 2);

        let decoded = Path::decode(&encoded);

        assert_eq!(&&*decoded, &steps);
    }
}
