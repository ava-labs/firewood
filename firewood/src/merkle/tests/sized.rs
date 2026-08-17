// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Tests for size-targeted streaming proof generation
//! ([`Merkle::range_proof_sized`] and [`Merkle::change_proof_sized`]).
//!
//! The contract under test, and which test pins each part:
//!
//! 1. **Budget** — the compressed wire fits the byte budget, unless a single
//!    entry alone exceeds it (`*_fits_and_matches_plain_api`).
//! 2. **Equivalence** — the wire is byte-identical to the plain proof API cut
//!    at the same entry count, so verifier correctness is inherited from the
//!    plain-API proof tests (`*_fits_and_matches_plain_api`).
//! 3. **Completeness** — paging by `natural_end` + last-key successor walks
//!    the whole keyspace/diff in order, with no gaps or duplicates, and every
//!    non-final chunk is reasonably full (`*_paging_covers_*`).
//! 4. **Progress** — an entry larger than the whole budget is still returned,
//!    alone and over budget, and paging moves past it
//!    (`test_range_sized_progresses_past_oversized_entry`).
//! 5. **Empty inputs** — an empty trie errors like the plain API; identical
//!    tries or a start key past the last key return an empty payload with
//!    `natural_end` set, so paging terminates (`*_empty_*`, `*_past_last_*`,
//!    `*_identical_tries_*`).
//! 6. **Estimator robustness** — wrong `ratio_hint`s (an optimistic hint
//!    overfills and forces the shrink path; a pessimistic one underfills and
//!    forces grow passes) and highly compressible values (grow path with the
//!    default hint) all still satisfy 1–3 (hint cases of
//!    `*_fits_and_matches_plain_api`; compressible case of
//!    `test_range_sized_paging_covers_keyspace`).

#![expect(
    clippy::arithmetic_side_effects,
    reason = "test-only index and size arithmetic on small, bounded values"
)]

use std::collections::BTreeMap;
use std::num::NonZeroUsize;

use test_case::test_case;

use super::init_merkle;
use crate::api::{self, FrozenRangeProof};
use crate::db::BatchOp;
use crate::merkle::sized::SizedProof;
use crate::merkle::{Key, Merkle, Value};
use firewood_storage::TrieReader;

const SEED: u64 = 0x243F_6A88_85A3_08D3;

fn xorshift(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

/// Deterministic key-value pairs: 32-byte pseudo-random keys. Values start
/// with `0x42` so they can never parse as an RLP list: ethhash re-encodes
/// values that look like account RLP, which would break the byte-equality
/// assertions here. `compressible` selects constant-byte values (which
/// compress far below the default ratio estimate, so chunks must grow)
/// instead of pseudo-random ones (incompressible, so chunks must shrink).
fn test_kvs(n: usize, compressible: bool) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut seed = SEED;
    (0..n)
        .map(|_| {
            let key: Vec<u8> = (0..32)
                .map(|_| (xorshift(&mut seed) & 0xFF) as u8)
                .collect();
            let len = 8 + (xorshift(&mut seed) % 64) as usize;
            let value: Vec<u8> = if compressible {
                vec![0x42; 200 + len]
            } else {
                std::iter::once(0x42u8)
                    .chain((0..len).map(|_| (xorshift(&mut seed) & 0xFF) as u8))
                    .collect()
            };
            (key, value)
        })
        .collect()
}

/// Modify every 3rd value, delete every 7th key, and add fresh keys, so a
/// diff contains puts, deletes, and inserts.
fn modified_kvs(base: &[(Vec<u8>, Vec<u8>)]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out: Vec<(Vec<u8>, Vec<u8>)> = base
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 7 != 0)
        .map(|(i, (k, v))| {
            if i % 3 == 0 {
                let mut v2 = v.clone();
                v2.push(0x99);
                (k.clone(), v2)
            } else {
                (k.clone(), v.clone())
            }
        })
        .collect();
    for i in 0u32..200 {
        let mut key = vec![0xEEu8; 28];
        key.extend_from_slice(&i.to_be_bytes());
        out.push((key, vec![0x42, 0x01, 0x02]));
    }
    out
}

/// The kvs as the trie stores them: sorted, later duplicates winning.
fn sorted_unique(kvs: &[(Vec<u8>, Vec<u8>)]) -> Vec<(Vec<u8>, Vec<u8>)> {
    kvs.iter()
        .cloned()
        .collect::<BTreeMap<_, _>>()
        .into_iter()
        .collect()
}

