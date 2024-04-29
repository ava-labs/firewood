// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use serde::{Deserialize, Serialize};
use std::{
    fmt::{self, Debug},
    ops::Index,
};

use crate::v2::api::KeyType;

static NIBBLES: [u8; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

// TODO: use smallvec
/// Path is part or all of a node's path in the trie.
#[derive(PartialEq, Eq, Clone, Serialize, Deserialize)]
pub struct Path<T> {
    odd_nibble_length: bool,
    /// Each element is 2 nibbles (or 1 in the case of the final byte
    /// when `odd_nibble_length` is true).
    bytes: T,
}

impl<T: KeyType> Debug for Path<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        for nib in self.bytes.as_ref().iter() {
            write!(f, "{:x}", *nib & 0xf)?;
        }
        Ok(())
    }
}

impl<T: KeyType> Index<usize> for Path<T> {
    type Output = u8;

    fn index(&self, index: usize) -> &Self::Output {
        let containing_byte = self
            .bytes
            .as_ref()
            .get(index / 2)
            .copied()
            .expect("index OOB");

        let as_nibble = if index % 2 == 0 {
            containing_byte >> 4
        } else {
            containing_byte & 0xf
        };

        &NIBBLES[as_nibble as usize]
    }
}

impl<'a, T: KeyType> IntoIterator for &'a Path<T> {
    type Item = u8;

    type IntoIter = PathIterator<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        PathIterator {
            data: self,
            head: 0,
            tail: self.bytes.as_ref().len(),
        }
    }
}

// pub(crate) struct OwnedPathIterator<'a, K: KeyType> {
//     data: Path<K>,
//     inner: PathIterator<'a, K>,
// }

// impl<'a, K: KeyType> Iterator for OwnedPathIterator<'a, K> {
//     type Item = u8;

//     fn next(&mut self) -> Option<Self::Item> {
//         self.inner.next()
//     }

//     fn size_hint(&self) -> (usize, Option<usize>) {
//         self.inner.size_hint()
//     }

//     fn nth(&mut self, n: usize) -> Option<Self::Item> {
//         self.inner.nth(n)
//     }
// }

pub(crate) struct PathIterator<'a, T> {
    data: &'a Path<T>,
    head: usize,
    tail: usize,
}

impl<'a, T: KeyType> Iterator for PathIterator<'a, T> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        if self.is_empty() {
            return None;
        }
        #[allow(clippy::indexing_slicing)]
        let result = Some(self.data[self.head]);
        self.head += 1;
        result
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.tail - self.head;
        (remaining, Some(remaining))
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.head += std::cmp::min(n, self.tail - self.head);
        self.next()
    }
}

impl<'a, T> PathIterator<'a, T> {
    #[inline(always)]
    pub const fn is_empty(&self) -> bool {
        self.head == self.tail
    }
}

impl<'a, T: KeyType> DoubleEndedIterator for PathIterator<'a, T> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.is_empty() {
            return None;
        }
        self.tail -= 1;
        #[allow(clippy::indexing_slicing)]
        Some(self.data[self.tail])
    }

    fn nth_back(&mut self, n: usize) -> Option<Self::Item> {
        self.tail -= std::cmp::min(n, self.tail - self.head);
        self.next_back()
    }
}

impl<T: KeyType> Path<T> {
    pub fn _new(bytes: T, odd_nibble_length: bool) -> Self {
        Self {
            bytes,
            odd_nibble_length,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        unimplemented!()
    }

    pub fn from_encoded(encoded: T) -> Self {
        unimplemented!()
    }

    /// Creates a new Path from a slice of nibbles.
    pub fn from_nibbles(nibbles: T) -> Self {
        todo!()

        // // TODO danlaine: this can probably be made more "rusty" with iterators
        // let num_bytes = nibbles.len() / 2 + 1;
        // let mut bytes = Vec::with_capacity(num_bytes);
        // for i in 0..num_bytes {
        //     #[allow(clippy::indexing_slicing)]
        //     let high = nibbles[2 * i + 2];
        //     let low = nibbles.get(2 * i + 1).copied().unwrap_or(0);
        //     bytes.push(high << 4 | low);
        // }

        // Self {
        //     bytes,
        //     odd_nibble_length: nibbles.len() % 2 == 1,
        // }
    }
}
