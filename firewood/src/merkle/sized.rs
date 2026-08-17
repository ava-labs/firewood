// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Size-targeted proof generation: a range/change proof whose compressed
//! wire size lands close to — and, except for a lone oversized entry, at or
//! under — a byte budget, starting at a key. Streams the payload once,
//! sizing the chunk through a measured compression-ratio estimate in a small
//! bounded number of proof builds; it deliberately trades exact maximality
//! for that bound, so a chunk is near-budget, not the largest possible.
//! Returns ≥ 1 entry while data remains so paging always makes progress,
//! even past an entry larger than the budget.

#![expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "byte/key counts (well below 2^52) are converted to and from f64 \
              for the compression-ratio estimate, which is re-checked against \
              the exact serialized length."
)]
#![allow(dead_code)]

use firewood_storage::{HashedNodeReader, TrieReader};
use integer_encoding::VarInt;

use super::{Key, Merkle, Value};
use crate::api::{self, FrozenChangeProof, FrozenProof, FrozenRangeProof};
use crate::db::BatchOp;
use crate::merkle::changes::DiffMerkleNodeStream;
use crate::proofs::{ChangeProof, Proof, RangeProof};

/// Stop growing once the wire reaches this fraction of the budget
const ACCEPT_FLOOR: f64 = 0.95;
/// Assumed compressed/uncompressed ratio when the caller gives no hint
const DEFAULT_COMPRESSION_RATIO: f64 = 0.52;
/// Ratio hints and measurements are clamped into `RATIO_MIN..=RATIO_MAX`
const RATIO_MIN: f64 = 0.05;
const RATIO_MAX: f64 = 2.0;
/// Cap on ratio-correction (grow) passes.
const MAX_GROW: usize = 6;

/// A sized proof `P` with its compressed `wire` bytes.
#[derive(Debug)]
pub struct SizedProof<P> {
    pub proof: P,
    pub wire: Vec<u8>,
    /// True once paging has reached the end of the keyspace/diff.
    pub natural_end: bool,
    /// Measured compression ratio of this chunk; pass it as `ratio_hint`
    /// when requesting the next chunk.
    pub ratio: f64,
}

fn varint_len(v: u64) -> u64 {
    let mut buf = [0u8; 10];
    v.encode_var(&mut buf) as u64
}

/// One proof flavor for [`stream_sized`]: what an item costs in uncompressed
/// body bytes, how to assemble a chunk proof, and how to serialize it.
trait ChunkBuilder {
    type Item;
    type Proof;

    /// Uncompressed body bytes `item` contributes to the payload.
    fn item_cost(item: &Self::Item) -> u64;

    /// The chunk proof (payload plus right edge) for `items`;
    /// `at_natural_end` is true when `items` reached the end of the stream.
    fn build(&self, items: &[Self::Item], at_natural_end: bool) -> Result<Self::Proof, api::Error>;

    /// Compressed wire bytes for `proof`.
    fn wire(proof: &Self::Proof) -> Vec<u8>;
}