/// A batch op reduced to comparable parts; the diff stream only emits `Put`
/// (value `Some`) and `Delete` (value `None`), so this is exact.
fn op_parts(op: &BatchOp<Key, Value>) -> (&[u8], Option<&[u8]>) {
    (&**op.key(), op.value().map(|v| &**v))
}

/// Page range proofs from the start of the keyspace until `natural_end`,
/// restarting each chunk at the successor of the previous last key.
fn page_range_chunks<T: TrieReader>(
    merkle: &Merkle<T>,
    budget: usize,
) -> Vec<SizedProof<FrozenRangeProof>> {
    let mut chunks = Vec::new();
    let mut start: Option<Vec<u8>> = None;
    loop {
        let sized = merkle
            .range_proof_sized(start.as_deref(), budget, None)
            .expect("sized proof");
        start = sized.proof.key_values().last().map(|(k, _)| {
            let mut next = k.to_vec();
            next.push(0);
            next
        });
        let natural_end = sized.natural_end;
        chunks.push(sized);
        if natural_end {
            return chunks;
        }
        assert!(start.is_some(), "non-final chunk must be non-empty");
        assert!(chunks.len() < 1000, "runaway paging");
    }
}

// Contracts 1, 2, and 6: fits the budget (except a lone entry), is
// byte-identical to the plain API at the same kv count, reports natural_end
// exactly when the whole keyspace was returned, and tolerates wrong hints.
#[test_case(512, None; "tiny budget forces a single entry")]
#[test_case(32 * 1024, None; "default ratio hint")]
#[test_case(32 * 1024, Some(0.05); "optimistic hint overfills then shrinks")]
#[test_case(32 * 1024, Some(2.0); "pessimistic hint underfills then grows")]
#[test_case(4 * 1024 * 1024, None; "budget covering the whole trie")]
fn test_range_sized_fits_and_matches_plain_api(budget: usize, ratio_hint: Option<f64>) {
    let kvs = test_kvs(2000, false);
    let total = sorted_unique(&kvs).len();
    let merkle = init_merkle(kvs.iter().map(|(k, v)| (k.clone(), v.clone())));

    let sized = merkle
        .range_proof_sized(None, budget, ratio_hint)
        .expect("sized proof");
    let kv_count = sized.proof.key_values().len();
    assert!(kv_count >= 1, "must return at least one kv");
    assert!(
        sized.wire.len() <= budget || kv_count == 1,
        "{} > {budget} with {kv_count} kvs",
        sized.wire.len()
    );
    assert_eq!(
        sized.natural_end,
        kv_count == total,
        "natural_end must mean the whole keyspace was returned"
    );

    let reference = merkle
        .range_proof(None, None, NonZeroUsize::new(kv_count))
        .expect("reference proof");
    let mut ref_wire = Vec::new();
    reference.write_to_vec(&mut ref_wire);
    assert_eq!(ref_wire, sized.wire, "sized proof differs from plain API");
}

#[test_case(512, None; "tiny budget forces a single op")]
#[test_case(16 * 1024, None; "default ratio hint")]
#[test_case(16 * 1024, Some(0.05); "optimistic hint overfills then shrinks")]
#[test_case(16 * 1024, Some(2.0); "pessimistic hint underfills then grows")]
#[test_case(4 * 1024 * 1024, None; "budget covering the whole diff")]
fn test_change_sized_fits_and_matches_plain_api(budget: usize, ratio_hint: Option<f64>) {
    let base = test_kvs(1500, false);
    let source = init_merkle(base.iter().map(|(k, v)| (k.clone(), v.clone())));
    let new = modified_kvs(&base);
    let target = init_merkle(new.iter().map(|(k, v)| (k.clone(), v.clone())));
    let total = target
        .change_proof(None, None, source.nodestore(), None)
        .expect("full diff")
        .batch_ops()
        .len();

    let sized = target
        .change_proof_sized(source.nodestore(), None, budget, ratio_hint)
        .expect("sized change proof");
    let op_count = sized.proof.batch_ops().len();
    assert!(op_count >= 1, "must return at least one op");
    assert!(
        sized.wire.len() <= budget || op_count == 1,
        "{} > {budget} with {op_count} ops",
        sized.wire.len()
    );
    assert_eq!(
        sized.natural_end,
        op_count == total,
        "natural_end must mean the whole diff was returned"
    );

    let reference = target
        .change_proof(None, None, source.nodestore(), NonZeroUsize::new(op_count))
        .expect("reference change proof");
    let mut ref_wire = Vec::new();
    reference.write_to_vec(&mut ref_wire);
    assert_eq!(
        ref_wire, sized.wire,
        "sized change proof differs from plain API"
    );
}

