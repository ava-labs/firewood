// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::fmt::{self, Debug};

pub const TRIE_HASH_LEN: usize = 32;

#[derive(PartialEq, Eq, Clone, Copy)]
pub struct TrieHash(pub [u8; TRIE_HASH_LEN]);

impl std::ops::Deref for TrieHash {
    type Target = [u8; TRIE_HASH_LEN];
    fn deref(&self) -> &[u8; TRIE_HASH_LEN] {
        &self.0
    }
}

impl Debug for TrieHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
        write!(f, "{}", hex::encode(self.0))
    }
}
