// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Reads and writes against a file-backed database holding Ethereum-shaped
//! state: 32-byte account keys with RLP account values, and 64-byte storage
//! keys (account key followed by slot key) under a subset of the accounts.
//!
//! Unlike the in-memory benchmarks in `hashops`, every read here goes through
//! the node cache and, on a miss, the file. Each hash mode builds its database
//! once, then reopens it in these configurations:
//!
//! - `cold`: a node cache that never holds read nodes, so every node below
//!   the root is read and deserialized from the file. The file was just
//!   written, so those reads are served from the OS page cache.
//! - `hot`: a node cache large enough for the whole trie, warmed by reading
//!   every key once, with the default parallel-propose setting.
//! - `hot/serial` and `hot/parallel`: the `hot` cache with proposals always
//!   applied serially or always in parallel, for a sweep over batch sizes.
//!
//! The Prometheus collector is installed by default because production
//! callers install it and metric recording sits on the hot paths. Set
//! `FIREWOOD_BENCH_METRICS=0` to run without a recorder.
//!
//! The state size and block size can be changed with `FIREWOOD_BENCH_ACCOUNTS`,
//! `FIREWOOD_BENCH_SLOTS`, and `FIREWOOD_BENCH_BLOCK_OPS`.
//!
//! The `hot` configuration runs last because its commit benchmarks grow the
//! database by one block per commit.

use std::hint::black_box;
use std::num::NonZero;
use std::path::Path;
use std::sync::Barrier;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use criterion::BatchSize;
use criterion::BenchmarkGroup;
use criterion::Criterion;
use criterion::Throughput;
use criterion::criterion_group;
use criterion::criterion_main;
use criterion::measurement::WallTime;
use firewood::NodeHashAlgorithm;
use firewood::api;
use firewood::api::DynDb;
use firewood::db::BatchOp;
use firewood::db::DbConfig;
use firewood::db::UseParallel;
use firewood::manager::CacheReadStrategy;
use firewood::manager::RevisionManagerConfig;
use firewood_storage::SeededRng;

/// Keys read per iteration of a read benchmark, per thread.
const READS: usize = 1024;

/// Operations per batch while populating the database.
const POPULATE_CHUNK: usize = 50_000;

/// Fraction of the populated accounts that own storage.
const CONTRACT_FRACTION: usize = 20;

/// Node cache budget for the `hot` configuration: large enough to hold the
/// default state many times over.
const HOT_CACHE_BYTES: usize = 4 << 30;

/// Batch sizes for the serial and parallel propose sweep.
const SWEEP_OPS: [usize; 5] = [16, 64, 256, 1024, 4096];

/// Blocks committed in a loop while `during_commit` reads run.
const BACKGROUND_BLOCKS: usize = 256;

/// Node cache budget for the `cold` configuration. Reads are never cached, so
/// this only holds what a commit writes.
const COLD_CACHE_BYTES: usize = 1 << 20;

struct Params {
    accounts: usize,
    slots: usize,
    block_ops: usize,
}

impl Params {
    fn from_env() -> Self {
        Self {
            accounts: env_usize("FIREWOOD_BENCH_ACCOUNTS", 100_000),
            slots: env_usize("FIREWOOD_BENCH_SLOTS", 400_000),
            block_ops: env_usize("FIREWOOD_BENCH_BLOCK_OPS", 1_000),
        }
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name).ok().map_or(default, |v| {
        v.parse().unwrap_or_else(|e| panic!("{name}: {e}"))
    })
}

fn metrics_enabled() -> bool {
    !matches!(std::env::var("FIREWOOD_BENCH_METRICS").as_deref(), Ok("0"))
}

/// The keys present in the populated database.
struct State {
    accounts: Vec<[u8; 32]>,
    /// The accounts that own storage; a prefix of `accounts`.
    contracts: usize,
    slots: Vec<[u8; 64]>,
}

fn random_array<const N: usize>(rng: &SeededRng) -> [u8; N] {
    let mut out = [0u8; N];
    rng.fill_bytes(&mut out);
    out
}