// Contract 3 (and 6 for the compressible case): paging covers the exact
// keyspace in order, every chunk fits the budget, and non-final chunks are
// at least half full — an implementation that never grew its estimate
// toward the budget would fail the fullness check on compressible data.
#[test_case(8 * 1024, false; "incompressible values")]
#[test_case(24 * 1024, false; "incompressible values with larger budget")]
#[test_case(8 * 1024, true; "compressible values exercise the grow path")]
fn test_range_sized_paging_covers_keyspace(budget: usize, compressible: bool) {
    let kvs = test_kvs(3000, compressible);
    let expected = sorted_unique(&kvs);
    let merkle = init_merkle(kvs.iter().map(|(k, v)| (k.clone(), v.clone())));

    let chunks = page_range_chunks(&merkle, budget);
    assert!(chunks.len() > 1, "budget should force multiple chunks");
    for (i, chunk) in chunks.iter().enumerate() {
        assert!(
            chunk.wire.len() <= budget,
            "chunk {i}: {} > {budget}",
            chunk.wire.len()
        );
        if !chunk.natural_end {
            assert!(
                chunk.wire.len() >= budget / 2,
                "chunk {i} underfilled: {} of {budget}",
                chunk.wire.len()
            );
        }
    }

    let collected: Vec<(Vec<u8>, Vec<u8>)> = chunks
        .iter()
        .flat_map(|c| c.proof.key_values().iter())
        .map(|(k, v)| (k.to_vec(), v.to_vec()))
        .collect();
    for (i, (got, want)) in collected.iter().zip(&expected).enumerate() {
        assert_eq!(got, want, "entry {i} out of sequence");
    }
    assert_eq!(
        collected.len(),
        expected.len(),
        "paging must cover the whole keyspace"
    );
}

// Contract 3 for change proofs: paging covers the exact diff in order.
#[test_case(4 * 1024)]
#[test_case(8 * 1024)]
fn test_change_sized_paging_covers_diff(budget: usize) {
    let base = test_kvs(2000, false);
    let source = init_merkle(base.iter().map(|(k, v)| (k.clone(), v.clone())));
    let new = modified_kvs(&base);
    let target = init_merkle(new.iter().map(|(k, v)| (k.clone(), v.clone())));

    // Reference diff: one unbounded change proof.
    let full = target
        .change_proof(None, None, source.nodestore(), None)
        .expect("full diff");
    let expected = full.batch_ops();
    assert!(expected.len() > 100, "diff should be non-trivial");

    let mut seen = 0usize;
    let mut start: Option<Vec<u8>> = None;
    let mut chunks = 0usize;
    loop {
        let sized = target
            .change_proof_sized(source.nodestore(), start.as_deref(), budget, None)
            .expect("sized change proof");
        assert!(
            sized.wire.len() <= budget,
            "chunk {chunks}: {} > {budget}",
            sized.wire.len()
        );
        if !sized.natural_end {
            assert!(
                sized.wire.len() >= budget / 2,
                "chunk {chunks} underfilled: {} of {budget}",
                sized.wire.len()
            );
        }
        for (i, op) in sized.proof.batch_ops().iter().enumerate() {
            assert_eq!(
                op_parts(op),
                op_parts(&expected[seen + i]),
                "chunk {chunks} op {i} out of sequence"
            );
        }
        seen += sized.proof.batch_ops().len();
        chunks += 1;
        if sized.natural_end {
            break;
        }
        let mut next = sized
            .proof
            .batch_ops()
            .last()
            .map(|op| op.key().to_vec())
            .expect("non-final chunk must be non-empty");
        next.push(0);
        start = Some(next);
        assert!(chunks < 1000, "runaway paging");
    }
    assert_eq!(seen, expected.len(), "paging must cover the whole diff");
    assert!(chunks > 1, "budget should force multiple chunks");
}

