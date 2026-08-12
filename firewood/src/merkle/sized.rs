// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Size-targeted proof generation: the largest range/change proof that fits a
//! compressed wire byte budget, starting at a key. Streams the payload once,
//! estimating compressed size through a ratio, and proves a right edge so the
//! caller can page. Always returns ≥ 1 entry so paging makes progress.

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
use crate::api::{self, FrozenChangeProof, FrozenRangeProof};
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

/// Uncompressed body bytes for one key-value entry.
fn kv_body_bytes(key: &[u8], value: &[u8]) -> u64 {
    varint_len(key.len() as u64)
        .saturating_add(key.len() as u64)
        .saturating_add(varint_len(value.len() as u64))
        .saturating_add(value.len() as u64)
}

/// Uncompressed body bytes for one batch op (1-byte tag + key, + value for `Put`).
fn op_body_bytes(op: &BatchOp<Key, Value>) -> u64 {
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

/// The largest proof built from `items` (positioned at the start key) whose
/// compressed wire fits `budget`. `cost` sizes an item's body bytes;
/// `build(items, at_natural_end)` assembles the proof and right edge;
/// `to_wire` compresses it. Returns `(proof, wire, at_natural_end)`.
///
/// Grows to fill the budget (correcting the ratio estimate against the real
/// wire length), then shrinks to fit. It never grows after shrinking: the
/// iterator sits past the pre-shrink tail, so re-growing would skip a gap.
fn stream_sized<Item, Pf>(
    mut items: impl Iterator<Item = Result<Item, api::Error>>,
    budget: usize,
    mut ratio: f64,
    fixed: u64,
    cost: impl Fn(&Item) -> u64,
    build: impl Fn(&[Item], bool) -> Result<Pf, api::Error>,
    to_wire: impl Fn(&Pf) -> Vec<u8>,
) -> Result<(Pf, Vec<u8>, bool), api::Error> {
    let mut kept: Vec<Item> = Vec::new();
    let mut body = 0u64;
    let mut natural = true;
    // Item that overflowed the budget; the iterator is past it, so it is
    // carried into the next grow pass.
    let mut pending: Option<Item> = None;

    // Append items (starting with any carried-over one) from `items` until the
    // body budget is hit. Returns how many were added.
    let mut fill = |kept: &mut Vec<Item>,
                    body: &mut u64,
                    natural: &mut bool,
                    pending: &mut Option<Item>,
                    budget_body: u64|
     -> Result<usize, api::Error> {
        let before = kept.len();
        let carried = pending.take().into_iter().map(Ok);
        for item in carried.chain(items.by_ref()) {
            let it = item?;
            let c = cost(&it);
            if !kept.is_empty() && body.saturating_add(c) > budget_body {
                *natural = false;
                *pending = Some(it);
                return Ok(kept.len().saturating_sub(before));
            }
            *body = body.saturating_add(c);
            kept.push(it);
            *natural = true;
        }
        Ok(kept.len().saturating_sub(before))
    };

    let mut budget_body = payload_budget(ratio, budget, fixed);
    fill(
        &mut kept,
        &mut body,
        &mut natural,
        &mut pending,
        budget_body,
    )?;
    let mut proof = build(&kept, natural)?;
    let mut wire = to_wire(&proof);

    // Grow: correct the ratio from the real wire length and refill.
    for _ in 0..MAX_GROW {
        if wire.len() as f64 >= budget as f64 * ACCEPT_FLOOR {
            break;
        }
        ratio = (wire.len() as f64 / body.saturating_add(fixed) as f64).clamp(0.05, 2.0);
        budget_body = payload_budget(ratio, budget, fixed);
        if fill(
            &mut kept,
            &mut body,
            &mut natural,
            &mut pending,
            budget_body,
        )? == 0
        {
            break; // stream exhausted
        }
        proof = build(&kept, natural)?;
        wire = to_wire(&proof);
    }

    // Shrink: truncate until it fits, using the measured bytes/entry.
    while wire.len() > budget && kept.len() > 1 {
        let per = (wire.len() as f64 / kept.len() as f64).max(1.0);
        let over = wire.len().saturating_sub(budget);
        let drop = ((over as f64 / per).ceil() as usize).saturating_add(8);
        kept.truncate(kept.len().saturating_sub(drop).max(1));
        natural = false;
        proof = build(&kept, natural)?;
        wire = to_wire(&proof);
    }

    Ok((proof, wire, natural))
}

fn to_wire_range(proof: &FrozenRangeProof) -> Vec<u8> {
    let mut out = Vec::new();
    proof.write_to_vec(&mut out);
    out
}

fn to_wire_change(proof: &FrozenChangeProof) -> Vec<u8> {
    let mut out = Vec::new();
    proof.write_to_vec(&mut out);
    out
}

impl<T: TrieReader> Merkle<T> {
    /// The largest range proof from `start_key` whose compressed wire fits
    /// `budget`, plus a right edge for paging. `ratio_hint` seeds the
    /// compression estimate — pass the previous chunk's measured ratio, or
    /// `None` on the first chunk. Always returns ≥ 1 key-value pair.
    ///
    /// # Errors
    ///
    /// Any error from proof generation ([`Merkle::prove`]) or iteration.
    pub fn range_proof_sized(
        &self,
        start_key: Option<&[u8]>,
        budget: usize,
        ratio_hint: Option<f64>,
    ) -> Result<SizedProof<FrozenRangeProof>, api::Error> {
        let ratio = ratio_hint.unwrap_or(DEFAULT_COMPRESSION_RATIO);
        let start_proof = match start_key {
            Some(key) => self.prove(key).map_err(api::Error::from)?,
            None => Proof::default(),
        };
        let empty = RangeProof::new(start_proof.clone(), Proof::default(), Box::new([]));
        let fixed = edge_overhead(to_wire_range(&empty).len());

        let items = self
            .key_value_iter_from_key(start_key.unwrap_or_default())
            .map(|r| r.map_err(api::Error::from));
        let build = |kvs: &[(Key, Value)], natural: bool| -> Result<FrozenRangeProof, api::Error> {
            let end = if natural {
                Proof::default()
            } else {
                let (last, _) = kvs
                    .last()
                    .ok_or_else(|| api::Error::InternalError("empty sized payload".into()))?;
                self.prove(last).map_err(api::Error::from)?
            };
            Ok(RangeProof::new(
                start_proof.clone(),
                end,
                kvs.to_vec().into_boxed_slice(),
            ))
        };

        let (proof, wire, natural_end) = stream_sized(
            items,
            budget,
            ratio,
            fixed,
            |(k, v)| kv_body_bytes(k, v),
            build,
            to_wire_range,
        )?;
        Ok(SizedProof {
            proof,
            wire,
            natural_end,
        })
    }
}

impl<T: HashedNodeReader> Merkle<T> {
    /// The largest change proof (against `source_trie`) from `start_key` whose
    /// compressed wire fits `budget`. Mirrors [`Merkle::range_proof_sized`];
    /// always returns ≥ 1 op.
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
        let ratio = ratio_hint.unwrap_or(DEFAULT_COMPRESSION_RATIO);
        let start_proof = match start_key {
            Some(key) => self.prove(key).map_err(api::Error::from)?,
            None => Proof::default(),
        };
        let empty = ChangeProof::new(start_proof.clone(), Proof::default(), Box::new([]));
        let fixed = edge_overhead(to_wire_change(&empty).len());

        let items = DiffMerkleNodeStream::new(
            source_trie,
            self.nodestore(),
            start_key.unwrap_or_default().into(),
        )
        .map_err(api::Error::from)?
        .map(|r| r.map_err(api::Error::from));
        // The plain change-proof API proves the last op key even at the natural
        // end, so the right edge is unconditional for a non-empty payload.
        let build = |ops: &[BatchOp<Key, Value>],
                     _natural: bool|
         -> Result<FrozenChangeProof, api::Error> {
            let end = match ops.last() {
                Some(op) => self.prove(op.key()).map_err(api::Error::from)?,
                None => Proof::default(),
            };
            Ok(ChangeProof::new(
                start_proof.clone(),
                end,
                ops.to_vec().into_boxed_slice(),
            ))
        };

        let (proof, wire, natural_end) = stream_sized(
            items,
            budget,
            ratio,
            fixed,
            op_body_bytes,
            build,
            to_wire_change,
        )?;
        Ok(SizedProof {
            proof,
            wire,
            natural_end,
        })
    }
}