fn account_value(rng: &SeededRng) -> Box<[u8]> {
    let nonce = rng.random_range(1u64..1_000).to_be_bytes();
    let balance = rng.random::<u64>().to_be_bytes();
    let mut s = rlp::RlpStream::new_list(4);
    s.append(&trim_leading_zeros(&nonce));
    s.append(&trim_leading_zeros(&balance));
    // The storage root is replaced during hashing in Ethereum mode.
    s.append(&[0u8; 32].as_slice());
    s.append(&random_array::<32>(rng).as_slice());
    s.out().to_vec().into_boxed_slice()
}

/// Returns an RLP-encoded storage value of 1 to 32 bytes, as Ethereum clients
/// store slot values.
fn slot_value(rng: &SeededRng) -> Box<[u8]> {
    let len = rng.random_range(1usize..=32);
    let mut raw = random_array::<32>(rng);
    if let Some(first) = raw.first_mut() {
        *first |= 1;
    }
    let bytes = raw.get(..len).expect("len is at most 32");
    rlp::encode(&bytes).to_vec().into_boxed_slice()
}

fn trim_leading_zeros(bytes: &[u8]) -> &[u8] {
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(bytes.len());
    bytes.get(start..).unwrap_or_default()
}

fn storage_key(account: &[u8; 32], slot: &[u8; 32]) -> [u8; 64] {
    let mut key = [0u8; 64];
    for (dst, src) in key.iter_mut().zip(account.iter().chain(slot)) {
        *dst = *src;
    }
    key
}

fn put(key: &[u8], value: Box<[u8]>) -> BatchOp<Box<[u8]>, Box<[u8]>> {
    BatchOp::Put {
        key: key.into(),
        value,
    }
}

impl State {
    fn generate(params: &Params, rng: &SeededRng) -> Self {
        let accounts: Vec<[u8; 32]> = (0..params.accounts).map(|_| random_array(rng)).collect();
        let contracts = (params.accounts / CONTRACT_FRACTION).max(1);
        let slots = (0..params.slots)
            .map(|_| {
                let owner = accounts
                    .get(rng.random_range(0..contracts))
                    .expect("contracts is at most accounts");
                storage_key(owner, &random_array(rng))
            })
            .collect();
        Self {
            accounts,
            contracts,
            slots,
        }
    }

    fn populate(&self, db: &dyn DynDb, rng: &SeededRng) {
        let ops: Vec<_> = self
            .accounts
            .iter()
            .map(|key| put(key, account_value(rng)))
            .chain(self.slots.iter().map(|key| put(key, slot_value(rng))))
            .collect();
        #[expect(clippy::disallowed_methods, reason = "POPULATE_CHUNK is non-zero")]
        for chunk in ops.chunks(POPULATE_CHUNK) {
            let batch: api::OwnedBatch = chunk.to_vec().into_boxed_slice();
            db.propose(batch)
                .expect("propose")
                .commit()
                .expect("commit");
        }
    }

    fn sample_accounts(&self, rng: &SeededRng, n: usize) -> Vec<Box<[u8]>> {
        (0..n)
            .map(|_| pick(&self.accounts, rng).as_slice().into())
            .collect()
    }

    fn sample_slots(&self, rng: &SeededRng, n: usize) -> Vec<Box<[u8]>> {
        (0..n)
            .map(|_| pick(&self.slots, rng).as_slice().into())
            .collect()
    }

    /// One account read for every three storage reads, roughly the mix of
    /// `BALANCE`/`EXTCODEHASH`-style and `SLOAD` reads in block execution.
    fn sample_mixed(&self, rng: &SeededRng, n: usize) -> Vec<Box<[u8]>> {
        (0..n)
            .map(|i| {
                if i % 4 == 0 {
                    pick(&self.accounts, rng).as_slice().into()
                } else {
                    pick(&self.slots, rng).as_slice().into()
                }
            })
            .collect()
    }

