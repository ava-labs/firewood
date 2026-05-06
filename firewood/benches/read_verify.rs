// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// Read-time hash verification benchmarks.
//
// Populates a database once, then measures `val()` throughput under each
// HashVerification flag combination. The default config (no branch/leaf
// verification) is the baseline; the goal is to quantify the cost of
// turning each flag on so we can recommend defaults to operators.
//
// Run with: cargo bench --bench read_verify

#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use firewood::api::{Db as _, DbView as _, Proposal as _};
use firewood::db::{BatchOp, Db, DbConfig};
use firewood::manager::RevisionManagerConfig;
use firewood_storage::{HashVerification, NodeHashAlgorithm};
use nonzero_ext::nonzero;
use rand::{RngExt, distr::Alphanumeric};
use std::iter::repeat_with;
use tempfile::TempDir;

const NKEYS: usize = 10_000;
const KEY_LEN: usize = 32;
const READS_PER_ITER: usize = 200;

/// Populate a database in `dir` with NKEYS random key/value pairs.
fn populate(dir: &std::path::Path) -> Vec<Vec<u8>> {
    let rng = &firewood_storage::SeededRng::from_option(Some(0x00C0_FFEE));
    let keys: Vec<Vec<u8>> = repeat_with(|| rng.sample_iter(&Alphanumeric).take(KEY_LEN).collect())
        .take(NKEYS)
        .collect();

    let cfg = DbConfig::builder()
        .node_hash_algorithm(NodeHashAlgorithm::compile_option())
        .truncate(true)
        .build();
    let db = Db::new(dir, cfg).unwrap();

    let batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> = keys
        .iter()
        .cloned()
        .map(|key| BatchOp::Put {
            key,
            value: b"v".to_vec(),
        })
        .collect();
    db.propose(batch).unwrap().commit().unwrap();
    db.close().unwrap();

    keys
}

/// Open `dir` with the given verification policy, time `READS_PER_ITER`
/// random reads against the committed revision. Forces a tiny in-process
/// node cache (`cache_read_strategy` is already `WritesOnly` by default)
/// so the only steady-state caching effect is the OS page cache.
fn bench_reads_with_policy(c: &mut Criterion, label: &str, policy: HashVerification) {
    let tmpdir = TempDir::new().unwrap();
    let keys = populate(tmpdir.path());

    let manager = RevisionManagerConfig::builder()
        .node_cache_memory_limit(nonzero!(1usize))
        .build();
    let cfg = DbConfig::builder()
        .node_hash_algorithm(NodeHashAlgorithm::compile_option())
        .manager(manager)
        .hash_verification(policy)
        .build();
    let db = Db::new(tmpdir.path(), cfg).unwrap();
    let root_hash = db.root_hash().unwrap();
    let rev = db.revision(root_hash).unwrap();

    let rng = &firewood_storage::SeededRng::from_option(Some(0x0000_BEEF));

    c.benchmark_group("read_verify")
        .sample_size(30)
        .bench_function(label, |b| {
            b.iter_batched(
                || {
                    // Sample READS_PER_ITER random keys from the populated set.
                    let mut sample = Vec::with_capacity(READS_PER_ITER);
                    for _ in 0..READS_PER_ITER {
                        let idx = (rng.next_u64() as usize) % keys.len();
                        sample.push(keys[idx].clone());
                    }
                    sample
                },
                |sample| {
                    for key in sample {
                        let _ = rev.val(&key).unwrap();
                    }
                },
                BatchSize::SmallInput,
            );
        });

    db.close().unwrap();
}

fn bench_default(c: &mut Criterion) {
    bench_reads_with_policy(c, "default", HashVerification::default());
}

fn bench_branches(c: &mut Criterion) {
    let p = HashVerification {
        branches: true,
        ..HashVerification::default()
    };
    bench_reads_with_policy(c, "branches", p);
}

fn bench_leaves(c: &mut Criterion) {
    let p = HashVerification {
        leaves: true,
        ..HashVerification::default()
    };
    bench_reads_with_policy(c, "leaves", p);
}

fn bench_branches_and_leaves(c: &mut Criterion) {
    let p = HashVerification {
        branches: true,
        leaves: true,
        ..HashVerification::default()
    };
    bench_reads_with_policy(c, "branches+leaves", p);
}

fn bench_all(c: &mut Criterion) {
    bench_reads_with_policy(c, "all", HashVerification::all());
}

/// The recommended operator config: leaves on (cheap, catches the bulk
/// of corruption), both root flags on (one-time costs at open and
/// rootstore lookup), branches off (too expensive for steady state).
fn bench_recommended(c: &mut Criterion) {
    let p = HashVerification {
        root_from_rootstore: true,
        root_recent: true,
        leaves: true,
        branches: false,
        ..HashVerification::default()
    };
    bench_reads_with_policy(c, "recommended", p);
}

criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = bench_default, bench_branches, bench_leaves, bench_branches_and_leaves, bench_all, bench_recommended
}

criterion_main!(benches);
