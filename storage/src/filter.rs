// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Mutable, persistent membership filter for the Firewood read path.
//!
//! The filter answers "is this key *definitely absent*?" so `get_value` can
//! skip the trie walk for misses. It must never produce a false negative (that
//! would skip a key that exists); a false positive only costs one unnecessary
//! walk.
//!
//! The default implementation is a **counting bloom filter** (4-bit, cache-line
//! blocked) which—unlike a plain bloom—supports `remove`, so the filter can be
//! maintained incrementally as keys are deleted instead of rebuilt from the KV
//! store. Counters that reach their maximum are *pinned* (never decremented),
//! which can only raise the false-positive rate, never cause a false negative.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Counter/index math is bounds-checked at construction; direct arithmetic is clearer."
)]

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::TrieHash;

mod hash;
pub use hash::key_hashes;

/// 4-bit counters → max value 15, packed 16 per `u64` word.
const COUNTER_MAX: u64 = 15;
const COUNTERS_PER_WORD: usize = 16;
/// One 64-byte cache line per block: 8 words × 16 counters = 128 counters.
const WORDS_PER_BLOCK: usize = 8;
const COUNTERS_PER_BLOCK: u64 = (WORDS_PER_BLOCK * COUNTERS_PER_WORD) as u64;

const MAGIC: &[u8; 8] = b"FWDFILT2";
const FORMAT_VERSION: u16 = 2;
const KIND_COUNTING_BLOOM: u8 = 1;

/// Membership filter abstraction. Implementors must guarantee: if `insert(k)`
/// was called more times than `remove(k)`, then `contains(k)` returns `true`
/// (no false negatives). `contains` may return `true` for never-inserted keys
/// (false positives).
pub trait MembershipFilter: Send + Sync + std::fmt::Debug {
    /// Record that `key` is present.
    fn insert(&self, key: &[u8]);
    /// Record one removal of `key`. **Contract:** `remove(k)` must be matched by
    /// a prior `insert(k)` (counting-filter semantics) — removing a key that was
    /// never inserted decrements counters shared with other keys and *can* cause
    /// a false negative. The Firewood integration guarantees matched removes via
    /// membership-transition tracking; where it cannot (range/prefix deletes) it
    /// uses the `retain` policy and does not call `remove`. Pinned (saturated)
    /// counters are never decremented.
    fn remove(&self, key: &[u8]);
    /// Returns `false` only if `key` is definitely absent.
    fn contains(&self, key: &[u8]) -> bool;
    /// Current statistics (fill ratio, saturation, estimated fpp).
    fn stats(&self) -> FilterStats;
    /// Persist to `path` durably (temp file + atomic rename), tagged with the
    /// root hash of the revision whose keys the snapshot is known to cover
    /// (`None` for an empty database).
    ///
    /// A checkpoint is only sound for a database whose current root matches
    /// the tag: keys committed after the snapshot are missing from it, and a
    /// missing key is exactly a false negative. Loaders must compare the tag
    /// before trusting the file.
    ///
    /// # Errors
    ///
    /// Returns any I/O error from creating, writing, syncing, or renaming the
    /// checkpoint file.
    fn save(&self, path: &Path, root: Option<&TrieHash>) -> std::io::Result<()>;
}

/// Snapshot of filter health, for metrics/diagnostics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterStats {
    /// Total counters in the filter.
    pub counters: u64,
    /// Counters with a nonzero value.
    pub nonzero: u64,
    /// Counters pinned at the maximum (saturated).
    pub saturated: u64,
    /// Configured probes per key.
    pub probes: u32,
    /// Size of the counter array in bytes.
    pub size_bytes: usize,
}

impl FilterStats {
    /// Fraction of counters that are nonzero.
    #[must_use]
    #[expect(clippy::cast_precision_loss, reason = "diagnostic ratio")]
    pub fn fill_ratio(&self) -> f64 {
        if self.counters == 0 {
            0.0
        } else {
            self.nonzero as f64 / self.counters as f64
        }
    }