/// A chunk proof `builder` assembles from a prefix of `items` (positioned at
/// the start key), sized to approach `budget` compressed wire bytes without
/// exceeding it — unless a single item alone does. `ratio_hint` seeds the
/// compressed ÷ uncompressed estimate and is sanitized here.
///
/// Grows: fills to the ratio-estimated body budget, re-deriving the ratio
/// from the real wire length until the wire reaches the accept floor.
/// Then shrinks: truncates until the wire fits. It never grows after
/// shrinking — the truncated tail was already consumed from the iterator,
/// so refilling would skip a key gap. Both bounds are deliberate: the chunk
/// is near-budget in a handful of proof builds, not the exact largest
/// fitting prefix.
fn stream_sized<B: ChunkBuilder>(
    builder: &B,
    items: impl Iterator<Item = Result<B::Item, api::Error>>,
    budget: usize,
    ratio_hint: Option<f64>,
) -> Result<SizedProof<B::Proof>, api::Error> {
    let mut ratio = ratio_hint
        .filter(|r| r.is_finite())
        .unwrap_or(DEFAULT_COMPRESSION_RATIO)
        .clamp(RATIO_MIN, RATIO_MAX);
    let mut items = items.peekable();
    let mut kept: Vec<B::Item> = Vec::new();
    let mut body = 0u64; // summed item_cost of `kept`
    let mut natural = true;

    // The empty chunk's wire doubles as the fixed edge overhead: its length
    // (the left edge), plus the same again (or a 6 KiB floor) for the right
    // edge.
    let mut proof = builder.build(&[], true)?;
    let mut wire = B::wire(&proof);
    let fixed = (wire.len() as u64).saturating_add((wire.len() as u64).max(6 * 1024));

    for _ in 0..=MAX_GROW {
        // Uncompressed body budget = compressed budget ÷ ratio − overhead.
        let budget_body = ((budget as f64 / ratio) as u64).saturating_sub(fixed);
        // Keep items while they fit; always take the first so paging
        // progresses even when a single item exceeds the whole budget.
        let before = kept.len();
        while let Some(peeked) = items.peek() {
            if let Ok(item) = peeked
                && !kept.is_empty()
                && body.saturating_add(B::item_cost(item)) > budget_body
            {
                break;
            }
            let Some(item) = items.next() else { break };
            let item = item?;
            body = body.saturating_add(B::item_cost(&item));
            kept.push(item);
        }
        if kept.len() == before {
            break;
        }
        natural = items.peek().is_none();
        proof = builder.build(&kept, natural)?;
        wire = B::wire(&proof);
        if natural || wire.len() as f64 >= budget as f64 * ACCEPT_FLOOR {
            break;
        }
        ratio = (wire.len() as f64 / body.saturating_add(fixed) as f64).clamp(RATIO_MIN, RATIO_MAX);
    }

    // Shrink: drop entries until the wire fits, but never below one so
    // paging progresses. Each step drops half of what the average per-entry
    // size suggests: the average understates what the tail entries actually
    // contribute when compressibility is uneven, and a full-step drop can
    // land far below the budget. Half-steps converge in a few builds while
    // keeping the chunk near the budget.
    while wire.len() > budget && kept.len() > 1 {
        let per_entry = (wire.len() as f64 / kept.len() as f64).max(1.0);
        let over = wire.len().saturating_sub(budget) as f64;
        let drop = ((over / per_entry / 2.0).ceil() as usize).max(1);
        kept.truncate(kept.len().saturating_sub(drop).max(1));
        natural = false;
        proof = builder.build(&kept, natural)?;
        wire = B::wire(&proof);
    }

    // Report the measured ratio so the caller can seed the next chunk.
    let body_kept = kept
        .iter()
        .fold(0u64, |sum, item| sum.saturating_add(B::item_cost(item)));
    if body_kept > 0 {
        ratio = (wire.len() as f64 / body_kept.saturating_add(fixed) as f64)
            .clamp(RATIO_MIN, RATIO_MAX);
    }
    Ok(SizedProof {
        proof,
        wire,
        natural_end: natural,
        ratio,
    })
}

/// Range-proof chunks: the right edge proves the last kv, or nothing when
/// the payload reached the natural end of the keyspace.
struct RangeChunkBuilder<'a, T> {
    merkle: &'a Merkle<T>,
    start_proof: &'a FrozenProof,
}

impl<T: TrieReader> ChunkBuilder for RangeChunkBuilder<'_, T> {
    type Item = (Key, Value);
    type Proof = FrozenRangeProof;

    fn item_cost((key, value): &Self::Item) -> u64 {
        varint_len(key.len() as u64)
            .saturating_add(key.len() as u64)
            .saturating_add(varint_len(value.len() as u64))
            .saturating_add(value.len() as u64)
    }

    fn build(&self, kvs: &[Self::Item], at_natural_end: bool) -> Result<Self::Proof, api::Error> {
        let end = match kvs.last() {
            Some((last, _)) if !at_natural_end => {
                self.merkle.prove(last).map_err(api::Error::from)?
            }
            _ => Proof::default(),
        };
        Ok(RangeProof::new(
            self.start_proof.clone(),
            end,
            kvs.to_vec().into_boxed_slice(),
        ))
    }

    fn wire(proof: &Self::Proof) -> Vec<u8> {
        let mut out = Vec::new();
        proof.write_to_vec(&mut out);
        out
    }
}

/// Change-proof chunks: the plain change-proof API proves the last op key
/// even at the natural end, so the right edge is unconditional for a
/// non-empty payload.
struct ChangeChunkBuilder<'a, T> {
    merkle: &'a Merkle<T>,
    start_proof: &'a FrozenProof,
}

