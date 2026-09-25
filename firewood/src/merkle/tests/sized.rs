// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Tests for [`Merkle::range_proof_sized`] and [`Merkle::change_proof_sized`].

#![expect(
    clippy::arithmetic_side_effects,
    reason = "test-only index and size arithmetic on small, bounded values"
)]

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use firewood_macros::hash_mode;
use firewood_storage::{
    Committed, DeletedNodeTracking, EthHash, HashMode, HashedNodeReader, MemStore, MerkleDbHash,
    NodeStore, SeededRng, TrieReader,
};
use test_case::test_case;

use super::init_merkle;
use crate::api::{self, FrozenChangeProof, FrozenRangeProof};
use crate::db::BatchOp;
use crate::merkle::sized::{CompressionRatio, SizedProof, SizingHint};
use crate::merkle::{Key, Merkle, Value};
use crate::proofs::{
    MAX_DECOMPRESSED_LEN, ProofError, lex_successor, verify_change_proof_structure,
    verify_range_proof_structure,
};

const SEED: u64 = 0x243F_6A88_85A3_08D3;

#[test_case(32 * 1024, None; "no hint")]
#[test_case(32 * 1024, Some(hint(5, 100)); "optimistic hint overfills then shrinks")]
#[test_case(32 * 1024, Some(hint(2, 1)); "pessimistic hint underfills then grows")]
#[test_case(32 * 1024, Some(hint(0, 0)); "zero hint is sanitized")]
#[test_case(4 * 1024 * 1024, None; "budget covering the whole trie")]
fn test_range_sized_matches_plain_proof(budget: usize, hint: Option<SizingHint>) {
    let kvs = test_kvs(2_000, false);
    let total = sorted_unique(&kvs).len();
    let merkle = init_merkle(kvs);

    let sized = merkle.range_proof_sized(None, budget, hint).unwrap();

    assert!(sized.wire.len() <= budget);
    assert_eq!(sized.natural_end, sized.proof.key_values().len() == total);
    verify_range_chunk(&merkle, None, &sized);
}

#[test_case(16 * 1024, None; "no hint")]
#[test_case(16 * 1024, Some(hint(5, 100)); "optimistic hint overfills then shrinks")]
#[test_case(16 * 1024, Some(hint(2, 1)); "pessimistic hint underfills then grows")]
#[test_case(16 * 1024, Some(hint(0, 0)); "zero hint is sanitized")]
#[test_case(4 * 1024 * 1024, None; "budget covering the whole diff")]
fn test_change_sized_matches_plain_proof(budget: usize, hint: Option<SizingHint>) {
    let base = test_kvs(1_500, false);
    let source = init_merkle(base.clone());
    let target = init_merkle(modified_kvs(&base));
    let total = full_diff(&target, &source).len();

    let sized = target
        .change_proof_sized(source.nodestore(), None, budget, hint)
        .unwrap();

    assert!(sized.wire.len() <= budget);
    assert_eq!(sized.natural_end, sized.proof.batch_ops().len() == total);
    verify_change_chunk(&target, &source, None, &sized);
}

#[test_case(8 * 1024, false; "incompressible values")]
#[test_case(8 * 1024, true; "compressible values")]
fn test_range_sized_paging_covers_keyspace(budget: usize, compressible: bool) {
    let kvs = test_kvs(3_000, compressible);
    let expected = sorted_unique(&kvs);
    let merkle = init_merkle(kvs);

    let chunks = page_range_chunks(&merkle, budget);

    assert!(chunks.len() > 1);
    assert_well_filled(&chunks, budget);
    assert!(
        chunks
            .iter()
            .flat_map(|c| c.proof.key_values().iter())
            .map(|(k, v)| (&**k, &**v))
            .eq(expected.iter().map(|(k, v)| (k.as_slice(), v.as_slice())))
    );
}