    /// Returns a block-sized batch: 60% updates of existing slots, 15% new
    /// slots under existing contracts, 20% account updates, and 5% new
    /// accounts.
    fn block(&self, rng: &SeededRng, ops: usize) -> api::OwnedBatch {
        (0..ops)
            .map(|_| match rng.random_range(0u32..100) {
                0..60 => put(pick(&self.slots, rng), slot_value(rng)),
                60..75 => {
                    let owner = self
                        .accounts
                        .get(rng.random_range(0..self.contracts))
                        .expect("contracts is at most accounts");
                    put(&storage_key(owner, &random_array(rng)), slot_value(rng))
                }
                75..95 => put(pick(&self.accounts, rng), account_value(rng)),
                _ => put(&random_array::<32>(rng), account_value(rng)),
            })
            .collect()
    }
}

fn pick<'a, T>(items: &'a [T], rng: &SeededRng) -> &'a T {
    items
        .get(rng.random_range(0..items.len()))
        .expect("index is below len")
}

fn open_db(dir: &Path, algorithm: NodeHashAlgorithm, config: Config) -> Box<dyn DynDb> {
    let (bytes, strategy) = match config {
        Config::Cold => (COLD_CACHE_BYTES, CacheReadStrategy::WritesOnly),
        Config::Hot | Config::HotSerial | Config::HotParallel => {
            (HOT_CACHE_BYTES, CacheReadStrategy::All)
        }
    };
    let mut cfg = DbConfig::builder()
        .manager(
            RevisionManagerConfig::builder()
                .node_cache_memory_limit(NonZero::new(bytes).expect("budget is non-zero"))
                .cache_read_strategy(strategy)
                .build(),
        )
        .build();
    match config {
        Config::HotSerial => cfg.use_parallel = UseParallel::Never,
        Config::HotParallel => cfg.use_parallel = UseParallel::Always,
        Config::Cold | Config::Hot => {}
    }
    firewood::open(dir, algorithm, cfg).expect("open database")
}

#[derive(Clone, Copy)]
enum Config {
    Cold,
    HotSerial,
    HotParallel,
    Hot,
}

impl Config {
    const ALL: [Config; 4] = [
        Config::Cold,
        Config::HotSerial,
        Config::HotParallel,
        Config::Hot,
    ];

    const fn name(self) -> &'static str {
        match self {
            Config::Cold => "cold",
            Config::HotSerial => "hot/serial",
            Config::HotParallel => "hot/parallel",
            Config::Hot => "hot",
        }
    }
}

fn latest_view(db: &dyn DynDb) -> api::ArcDynDbView {
    let root = db.root_hash().expect("populated database has a root");
    db.revision(root).expect("latest revision")
}

fn bench_reads(
    group: &mut BenchmarkGroup<'_, WallTime>,
    name: &str,
    view: &dyn api::DynDbView,
    keys: &[Box<[u8]>],
) {
    group.throughput(Throughput::Elements(keys.len() as u64));
    group.bench_function(name, |b| {
        b.iter(|| {
            for key in keys {
                black_box(view.val(key).expect("read"));
            }
        });
    });
}

/// Reads from `threads` threads at once, each over its own key list. One
/// iteration is one pass by every thread.
fn bench_concurrent_reads(
    group: &mut BenchmarkGroup<'_, WallTime>,
    view: &api::ArcDynDbView,
    keys: &[Vec<Box<[u8]>>],
) {
    let threads = keys.len();
    group.throughput(Throughput::Elements(
        keys.iter().map(Vec::len).sum::<usize>() as u64,
    ));
    group.bench_function(format!("read/revision/hot/mixed/threads_{threads}"), |b| {
        b.iter_custom(|iters| {
            let start = Barrier::new(threads.saturating_add(1));
            std::thread::scope(|s| {
                for thread_keys in keys {
                    let start = &start;
                    s.spawn(move || {
                        start.wait();
                        for _ in 0..iters {
                            for key in thread_keys {
                                black_box(view.val(key).expect("read"));
                            }
                        }
                    });
                }
                start.wait();
                let began = Instant::now();
                // Leaving the scope joins every thread.
                began
            })
            .elapsed()
        });
    });
}

