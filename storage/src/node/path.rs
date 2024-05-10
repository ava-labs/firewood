// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// TODO: remove bitflags, we only use one bit
use bitflags::bitflags;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::{
    fmt::{self, Debug},
    iter::once,
};

/// Path is part or all of a node's path in the trie.
/// Each element is a nibble.
#[derive(PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Path(pub SmallVec<[u8; 32]>);

impl Debug for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        for nib in self.0.iter() {
            if *nib > 0xf {
                write!(f, "[invalid {:02x}] ", *nib)?;
            } else {
                write!(f, "{:x} ", *nib)?;
            }
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

impl<T: AsRef<[u8]>> From<T> for Path {
    fn from(value: T) -> Self {
        Self(SmallVec::from_slice(value.as_ref()))
    }
}

bitflags! {
    // should only ever be the size of a nibble
    struct Flags: u8 {
        const ODD_LEN  = 0b0001;
    }
}

impl Path {
    /// Return an iterator over the encoded bytes
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

    /// Creates a Path from a [Iterator] or other iterator that returns
    /// nibbles
    pub fn from_nibbles_iterator<T: Iterator<Item = u8>>(nibbles_iter: T) -> Self {
        Path(SmallVec::from_iter(nibbles_iter))
    }

    /// Creates an empty Path
    #[cfg(test)]
    pub fn new() -> Self {
        Path(SmallVec::new())
    }

    /// Read from an iterator that returns nibbles with a prefix
    /// The prefix is one optional byte -- if not present, the Path is empty
    /// If there is one byte, and the byte contains a [Flags::ODD_LEN] (0x1)
    /// then there is another discarded byte after that.
    #[cfg(test)]
    pub fn from_encoded_iter<Iter: Iterator<Item = u8>>(mut iter: Iter) -> Self {
        let flags = Flags::from_bits_retain(iter.next().unwrap_or_default());

        if !flags.contains(Flags::ODD_LEN) {
            let _ = iter.next();
        }

        Self(iter.collect())
    }

    /// Add nibbles to the end of a path
    pub fn extend<T: IntoIterator<Item = u8>>(&mut self, iter: T) {
        self.0.extend(iter)
    }

    /// Create an iterator that returns the bytes from the underlying nibbles
    /// If there is an odd nibble at the end, it is dropped
    pub fn bytes_iter(&self) -> BytesIterator<'_> {
        BytesIterator { iter: self.iter() }
    }

    /// Create a boxed set of bytes from the Path
    pub fn bytes(&self) -> Box<[u8]> {
        self.bytes_iter().collect()
    }
}

#[derive(Debug)]
pub struct BytesIterator<'a> {
    iter: std::slice::Iter<'a, u8>,
}

impl Iterator for BytesIterator<'_> {
    type Item = u8;
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(&hi) = self.iter.next() {
            if let Some(&lo) = self.iter.next() {
                return Some(hi * 16 + lo);
            }
        }
        None
    }

    // this helps make the collection into a box faster
    fn size_hint(&self) -> (usize, Option<usize>) {
        (
            self.iter.size_hint().0 / 2,
            self.iter.size_hint().1.map(|max| max / 2),
        )
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use test_case::test_case;

    #[test_case([0, 0, 2, 3], [2, 3])]
    #[test_case([1, 2, 3, 4], [2, 3, 4])]
    fn encode_decode<T: AsRef<[u8]> + PartialEq + Debug, U: AsRef<[u8]>>(encode: T, expected: U) {
        let from_encoded = Path::from_encoded_iter(encode.as_ref().iter().copied());
        assert_eq!(
            from_encoded.0,
            SmallVec::<[u8; 32]>::from_slice(expected.as_ref())
        );
        let to_encoded = from_encoded.iter_encoded().collect::<SmallVec<[u8; 32]>>();
        assert_eq!(encode.as_ref(), to_encoded.as_ref());
    }
}
