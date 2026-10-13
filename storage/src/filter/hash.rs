// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Two independent 64-bit hashes of a key, for the blocked counting filter.
//!
//! Firewood keys are usually already cryptographic hashes (32/64 bytes), but
//! this hashes arbitrary bytes robustly so the filter works for any key type.

const SEED1: u64 = 0x9E37_79B9_7F4A_7C15;
const SEED2: u64 = 0xC2B2_AE3D_27D4_EB4F;

const fn mix64(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    x ^= x >> 33;
    x = x.wrapping_mul(0xC4CE_B9FE_1A85_EC53);
    x ^ (x >> 33)
}

fn hash_one(key: &[u8], seed: u64) -> u64 {
    let mut h = seed ^ (key.len() as u64);
    let (words, rem) = key.as_chunks::<8>();
    for chunk in words {
        h = mix64(h ^ u64::from_le_bytes(*chunk));
    }
    if !rem.is_empty() {
        // Zero-extend the trailing partial word (little-endian).
        let tail = rem
            .iter()
            .rev()
            .fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
        h = mix64(h ^ tail);
    }
    h
}

/// Two independent 64-bit hashes of `key` (block selector, probe base).
#[must_use]
pub fn key_hashes(key: &[u8]) -> (u64, u64) {
    (hash_one(key, SEED1), hash_one(key, SEED2))
}