/// Benchmarks for one open of the database.
struct Run<'a> {
    db: &'a dyn DynDb,
    state: &'a State,
    rng: &'a SeededRng,
    block_ops: usize,
    config: Config,
}

impl Run<'_> {
    fn warm(&self, view: &dyn api::DynDbView) {
        let accounts = self.state.accounts.iter().map(<[u8; 32]>::as_slice);
        let slots = self.state.slots.iter().map(<[u8; 64]>::as_slice);
        for key in accounts.chain(slots) {
            black_box(view.val(key).expect("warm"));
        }
    }

    fn revision_reads(&self, group: &mut BenchmarkGroup<'_, WallTime>, view: &dyn api::DynDbView) {
        let cache = self.config.name();
        let accounts = self.state.sample_accounts(self.rng, READS);
        bench_reads(
            group,
            &format!("read/revision/{cache}/account"),
            view,
            &accounts,
        );
        let slots = self.state.sample_slots(self.rng, READS);
        bench_reads(
            group,
            &format!("read/revision/{cache}/storage"),
            view,
            &slots,
        );
    }

    fn concurrent_reads(&self, group: &mut BenchmarkGroup<'_, WallTime>, view: &api::ArcDynDbView) {
        for threads in [1, 4, 8] {
            let keys: Vec<_> = (0..threads)
                .map(|_| self.state.sample_mixed(self.rng, READS))
                .collect();
            bench_concurrent_reads(group, view, &keys);
        }
    }

    fn proposal_reads(&self, group: &mut BenchmarkGroup<'_, WallTime>) {
        let proposal = self.db.propose(self.block()).expect("propose");
        let mixed = self.state.sample_mixed(self.rng, READS);
        bench_reads(group, "read/proposal/hot/mixed", &*proposal.view(), &mixed);
    }

    fn block(&self) -> api::OwnedBatch {
        self.state.block(self.rng, self.block_ops)
    }

    /// Proposes batches of several sizes, to compare serial and parallel
    /// application across the batch-size threshold.
    fn propose_sweep(&self, group: &mut BenchmarkGroup<'_, WallTime>) {
        for ops in SWEEP_OPS {
            group.throughput(Throughput::Elements(ops as u64));
            group.bench_function(format!("propose/{}/ops_{ops}", self.config.name()), |b| {
                b.iter_batched(
                    || self.state.block(self.rng, ops),
                    |batch| self.db.propose(batch).expect("propose"),
                    BatchSize::SmallInput,
                );
            });
        }
    }

    /// Runs `measure` while another thread commits blocks back to back.
    fn with_background_commits(&self, measure: impl FnOnce()) {
        let blocks: Vec<_> = (0..BACKGROUND_BLOCKS).map(|_| self.block()).collect();
        let db = self.db;
        let stop = AtomicBool::new(false);
        std::thread::scope(|s| {
            s.spawn(|| {
                for batch in blocks.iter().cycle() {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    db.propose(batch.clone())
                        .expect("propose")
                        .commit()
                        .expect("commit");
                }
            });
            measure();
            stop.store(true, Ordering::Relaxed);
        });
    }

    /// Reads while another thread commits, timed in two parts: looking up
    /// the latest revision, as a caller does once per block, and the reads
    /// against it.
    fn reads_during_commit(&self, group: &mut BenchmarkGroup<'_, WallTime>) {
        let keys = self.state.sample_mixed(self.rng, READS);
        let db = self.db;

        group.throughput(Throughput::Elements(1));
        group.bench_function("latest_revision/hot/during_commit", |b| {
            self.with_background_commits(|| b.iter(|| latest_view(db)));
        });

        group.throughput(Throughput::Elements(READS as u64));
        group.bench_function("read/revision/hot/mixed/during_commit", |b| {
            self.with_background_commits(|| {
                b.iter_custom(|iters| {
                    let mut total = Duration::ZERO;
                    for _ in 0..iters {
                        let view = latest_view(db);
                        let began = Instant::now();
                        for key in &keys {
                            black_box(view.val(key).expect("read"));
                        }
                        total = total.saturating_add(began.elapsed());
                    }
                    total
                });
            });
        });
    }

    fn propose(&self, group: &mut BenchmarkGroup<'_, WallTime>) {
        group.throughput(Throughput::Elements(self.block_ops as u64));
        group.bench_function(format!("propose/{}", self.config.name()), |b| {
            b.iter_batched(
                || self.block(),
                |batch| self.db.propose(batch).expect("propose"),
                BatchSize::SmallInput,
            );
        });
    }

    /// Proposes on top of an uncommitted proposal, as a block built on an
    /// unaccepted parent does.
    fn propose_on_proposal(&self, group: &mut BenchmarkGroup<'_, WallTime>) {
        group.throughput(Throughput::Elements(self.block_ops as u64));
        group.bench_function("propose/on_proposal/hot", |b| {
            b.iter_batched(
                || {
                    (
                        self.db.propose(self.block()).expect("propose"),
                        self.block(),
                    )
                },
                |(parent, batch)| {
                    let child = parent.propose(batch).expect("propose");
                    (parent, child)
                },
                BatchSize::SmallInput,
            );
        });
    }

    /// Proposes and commits one block per iteration, including any wait for
    /// the previous block to persist.
    fn commit(&self, group: &mut BenchmarkGroup<'_, WallTime>) {
        group.throughput(Throughput::Elements(self.block_ops as u64));
        group.bench_function("commit/hot", |b| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let batch = self.block();
                    let began = Instant::now();
                    self.db
                        .propose(batch)
                        .expect("propose")
                        .commit()
                        .expect("commit");
                    total = total.saturating_add(began.elapsed());
                }
                total
            });
        });
    }
}

