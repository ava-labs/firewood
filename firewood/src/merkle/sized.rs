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
//! The sizing engine is **streaming**: walk the key-value (or diff) iterator
//! once, accumulating the exact uncompressed body size, stop at the budget
//! mapped through a compression-ratio estimate, then prove the edges and
//! serialize. It materializes no throwaway proofs; if the first serialization
//! misses the budget it re-estimates the ratio from the measured wire length
//! and extends or truncates a bounded number of times.
//!
//! Both entry points guarantee ≥ 1 payload entry even if a single entry
//! exceeds the budget (callers must be able to make progress), and return
//! [`SizedProofStats`] describing the work performed.

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

/// Work accounting for one sized-proof request.
#[derive(Debug, Clone, Copy, Default)]
pub struct SizedProofStats {
    /// Single-key edge proofs (the left edge and each right-edge rebuild).
    pub edge_proofs: u32,
    /// Serializations of a candidate proof (each includes one zstd compression).
    pub serializations: u32,
    /// Total payload entries materialized, including any later thrown away.
    pub kvs_materialized: u64,
    /// Payload entries in the returned proof.
    pub kv_count: u32,
    /// Compressed wire length of the returned proof.
    pub wire_len: u64,
    /// True if the proof reached the natural end of the keyspace/diff.
    pub natural_end: bool,
}

/// Result of a sized range-proof request: the proof, its compressed wire bytes
/// (already computed for sizing — callers should send these rather than
/// re-serialize), and the work stats.
#[derive(Debug)]
pub struct SizedProof {
    /// The generated proof.
    pub proof: FrozenRangeProof,
    /// The compressed wire bytes.
    pub wire: Vec<u8>,
    /// Work accounting.
    pub stats: SizedProofStats,
}

