// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::fmt::{self, Debug};

use serde::{
    de::{SeqAccess, Visitor},
    Deserialize, Serialize,
};
use sha3::digest::{generic_array::GenericArray, typenum};

/// A hash value inside a merkle trie
/// We use the same type as returned by sha3 here to avoid copies
#[derive(PartialEq, Eq, Clone, Default)]
pub struct TrieHash(GenericArray<u8, typenum::U32>);

impl std::ops::Deref for TrieHash {
    type Target = GenericArray<u8, typenum::U32>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for TrieHash {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
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

impl Serialize for TrieHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for TrieHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_bytes(TrieVisitor)
    }
}

struct TrieVisitor;

impl<'de> Visitor<'de> for TrieVisitor {
    type Value = TrieHash;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an array of u8 hash bytes")
    }

    #[inline]
    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut hash = TrieHash::default();
        for (idx, dest) in hash.iter_mut().enumerate() {
            if let Some(byte) = seq.next_element()? {
                *dest = byte;
            } else {
                return Err(serde::de::Error::invalid_length(idx, &self));
            }
        }
        Ok(hash)
    }
}
