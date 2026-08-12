// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! # Size-targeted proof generation
//!
//! Production state sync fills each response up to a wire byte budget
//! (`DefaultRequestByteSizeLimit`, 2 MiB − 4 KiB), but [`Merkle::range_proof`]
//! only limits by *key count*, so today the *client* binary-searches
//! `max_length` from outside — paying a full proof generation and
//! serialization per probe. This module moves the sizing into firewood: the
//! caller asks for "as much proof as fits in `budget` bytes starting at
//! `start_key`", and gets back a proof whose compressed wire form fits, with a
//! right edge so the next chunk resumes after the last returned key.
//!
//! The engine ([`stream_sized`]) walks the key-value (or diff) iterator once,
//! accumulating the exact uncompressed body size, stops at the budget mapped
//! through a compression-ratio estimate, then proves the edges and serializes.
//! It materializes no throwaway proofs; if the first serialization misses the
//! budget it re-estimates the ratio from the measured wire length and extends
//! or truncates a bounded number of times.
//!
//! Both entry points guarantee ≥ 1 payload entry even if a single entry
//! exceeds the budget, so a paging caller can always make progress.

#![expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "sizing heuristics convert between byte counts, key counts, and \
              ratio estimates; all quantities are bounded by proof sizes \
              (well below 2^52) and estimates are re-checked against exact \
              serialized lengths."
)]
// Public sizing API with no in-crate caller yet; the state-sync / FFI consumer
// that drives it lands separately.
#![allow(dead_code)]

use firewood_storage::{HashedNodeReader, TrieReader};
use integer_encoding::VarInt;

use super::{Key, Merkle, Value};
use crate::api::{self, FrozenChangeProof, FrozenRangeProof};
use crate::db::BatchOp;
use crate::merkle::changes::DiffMerkleNodeStream;
use crate::proofs::{ChangeProof, Proof, RangeProof};

/// Fraction of the budget a result must reach before we stop growing it.
const ACCEPT_FLOOR: f64 = 0.95;
/// Assumed zstd ratio (compressed ÷ uncompressed) when the caller provides no
/// hint. Measured ~0.5 on C-Chain proof bodies.
const DEFAULT_COMPRESSION_RATIO: f64 = 0.52;
/// Maximum extend/truncate passes after the first serialization.
const MAX_ADJUST: usize = 6;

/// A sized range proof and its compressed wire bytes (already computed for
/// sizing — send these rather than re-serialize). `natural_end` is true when
/// the payload reached the end of the keyspace (paging is done).
#[derive(Debug)]
pub struct SizedProof {
    pub proof: FrozenRangeProof,
    pub wire: Vec<u8>,
    pub natural_end: bool,
}

/// A sized change proof and its compressed wire bytes. See [`SizedProof`].
#[derive(Debug)]
pub struct SizedChangeProof {
    pub proof: FrozenChangeProof,
    pub wire: Vec<u8>,
    pub natural_end: bool,
}

/// Exact uncompressed body bytes one `(key, value)` payload entry costs:
/// `varint(key_len) :: key :: varint(value_len) :: value`.
fn kv_body_bytes(key: &[u8], value: &[u8]) -> u64 {
    varint_len(key.len() as u64)
        .saturating_add(key.len() as u64)
        .saturating_add(varint_len(value.len() as u64))
        .saturating_add(value.len() as u64)
}

/// Exact uncompressed body bytes one `BatchOp` payload entry costs: a 1-byte
/// op tag, then `varint(key_len) :: key` and, for `Put`,
/// `varint(value_len) :: value`.
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

/// Uncompressed body budget: the compressed budget scaled up by `ratio`, less
/// the fixed edge overhead.
fn payload_budget(ratio: f64, budget: usize, fixed: u64) -> u64 {
    let total = (budget as f64 / ratio) as u64;
    total.saturating_sub(fixed)
}