#[test_case(4 * 1024, false; "incompressible values")]
#[test_case(8 * 1024, true; "compressible values")]
fn test_change_sized_paging_covers_diff(budget: usize, compressible: bool) {
    let base = test_kvs(2_000, compressible);
    let source = init_merkle(base.clone());
    let target = init_merkle(modified_kvs(&base));
    let expected = full_diff(&target, &source);
    assert!(expected.len() > 100, "diff should be non-trivial");

    let chunks = page_change_chunks(&target, &source, budget);

    assert!(chunks.len() > 1);
    assert_well_filled(&chunks, budget);
    let ops: Vec<_> = chunks
        .iter()
        .flat_map(|c| c.proof.batch_ops().iter().cloned())
        .collect();
    assert_eq!(ops, expected);
}

/// A chained hint keeps every later chunk inside the acceptance window on
/// both compressible and incompressible data.
#[test_case(16 * 1024, false; "incompressible")]
#[test_case(16 * 1024, true; "compressible")]
fn test_range_sized_chained_hint_stays_in_window(budget: usize, compressible: bool) {
    let merkle = init_merkle(test_kvs(4_000, compressible));

    let chunks = page_range_chunks(&merkle, budget);

    assert!(chunks.len() > 3);
    for (i, chunk) in chunks.iter().enumerate().skip(1) {
        let len = chunk.wire.len();
        assert!(len <= budget, "chunk {i}: {len} > {budget}");
        if !chunk.natural_end {
            assert!(
                len * 100 >= budget * 90,
                "chunk {i} underfilled: {len} of {budget}"
            );
        }
    }
}

/// When the budget exceeds what the decoder's body limit allows, the body
/// limit bounds the chunk and nearly all of it is used.
#[test]
fn test_sized_body_limit_bounds_the_chunk() {
    let budget = 4 * 1024 * 1024;
    let base = half_entropy_kvs(22_000, 1024);
    let source = init_merkle(base.clone());
    let target = init_merkle(modified_kvs(&base));

    let range = target.range_proof_sized(None, budget, None).unwrap();
    let change = target
        .change_proof_sized(source.nodestore(), None, budget, None)
        .unwrap();

    for (wire, natural_end, body) in [
        (
            &range.wire,
            range.natural_end,
            body_of(|out| range.proof.write_body_to_vec(out)),
        ),
        (
            &change.wire,
            change.natural_end,
            body_of(|out| change.proof.write_body_to_vec(out)),
        ),
    ] {
        assert!(wire.len() <= budget);
        assert!(!natural_end, "corpus must exceed one chunk");
        assert!(body <= MAX_DECOMPRESSED_LEN);
        assert!(
            body >= MAX_DECOMPRESSED_LEN * 9 / 10,
            "body {body} uses less than 90% of the limit"
        );
    }
}

/// A few oversized values among small ones: shrinking must drop the heavy
/// tail entry, not a uniform share, so chunks stay well filled.
#[test]
fn test_range_sized_heavy_tail_entries_stay_well_filled() {
    let budget = 8 * 1024;
    let rng = SeededRng::new(SEED ^ 0x7E57);
    let mut kvs = test_kvs(600, false);
    for (_, value) in kvs.iter_mut().step_by(40) {
        *value = std::iter::once(0x42u8)
            .chain((0..3 * 1024).map(|_| rng.random()))
            .collect();
    }
    let merkle = init_merkle(kvs);

    let chunks = page_range_chunks(&merkle, budget);

    assert_well_filled(&chunks, budget);
}

/// Constant values compress beyond the ratio decoders accept; the
/// serializer refuses them rather than emitting an undecodable message.
#[test]
fn test_sized_constant_values_are_refused_as_undecodable() {
    let kvs: Vec<_> = (0u32..120)
        .map(|i| (i.to_be_bytes().to_vec(), vec![0x42; 64 * 1024]))
        .collect();
    let target = init_merkle(kvs);
    let source = init_merkle(Vec::<(Vec<u8>, Vec<u8>)>::new());

    assert!(matches!(
        target.range_proof_sized(None, usize::MAX, None),
        Err(api::Error::ProofError(
            ProofError::BodyTooCompressible { .. }
        ))
    ));
    assert!(matches!(
        target.change_proof_sized(source.nodestore(), None, usize::MAX, None),
        Err(api::Error::ProofError(
            ProofError::BodyTooCompressible { .. }
        ))
    ));
}

