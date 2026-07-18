// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use criterion::{Criterion, criterion_group, criterion_main};
use firewood::api::{Db as _, Proposal as _};
use firewood::db::{BatchOp, Db, DbConfig};
use firewood::manager::RevisionManagerConfig;
use firewood_storage::NodeHashAlgorithm;
use rand::{RngExt, distr::Alphanumeric};
use std::iter::repeat_with;
use std::num::NonZeroU64;

#[expect(clippy::unwrap_used)]
fn bench_deferred_persistence<const N: usize, const MAX_REVISION_RECOVERY_LAG: u64>(
    criterion: &mut Criterion,
) {
    const KEY_LEN: usize = 4;
    let rng = &firewood_storage::SeededRng::from_option(Some(1234));
    let max_revision_recovery_lag = NonZeroU64::new(MAX_REVISION_RECOVERY_LAG).unwrap();
    let max_revisions = max_revision_recovery_lag.get().wrapping_add(1) as usize;

    criterion
        .benchmark_group("deferred_persistence")
        .sample_size(20)
        .bench_function(
            format!("max_revision_recovery_lag_{MAX_REVISION_RECOVERY_LAG}"),
            |b| {
                b.iter_batched(
                    || {
                        let batch_ops: Vec<_> =
                            repeat_with(|| rng.sample_iter(&Alphanumeric).take(KEY_LEN).collect())
                                .map(|key: Vec<_>| BatchOp::Put {
                                    key,
                                    value: vec![b'v'],
                                })
                                .take(N)
                                .collect();
                        batch_ops
                    },
                    |batch_ops| {
                        let tmpdir = tempfile::tempdir().unwrap();
                        let dbcfg = DbConfig::builder()
                            .node_hash_algorithm(NodeHashAlgorithm::compile_option())
                            .manager(
                                RevisionManagerConfig::builder()
                                    .max_revisions(max_revisions)
                                    .max_revision_recovery_lag(max_revision_recovery_lag)
                                    .build(),
                            )
                            .build();
                        let db = Db::new(tmpdir, dbcfg).unwrap();

                        for op in batch_ops {
                            let proposal = db.propose(vec![op]).unwrap();
                            proposal.commit().unwrap();
                        }

                        db.close().unwrap();
                    },
                    criterion::BatchSize::SmallInput,
                );
            },
        );
}

// Recovery lag values span powers of 10 (1, 10, 100, 1_000) to show the
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
