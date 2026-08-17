// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Size-targeted proof generation: the largest range/change proof that fits a
//! compressed wire byte budget, starting at a key. Streams the payload once,
//! estimating compressed size through a ratio, and proves a right edge so the
//! caller can page. Returns ≥ 1 entry while data remains so paging always
//! makes progress, even past an entry larger than the budget.

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

/// Stop growing once the wire reaches this fraction of the budget.
const ACCEPT_FLOOR: f64 = 0.95;
/// Assumed compressed ÷ uncompressed ratio when the caller gives no hint
/// (~0.5 measured on C-Chain proof bodies).
const DEFAULT_COMPRESSION_RATIO: f64 = 0.52;
/// Cap on ratio-correction (grow) passes.
const MAX_GROW: usize = 6;

/// A sized proof `P` with its compressed `wire` bytes. `natural_end` is true
/// once paging has reached the end of the keyspace/diff.
#[derive(Debug)]
pub struct SizedProof<P> {
    pub proof: P,
    pub wire: Vec<u8>,
    pub natural_end: bool,
}

fn varint_len(v: u64) -> u64 {
    let mut buf = [0u8; 10];
    v.encode_var(&mut buf) as u64
}

/// Uncompressed body budget = compressed budget ÷ ratio, minus edge overhead.
fn payload_budget(ratio: f64, budget: usize, fixed: u64) -> u64 {
    ((budget as f64 / ratio) as u64).saturating_sub(fixed)
}

/// Fixed edge overhead: the left edge's empty-proof wire size, plus the same
/// again (or a 6 KiB floor) reserved for the right edge.
fn edge_overhead(empty_wire_len: usize) -> u64 {
    let start = empty_wire_len as u64;
    start.saturating_add(start.max(6 * 1024))
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

/// Streaming state for [`stream_sized`]: the items not yet consumed, the
/// prefix kept so far, and its summed body-byte cost.
struct SizedStream<B: ChunkBuilder, I: Iterator<Item = Result<B::Item, api::Error>>> {
    items: std::iter::Peekable<I>,
    kept: Vec<B::Item>,
    /// Summed [`ChunkBuilder::item_cost`] of everything in `kept`.
    body: u64,
}

impl<B: ChunkBuilder, I: Iterator<Item = Result<B::Item, api::Error>>> SizedStream<B, I> {
    fn new(items: I) -> Self {
        Self {
            items: items.peekable(),
            kept: Vec::new(),
            body: 0,
        }
    }

    /// Move items into `kept` until the next one would push `body` past
    /// `budget_body`. The first item is always taken so a chunk is never
    /// empty while items remain. Returns the number of items appended.
    fn fill(&mut self, budget_body: u64) -> Result<usize, api::Error> {
        let before = self.kept.len();
        while let Some(peeked) = self.items.peek() {
            if let Ok(item) = peeked {
                let item_body = B::item_cost(item);
                if !self.kept.is_empty() && self.body.saturating_add(item_body) > budget_body {
                    break;
                }
            }
            let Some(item) = self.items.next() else { break };
            let item = item?;
            self.body = self.body.saturating_add(B::item_cost(&item));
            self.kept.push(item);
        }
        Ok(self.kept.len().saturating_sub(before))
    }

    /// True once every item has been consumed into `kept`: the natural end
    /// of the keyspace or diff.
    fn is_exhausted(&mut self) -> bool {
        self.items.peek().is_none()
    }
}

/// The largest proof `builder` can assemble from `items` (positioned at the
/// start key) whose compressed wire fits `budget`. `ratio` seeds the
/// compressed ÷ uncompressed estimate. Returns `(proof, wire, at_natural_end)`.
///
/// Reserves edge overhead measured from an empty chunk, then runs three
/// steps: an initial fill using the estimated ratio, grow passes that
/// re-derive the ratio from the real wire length, and a final shrink that
/// truncates until the wire fits. It never grows after shrinking: the
/// truncated tail was already consumed from the iterator, so refilling would
/// skip a key gap.
fn stream_sized<B: ChunkBuilder>(
    builder: &B,
    items: impl Iterator<Item = Result<B::Item, api::Error>>,
    budget: usize,
    mut ratio: f64,
) -> Result<(B::Proof, Vec<u8>, bool), api::Error> {
    let fixed = edge_overhead(B::wire(&builder.build(&[], true)?).len());
    let mut stream = SizedStream::<B, _>::new(items);

    stream.fill(payload_budget(ratio, budget, fixed))?;
    let mut natural = stream.is_exhausted();
    let mut proof = builder.build(&stream.kept, natural)?;
    let mut wire = B::wire(&proof);

    // Grow: correct the ratio estimate against the real wire length and
    // refill, until the wire reaches the accept floor or the items run out.
    for _ in 0..MAX_GROW {
        if natural || wire.len() as f64 >= budget as f64 * ACCEPT_FLOOR {
            break;
        }
        ratio = (wire.len() as f64 / stream.body.saturating_add(fixed) as f64).clamp(0.05, 2.0);
        if stream.fill(payload_budget(ratio, budget, fixed))? == 0 {
            break;
        }
        natural = stream.is_exhausted();
        proof = builder.build(&stream.kept, natural)?;
        wire = B::wire(&proof);
    }

    // Shrink: drop entries (estimated from the measured bytes per entry)
    // until the wire fits, but never below one entry so paging progresses.
    while wire.len() > budget && stream.kept.len() > 1 {
        let per_entry = (wire.len() as f64 / stream.kept.len() as f64).max(1.0);
        let over = wire.len().saturating_sub(budget);
        let drop = ((over as f64 / per_entry).ceil() as usize).saturating_add(8);
        stream
            .kept
            .truncate(stream.kept.len().saturating_sub(drop).max(1));
        natural = false;
        proof = builder.build(&stream.kept, natural)?;
        wire = B::wire(&proof);
    }

    Ok((proof, wire, natural))
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
    /// The largest range proof from `start_key` whose compressed wire fits
    /// `budget`, plus a right edge for paging. `ratio_hint` seeds the
    /// compression estimate — pass the previous chunk's measured ratio, or
    /// `None` on the first chunk.
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

        let (proof, wire, natural_end) = stream_sized(
            &RangeChunkBuilder {
                merkle: self,
                start_proof: &start_proof,
            },
            items,
            budget,
            ratio_hint.unwrap_or(DEFAULT_COMPRESSION_RATIO),
        )?;
        if start_key.is_none() && proof.key_values().is_empty() {
            return Err(api::Error::RangeProofOnEmptyTrie);
        }
        Ok(SizedProof {
            proof,
            wire,
            natural_end,
        })
    }
}

impl<T: HashedNodeReader> Merkle<T> {
    /// The largest change proof (against `source_trie`) from `start_key` whose
    /// compressed wire fits `budget`. Mirrors [`Merkle::range_proof_sized`],
    /// returning at least one op while diff entries remain. Identical tries
    /// (or a `start_key` past the last difference) yield an empty payload
    /// with `natural_end` set, matching [`Merkle::change_proof`].
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

        let (proof, wire, natural_end) = stream_sized(
            &ChangeChunkBuilder {
                merkle: self,
                start_proof: &start_proof,
            },
            items,
            budget,
            ratio_hint.unwrap_or(DEFAULT_COMPRESSION_RATIO),
        )?;
        Ok(SizedProof {
            proof,
            wire,
            natural_end,
        })
    }
}