/// The budget is a message limit: a chunk that cannot fit it is an error,
/// never an oversized proof. Paging reaches the oversized entry, reports
/// it, and a budget that holds it pages through.
#[test]
fn test_sized_budget_below_one_entry_is_an_error() {
    let budget = 8 * 1024;
    let rng = SeededRng::new(SEED ^ 0xDEAD_BEEF);
    let mut kvs = test_kvs(200, false);
    let big_key = vec![0x77u8; 32];
    let big_value: Vec<u8> = std::iter::once(0x42u8)
        .chain((0..64 * 1024).map(|_| rng.random()))
        .collect();
    kvs.push((big_key.clone(), big_value));
    let expected = sorted_unique(&kvs);
    let merkle = init_merkle(kvs);
    let source = init_merkle(Vec::<(Vec<u8>, Vec<u8>)>::new());

    assert!(matches!(
        merkle.range_proof_sized(None, 0, None),
        Err(api::Error::ProofOverBudget { wire, budget: 0 }) if wire > 0
    ));
    assert!(matches!(
        merkle.change_proof_sized(source.nodestore(), None, 0, None),
        Err(api::Error::ProofOverBudget { wire, budget: 0 }) if wire > 0
    ));

    let mut start: Option<Vec<u8>> = None;
    let mut covered = 0usize;
    let err = loop {
        match merkle.range_proof_sized(start.as_deref(), budget, None) {
            Ok(chunk) => {
                assert!(chunk.wire.len() <= budget);
                assert!(!chunk.natural_end, "must stop at the big entry");
                covered += chunk.proof.key_values().len();
                start = chunk
                    .proof
                    .key_values()
                    .last()
                    .map(|(k, _)| lex_successor(k).into_vec());
            }
            Err(err) => break err,
        }
    };
    assert!(
        matches!(err, api::Error::ProofOverBudget { wire, budget: b } if wire > b && b == budget),
        "{err:?}"
    );
    let big_at = expected.iter().position(|(k, _)| *k == big_key).unwrap();
    assert_eq!(covered, big_at, "everything before the big entry pages");

    let chunks = page_range_chunks(&merkle, 128 * 1024);
    assert_eq!(
        chunks
            .iter()
            .map(|c| c.proof.key_values().len())
            .sum::<usize>(),
        expected.len()
    );
}

#[test]
fn test_sized_empty_trie_errors_like_plain_api() {
    let empty = init_merkle(Vec::<(Vec<u8>, Vec<u8>)>::new());
    let target = init_merkle([(b"key", b"value")]);

    assert!(matches!(
        empty.range_proof_sized(None, 8 * 1024, None),
        Err(api::Error::RangeProofOnEmptyTrie)
    ));
    assert!(matches!(
        empty.range_proof(None, None, None),
        Err(api::Error::RangeProofOnEmptyTrie)
    ));
    for start in [b"".as_slice(), b"key"] {
        assert!(matches!(
            empty.range_proof_sized(Some(start), 8 * 1024, None),
            Err(api::Error::ProofError(ProofError::Empty))
        ));
    }
    assert!(matches!(
        empty.change_proof_sized(target.nodestore(), None, 8 * 1024, None),
        Err(api::Error::ProofError(ProofError::Empty))
    ));
    assert!(matches!(
        empty.change_proof(None, None, target.nodestore(), None),
        Err(api::Error::ProofError(ProofError::Empty))
    ));
}

/// A start past the last key, or identical tries, yield an empty proof at
/// the natural end, byte-equal to the plain API's.
#[test]
fn test_sized_exhausted_inputs_yield_empty_natural_end() {
    let kvs = test_kvs(300, false);
    let merkle = init_merkle(kvs.clone());
    let past_end = vec![0xFFu8; 33]; // greater than every 32-byte key

    let range = merkle
        .range_proof_sized(Some(&past_end), 8 * 1024, None)
        .unwrap();
    assert!(range.proof.key_values().is_empty());
    assert!(range.natural_end);
    verify_range_chunk(&merkle, Some(&past_end), &range);

    let same = init_merkle(kvs);
    let change = merkle
        .change_proof_sized(same.nodestore(), None, 8 * 1024, None)
        .unwrap();
    assert!(change.proof.batch_ops().is_empty());
    assert!(change.natural_end);
    verify_change_chunk(&merkle, &same, None, &change);
}

