// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use firewood::db::{BatchOp, Db, DbConfig};
use firewood::v2::api::{Db as _, DbView as _, Proposal as _, Reconstructible as _};
use firewood_storage::NodeHashAlgorithm;
use rand::{RngExt, distr::Alphanumeric};
use std::hint::black_box;
use std::iter::repeat_with;
use tempfile::TempDir;

const INITIAL_ITEMS: usize = 100;
const PROPOSAL_COUNT: usize = 2_048;
const PROPOSAL_ITEMS: usize = 100;
const KEY_LEN: usize = 16;
const VALUE_LEN: usize = 32;

type BenchOp = BatchOp<Vec<u8>, Vec<u8>>;
type BenchBatch = Vec<BenchOp>;

fn make_batch(rng: &firewood_storage::SeededRng, count: usize) -> BenchBatch {
    repeat_with(|| {
        let key: Vec<u8> = rng.sample_iter(&Alphanumeric).take(KEY_LEN).collect();
        let value: Vec<u8> = rng.sample_iter(&Alphanumeric).take(VALUE_LEN).collect();
        BatchOp::Put { key, value }
    })
    .take(count)
    .collect()
}

fn generate_batches() -> (BenchBatch, Vec<BenchBatch>) {
    let rng = &firewood_storage::SeededRng::from_option(Some(1234));
    let initial = make_batch(rng, INITIAL_ITEMS);
    let batches = (0..PROPOSAL_COUNT)
        .map(|_| make_batch(rng, PROPOSAL_ITEMS))
        .collect();
    (initial, batches)
}

#[expect(clippy::unwrap_used)]
fn bench_proposal_chain(criterion: &mut Criterion) {
    criterion
        .benchmark_group("proposal_chain")
        .sample_size(10)
        .warm_up_time(std::time::Duration::from_secs(5))
        .bench_function("nested", |b| {
            b.iter_batched(
                generate_batches,
                |(initial, batches)| {
                    let db_dir = TempDir::new().unwrap();
                    let db_path = db_dir.path().join("benchmark_db");
                    let cfg = DbConfig::builder()
                        .node_hash_algorithm(NodeHashAlgorithm::compile_option())
                        .truncate(true)
                        .build();
                    let db = Db::new(db_path, cfg).unwrap();

                    db.propose(initial).unwrap().commit().unwrap();

                    let mut batches_iter = batches.into_iter();
                    let first_batch = batches_iter.next().unwrap();
                    let mut proposal = db.propose(first_batch).unwrap();

                    for batch in batches_iter {
                        proposal = proposal.propose(batch).unwrap();
                    }

                    // Calculate the hash once at the end to ensure all work is actually done
                    black_box(proposal.root_hash());
                },
                BatchSize::SmallInput,
            );
        });
}

#[expect(clippy::unwrap_used)]
fn bench_reconstructed_chain(criterion: &mut Criterion) {
    criterion
        .benchmark_group("reconstructed_chain")
        .sample_size(10)
        .warm_up_time(std::time::Duration::from_secs(5))
        .bench_function("nested", |b| {
            b.iter_batched(
                generate_batches,
                |(initial, batches)| {
                    let db_dir = TempDir::new().unwrap();
                    let db_path = db_dir.path().join("benchmark_db");
                    let cfg = DbConfig::builder()
                        .node_hash_algorithm(NodeHashAlgorithm::compile_option())
                        .truncate(true)
                        .build();
                    let db = Db::new(db_path, cfg).unwrap();

                    db.propose(initial).unwrap().commit().unwrap();
                    let root_hash = db.root_hash().unwrap();
                    let historical = db.revision(root_hash).unwrap();

                    let mut batches_iter = batches.into_iter();
                    let first_batch = batches_iter.next().unwrap();
                    let mut reconstructed = historical.as_ref().reconstruct(first_batch).unwrap();

                    for batch in batches_iter {
                        reconstructed = reconstructed.reconstruct(batch).unwrap();
                    }

                    // Calculate the hash once at the end to ensure all work is actually done
                    black_box(reconstructed.root_hash());
                },
                BatchSize::SmallInput,
            );
        });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .measurement_time(std::time::Duration::from_secs(30))
        .warm_up_time(std::time::Duration::from_secs(5));
    targets = bench_proposal_chain, bench_reconstructed_chain
}

criterion_main!(benches);