    /// Rough false-positive probability estimate: `fill_ratio ^ probes`
    /// (treating a nonzero counter as a "set bit"). Approximate.
    #[must_use]
    pub fn estimated_fpp(&self) -> f64 {
        self.fill_ratio()
            .powi(i32::try_from(self.probes).unwrap_or(i32::MAX))
    }
}

/// A cache-line-blocked counting bloom filter with atomic 4-bit counters.
#[derive(Debug)]
pub struct CountingBloom {
    /// `nblocks - 1`; `nblocks` is a power of two.
    block_mask: u64,
    probes: u32,
    words: Box<[AtomicU64]>,
}

impl CountingBloom {
    /// Create an empty filter sized for `expected_keys` at `counters_per_key`
    /// counters each (analogous to bits-per-key for a plain bloom). The probe
    /// count is derived as `round(counters_per_key * ln2)`, clamped to a sane
    /// range, and the block count is rounded up to a power of two.
    ///
    /// # Panics
    ///
    /// Panics if `expected_keys` or `counters_per_key` is zero.
    #[must_use]
    pub fn new(expected_keys: u64, counters_per_key: u32) -> Self {
        assert!(
            expected_keys > 0 && counters_per_key > 0,
            "empty filter sizing"
        );
        let probes = Self::probes_for(counters_per_key);
        let total_counters = expected_keys.saturating_mul(u64::from(counters_per_key));
        let nblocks = total_counters
            .div_ceil(COUNTERS_PER_BLOCK)
            .max(1)
            .next_power_of_two();
        Self::with_blocks(nblocks, probes)
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "rounded value is small and positive (clamped below)"
    )]
    fn probes_for(counters_per_key: u32) -> u32 {
        // optimal k = (m/n) ln2; clamp to [1, COUNTERS_PER_BLOCK].
        let k = (f64::from(counters_per_key) * std::f64::consts::LN_2).round() as u32;
        k.clamp(1, COUNTERS_PER_BLOCK as u32)
    }

    fn with_blocks(nblocks: u64, probes: u32) -> Self {
        assert!(nblocks.is_power_of_two(), "nblocks must be a power of two");
        let nwords = usize::try_from(nblocks).expect("nblocks fits usize") * WORDS_PER_BLOCK;
        let words = std::iter::repeat_with(|| AtomicU64::new(0))
            .take(nwords)
            .collect();
        Self {
            block_mask: nblocks - 1,
            probes,
            words,
        }
    }

    /// Number of blocks (cache lines).
    #[must_use]
    pub const fn blocks(&self) -> u64 {
        self.block_mask + 1
    }

    /// Counter-array size in bytes.
    #[must_use]
    pub const fn size_bytes(&self) -> usize {
        self.words.len() * 8
    }

    /// The word index and intra-word shift for counter `c` of block `b`.
    #[inline]
    fn locate(&self, block: u64, counter: u64) -> (usize, u32) {
        let word_in_block = (counter / COUNTERS_PER_WORD as u64) as usize;
        let nibble_in_word = (counter % COUNTERS_PER_WORD as u64) as u32;
        let word_idx =
            usize::try_from(block).expect("block fits usize") * WORDS_PER_BLOCK + word_in_block;
        (word_idx, nibble_in_word * 4)
    }

    /// Visit each of the `probes` counter locations for `key`.
    #[inline]
    fn for_each_probe(&self, key: &[u8], mut f: impl FnMut(usize, u32)) {
        let (h1, h2) = key_hashes(key);
        // Block from h1; probe base and stride BOTH from h2 so the stride is
        // independent of the block. (Deriving the stride from h1 makes every key
        // in a block share a stride, producing structured collisions and a much
        // higher false-positive rate.)
        let block = h1 & self.block_mask;
        let step = (h2 >> 32) | 1;
        let mut g = h2;
        for _ in 0..self.probes {
            let counter = g % COUNTERS_PER_BLOCK;
            let (word_idx, shift) = self.locate(block, counter);
            f(word_idx, shift);
            g = g.wrapping_add(step);
        }
    }

    fn word(&self, idx: usize) -> &AtomicU64 {
        self.words.get(idx).expect("probe within array")
    }
}