#[test]
fn test_change_sized_from_empty_source_covers_target() {
    let target = init_merkle([(b"key", b"value")]);
    let empty = init_merkle(Vec::<(Vec<u8>, Vec<u8>)>::new());

    let change = target
        .change_proof_sized(empty.nodestore(), None, 4096, None)
        .unwrap();

    assert_eq!(change.proof.batch_ops().len(), 1);
    assert!(change.natural_end);
    verify_change_chunk(&target, &empty, None, &change);
}

/// Inclusive lower bounds on existing, absent, and prefix keys.
#[test]
fn test_sized_bounded_starts_and_prefix_keys() {
    let kvs = [b"".as_slice(), b"a", b"a\0", b"ab", b"abc", b"b", b"z"]
        .map(|key| (key.to_vec(), vec![0x42; 128]));
    let merkle = init_merkle(kvs.clone());
    let source = init_merkle(Vec::<(Vec<u8>, Vec<u8>)>::new());

    for start in [b"".as_slice(), b"a", b"aa", b"ab", b"zz"] {
        for budget in [1024, usize::MAX] {
            let range = merkle.range_proof_sized(Some(start), budget, None).unwrap();
            verify_range_chunk(&merkle, Some(start), &range);
            let change = merkle
                .change_proof_sized(source.nodestore(), Some(start), budget, None)
                .unwrap();
            verify_change_chunk(&merkle, &source, Some(start), &change);
        }
    }
}

/// Values and keys around the varint length-prefix boundaries.
#[test]
fn test_sized_length_prefix_boundaries() {
    let rng = SeededRng::new(SEED);
    for len in [127, 128, 16_383, 16_384] {
        let value: Vec<u8> = (0..len).map(|_| rng.random()).collect();
        let target = init_merkle([(vec![0x42; len.min(128)], value)]);
        let source = init_merkle(Vec::<(Vec<u8>, Vec<u8>)>::new());

        let range = target.range_proof_sized(None, usize::MAX, None).unwrap();
        assert_eq!(range.proof.key_values().len(), 1);
        assert!(range.natural_end);
        verify_range_chunk(&target, None, &range);

        let change = target
            .change_proof_sized(source.nodestore(), None, usize::MAX, None)
            .unwrap();
        assert_eq!(change.proof.batch_ops().len(), 1);
        assert!(change.natural_end);
        verify_change_chunk(&target, &source, None, &change);
    }
}

/// A committed source and an immutable proposal as the target.
#[hash_mode]
#[test]
fn test_sized_immutable_proposal_target<H: HashMode>() {
    let store: NodeStore<Committed, MemStore, H> = NodeStore::new_empty_committed(
        Arc::new(MemStore::new(Vec::new())),
        DeletedNodeTracking::Enabled,
    );
    let base = Merkle::from(store);
    let mut proposal = base.fork().unwrap();
    proposal.insert(b"key", b"value".as_slice().into()).unwrap();
    let target = proposal.hash();

    let range = target.range_proof_sized(None, 1_024, None).unwrap();
    assert_eq!(range.proof.hash_mode(), H::ALGORITHM);
    verify_range_chunk(&target, None, &range);

    // The source is a committed store, the target an immutable proposal,
    // so only the decoded structure can be verified here.
    let change = target
        .change_proof_sized(base.nodestore(), None, 1_024, None)
        .unwrap();
    let decoded = FrozenChangeProof::from_slice(&change.wire).unwrap();
    assert_eq!(decoded.hash_mode(), H::ALGORITHM);
    verify_change_proof_structure(
        &decoded,
        target.nodestore().root_hash().unwrap(),
        None,
        None,
        H::ALGORITHM,
        None,
    )
    .unwrap();
}

