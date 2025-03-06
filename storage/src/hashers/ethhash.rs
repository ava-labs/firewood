// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// Ethereum compatible hashing algorithm.

use std::iter::once;

use crate::{hashednode::HasUpdate, logger::trace, HashType, Hashable, Preimage, ValueDigest};
use sha3::{Digest, Keccak256};
use smallvec::SmallVec;

use rlp::RlpStream;

impl HasUpdate for Keccak256 {
    fn update<T: AsRef<[u8]>>(&mut self, data: T) {
        sha3::Digest::update(self, data)
    }
}

// Takes a set of nibbles and converts them to a set of bytes that we can hash
// The input consists of nibbles, but there may be an invalid nibble at the end of 0x10
// which indicates that we need to set bit 5 of the first output byte
// The input may also have an odd number of nibbles, in which case the first output byte
// will have bit 4 set and the low nibble will be the low nibble of the first byte
// Restated: 00ABCCCC
// 0 is always 0
// A is 1 if this is a leaf
// B is 1 if the input had an odd number of nibbles
// CCCC is the first nibble if B is 1, otherwise it is all 0s

fn nibbles_to_eth_compact<T: AsRef<[u8]>>(nibbles: T, is_leaf: bool) -> SmallVec<[u8; 32]> {
    let mut nibbles = nibbles.as_ref();
    let mut first_byte = if is_leaf { 0x20 } else { 0x00 };

    if nibbles.len() & 1 == 1 {
        first_byte |= (1 << 4) | nibbles[0];
        nibbles = &nibbles[1..];
    }
    debug_assert!(nibbles.len() % 2 == 0);
    once(first_byte)
        .chain(nibbles.chunks(2).map(|chunk| (chunk[0] << 4) | chunk[1]))
        .collect()
}

impl<T: Hashable> Preimage for T {
    fn to_hash(&self) -> HashType {
        // first collect the thing that would be hashed, and if it's smaller than a hash,
        // just use it directly
        let mut collector = SmallVec::with_capacity(32);
        self.write(&mut collector);
        if collector.len() >= 32 {
            HashType::Hash(Keccak256::digest(collector).into())
        } else {
            HashType::Rlp(collector)
        }
    }

    fn write(&self, buf: &mut impl HasUpdate) {
        if self.children().size_hint().1.unwrap_or(1) == 0 {
            // since there are no children, this must be a leaf
            // we append two items, the partial_path, encoded, and the value
            // note that leaves must always have a value, so we know there
            // will be 2 items
            let mut rlp = RlpStream::new_list(2);

            rlp.append(&&*nibbles_to_eth_compact(
                self.partial_path().collect::<Box<_>>(),
                true,
            ));
            match self.value_digest().expect("must have a value") {
                ValueDigest::Value(bytes) => rlp.append(&bytes),
                ValueDigest::Hash(hash) => rlp.append(&hash),
            };
            let bytes = rlp.out();
            if crate::logger::trace_enabled() {
                trace!("serialized leaf-rlp: {:?}", hex::encode(&bytes));
            }
            buf.update(&bytes);
        } else {
            // for a branch, there are always 16 children and a value
            // some of these are not present, which we encode as RLP empty_data
            let mut rlp = RlpStream::new_list(17);
            let mut child_iter = self.children().peekable();
            for index in 0..=15 {
                if let Some(&(child_index, digest)) = child_iter.peek() {
                    if child_index == index {
                        match digest {
                            HashType::Hash(hash) => rlp.append(&hash.as_slice()),
                            HashType::Rlp(rlp_bytes) => rlp.append_raw(rlp_bytes, 1),
                        };
                        child_iter.next();
                    } else {
                        rlp.append_empty_data();
                    }
                } else {
                    // exhausted all indexes
                    rlp.append_empty_data();
                }
            }
            if let Some(digest) = self.value_digest() {
                rlp.append(&*digest);
            } else {
                rlp.append_empty_data();
            }
            let bytes = rlp.out();
            if crate::logger::trace_enabled() {
                trace!("pass 1 bytes {:02X?}", hex::encode(&bytes));
            }

            let partial_path = self.partial_path().collect::<Box<_>>();
            if partial_path.is_empty() {
                if crate::logger::trace_enabled() {
                    trace!("pass 2=bytes {:02X?}", hex::encode(&bytes));
                }
                buf.update(bytes);
            } else {
                let mut final_bytes = RlpStream::new_list(2);
                final_bytes.append(&&*nibbles_to_eth_compact(partial_path, false));

                // if the RLP is short enough, we can use it as-is, otherwise we hash it
                // to make the maximum length 32 bytes
                if bytes.len() > 31 {
                    let hashed_bytes = Keccak256::digest(bytes);
                    final_bytes.append(&hashed_bytes.as_slice());
                } else {
                    final_bytes.append(&bytes);
                }
                let final_bytes = final_bytes.out();
                if crate::logger::trace_enabled() {
                    trace!("pass 2 bytes {:02X?}", hex::encode(&final_bytes));
                }
                buf.update(final_bytes);
            }
        }
    }
}

#[cfg(test)]
mod test {
    use test_case::test_case;

    #[test_case(&[], false, &[0x00])]
    #[test_case(&[], true, &[0x20])]
    #[test_case(&[1, 2, 3, 4, 5], false, &[0x11, 0x23, 0x45])]
    #[test_case(&[0, 1, 2, 3, 4, 5], false, &[0x00, 0x01, 0x23, 0x45])]
    #[test_case(&[15, 1, 12, 11, 8], true, &[0x3f, 0x1c, 0xb8])]
    #[test_case(&[0, 15, 1, 12, 11, 8], true, &[0x20, 0x0f, 0x1c, 0xb8])]
    fn test_hex_to_compact(hex: &[u8], has_value: bool, expected_compact: &[u8]) {
        assert_eq!(
            &*super::nibbles_to_eth_compact(hex, has_value),
            expected_compact
        );
    }
}
