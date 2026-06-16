// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod ethhash;
mod trie_hash;

pub use ethhash::HashOrRlp;
pub use trie_hash::{InvalidTrieHashLength, TrieHash};

/// The type of a hash. For ethereum compatible hashes, this might be a RLP encoded
/// value if it's small enough to fit in less than 32 bytes. For merkledb compatible
/// hashes, it's always a `TrieHash` (carried as the `Hash` variant).
///
/// This is now an unconditional enum so that both hashing schemes can be
/// compiled into a single binary. The on-disk byte layout per scheme is
/// governed by the per-mode codec on [`HashMode`](crate::HashMode), not by this
/// type directly.
pub type HashType = HashOrRlp;

/// A trait to convert a value into a [`HashType`].
///
/// Kept because call sites use it; with the unified [`HashType`] enum the
/// conversion is unconditional (`TrieHash` becomes the `Hash` variant).
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