/// Result of a sized change-proof request. See [`SizedProof`].
#[derive(Debug)]
pub struct SizedChangeProof {
    /// The generated proof.
    pub proof: FrozenChangeProof,
    /// The compressed wire bytes.
    pub wire: Vec<u8>,
    /// Work accounting.
    pub stats: SizedProofStats,
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

fn serialize_range(proof: &FrozenRangeProof, stats: &mut SizedProofStats) -> Vec<u8> {
    let mut out = Vec::new();
    proof.write_to_vec(&mut out);
    stats.serializations = stats.serializations.saturating_add(1);
    out
}

fn serialize_change(proof: &FrozenChangeProof, stats: &mut SizedProofStats) -> Vec<u8> {
    let mut out = Vec::new();
    proof.write_to_vec(&mut out);
    stats.serializations = stats.serializations.saturating_add(1);
    out
}

/// Uncompressed body budget: the compressed budget scaled up by `ratio`, less
/// the fixed edge overhead.
fn payload_budget(ratio: f64, budget: usize, fixed: u64) -> u64 {
    let total = (budget as f64 / ratio) as u64;
    total.saturating_sub(fixed)
}

impl<T: TrieReader> Merkle<T> {
    /// Generate the largest range proof starting at `start_key` whose
    /// compressed wire form fits within `budget` bytes.
    ///
    /// `ratio_hint` seeds the compression-ratio estimate (compressed ÷
    /// uncompressed, e.g. from the previous chunk); it is re-measured against
    /// the real serialized length after the first pass.
    ///
    /// The returned proof always contains at least one key-value pair (so a
    /// caller paging through the keyspace can make progress), even if that
    /// single pair exceeds the budget.
    ///
    /// # Errors
    ///
    /// Any error from proof generation ([`Merkle::prove`]) or iteration.
    #[expect(
        clippy::too_many_lines,
        reason = "single-pass accumulation plus its bounded convergence loop"
    )]
    pub fn range_proof_sized(
        &self,
        start_key: Option<&[u8]>,
        budget: usize,
        ratio_hint: Option<f64>,
    ) -> Result<SizedProof, api::Error> {
        let mut ratio = ratio_hint.unwrap_or(DEFAULT_COMPRESSION_RATIO);
        let mut stats = SizedProofStats::default();

        // Left edge is fixed; measure its exact wire cost by serializing an
        // empty-payload proof around it.
        let start_proof = match start_key {
            Some(key) => {
                stats.edge_proofs = stats.edge_proofs.saturating_add(1);
                self.prove(key).map_err(api::Error::from)?
            }
            None => Proof::default(),
        };
        let edge_probe: FrozenRangeProof =
            RangeProof::new(start_proof.clone(), Proof::default(), Box::new([]));
        let mut scratch = Vec::new();
        edge_probe.write_to_vec(&mut scratch);
        let start_overhead = scratch.len() as u64;
        // The right edge is a chain of similar depth; reserve the same (or a
        // floor for the no-left-edge first chunk).
        let end_allowance = start_overhead.max(6 * 1024);
        let fixed = start_overhead.saturating_add(end_allowance);

        let mut kvs: Vec<(Key, Value)> = Vec::new();
        let mut body = 0u64;
        let mut iter = self.key_value_iter_from_key(start_key.unwrap_or_default());
        let mut natural = true;
        // The pair that overflowed the running budget. The iterator has
        // already advanced past it, so it must be carried into any extension
        // pass or the payload would have a gap.
        let mut pending: Option<(Key, Value)> = None;

        let mut budget_body = payload_budget(ratio, budget, fixed);
        for item in iter.by_ref() {
            let (k, v) = item.map_err(api::Error::from)?;
            let cost = kv_body_bytes(&k, &v);
            if !kvs.is_empty() && body.saturating_add(cost) > budget_body {
                natural = false;
                pending = Some((k, v));
                break;
            }
            body = body.saturating_add(cost);
            kvs.push((k, v));
        }
        stats.kvs_materialized = kvs.len() as u64;

        let assemble = |kvs: &[(Key, Value)],
                        natural: bool,
                        stats: &mut SizedProofStats|
         -> Result<FrozenRangeProof, api::Error> {
            let end_proof = if natural {
                Proof::default()
            } else {
                let (last, _) = kvs
                    .last()
                    .ok_or_else(|| api::Error::InternalError("empty sized payload".into()))?;
                stats.edge_proofs = stats.edge_proofs.saturating_add(1);
                self.prove(last).map_err(api::Error::from)?
            };
            Ok(RangeProof::new(
                start_proof.clone(),
                end_proof,
                kvs.to_vec().into_boxed_slice(),
            ))
        };

        let mut proof = assemble(&kvs, natural, &mut stats)?;
        let mut wire = serialize_range(&proof, &mut stats);

        // Measure the real ratio, then extend (iterator is still positioned)
        // or truncate to converge on the budget. Once we truncate we stop
        // extending: truncation drops the tail of `kvs`, but `pending` and the
        // iterator are positioned past the pre-truncate end, so extending
        // afterward would skip the gap between them.
        let mut truncated = false;
        for _ in 0..MAX_ADJUST {
            if wire.len() <= budget
                && (natural || wire.len() as f64 >= budget as f64 * ACCEPT_FLOOR)
            {
                break;
            }
            if wire.len() > budget {
                let bpk = (wire.len() as f64 / kvs.len() as f64).max(1.0);
                let over = wire.len().saturating_sub(budget);
                let drop = ((over as f64 / bpk).ceil() as usize).saturating_add(8);
                let keep = kvs.len().saturating_sub(drop).max(1);
                kvs.truncate(keep);
                truncated = true;
                natural = false;
                proof = assemble(&kvs, natural, &mut stats)?;
                wire = serialize_range(&proof, &mut stats);
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
                // The carried-over pair goes first: the iterator is already
                // past it.
                let carried = pending.take().into_iter().map(Ok);
                for item in carried.chain(iter.by_ref()) {
                    let (k, v) = item.map_err(api::Error::from)?;
                    let cost = kv_body_bytes(&k, &v);
                    if body.saturating_add(cost) > budget_body {
                        natural = false;
                        pending = Some((k, v));
                        extended = true;
                        break;
                    }
                    body = body.saturating_add(cost);
                    kvs.push((k, v));
                    extended = true;
                    natural = true;
                }
                stats.kvs_materialized = kvs.len() as u64;
                if !extended {
                    break; // keyspace exhausted
                }
                proof = assemble(&kvs, natural, &mut stats)?;
                wire = serialize_range(&proof, &mut stats);
            }
        }

        stats.kv_count = kvs.len() as u32;
        stats.wire_len = wire.len() as u64;
        stats.natural_end = natural;
        Ok(SizedProof { proof, wire, stats })
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
    #[expect(
        clippy::too_many_lines,
        reason = "single-pass accumulation plus its bounded convergence loop, \
                  mirroring the range-proof streaming implementation"
    )]
    pub fn change_proof_sized(
        &self,
        source_trie: &T,
        start_key: Option<&[u8]>,
        budget: usize,
        ratio_hint: Option<f64>,
    ) -> Result<SizedChangeProof, api::Error> {
        let mut ratio = ratio_hint.unwrap_or(DEFAULT_COMPRESSION_RATIO);
        let mut stats = SizedProofStats::default();

        let start_proof = match start_key {
            Some(key) => {
                stats.edge_proofs = stats.edge_proofs.saturating_add(1);
                self.prove(key).map_err(api::Error::from)?
            }
            None => Proof::default(),
        };
        let edge_probe: FrozenChangeProof =
            ChangeProof::new(start_proof.clone(), Proof::default(), Box::new([]));
        let mut scratch = Vec::new();
        edge_probe.write_to_vec(&mut scratch);
        let start_overhead = scratch.len() as u64;
        let end_allowance = start_overhead.max(6 * 1024);
        let fixed = start_overhead.saturating_add(end_allowance);

        let mut ops: Vec<BatchOp<Key, Value>> = Vec::new();
        let mut body = 0u64;
        let mut iter = DiffMerkleNodeStream::new(
            source_trie,
            self.nodestore(),
            start_key.unwrap_or_default().into(),
        )
        .map_err(api::Error::from)?;
        let mut natural = true;
        let mut pending: Option<BatchOp<Key, Value>> = None;

        let mut budget_body = payload_budget(ratio, budget, fixed);
        for item in iter.by_ref() {
            let op = item.map_err(api::Error::from)?;
            let cost = op_body_bytes(&op);
            if !ops.is_empty() && body.saturating_add(cost) > budget_body {
                natural = false;
                pending = Some(op);
                break;
            }
            body = body.saturating_add(cost);
            ops.push(op);
        }
        stats.kvs_materialized = ops.len() as u64;

        // Unlike range proofs, the plain change-proof API (with no requested
        // end key) proves the last op key even at the natural end, so the
        // right edge is unconditional for non-empty payloads.
        let assemble = |ops: &[BatchOp<Key, Value>],
                        stats: &mut SizedProofStats|
         -> Result<FrozenChangeProof, api::Error> {
            let end_proof = match ops.last() {
                Some(op) => {
                    stats.edge_proofs = stats.edge_proofs.saturating_add(1);
                    self.prove(op.key()).map_err(api::Error::from)?
                }
                None => Proof::default(),
            };
            Ok(ChangeProof::new(
                start_proof.clone(),
                end_proof,
                ops.to_vec().into_boxed_slice(),
            ))
        };

        let mut proof = assemble(&ops, &mut stats)?;
        let mut wire = serialize_change(&proof, &mut stats);

        // See `range_proof_sized`: stop extending once we truncate.
        let mut truncated = false;
        for _ in 0..MAX_ADJUST {
            if wire.len() <= budget
                && (natural || wire.len() as f64 >= budget as f64 * ACCEPT_FLOOR)
            {
                break;
            }
            if wire.len() > budget {
                let bpo = (wire.len() as f64 / ops.len() as f64).max(1.0);
                let over = wire.len().saturating_sub(budget);
                let drop = ((over as f64 / bpo).ceil() as usize).saturating_add(8);
                let keep = ops.len().saturating_sub(drop).max(1);
                ops.truncate(keep);
                truncated = true;
                natural = false;
                proof = assemble(&ops, &mut stats)?;
                wire = serialize_change(&proof, &mut stats);
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
                let carried = pending.take().into_iter().map(Ok);
                for item in carried.chain(iter.by_ref()) {
                    let op = item.map_err(api::Error::from)?;
                    let cost = op_body_bytes(&op);
                    if body.saturating_add(cost) > budget_body {
                        natural = false;
                        pending = Some(op);
                        extended = true;
                        break;
                    }
                    body = body.saturating_add(cost);
                    ops.push(op);
                    extended = true;
                    natural = true;
                }
                stats.kvs_materialized = ops.len() as u64;
                if !extended {
                    break;
                }
                proof = assemble(&ops, &mut stats)?;
                wire = serialize_change(&proof, &mut stats);
            }
        }

        stats.kv_count = ops.len() as u32;
        stats.wire_len = wire.len() as u64;
        stats.natural_end = natural;
        Ok(SizedChangeProof { proof, wire, stats })
    }
}
