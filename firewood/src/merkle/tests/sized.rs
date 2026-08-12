// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Tests for size-targeted streaming proof generation.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "test-only index and size arithmetic on small, bounded values"
)]

use super::init_merkle;

fn test_kvs(n: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    // Deterministic pseudo-random keys/values with realistic sizes.
    let mut seed = 0x243F_6A88_85A3_08D3u64;
    let mut rand = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    (0..n)
        .map(|_| {
            let key: Vec<u8> = (0..32).map(|_| (rand() & 0xFF) as u8).collect();
            // First byte fixed below 0xC0 so no value can parse as an RLP
            // list: ethhash's account handling re-encodes values that look
            // like account RLP, which would break the byte-equality
            // assertions here.
            let value: Vec<u8> = std::iter::once(0x42u8)
                .chain((0..(8 + (rand() % 64) as usize)).map(|_| (rand() & 0xFF) as u8))
                .collect();
            (key, value)
        })
        .collect()
}

fn modified_kvs(base: &[(Vec<u8>, Vec<u8>)]) -> Vec<(Vec<u8>, Vec<u8>)> {
    // Modify every 3rd value, delete every 7th key, add fresh keys — a
    // realistic diff shape.
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

#[test]
fn test_sized_fits_and_matches_plain_api() {
    let kvs = test_kvs(2000);
    let merkle = init_merkle(kvs.iter().map(|(k, v)| (k.clone(), v.clone())));
    let budget = 32 * 1024;

    let sized = merkle
        .range_proof_sized(None, budget, None)
        .expect("sized proof");
    assert!(
        sized.wire.len() <= budget,
        "{} > {budget}",
        sized.wire.len()
    );
    assert!(sized.stats.kv_count >= 1);
    assert_eq!(sized.stats.wire_len as usize, sized.wire.len());

    // Byte-equivalence with the plain API at the same kv count.
    let reference = merkle
        .range_proof(
            None,
            None,
            std::num::NonZeroUsize::new(sized.stats.kv_count as usize),
        )
        .expect("reference proof");
    let mut ref_wire = Vec::new();
    reference.write_to_vec(&mut ref_wire);
    assert_eq!(
        ref_wire, sized.wire,
        "sized proof differs from plain API at kv_count={}",
        sized.stats.kv_count
    );
}

#[test]
fn test_sized_paging_covers_keyspace() {
    let kvs = test_kvs(3000);
    // Later inserts overwrite earlier ones; mirror that with a BTreeMap.
    let sorted: Vec<(Vec<u8>, Vec<u8>)> = kvs
        .iter()
        .cloned()
        .collect::<std::collections::BTreeMap<_, _>>()
        .into_iter()
        .collect();
    let merkle = init_merkle(kvs.iter().map(|(k, v)| (k.clone(), v.clone())));
    let budget = 24 * 1024;

    let mut seen = 0usize;
    let mut start: Option<Vec<u8>> = None;
    let mut chunks = 0;
    loop {
        let sized = merkle
            .range_proof_sized(start.as_deref(), budget, None)
            .expect("sized proof");
        // Every chunk's keys must continue the sorted keyspace exactly.
        for (i, (k, v)) in sized.proof.key_values().iter().enumerate() {
            let (ek, ev) = &sorted[seen + i];
            assert_eq!(
                (k.as_ref(), v.as_ref()),
                (ek.as_slice(), ev.as_slice()),
                "chunk {chunks} entry {i} out of sequence"
            );
        }
        seen += sized.proof.key_values().len();
        chunks += 1;
        if sized.stats.natural_end {
            break;
        }
        let last = sized
            .proof
            .key_values()
            .last()
            .map(|(k, _)| k.to_vec())
            .expect("non-empty chunk");
        let mut next = last;
        next.push(0);
        start = Some(next);
        assert!(chunks < 1000, "runaway paging");
    }
    assert_eq!(seen, sorted.len(), "paging must cover the whole keyspace");
    assert!(chunks > 1, "budget should force multiple chunks");
}

#[test]
fn test_sized_single_kv_exceeding_budget_still_progresses() {
    let kvs = [
        (vec![1u8; 32], vec![7u8; 4096]),
        (vec![2u8; 32], vec![8u8; 4096]),
    ];
    let merkle = init_merkle(kvs.iter().map(|(k, v)| (k.clone(), v.clone())));
    let sized = merkle
        .range_proof_sized(None, 512, None)
        .expect("sized proof");
    assert!(
        sized.stats.kv_count >= 1,
        "must return at least one kv to make progress"
    );
}

#[test]
fn test_change_sized_fits_and_matches_plain_api() {
    let base = test_kvs(1500);
    let new = modified_kvs(&base);
    let source = init_merkle(base.iter().map(|(k, v)| (k.clone(), v.clone())));
    let target = init_merkle(new.iter().map(|(k, v)| (k.clone(), v.clone())));
    let budget = 16 * 1024;

    let sized = target
        .change_proof_sized(source.nodestore(), None, budget, None)
        .expect("sized change proof");
    assert!(
        sized.wire.len() <= budget,
        "{} > {budget}",
        sized.wire.len()
    );
    assert!(sized.stats.kv_count >= 1);

    let reference = target
        .change_proof(
            None,
            None,
            source.nodestore(),
            std::num::NonZeroUsize::new(sized.stats.kv_count as usize),
        )
        .expect("reference change proof");
    let mut ref_wire = Vec::new();
    reference.write_to_vec(&mut ref_wire);
    assert_eq!(
        ref_wire, sized.wire,
        "sized change proof differs from plain API at op_count={}",
        sized.stats.kv_count
    );
}

#[test]
fn test_change_sized_paging_covers_diff() {
    let base = test_kvs(2000);
    let new = modified_kvs(&base);
    let source = init_merkle(base.iter().map(|(k, v)| (k.clone(), v.clone())));
    let target = init_merkle(new.iter().map(|(k, v)| (k.clone(), v.clone())));

    // Reference diff: one unbounded change proof.
    let full = target
        .change_proof(None, None, source.nodestore(), None)
        .expect("full diff");
    let expected = full.batch_ops();
    assert!(expected.len() > 100, "diff should be non-trivial");

    let budget = 8 * 1024;
    let mut seen = 0usize;
    let mut start: Option<Vec<u8>> = None;
    let mut chunks = 0;
    loop {
        let sized = target
            .change_proof_sized(source.nodestore(), start.as_deref(), budget, None)
            .expect("sized change proof");
        for (i, op) in sized.proof.batch_ops().iter().enumerate() {
            let exp = &expected[seen + i];
            assert_eq!(
                format!("{op:?}"),
                format!("{exp:?}"),
                "chunk {chunks} op {i} out of sequence"
            );
        }
        seen += sized.proof.batch_ops().len();
        chunks += 1;
        if sized.stats.natural_end {
            break;
        }
        let last = sized
            .proof
            .batch_ops()
            .last()
            .map(|op| op.key().to_vec())
            .expect("non-empty chunk");
        let mut next = last;
        next.push(0);
        start = Some(next);
        assert!(chunks < 1000, "runaway paging");
    }
    assert_eq!(seen, expected.len(), "paging must cover the whole diff");
    assert!(chunks > 1, "budget should force multiple chunks");
}
