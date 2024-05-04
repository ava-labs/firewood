// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::fmt::{self, Debug};

use sha3::digest::{generic_array::GenericArray, typenum};

/// A hash value inside a merkle trie
/// We use the same type as returned by sha3 here to avoid copies
#[derive(PartialEq, Eq, Clone)]
pub struct TrieHash(GenericArray<u8, typenum::U32>);

impl std::ops::Deref for TrieHash {
    type Target = GenericArray<u8, typenum::U32>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Debug for TrieHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "{}", hex::encode(self.0))
    }
}

impl From<[u8; 32]> for TrieHash {
    fn from(value: [u8; Self::len()]) -> Self {
        TrieHash(value.into())
    }
}

impl From<GenericArray<u8, typenum::U32>> for TrieHash {
    fn from(value: GenericArray<u8, typenum::U32>) -> Self {
        TrieHash(value)
    }
}

impl TrieHash {
    /// Return the length of a TrieHash
    const fn len() -> usize {
        std::mem::size_of::<TrieHash>()
    }
}
