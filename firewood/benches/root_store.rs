// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::HashMap;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use firewood::root_store::{LMDBStore, RocksDBStore, RootStore, RootStoreBuilder, SQLiteStore};
use firewood_storage::{LinearAddress, SeededRng, TrieHash};
use rand::Rng;

fn bench_lmdb_writes_prepopulated<const NKEYS: usize>(criterion: &mut Criterion) {
    bench_writes_with_prepopulated_root_store::<LMDBStore>(criterion, NKEYS, "LMDB");
}

fn bench_rocksdb_writes_prepopulated<const NKEYS: usize>(criterion: &mut Criterion) {
    bench_writes_with_prepopulated_root_store::<RocksDBStore>(criterion, NKEYS, "RocksDB");
}

fn bench_sqlite_writes_prepopulated<const NKEYS: usize>(criterion: &mut Criterion) {
    bench_writes_with_prepopulated_root_store::<SQLiteStore>(criterion, NKEYS, "SQLite");
}

fn bench_lmdb_reads_prepopulated<const NKEYS: usize>(criterion: &mut Criterion) {
    bench_reads_with_prepopulated_root_state::<LMDBStore>(criterion, NKEYS, "LMDB");
}

fn bench_rocksdb_reads_prepopulated<const NKEYS: usize>(criterion: &mut Criterion) {
    bench_reads_with_prepopulated_root_state::<RocksDBStore>(criterion, NKEYS, "RocksDB");
}

fn bench_sqlite_reads_prepopulated<const NKEYS: usize>(criterion: &mut Criterion) {
    bench_reads_with_prepopulated_root_state::<SQLiteStore>(criterion, NKEYS, "SQLite");
}

// bench_writes_with_prepopulated_root_store benchmarks an instance of T by
// first prepopulating it with 1 million key-value pairs before benchmarking
// its write performance
#[expect(clippy::unwrap_used)]
fn bench_writes_with_prepopulated_root_store<T: RootStore + RootStoreBuilder<T>>(
    criterion: &mut Criterion,
    n_keys: usize,
    group_name: &str,
) {
    let mut rng = firewood_storage::SeededRng::new(1234);

    criterion
        .benchmark_group(group_name)
        .sample_size(30)
        .bench_function("writes against prepopulated RootStore", |b| {
            b.iter_batched(
                || {
                    let db_path = tempfile::tempdir().unwrap();
                    let root_store = T::new(&db_path).unwrap();

                    let prepopulated_pairs = new_kv_pairs(1_000_000, &mut rng);

                    root_store.add_roots_batch(&prepopulated_pairs).unwrap();

                    (root_store, new_kv_pairs(n_keys, &mut rng), db_path)
                },
                |(root_store, pairs, db_path)| {
                    for (key, value) in &pairs {
                        root_store.add_root(key, value).unwrap();
                    }

                    db_path.close().unwrap();
                },
                BatchSize::SmallInput,
            );
        });
}

// bench_reads_with_prepopulated_root_store benchmarks an instance of T by
// first prepopulating it with 1 million key-value pairs before benchmarking
// its read performance
#[expect(clippy::unwrap_used)]
fn bench_reads_with_prepopulated_root_state<T: RootStore + RootStoreBuilder<T>>(
    criterion: &mut Criterion,
    n_keys: usize,
    group_name: &str,
) {
    let mut rng = firewood_storage::SeededRng::new(1234);

    // Prepopulate ONCE before all iterations
    let db_path = tempfile::tempdir().unwrap();
    let root_store = T::new(&db_path).unwrap();

    let prepopulated_pairs = new_kv_pairs(1_000_000, &mut rng);
    root_store.add_roots_batch(&prepopulated_pairs).unwrap();

    let pairs = new_kv_pairs(n_keys, &mut rng);

    for (key, value) in &pairs {
        root_store.add_root(key, value).unwrap();
    }

    criterion
        .benchmark_group(group_name)
        .sample_size(20)
        .bench_function("reads against prepopulated state", |b| {
            b.iter(|| {
                for key in pairs.keys() {
                    root_store.get(key).unwrap();
                }
            });
        });

    db_path.close().unwrap();
}

// new_kv_pairs is a helper function that produces num key-value pairs
#[expect(clippy::unwrap_used)]
fn new_kv_pairs(num: usize, rng: &mut SeededRng) -> HashMap<TrieHash, LinearAddress> {
    (0..num)
        .map(|_| {
            let mut hash_bytes = [0u8; 32];
            rng.fill(&mut hash_bytes);

            let key = TrieHash::from_bytes(hash_bytes);
            let value = LinearAddress::new(rng.random_range(1..u64::MAX)).unwrap();

            (key, value)
        })
        .collect()
}

criterion_group! {
    name = benches;
    config = Criterion::default().measurement_time(std::time::Duration::from_secs(10));
    targets =
        bench_rocksdb_writes_prepopulated::<20>,
        bench_lmdb_writes_prepopulated::<20>,
        bench_sqlite_writes_prepopulated::<20>,
        bench_rocksdb_reads_prepopulated<20>,
        bench_lmdb_reads_prepopulated<20>,
        bench_sqlite_reads_prepopulated<20>,
}

criterion_main!(benches);
