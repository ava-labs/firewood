// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Benchmark for `collapse_strip` recursion during change-proof verification.
//!
//! Builds a dense trie under a common 2-byte prefix, commits a start root, then
//! applies a tiny in-range update and benchmarks `Db::verify_change_proof`.
//! Verifying the change proof exercises `Merkle::collapse_strip` and the new
//! `child_in_range` recursion used to decide whether off-path children can be
//! stripped.

use criterion::{Criterion, criterion_group, criterion_main};
use firewood::{NodeHashAlgorithm, api::BatchOp, db::DbConfig, open};
use firewood_storage::SeededRng;

const KEY_COUNT: usize = 10_000;
const KEY_PREFIX: &[u8] = &[0xab, 0xcd];
const KEY_SUFFIX_LEN: usize = 30;
const KEY_LEN: usize = KEY_PREFIX.len() + KEY_SUFFIX_LEN;
const INDEX_END: usize = KEY_PREFIX.len() + 4;
const RANGE_SIZE: usize = 10;

type OwnedBatch = Box<[BatchOp<Box<[u8]>, Box<[u8]>>]>;

fn build_batch(keys: &[[u8; KEY_LEN]], value: &[u8]) -> OwnedBatch {
    keys.iter()
        .map(|key| BatchOp::Put {
            key: Box::from(key.as_slice()),
            value: Box::from(value),
        })
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

#[expect(clippy::indexing_slicing, clippy::arithmetic_side_effects)]
fn generate_sorted_keys(rng: &SeededRng) -> Vec<[u8; KEY_LEN]> {
    let mut keys: Vec<[u8; KEY_LEN]> = (0..KEY_COUNT)
        .map(|i| {
            let mut key = [0u8; KEY_LEN];

            for (j, &b) in KEY_PREFIX.iter().enumerate() {
                key[j] = b;
            }

            let idx = (i as u32).to_be_bytes();
            for (j, &b) in idx.iter().enumerate() {
                key[KEY_PREFIX.len() + j] = b;
            }

            rng.fill_bytes(&mut key[INDEX_END..]);
            key
        })
        .collect();

    keys.sort_unstable();
    keys.dedup();
    keys
}

fn bench_collapse(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("collapse");
    group.sample_size(30);

    let dir = tempfile::tempdir().expect("tempdir should be created");
    let cfg = DbConfig::builder()
        .node_hash_algorithm(NodeHashAlgorithm::MerkleDB)
        .truncate(true)
        .build();
    let db = open(dir.path(), cfg).expect("db should open");

    let rng = SeededRng::new(0xdead_beef);
    let keys = generate_sorted_keys(&rng);
    assert!(keys.len() >= KEY_COUNT);

    let initial_batch = build_batch(&keys, b"initial");
    db.propose(initial_batch)
        .expect("propose should succeed")
        .commit()
        .expect("commit should succeed");
    let start_root = db.root_hash().expect("start root should exist");

    let window_start = keys.len() / 2;
    let first = *keys.get(window_start).expect("index in range");
    let last = *keys
        .get(window_start.saturating_add(RANGE_SIZE - 1))
        .expect("index in range");

    let update_key = *keys
        .get(window_start.saturating_add(RANGE_SIZE / 2))
        .expect("index in range");
    let update_batch = Box::from([BatchOp::Put {
        key: Box::from(update_key.as_slice()),
        value: Box::from(b"updated".as_slice()),
    }]) as OwnedBatch;
    db.propose(update_batch)
        .expect("propose should succeed")
        .commit()
        .expect("commit should succeed");
    let end_root = db.root_hash().expect("end root should exist");

    let proof = db
        .change_proof(
            start_root,
            end_root.clone(),
            Some(first.as_slice()),
            Some(last.as_slice()),
            None,
        )
        .expect("change proof should be generated");

    group.bench_function("change_proof/verify/dense_subtrie", |b| {
        b.iter(|| {
            db.verify_change_proof(
                &proof,
                end_root.clone(),
                Some(first.as_slice()),
                Some(last.as_slice()),
                None,
            )
            .expect("verify change proof should succeed");
        });
    });

    group.finish();
}

criterion_group!(benches, bench_collapse);
criterion_main!(benches);
