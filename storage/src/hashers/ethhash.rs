// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// Ethereum compatible hashing algorithm.

use sha3::{Digest, Keccak256};

use crate::{hashednode::HasUpdate, Hashable, Preimage, TrieHash};

use rlp::RlpStream;

impl HasUpdate for Keccak256 {
    fn update<T: AsRef<[u8]>>(&mut self, data: T) {
        sha3::Digest::update(self, data)
    }
}

impl<T: Hashable> Preimage for T {
    fn to_hash(&self) -> TrieHash {
        let mut hasher = sha3::Keccak256::new();

        self.write(&mut hasher);
        hasher.finalize().into()
    }

    fn write(&self, buf: &mut impl HasUpdate) {
        // accumulate the key, but if the key is longer than 32 bytes, just take the remainder
        let mut key = Vec::with_capacity(32);
        while let Some((len, part)) = self.key().enumerate().next() {
            if len == 33 {
                key.truncate(0);
            }
            key.push(part);
        }

        let mut rlp = RlpStream::new();
        rlp.begin_list(1) // TODO: 3
            .append(&&*key);
        // TODO .append(&self.value_digest())
        // TODO .append(&self.children().collect::<Vec<_>>())
        buf.update(rlp.out())
    }
}