impl<T: HashedNodeReader> ChunkBuilder for ChangeChunkBuilder<'_, T> {
    type Item = BatchOp<Key, Value>;
    type Proof = FrozenChangeProof;

    /// 1-byte tag + key, + value for `Put`.
    fn item_cost(op: &Self::Item) -> u64 {
        let key = op.key();
        let mut bytes = 1u64
            .saturating_add(varint_len(key.len() as u64))
            .saturating_add(key.len() as u64);
        if let BatchOp::Put { value, .. } = op {
            bytes = bytes
                .saturating_add(varint_len(value.len() as u64))
                .saturating_add(value.len() as u64);
        }
        bytes
    }

    fn build(&self, ops: &[Self::Item], _at_natural_end: bool) -> Result<Self::Proof, api::Error> {
        let end = match ops.last() {
            Some(op) => self.merkle.prove(op.key()).map_err(api::Error::from)?,
            None => Proof::default(),
        };
        Ok(ChangeProof::new(
            self.start_proof.clone(),
            end,
            ops.to_vec().into_boxed_slice(),
        ))
    }

    fn wire(proof: &Self::Proof) -> Vec<u8> {
        let mut out = Vec::new();
        proof.write_to_vec(&mut out);
        out
    }
}

impl<T: TrieReader> Merkle<T> {
    /// A range proof from `start_key` sized to approach `budget` compressed
    /// wire bytes without exceeding it, plus a right edge for paging. The
    /// chunk is near-budget, produced in a handful of proof builds — not
    /// guaranteed to be the largest possible prefix that fits. `ratio_hint`
    /// seeds the compression estimate — pass the previous chunk's
    /// [`SizedProof::ratio`], or `None` on the first chunk; non-finite or
    /// out-of-range hints are sanitized.
    ///
    /// Returns at least one key-value pair while keys at or after `start_key`
    /// remain (a single entry may exceed `budget`; paging still progresses).
    /// A `start_key` past the last key yields an empty payload with
    /// `natural_end` set.
    ///
    /// # Errors
    ///
    /// * [`api::Error::RangeProofOnEmptyTrie`] - if the trie is empty and
    ///   `start_key` is `None`, matching [`Merkle::range_proof`].
    /// * Any error from proof generation ([`Merkle::prove`]) or iteration.
    pub fn range_proof_sized(
        &self,
        start_key: Option<&[u8]>,
        budget: usize,
        ratio_hint: Option<f64>,
    ) -> Result<SizedProof<FrozenRangeProof>, api::Error> {
        let start_proof = match start_key {
            Some(key) => self.prove(key).map_err(api::Error::from)?,
            None => Proof::default(),
        };
        let items = self
            .key_value_iter_from_key(start_key.unwrap_or_default())
            .map(|r| r.map_err(api::Error::from));

        let sized = stream_sized(
            &RangeChunkBuilder {
                merkle: self,
                start_proof: &start_proof,
            },
            items,
            budget,
            ratio_hint,
        )?;
        if start_key.is_none() && sized.proof.key_values().is_empty() {
            return Err(api::Error::RangeProofOnEmptyTrie);
        }
        Ok(sized)
    }
}

impl<T: HashedNodeReader> Merkle<T> {
    /// A change proof (against `source_trie`) from `start_key` sized to
    /// approach `budget` compressed wire bytes without exceeding it. Mirrors
    /// [`Merkle::range_proof_sized`], returning at least one op while diff
    /// entries remain. Identical tries (or a `start_key` past the last
    /// difference) yield an empty payload with `natural_end` set, matching
    /// [`Merkle::change_proof`].
    ///
    /// # Errors
    ///
    /// Any error from proof generation or diff iteration.
    pub fn change_proof_sized(
        &self,
        source_trie: &T,
        start_key: Option<&[u8]>,
        budget: usize,
        ratio_hint: Option<f64>,
    ) -> Result<SizedProof<FrozenChangeProof>, api::Error> {
        let start_proof = match start_key {
            Some(key) => self.prove(key).map_err(api::Error::from)?,
            None => Proof::default(),
        };
        let items = DiffMerkleNodeStream::new(
            source_trie,
            self.nodestore(),
            start_key.unwrap_or_default().into(),
        )
        .map_err(api::Error::from)?
        .map(|r| r.map_err(api::Error::from));

        stream_sized(
            &ChangeChunkBuilder {
                merkle: self,
                start_proof: &start_proof,
            },
            items,
            budget,
            ratio_hint,
        )
    }
}
