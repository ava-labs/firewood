// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Tests for size-targeted streaming proof generation.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "test-only index and size arithmetic on small, bounded values"
)]

use test_case::test_case;

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
            // First byte fixed below 0xC0 so no value can parse as an RLP list:
            // ethhash re-encodes values that look like account RLP, which would
            // break the byte-equality assertions here.
            let value: Vec<u8> = std::iter::once(0x42u8)
                .chain((0..(8 + (rand() % 64) as usize)).map(|_| (rand() & 0xFF) as u8))
                .collect();
            (key, value)
        })
        .collect()
}

fn modified_kvs(base: &[(Vec<u8>, Vec<u8>)]) -> Vec<(Vec<u8>, Vec<u8>)> {
    // Modify every 3rd value, delete every 7th key, add fresh keys.
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

// A tiny budget forces a single entry that can't fit (progress guarantee); a
// normal budget exercises the fit-and-fill path.
#[test_case(512)]
#[test_case(32 * 1024)]
fn test_range_sized_fits_and_matches_plain_api(budget: usize) {
    let kvs = test_kvs(2000);
    let merkle = init_merkle(kvs.iter().map(|(k, v)| (k.clone(), v.clone())));

    let sized = merkle
        .range_proof_sized(None, budget, None)
        .expect("sized proof");
    let kv_count = sized.proof.key_values().len();
    assert!(kv_count >= 1, "must return at least one kv");
    assert!(
        sized.wire.len() <= budget || kv_count == 1,
        "{} > {budget} with {kv_count} kvs",
        sized.wire.len()
    );

    // Byte-equivalence with the plain API at the same kv count.
    let reference = merkle
        .range_proof(None, None, std::num::NonZeroUsize::new(kv_count))
        .expect("reference proof");
    let mut ref_wire = Vec::new();
    reference.write_to_vec(&mut ref_wire);
    assert_eq!(ref_wire, sized.wire, "sized proof differs from plain API");
}

#[test_case(512)]
#[test_case(16 * 1024)]
fn test_change_sized_fits_and_matches_plain_api(budget: usize) {
    let base = test_kvs(1500);
    let source = init_merkle(base.iter().map(|(k, v)| (k.clone(), v.clone())));
    let new = modified_kvs(&base);
    let target = init_merkle(new.iter().map(|(k, v)| (k.clone(), v.clone())));

    let sized = target
        .change_proof_sized(source.nodestore(), None, budget, None)
        .expect("sized change proof");
    let op_count = sized.proof.batch_ops().len();
    assert!(op_count >= 1, "must return at least one op");
    assert!(
        sized.wire.len() <= budget || op_count == 1,
        "{} > {budget} with {op_count} ops",
        sized.wire.len()
    );

    let reference = target
        .change_proof(
            None,
            None,
            source.nodestore(),
            std::num::NonZeroUsize::new(op_count),
        )
        .expect("reference change proof");
    let mut ref_wire = Vec::new();
    reference.write_to_vec(&mut ref_wire);
    assert_eq!(
        ref_wire, sized.wire,
        "sized change proof differs from plain API"
    );
}

#[test_case(8 * 1024)]
#[test_case(24 * 1024)]
fn test_range_sized_paging_covers_keyspace(budget: usize) {
    let kvs = test_kvs(3000);
    // Later inserts overwrite earlier ones; mirror that with a BTreeMap.
    let sorted: Vec<(Vec<u8>, Vec<u8>)> = kvs
        .iter()
        .cloned()
        .collect::<std::collections::BTreeMap<_, _>>()
        .into_iter()
        .collect();
    let merkle = init_merkle(kvs.iter().map(|(k, v)| (k.clone(), v.clone())));

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
        if sized.natural_end {
            break;
        }
        let mut next = sized
            .proof
            .key_values()
            .last()
            .map(|(k, _)| k.to_vec())
            .expect("non-empty chunk");
        next.push(0);
        start = Some(next);
        assert!(chunks < 1000, "runaway paging");
    }
    assert_eq!(seen, sorted.len(), "paging must cover the whole keyspace");
    assert!(chunks > 1, "budget should force multiple chunks");
}

#[test_case(4 * 1024)]
#[test_case(8 * 1024)]
fn test_change_sized_paging_covers_diff(budget: usize) {
    let base = test_kvs(2000);
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
    let mut chunks = 0;
    loop {
        let sized = target
            .change_proof_sized(source.nodestore(), start.as_deref(), budget, None)
            .expect("sized change proof");
        for (i, op) in sized.proof.batch_ops().iter().enumerate() {
            assert_eq!(
                format!("{op:?}"),
                format!("{:?}", expected[seen + i]),
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
            .expect("non-empty chunk");
        next.push(0);
        start = Some(next);
        assert!(chunks < 1000, "runaway paging");
    }
    assert_eq!(seen, expected.len(), "paging must cover the whole diff");
    assert!(chunks > 1, "budget should force multiple chunks");
}