impl MembershipFilter for CountingBloom {
    fn insert(&self, key: &[u8]) {
        self.for_each_probe(key, |word_idx, shift| {
            let word = self.word(word_idx);
            // CAS loop: bump this nibble, saturating at COUNTER_MAX, without
            // disturbing the 15 neighbouring counters in the word.
            let mut cur = word.load(Ordering::Relaxed);
            loop {
                let nib = (cur >> shift) & COUNTER_MAX;
                if nib >= COUNTER_MAX {
                    break; // pinned
                }
                let next = (cur & !(COUNTER_MAX << shift)) | ((nib + 1) << shift);
                match word.compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed) {
                    Ok(_) => break,
                    Err(observed) => cur = observed,
                }
            }
        });
    }

    fn remove(&self, key: &[u8]) {
        self.for_each_probe(key, |word_idx, shift| {
            let word = self.word(word_idx);
            let mut cur = word.load(Ordering::Relaxed);
            loop {
                let nib = (cur >> shift) & COUNTER_MAX;
                // Leave 0 (underflow guard) and MAX (pinned/saturated) alone.
                if nib == 0 || nib >= COUNTER_MAX {
                    break;
                }
                let next = (cur & !(COUNTER_MAX << shift)) | ((nib - 1) << shift);
                match word.compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed) {
                    Ok(_) => break,
                    Err(observed) => cur = observed,
                }
            }
        });
    }

    fn contains(&self, key: &[u8]) -> bool {
        let mut present = true;
        self.for_each_probe(key, |word_idx, shift| {
            if present {
                let nib = (self.word(word_idx).load(Ordering::Relaxed) >> shift) & COUNTER_MAX;
                if nib == 0 {
                    present = false;
                }
            }
        });
        present
    }

    fn stats(&self) -> FilterStats {
        let mut nonzero = 0u64;
        let mut saturated = 0u64;
        for word in &self.words {
            let mut w = word.load(Ordering::Relaxed);
            for _ in 0..COUNTERS_PER_WORD {
                let nib = w & COUNTER_MAX;
                if nib != 0 {
                    nonzero += 1;
                    if nib >= COUNTER_MAX {
                        saturated += 1;
                    }
                }
                w >>= 4;
            }
        }
        FilterStats {
            counters: self.words.len() as u64 * COUNTERS_PER_WORD as u64,
            nonzero,
            saturated,
            probes: self.probes,
            size_bytes: self.size_bytes(),
        }
    }

    fn save(&self, path: &Path, root: Option<&TrieHash>) -> std::io::Result<()> {
        save_counting(self, path, root)
    }
}

// ----- persistence -----