// Contract 4: an entry larger than the whole budget is returned alone in an
// over-budget chunk, and paging still covers everything around it.
#[test]
fn test_range_sized_progresses_past_oversized_entry() {
    let budget = 8 * 1024;
    let mut kvs = test_kvs(200, false);
    let big_key = vec![0x77u8; 32];
    let mut seed = SEED ^ 0xDEAD_BEEF;
    let big_value: Vec<u8> = std::iter::once(0x42u8)
        .chain((0..64 * 1024).map(|_| (xorshift(&mut seed) & 0xFF) as u8))
        .collect();
    kvs.push((big_key.clone(), big_value));
    let expected = sorted_unique(&kvs);
    let merkle = init_merkle(kvs.iter().map(|(k, v)| (k.clone(), v.clone())));

    let chunks = page_range_chunks(&merkle, budget);
    let collected: Vec<(Vec<u8>, Vec<u8>)> = chunks
        .iter()
        .flat_map(|c| c.proof.key_values().iter())
        .map(|(k, v)| (k.to_vec(), v.to_vec()))
        .collect();
    for (i, (got, want)) in collected.iter().zip(&expected).enumerate() {
        assert_eq!(got, want, "entry {i} out of sequence");
    }
    assert_eq!(
        collected.len(),
        expected.len(),
        "paging must cover the whole keyspace"
    );

    // An over-budget chunk is legitimate only when it cannot shrink further.
    // Beyond the oversized entry's own chunk, the successor chunk may also
    // exceed the budget: its start proof carries the node holding the value
    // (under ethhash the full RLP-encoded node).
    let over_budget: Vec<_> = chunks.iter().filter(|c| c.wire.len() > budget).collect();
    assert!(
        !over_budget.is_empty(),
        "the oversized entry must force an over-budget chunk"
    );
    for chunk in &over_budget {
        assert_eq!(
            chunk.proof.key_values().len(),
            1,
            "an over-budget chunk must be down to a single entry"
        );
    }
    assert!(
        over_budget
            .iter()
            .any(|c| &*c.proof.key_values()[0].0 == big_key.as_slice()),
        "the oversized entry's own chunk must be among the over-budget ones"
    );
}

// Contract 5: an empty trie errors exactly like the plain API.
#[test]
fn test_range_sized_empty_trie_errors_like_plain_api() {
    let merkle = init_merkle(Vec::<(Vec<u8>, Vec<u8>)>::new());
    assert!(matches!(
        merkle.range_proof_sized(None, 8 * 1024, None),
        Err(api::Error::RangeProofOnEmptyTrie)
    ));
    assert!(matches!(
        merkle.range_proof(None, None, None),
        Err(api::Error::RangeProofOnEmptyTrie)
    ));
}

// Contract 5: a start key past the last key yields an empty payload with
// natural_end set (terminating paging), byte-identical to the plain API.
#[test]
fn test_range_sized_start_past_last_key_yields_empty_natural_end() {
    let kvs = test_kvs(500, false);
    let merkle = init_merkle(kvs.iter().map(|(k, v)| (k.clone(), v.clone())));
    let start = vec![0xFFu8; 33]; // greater than every 32-byte key

    let sized = merkle
        .range_proof_sized(Some(&start), 8 * 1024, None)
        .expect("sized proof");
    assert!(sized.proof.key_values().is_empty());
    assert!(sized.natural_end);

    let plain = merkle
        .range_proof(Some(&start), None, None)
        .expect("plain proof");
    let mut plain_wire = Vec::new();
    plain.write_to_vec(&mut plain_wire);
    assert_eq!(plain_wire, sized.wire);
}

// Contract 5: identical tries yield an empty change proof with natural_end
// set (terminating paging), byte-identical to the plain API.
#[test]
fn test_change_sized_identical_tries_yield_empty_natural_end() {
    let kvs = test_kvs(300, false);
    let source = init_merkle(kvs.iter().map(|(k, v)| (k.clone(), v.clone())));
    let target = init_merkle(kvs.iter().map(|(k, v)| (k.clone(), v.clone())));

    let sized = target
        .change_proof_sized(source.nodestore(), None, 8 * 1024, None)
        .expect("sized change proof");
    assert!(sized.proof.batch_ops().is_empty());
    assert!(sized.natural_end);

    let plain = target
        .change_proof(None, None, source.nodestore(), None)
        .expect("plain change proof");
    let mut plain_wire = Vec::new();
    plain.write_to_vec(&mut plain_wire);
    assert_eq!(plain_wire, sized.wire);
}