/// The largest proof, built from `items` (already positioned at the start
/// key), whose compressed wire fits `budget`. `cost` sizes one item's
/// uncompressed body bytes; `build(items, natural_end)` assembles the proof
/// and its right edge; `serialize` produces the compressed wire. `fixed` is
/// the measured edge overhead. Returns `(proof, wire, natural_end)`.
///
/// Grows until over budget, then truncates back under. Once we truncate we
/// stop extending: truncation drops the tail of `kept`, but the iterator (and
/// any carried-over item) sit past the pre-truncate end, so extending
/// afterward would skip the gap between them.
fn stream_sized<I, P, Pf>(
    mut items: I,
    budget: usize,
    mut ratio: f64,
    fixed: u64,
    cost: impl Fn(&P) -> u64,
    build: impl Fn(&[P], bool) -> Result<Pf, api::Error>,
    serialize: impl Fn(&Pf) -> Vec<u8>,
) -> Result<(Pf, Vec<u8>, bool), api::Error>
where
    I: Iterator<Item = Result<P, api::Error>>,
{
    let mut kept: Vec<P> = Vec::new();
    let mut body = 0u64;
    let mut natural = true;
    // The item that overflowed the running budget; the iterator has advanced
    // past it, so it is carried into the next extension pass.
    let mut pending: Option<P> = None;

    let mut budget_body = payload_budget(ratio, budget, fixed);
    for item in items.by_ref() {
        let p = item?;
        let c = cost(&p);
        if !kept.is_empty() && body.saturating_add(c) > budget_body {
            natural = false;
            pending = Some(p);
            break;
        }
        body = body.saturating_add(c);
        kept.push(p);
    }

    let mut proof = build(&kept, natural)?;
    let mut wire = serialize(&proof);

    let mut truncated = false;
    for _ in 0..MAX_ADJUST {
        if wire.len() <= budget && (natural || wire.len() as f64 >= budget as f64 * ACCEPT_FLOOR) {
            break;
        }
        if wire.len() > budget {
            let per = (wire.len() as f64 / kept.len() as f64).max(1.0);
            let over = wire.len().saturating_sub(budget);
            let drop = ((over as f64 / per).ceil() as usize).saturating_add(8);
            let keep = kept.len().saturating_sub(drop).max(1);
            kept.truncate(keep);
            truncated = true;
            natural = false;
            proof = build(&kept, natural)?;
            wire = serialize(&proof);
            if keep == 1 {
                break;
            }
        } else {
            if truncated {
                break;
            }
            ratio = (wire.len() as f64 / body.saturating_add(fixed) as f64).clamp(0.05, 2.0);
            budget_body = payload_budget(ratio, budget, fixed);
            let mut extended = false;
            // The carried-over item goes first: the iterator is already past it.
            let carried = pending.take().into_iter().map(Ok);
            for item in carried.chain(items.by_ref()) {
                let p = item?;
                let c = cost(&p);
                if body.saturating_add(c) > budget_body {
                    natural = false;
                    pending = Some(p);
                    extended = true;
                    break;
                }
                body = body.saturating_add(c);
                kept.push(p);
                extended = true;
                natural = true;
            }
            if !extended {
                break; // stream exhausted
            }
            proof = build(&kept, natural)?;
            wire = serialize(&proof);
        }
    }

    Ok((proof, wire, natural))
}

/// Fixed edge overhead: the empty-payload wire size (left edge), plus the same
/// again (or a floor) reserved for the right edge.
fn edge_overhead(empty_wire_len: usize) -> u64 {
    let start = empty_wire_len as u64;
    start.saturating_add(start.max(6 * 1024))
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
    /// Generate the largest range proof starting at `start_key` whose
    /// compressed wire form fits within `budget` bytes.
    ///
    /// `ratio_hint` seeds the compression-ratio estimate (compressed ÷
    /// uncompressed, e.g. from the previous chunk); it is re-measured against
    /// the real serialized length after the first pass.
    ///
    /// The returned proof always contains at least one key-value pair, so a
    /// caller paging through the keyspace can make progress even if that single
    /// pair exceeds the budget.
    ///
    /// # Errors
    ///
    /// Any error from proof generation ([`Merkle::prove`]) or iteration.
    pub fn range_proof_sized(
        &self,
        start_key: Option<&[u8]>,
        budget: usize,
        ratio_hint: Option<f64>,
    ) -> Result<SizedProof, api::Error> {
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
    /// Generate the largest change proof (against `source_trie` as the old
    /// revision, `self` as the new) starting at `start_key` whose compressed
    /// wire form fits within `budget` bytes.
    ///
    /// Mirrors [`Merkle::range_proof_sized`]; the proof always contains at
    /// least one op so a paging caller can make progress.
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
    ) -> Result<SizedChangeProof, api::Error> {
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
        // Unlike range proofs, the plain change-proof API (no requested end
        // key) proves the last op key even at the natural end, so the right
        // edge is unconditional for non-empty payloads.
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
        Ok(SizedChangeProof {
            proof,
            wire,
            natural_end,
        })
    }
}