#[cfg(feature = "ethhash")]
#[test]
fn test_sized_account_with_storage_children() {
    let account = vec![0x42; 32];
    let value = super::ethhash::rlp_encode_account(1, 100, &[0; 32], &[0x55; 32]);
    let mut kvs = vec![(account.clone(), value.into_vec())];
    for suffix in [0x11, 0x22] {
        let key = [account.as_slice(), &[suffix; 32]].concat();
        kvs.push((key, super::ethhash::rlp_encode_storage(&[suffix; 32])));
    }
    let source = init_merkle(vec![kvs.first().unwrap().clone()]);
    let target = init_merkle(kvs);

    let chunks = page_range_chunks(&target, 64 * 1024);
    assert_eq!(
        chunks
            .iter()
            .map(|c| c.proof.key_values().len())
            .sum::<usize>(),
        3
    );
    let change = target
        .change_proof_sized(source.nodestore(), Some(&[]), 16 * 1024, None)
        .unwrap();
    verify_change_chunk(&target, &source, Some(&[]), &change);
}

fn hint(compressed: usize, uncompressed: usize) -> SizingHint {
    SizingHint {
        ratio: CompressionRatio::measured(compressed, uncompressed),
        edges: None,
    }
}

/// 32-byte pseudo-random keys. Values start with 0x42 so ethhash never
/// re-encodes them; `compressible` picks constant over pseudo-random bytes.
fn test_kvs(n: usize, compressible: bool) -> Vec<(Vec<u8>, Vec<u8>)> {
    let rng = SeededRng::new(SEED);
    (0..n)
        .map(|_| {
            let key: Vec<u8> = (0..32).map(|_| rng.random()).collect();
            let len = 8 + rng.random_range(0..64);
            let value: Vec<u8> = if compressible {
                vec![0x42; 200 + len]
            } else {
                std::iter::once(0x42u8)
                    .chain((0..len).map(|_| rng.random()))
                    .collect()
            };
            (key, value)
        })
        .collect()
}

/// 32-byte keys with `value_len`-byte values of ~3 bits of entropy per
/// byte, compressing ~2.5-3:1, so a body at the decoder's limit lands well
/// under a 4 MiB budget.
fn half_entropy_kvs(n: usize, value_len: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    let rng = SeededRng::new(SEED ^ 0x5DEE_CE66);
    (0..n)
        .map(|_| {
            let key: Vec<u8> = (0..32).map(|_| rng.random()).collect();
            let value: Vec<u8> = std::iter::once(0x42u8)
                .chain((1..value_len).map(|_| rng.random::<u8>() & 0x07))
                .collect();
            (key, value)
        })
        .collect()
}

