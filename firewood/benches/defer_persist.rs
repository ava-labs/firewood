// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::iter::repeat_with;
use std::num::NonZeroU64;

use criterion::{Criterion, criterion_group, criterion_main};
use firewood::api;
use firewood::db::{BatchOp, DbConfig};
use firewood::manager::RevisionManagerConfig;
use firewood::open;
use firewood_storage::{DefaultHashMode, HashMode};
use rand::{RngExt, distr::Alphanumeric};

#[expect(clippy::unwrap_used)]
fn bench_deferred_persistence<const N: usize, const COMMIT_COUNT: u64>(criterion: &mut Criterion) {
    const KEY_LEN: usize = 4;
    let rng = &firewood_storage::SeededRng::from_option(Some(1234));
    let commit_count = NonZeroU64::new(COMMIT_COUNT).unwrap();
    let max_revisions = commit_count.get().wrapping_add(1) as usize;

    criterion
        .benchmark_group("deferred_persistence")
        .sample_size(20)
        .bench_function(format!("commit_count_{COMMIT_COUNT}"), |b| {
            b.iter_batched(
                || {
                    let batch_ops: Vec<_> =
                        repeat_with(|| rng.sample_iter(&Alphanumeric).take(KEY_LEN).collect())
                            .map(|key: Vec<_>| BatchOp::Put {
                                key: key.into_boxed_slice(),
                                value: vec![b'v'].into_boxed_slice(),
                            })
                            .take(N)
                            .collect();
                    batch_ops
                },
                |batch_ops| {
                    let tmpdir = tempfile::tempdir().unwrap();
                    let dbcfg = DbConfig::builder()
                        .node_hash_algorithm(DefaultHashMode::ALGORITHM)
                        .manager(
                            RevisionManagerConfig::builder()
                                .max_revisions(max_revisions)
                                .deferred_persistence_commit_count(commit_count)
                                .build(),
                        )
                        .build();
                    let db = open(tmpdir, dbcfg).unwrap();

                    for op in batch_ops {
                        let batch: api::OwnedBatch = Box::new([op]);
                        let proposal = db.propose(batch).unwrap();
                        proposal.commit().unwrap();
                    }

                    db.close().unwrap();
                },
                criterion::BatchSize::SmallInput,
            );
        });
}

// Commit count values span powers of 10 (1, 10, 100, 1_000) to show the
// performance curve from persisting every commit to persisting just once.
criterion_group! {
    name = benches;
    config = Criterion::default();
    targets = bench_deferred_persistence::<1_000, 1>,
              bench_deferred_persistence::<1_000, 10>,
              bench_deferred_persistence::<1_000, 100>,
              bench_deferred_persistence::<1_000, 1_000>
}

criterion_main!(benches);