fn bench_state(criterion: &mut Criterion) {
    if metrics_enabled() {
        metrics_collector_prometheus::PrometheusCollectorBuilder::new()
            .install()
            .expect("install the Prometheus collector");
    }
    let params = Params::from_env();

    for algorithm in [NodeHashAlgorithm::MerkleDB, NodeHashAlgorithm::Ethereum] {
        let rng = SeededRng::from_option(Some(1234));
        let state = State::generate(&params, &rng);
        let tmp = tempfile::tempdir().expect("create temp dir");
        let db = open_db(tmp.path(), algorithm, Config::Cold);
        state.populate(&*db, &rng);
        db.close().expect("close");

        let mut group = criterion.benchmark_group(format!("State/{algorithm:?}"));
        group.sample_size(20);

        for config in Config::ALL {
            let db = open_db(tmp.path(), algorithm, config);
            let run = Run {
                db: &*db,
                state: &state,
                rng: &rng,
                block_ops: params.block_ops,
                config,
            };

            match config {
                Config::Cold => {
                    run.revision_reads(&mut group, &*latest_view(&*db));
                    run.propose(&mut group);
                }
                Config::HotSerial | Config::HotParallel => {
                    run.warm(&*latest_view(&*db));
                    run.propose_sweep(&mut group);
                }
                Config::Hot => {
                    // Holding a revision keeps it from being reaped, so the
                    // commit benchmarks below run without one.
                    {
                        let view = latest_view(&*db);
                        run.warm(&*view);
                        run.revision_reads(&mut group, &*view);
                        run.concurrent_reads(&mut group, &view);
                    }
                    run.proposal_reads(&mut group);
                    run.propose(&mut group);
                    run.propose_on_proposal(&mut group);
                    run.reads_during_commit(&mut group);
                    run.commit(&mut group);
                }
            }

            db.close().expect("close");
        }

        group.finish();
    }
}

criterion_group!(benches, bench_state);
criterion_main!(benches);