/// Modify every third value, delete every seventh key, and scatter inserts.
fn modified_kvs(base: &[(Vec<u8>, Vec<u8>)]) -> Vec<(Vec<u8>, Vec<u8>)> {
    let mut out: Vec<(Vec<u8>, Vec<u8>)> = base
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 7 != 0)
        .map(|(i, (k, v))| {
            let mut v = v.clone();
            if i % 3 == 0 {
                v.push(0x99);
            }
            (k.clone(), v)
        })
        .collect();
    let rng = SeededRng::new(SEED ^ 1);
    for _ in 0..200 {
        let key: Vec<u8> = (0..32).map(|_| rng.random()).collect();
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

fn full_diff<T: HashedNodeReader>(
    target: &Merkle<T>,
    source: &Merkle<T>,
) -> Vec<BatchOp<Key, Value>> {
    target
        .change_proof(None, None, source.nodestore(), None)
        .unwrap()
        .batch_ops()
        .to_vec()
}

fn body_of(write: impl FnOnce(&mut Vec<u8>)) -> usize {
    let mut body = Vec::new();
    write(&mut body);
    body.len()
}

/// Page range proofs from the start until `natural_end`, verifying each.
fn page_range_chunks<T: TrieReader + HashedNodeReader>(
    merkle: &Merkle<T>,
    budget: usize,
) -> Vec<SizedProof<FrozenRangeProof>> {
    let mut chunks = Vec::new();
    let mut start: Option<Vec<u8>> = None;
    let mut hint = None;
    loop {
        let sized = merkle
            .range_proof_sized(start.as_deref(), budget, hint)
            .unwrap();
        hint = Some(sized.hint);
        verify_range_chunk(merkle, start.as_deref(), &sized);
        assert!(!sized.proof.key_values().is_empty(), "no empty pages");
        let next = sized
            .proof
            .key_values()
            .last()
            .map(|(k, _)| lex_successor(k).into_vec());
        assert!(next > start, "each page must advance");
        start = next;
        let natural_end = sized.natural_end;
        chunks.push(sized);
        if natural_end {
            return chunks;
        }
    }
}

/// Page change proofs from the start until `natural_end`, verifying each.
fn page_change_chunks<T: HashedNodeReader>(
    target: &Merkle<T>,
    source: &Merkle<T>,
    budget: usize,
) -> Vec<SizedProof<FrozenChangeProof>> {
    let mut chunks = Vec::new();
    let mut start: Option<Vec<u8>> = None;
    let mut hint = None;
    loop {
        let sized = target
            .change_proof_sized(source.nodestore(), start.as_deref(), budget, hint)
            .unwrap();
        hint = Some(sized.hint);
        verify_change_chunk(target, source, start.as_deref(), &sized);
        assert!(!sized.proof.batch_ops().is_empty(), "no empty pages");
        let next = sized
            .proof
            .batch_ops()
            .last()
            .map(|op| lex_successor(op.key()).into_vec());
        assert!(next > start, "each page must advance");
        start = next;
        let natural_end = sized.natural_end;
        chunks.push(sized);
        if natural_end {
            return chunks;
        }
    }
}

/// Every chunk fits the budget and, except the last, is at least half full.
fn assert_well_filled<P>(chunks: &[SizedProof<P>], budget: usize) {
    for (i, chunk) in chunks.iter().enumerate() {
        let len = chunk.wire.len();
        assert!(len <= budget, "chunk {i}: {len} > {budget}");
        if !chunk.natural_end {
            assert!(
                len >= budget / 2,
                "chunk {i} underfilled: {len} of {budget}"
            );
        }
    }
}

/// The wire decodes, verifies against the root, and is byte-equal to the
/// plain range proof of the same length.
fn verify_range_chunk<T: TrieReader + HashedNodeReader>(
    merkle: &Merkle<T>,
    start: Option<&[u8]>,
    chunk: &SizedProof<FrozenRangeProof>,
) {
    let decoded = FrozenRangeProof::from_slice(&chunk.wire).unwrap();
    verify_range_proof_structure(
        &decoded,
        merkle.nodestore().root_hash().unwrap(),
        start,
        None,
        merkle.nodestore().node_hash_algorithm(),
        None,
    )
    .unwrap();
    let plain = merkle
        .range_proof(start, None, NonZeroUsize::new(decoded.key_values().len()))
        .unwrap();
    let mut expected = Vec::new();
    plain.write_to_vec(&mut expected).unwrap();
    assert_eq!(chunk.wire, expected);
}

/// The wire decodes, verifies against the target root, and is byte-equal
/// to the plain change proof of the same length.
fn verify_change_chunk<T: HashedNodeReader>(
    target: &Merkle<T>,
    source: &Merkle<T>,
    start: Option<&[u8]>,
    chunk: &SizedProof<FrozenChangeProof>,
) {
    let decoded = FrozenChangeProof::from_slice(&chunk.wire).unwrap();
    verify_change_proof_structure(
        &decoded,
        target.nodestore().root_hash().unwrap(),
        start,
        None,
        target.nodestore().node_hash_algorithm(),
        None,
    )
    .unwrap();
    let plain = target
        .change_proof(
            start,
            None,
            source.nodestore(),
            NonZeroUsize::new(decoded.batch_ops().len()),
        )
        .unwrap();
    let mut expected = Vec::new();
    plain.write_to_vec(&mut expected).unwrap();
    assert_eq!(chunk.wire, expected);
}
