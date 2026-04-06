// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod ethhash;
mod trie_hash;

pub use ethhash::HashOrRlp as HashType;
pub use trie_hash::{InvalidTrieHashLength, TrieHash};

/// A trait to convert a value into a [`HashType`].
///
/// This is used to allow callers that have a `TrieHash` to obtain a `HashType`
/// in the format used by in-memory trie structures.
pub trait IntoHashType {
    /// Converts the value into a `HashType`.
    #[must_use]
    fn into_hash_type(self) -> HashType;
}

impl IntoHashType for crate::TrieHash {
    #[inline]
    fn into_hash_type(self) -> HashType {
        self.into()
    }
}