fn crc64(seed: u64, bytes: &[u8]) -> u64 {
    // Small, dependency-free FNV-1a-style checksum for torn-write detection.
    let mut h = seed ^ 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Fixed header size: magic, version, kind, counter bits, probes, block count,
/// root-tag flag, root tag.
const HEADER_LEN: usize = 8 + 2 + 1 + 1 + 4 + 8 + 1 + 32;

fn save_counting(
    filter: &CountingBloom,
    path: &Path,
    root: Option<&TrieHash>,
) -> std::io::Result<()> {
    // Write to a temp sibling then atomically rename, so a crash mid-write
    // never corrupts an existing good checkpoint.
    let tmp = path.with_extension("fwdfilter.tmp");
    {
        let mut out = BufWriter::new(File::create(&tmp)?);
        let mut header = Vec::with_capacity(HEADER_LEN);
        header.extend_from_slice(MAGIC);
        header.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        header.push(KIND_COUNTING_BLOOM);
        header.push(4u8); // counter_bits
        header.extend_from_slice(&filter.probes.to_le_bytes());
        header.extend_from_slice(&filter.blocks().to_le_bytes());
        match root {
            Some(root) => {
                header.push(1);
                header.extend_from_slice(root.as_ref());
            }
            None => header.extend_from_slice(&[0u8; 33]),
        }
        let hcrc = crc64(0, &header);
        out.write_all(&header)?;
        out.write_all(&hcrc.to_le_bytes())?;

        let mut body_crc = 0xfeed_face_dead_beefu64;
        for word in &filter.words {
            let bytes = word.load(Ordering::Relaxed).to_le_bytes();
            body_crc = crc64(body_crc, &bytes);
            out.write_all(&bytes)?;
        }
        out.write_all(&body_crc.to_le_bytes())?;
        out.flush()?;
        out.get_ref().sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// Load a counting filter previously written by [`MembershipFilter::save`],
/// returning it with the root-hash tag it was saved under.
///
/// The caller must check the tag against the database's current root before
/// using the filter; see [`MembershipFilter::save`].
///
/// # Errors
///
/// Returns an error (and the caller should treat the filter as absent) if the
/// file is missing, the magic/version/params are wrong, or any CRC fails. A
/// corrupt filter is never trusted, since that could risk false negatives.
///
/// # Panics
///
/// Does not panic on malformed input (errors are returned); the internal
/// `expect`s are on fixed-size slice conversions that cannot fail.
pub fn load_counting(path: &Path) -> std::io::Result<(CountingBloom, Option<TrieHash>)> {
    let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_owned());
    let mut input = BufReader::with_capacity(1 << 20, File::open(path)?);

    let mut header = [0u8; HEADER_LEN];
    input.read_exact(&mut header)?;
    if header.get(..8) != Some(MAGIC.as_slice()) {
        return Err(bad("bad filter magic"));
    }
    let version = u16::from_le_bytes(header[8..10].try_into().expect("slice"));
    if version != FORMAT_VERSION {
        return Err(bad("unsupported filter version"));
    }
    if header[10] != KIND_COUNTING_BLOOM {
        return Err(bad("unexpected filter kind"));
    }
    if header[11] != 4 {
        return Err(bad("unexpected counter width"));
    }
    let probes = u32::from_le_bytes(header[12..16].try_into().expect("slice"));
    let nblocks = u64::from_le_bytes(header[16..24].try_into().expect("slice"));
    if nblocks == 0 || !nblocks.is_power_of_two() {
        return Err(bad("block count must be a nonzero power of two"));
    }
    let root = match header[24] {
        0 => None,
        1 => Some(TrieHash::from_bytes(
            header[25..57].try_into().expect("slice"),
        )),
        _ => return Err(bad("bad root tag flag")),
    };
    let mut hcrc_buf = [0u8; 8];
    input.read_exact(&mut hcrc_buf)?;
    if u64::from_le_bytes(hcrc_buf) != crc64(0, &header) {
        return Err(bad("header crc mismatch"));
    }

    let nwords = usize::try_from(nblocks).map_err(|_| bad("filter too large"))? * WORDS_PER_BLOCK;
    let mut words = Vec::with_capacity(nwords);
    let mut body_crc = 0xfeed_face_dead_beefu64;
    let mut buf = [0u8; 8];
    for _ in 0..nwords {
        input.read_exact(&mut buf)?;
        body_crc = crc64(body_crc, &buf);
        words.push(AtomicU64::new(u64::from_le_bytes(buf)));
    }
    input.read_exact(&mut buf)?;
    if u64::from_le_bytes(buf) != body_crc {
        return Err(bad("body crc mismatch (torn write?)"));
    }

    Ok((
        CountingBloom {
            block_mask: nblocks - 1,
            probes,
            words: words.into_boxed_slice(),
        },
        root,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(n: u64, salt: u64) -> Vec<[u8; 32]> {
        (0..n)
            .map(|i| {
                let mut k = [0u8; 32];
                let src = i
                    .to_le_bytes()
                    .into_iter()
                    .chain(salt.to_le_bytes())
                    .chain(i.wrapping_mul(0x9e37_79b9).to_le_bytes());
                for (dst, b) in k.iter_mut().zip(src) {
                    *dst = b;
                }
                k
            })
            .collect()
    }

    #[test]
    fn no_false_negatives_after_inserts() {
        let f = CountingBloom::new(10_000, 12);
        let ks = keys(10_000, 1);
        for k in &ks {
            f.insert(k);
        }
        for k in &ks {
            assert!(f.contains(k), "false negative after insert");
        }
    }

    #[test]
    fn delete_then_absent_or_fp_only() {
        let f = CountingBloom::new(10_000, 14);
        let ks = keys(5_000, 2);
        for k in &ks {
            f.insert(k);
        }
        // Remove half; the removed keys should mostly become absent (some may
        // remain as false positives, which is allowed). The kept half must
        // never become a false negative.
        for k in &ks[..2_500] {
            f.remove(k);
        }
        for k in &ks[2_500..] {
            assert!(
                f.contains(k),
                "removing other keys must not lose a kept key"
            );
        }
        let still_present = ks[..2_500].iter().filter(|k| f.contains(&k[..])).count();
        // After deleting 2500, the vast majority should read absent.
        assert!(
            still_present < 250,
            "too many removed keys still present: {still_present}/2500"
        );
    }

    #[test]
    fn matched_insert_remove_reinsert() {
        // The supported contract: removes are matched to prior inserts. A key
        // re-inserted after removal must be present again, and matched churn
        // must never lose a key.
        let f = CountingBloom::new(1_000, 12);
        let present = keys(1_000, 3);
        for k in &present {
            f.insert(k);
        }
        for k in &present {
            f.remove(k); // matched
            f.insert(k); // re-insert
        }
        for k in &present {
            assert!(f.contains(k), "matched churn lost a key");
        }
    }

    #[test]
    fn over_remove_does_not_panic() {
        // Unmatched removes are caller error (may cause false positives->absent)
        // but must at least be memory-safe / not panic (underflow guard).
        let f = CountingBloom::new(1_000, 12);
        for k in &keys(1_000, 999) {
            f.remove(k); // never inserted; underflow-guarded, no panic
        }
        assert_eq!(f.stats().nonzero, 0, "removes on empty filter set nothing");
    }

    #[test]
    fn false_positive_rate_reasonable() {
        let f = CountingBloom::new(50_000, 12);
        for k in &keys(50_000, 4) {
            f.insert(k);
        }
        let fps = keys(50_000, 0xdead)
            .iter()
            .filter(|k| f.contains(&k[..]))
            .count();
        // ~12 counters/key → well under 2%.
        assert!(fps < 1_000, "fpp too high: {fps}/50000");
    }

    #[test]
    fn saturation_is_safe() {
        // Hammer one key far past the counter max, then remove many times: it
        // must still be present (pinned counters never underflow).
        let f = CountingBloom::new(64, 8);
        let k = [7u8; 32];
        for _ in 0..100 {
            f.insert(&k);
        }
        for _ in 0..100 {
            f.remove(&k);
        }
        assert!(f.contains(&k), "saturated key lost after removes");
    }

    #[test]
    fn save_load_roundtrip() {
        let f = CountingBloom::new(2_000, 12);
        let ks = keys(2_000, 5);
        for k in &ks {
            f.insert(k);
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("m.fwdfilter");
        let root = TrieHash::from_bytes([0xab; 32]);
        f.save(&path, Some(&root)).expect("save");
        let (g, tag) = load_counting(&path).expect("load");
        for k in &ks {
            assert!(g.contains(k), "lost key across save/load");
        }
        assert_eq!(f.stats().nonzero, g.stats().nonzero);
        assert_eq!(f.blocks(), g.blocks());
        assert_eq!(tag, Some(root), "root tag lost across save/load");

        f.save(&path, None).expect("save untagged");
        let (_, tag) = load_counting(&path).expect("load untagged");
        assert_eq!(tag, None);
    }

    #[test]
    fn load_rejects_corruption() {
        let f = CountingBloom::new(512, 12);
        f.insert(b"hello");
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("c.fwdfilter");
        f.save(&path, None).expect("save");
        let mut bytes = std::fs::read(&path).expect("read");
        let n = bytes.len();
        bytes[n - 4] ^= 0xff; // corrupt the body
        std::fs::write(&path, &bytes).expect("write");
        assert!(load_counting(&path).is_err(), "corruption not detected");
    }
}
