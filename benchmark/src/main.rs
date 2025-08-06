// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.
//

#![expect(
    clippy::arithmetic_side_effects,
    reason = "Found 2 occurrences after enabling the lint."
)]
#![doc = include_str!("../README.md")]

mod create;
mod single;
mod tenkrandom;
mod zipf;

use std::borrow::Cow;
use std::iter::Map;
use std::net::{Ipv6Addr, SocketAddr};
use std::num::{NonZeroU64, NonZeroUsize};
use std::ops::Range;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use fastrace::collector::Config;
use fastrace_opentelemetry::OpenTelemetryReporter;
use firewood::db::{BatchOp, Db, DbConfig};
use firewood::logger::debug;
use firewood::manager::{CacheReadStrategy, RevisionManagerConfig};
use log::{LevelFilter, trace};
use metrics_exporter_prometheus::PrometheusBuilder;
use metrics_util::MetricKindMask;
use opentelemetry::InstrumentationScope;
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::Resource;
use pretty_duration::pretty_duration;
use sha2::{Digest, Sha256};

const ONE: NonZeroU64 = const { NonZeroU64::new(1).unwrap() };
const FOUR: NonZeroU64 = const { NonZeroU64::new(4).unwrap() };
const ONE_K: NonZeroU64 = const { NonZeroU64::new(1_000).unwrap() };
const TEN_K: NonZeroU64 = const { NonZeroU64::new(10_000).unwrap() };
const DEFAULT_CACHE_SIZE: NonZeroUsize = const { NonZeroUsize::new(1_500_000).unwrap() };
const DEFAULT_REVISIONS: NonZeroUsize = const { NonZeroUsize::new(128).unwrap() };

#[derive(Parser, Debug)]
struct Args {
    #[clap(flatten)]
    global_opts: GlobalOpts,

    #[clap(subcommand)]
    test_name: TestName,
}

#[derive(clap::Args, Debug)]
struct GlobalOpts {
    #[arg(
        short = 'e',
        long,
        default_value_t = false,
        help = "Enable telemetry server reporting"
    )]
    telemetry_server: bool,
    #[arg(short, long, default_value_t = TEN_K)]
    batch_size: NonZeroU64,
    #[arg(short, long, default_value_t = ONE_K)]
    number_of_batches: NonZeroU64,
    #[arg(short, long, default_value_t = DEFAULT_CACHE_SIZE)]
    cache_size: NonZeroUsize,
    #[arg(short, long, default_value_t = DEFAULT_REVISIONS)]
    revisions: NonZeroUsize,
    #[arg(
        short = 'p',
        long,
        default_value_t = 3000,
        help = "Port to listen for prometheus"
    )]
    prometheus_port: u16,
    #[arg(
        short = 's',
        long,
        default_value_t = false,
        help = "Dump prometheus stats on exit"
    )]
    stats_dump: bool,

    #[arg(
        long,
        short = 'l',
        required = false,
        help = "Log level. Respects RUST_LOG.",
        value_name = "LOG_LEVEL",
        num_args = 1,
        value_parser = ["trace", "debug", "info", "warn", "none"],
        default_value_t = String::from("info"),
    )]
    log_level: String,
    #[arg(
        long,
        short = 'd',
        required = false,
        help = "Use this database name instead of the default",
        default_value = PathBuf::from("benchmark_db").into_os_string(),
    )]
    dbname: PathBuf,
    #[arg(
        long,
        short = 't',
        required = false,
        help = "Terminate the test after this many minutes",
        default_value_t = 65
    )]
    duration_minutes: u64,
    #[arg(
        long,
        short = 'C',
        required = false,
        help = "Read cache strategy",
        value_enum,
        default_value_t = ArgCacheReadStrategy::WritesOnly
    )]
    cache_read_strategy: ArgCacheReadStrategy,
}

impl GlobalOpts {
    #[must_use]
    const fn duration(&self) -> Duration {
        Duration::from_secs(self.duration_minutes * 60)
    }

    const fn key_range(&self) -> Option<Range<u64>> {
        match self.number_of_batches.checked_mul(self.batch_size) {
            Some(count) => Some(0..count.get()),
            None => None,
        }
    }

    const fn key_range_batches(&self) -> Option<SplitRange> {
        match self.key_range() {
            Some(range) => Some(SplitRange {
                range,
                size: self.batch_size,
            }),
            None => None,
        }
    }
}

#[derive(Debug, PartialEq, ValueEnum, Clone)]
pub enum ArgCacheReadStrategy {
    WritesOnly,
    BranchReads,
    All,
}

impl From<ArgCacheReadStrategy> for CacheReadStrategy {
    fn from(arg: ArgCacheReadStrategy) -> Self {
        match arg {
            ArgCacheReadStrategy::WritesOnly => CacheReadStrategy::WritesOnly,
            ArgCacheReadStrategy::BranchReads => CacheReadStrategy::BranchReads,
            ArgCacheReadStrategy::All => CacheReadStrategy::All,
        }
    }
}

#[derive(Debug, Subcommand)]
enum TestName {
    /// Create a database
    Create(create::Create),

    /// Insert batches of random keys
    TenKRandom(tenkrandom::TenKRandom),

    /// Insert batches of keys following a Zipf distribution
    Zipf(zipf::Zipf),

    /// Repeatedly update a single row
    Single(single::Single),
}

trait TestRunner {
    async fn run(&self, db: &Db, args: &GlobalOpts) -> anyhow::Result<()>;
}

impl TestRunner for TestName {
    async fn run(&self, db: &Db, args: &GlobalOpts) -> anyhow::Result<()> {
        match self {
            TestName::Create(runner) => runner.run(db, args).await,
            TestName::TenKRandom(runner) => runner.run(db, args).await,
            TestName::Zipf(runner) => runner.run(db, args).await,
            TestName::Single(runner) => runner.run(db, args).await,
        }
    }
}

#[global_allocator]
#[cfg(unix)]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let mut args = Args::parse();

    if args.global_opts.telemetry_server {
        let reporter = OpenTelemetryReporter::new(
            SpanExporter::builder()
                .with_tonic()
                .with_endpoint("http://127.0.0.1:4317".to_string())
                .with_protocol(opentelemetry_otlp::Protocol::Grpc)
                .with_timeout(opentelemetry_otlp::OTEL_EXPORTER_OTLP_TIMEOUT_DEFAULT)
                .build()
                .expect("initialize oltp exporter"),
            Cow::Owned(
                Resource::builder()
                    .with_service_name("avalabs.firewood.benchmark")
                    .build(),
            ),
            InstrumentationScope::builder("firewood")
                .with_version(env!("CARGO_PKG_VERSION"))
                .build(),
        );
        fastrace::set_reporter(reporter, Config::default());
    }

    // set the number of batches to 1 for single tests (this influcences the key range)
    if matches!(args.test_name, TestName::Single(_)) {
        args.global_opts.number_of_batches = ONE;
        args.global_opts.batch_size = Ord::min(args.global_opts.batch_size, ONE_K);
    }

    env_logger::Builder::new()
        .filter(None, LevelFilter::Info)
        .write_style(env_logger::WriteStyle::Always)
        .target(env_logger::Target::Stderr)
        .filter_level(
            args.global_opts
                .log_level
                .as_str()
                .parse()
                .unwrap_or_else(|err| {
                    // cannot `warn!` because the logger is not set up yet
                    eprintln!(
                        "{err}: '{}'. Defaulting to 'info'",
                        args.global_opts.log_level
                    );
                    LevelFilter::Info
                }),
        )
        .init();
    debug!("logging initialized");

    // Manually set up prometheus
    let builder = PrometheusBuilder::new();
    let (prometheus_recorder, listener_future) = builder
        .with_http_listener(SocketAddr::new(
            Ipv6Addr::UNSPECIFIED.into(),
            args.global_opts.prometheus_port,
        ))
        .idle_timeout(
            MetricKindMask::COUNTER | MetricKindMask::HISTOGRAM,
            Some(Duration::from_secs(10)),
        )
        .build()
        .expect("unable in run prometheusbuilder");

    // Clone the handle so we can dump the stats at the end
    let prometheus_handle = prometheus_recorder.handle();
    metrics::set_global_recorder(prometheus_recorder)?;
    tokio::spawn(listener_future);

    let mgrcfg = RevisionManagerConfig::builder()
        .node_cache_size(args.global_opts.cache_size)
        .free_list_cache_size(
            args.global_opts
                .batch_size
                .checked_mul(const { NonZeroU64::new(4).unwrap() })
                .expect("batch_size * 4 should not overflow")
                .try_into()
                .expect("batch_size * 4 should fit in usize"),
        )
        .cache_read_strategy(args.global_opts.cache_read_strategy.clone().into())
        .max_revisions(args.global_opts.revisions.get())
        .build();
    let cfg = DbConfig::builder()
        .truncate(matches!(args.test_name, TestName::Create(_)))
        .manager(mgrcfg)
        .build();

    let db = Db::new(args.global_opts.dbname.clone(), cfg)
        .await
        .expect("db initiation should succeed");

    args.test_name.run(&db, &args.global_opts).await?;

    if args.global_opts.stats_dump {
        println!("{}", prometheus_handle.render());
    }

    fastrace::flush();

    Ok(())
}

type Sha256Digest = sha2::digest::Output<Sha256>;

fn keygen(idx: u64) -> (u64, Sha256Digest) {
    use std::cell::RefCell;

    struct HashCache<const N: usize> {
        cache: [Option<(u64, Sha256Digest)>; N],
    }

    impl<const N: usize> HashCache<N> {
        const fn new() -> Self {
            Self {
                cache: [const { None }; N],
            }
        }

        #[expect(clippy::indexing_slicing)]
        fn get(&mut self, key: u64) -> (u64, Sha256Digest) {
            #[inline]
            fn hash_idx(idx: u64) -> Sha256Digest {
                Sha256::digest(idx.to_ne_bytes())
            }

            // so we can test without caching
            if N == 0 {
                return (key, hash_idx(key));
            }

            let slot = (key as usize) % N;
            if let Some((cached_key, digest)) = self.cache[slot] {
                if cached_key == key {
                    return (cached_key, digest);
                }
            }

            let digest = hash_idx(key);
            self.cache[slot] = Some((key, digest));
            (key, digest)
        }
    }

    std::thread_local! {
        static CACHE: RefCell<HashCache<1024>> = const { RefCell::new(HashCache::new()) };
    }

    CACHE.with_borrow_mut(move |cache| cache.get(idx))
}

async fn repeat_for(
    duration: Duration,
    mut factory: impl AsyncFnMut(u64) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    const ONE_MINUTE: Duration = Duration::from_secs(60);

    let start = Instant::now();
    let mut last = start;
    let deadline = start + duration;
    for batch_id in (0..=u64::MAX).cycle() {
        factory(batch_id)
            .await
            .with_context(|| format!("failed to run batch {batch_id}"))?;

        let now = Instant::now();

        let elpased_1k = now.duration_since(last);
        if batch_id > 0
            && log::log_enabled!(log::Level::Debug)
            && (batch_id % 1000 == 0 || elpased_1k >= ONE_MINUTE)
        {
            let elapsed_all = now.duration_since(start);
            debug!(
                "completed {batch_id} batches in {} ({} total)",
                pretty_duration(&elpased_1k, None),
                pretty_duration(&elapsed_all, None),
            );

            last = now;
        }

        if now >= deadline {
            break;
        }
    }

    Ok(())
}

type Keygen = fn(u64) -> (u64, Sha256Digest);

trait KeygenExt: Sized {
    fn into_insert_op(self) -> BatchOp<Sha256Digest, Sha256Digest>;

    fn into_delete_op(self) -> BatchOp<Sha256Digest, Sha256Digest>;

    fn into_update_op(self, value: Sha256Digest) -> BatchOp<Sha256Digest, Sha256Digest>;
}

impl KeygenExt for (u64, Sha256Digest) {
    fn into_insert_op(self) -> BatchOp<Sha256Digest, Sha256Digest> {
        let (key, digest) = self;
        trace!("inserting {key} with digest {digest:064x}");
        BatchOp::Put {
            key: digest,
            value: digest,
        }
    }

    fn into_delete_op(self) -> BatchOp<Sha256Digest, Sha256Digest> {
        let (key, digest) = self;
        trace!("deleting {key} with digest {digest:064x}");
        BatchOp::Delete { key: digest }
    }

    fn into_update_op(self, value: Sha256Digest) -> BatchOp<Sha256Digest, Sha256Digest> {
        let (key, digest) = self;
        trace!("updating {key} with digest {digest:064x} to {value:064x}");
        BatchOp::Put { key: digest, value }
    }
}

impl KeygenExt for &(u64, Sha256Digest) {
    fn into_insert_op(self) -> BatchOp<Sha256Digest, Sha256Digest> {
        (*self).into_insert_op()
    }

    fn into_delete_op(self) -> BatchOp<Sha256Digest, Sha256Digest> {
        (*self).into_delete_op()
    }

    fn into_update_op(self, value: Sha256Digest) -> BatchOp<Sha256Digest, Sha256Digest> {
        (*self).into_update_op(value)
    }
}

impl KeygenExt for u64 {
    fn into_insert_op(self) -> BatchOp<Sha256Digest, Sha256Digest> {
        keygen(self).into_insert_op()
    }

    fn into_delete_op(self) -> BatchOp<Sha256Digest, Sha256Digest> {
        keygen(self).into_delete_op()
    }

    fn into_update_op(self, value: Sha256Digest) -> BatchOp<Sha256Digest, Sha256Digest> {
        keygen(self).into_update_op(value)
    }
}

impl KeygenExt for &u64 {
    fn into_insert_op(self) -> BatchOp<Sha256Digest, Sha256Digest> {
        (*self).into_insert_op()
    }

    fn into_delete_op(self) -> BatchOp<Sha256Digest, Sha256Digest> {
        (*self).into_delete_op()
    }

    fn into_update_op(self, value: Sha256Digest) -> BatchOp<Sha256Digest, Sha256Digest> {
        (*self).into_update_op(value)
    }
}

trait KeygenIterExt: Iterator<Item: KeygenExt> {
    fn iter_insert_ops(self) -> impl Iterator<Item = BatchOp<Sha256Digest, Sha256Digest>>
    where
        Self: Sized,
    {
        self.map(KeygenExt::into_insert_op)
    }

    fn iter_delete_ops(self) -> impl Iterator<Item = BatchOp<Sha256Digest, Sha256Digest>>
    where
        Self: Sized,
    {
        self.map(KeygenExt::into_delete_op)
    }

    fn iter_update_ops(
        self,
        value: Sha256Digest,
    ) -> impl Iterator<Item = BatchOp<Sha256Digest, Sha256Digest>>
    where
        Self: Sized,
    {
        self.map(move |keygen| keygen.into_update_op(value))
    }
}

impl<I: Iterator<Item: KeygenExt>> KeygenIterExt for I {}

trait RangeExt {
    fn into_range(self) -> Range<u64>;

    #[inline]
    fn split(self, size: NonZeroU64) -> SplitRange
    where
        Self: Sized,
    {
        SplitRange {
            range: self.into_range(),
            size,
        }
    }

    #[inline]
    fn hashed_key_range(self) -> Map<Range<u64>, Keygen>
    where
        Self: Sized,
    {
        self.into_range().map(keygen)
    }

    fn random_batch<D, R>(self, size: NonZeroU64, dist: D, rng: R) -> RandomBatch<D, R>
    where
        Self: Sized,
        D: rand::distr::Distribution<u64>,
        R: rand::Rng,
    {
        let range = self.into_range();
        let range_len = range
            .end
            .checked_sub(range.start)
            .and_then(NonZeroU64::new)
            .expect("range must not be empty");
        RandomBatch {
            range,
            range_len,
            size: (size.get() - 1) as usize,
            dist,
            rng,
        }
    }
}

impl RangeExt for Range<u64> {
    #[inline]
    fn into_range(self) -> Range<u64> {
        self
    }
}

impl RangeExt for &Range<u64> {
    #[inline]
    fn into_range(self) -> Range<u64> {
        self.clone()
    }
}

struct RandomBatch<D, R> {
    range: Range<u64>,
    range_len: NonZeroU64,
    size: usize,
    dist: D,
    rng: R,
}

impl<D, R> Iterator for RandomBatch<D, R>
where
    D: rand::distr::Distribution<u64>,
    R: rand::Rng,
{
    type Item = (u64, Sha256Digest);

    fn next(&mut self) -> Option<Self::Item> {
        self.size = self.size.checked_sub(1)?;
        let key = (self.dist.sample(&mut self.rng) % self.range_len) + self.range.start;
        Some(keygen(key))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.size;
        (len, Some(len))
    }
}

#[derive(Clone)]
struct SplitRange {
    range: Range<u64>,
    size: NonZeroU64,
}

impl SplitRange {
    const fn len(&self) -> usize {
        macro_rules! ok {
            ($val:expr) => {
                match $val {
                    Some(val) => val,
                    None => return 0,
                }
            };
        }

        let len = ok!(self.range.end.checked_sub(self.range.start));
        let len = ok!(NonZeroU64::new(len));
        let len = ok!(len.get().checked_div(self.size.get()));

        len as usize
    }
}

impl Iterator for SplitRange {
    type Item = Range<u64>;

    fn next(&mut self) -> Option<Self::Item> {
        let len = self
            .range
            .end
            .checked_sub(self.range.start)
            .and_then(NonZeroU64::new)?;
        let size = Ord::min(len, self.size).get();
        let result = self.range.start..self.range.start.checked_add(size)?;
        self.range.start = result.end;
        Some(result)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl ExactSizeIterator for SplitRange {
    fn len(&self) -> usize {
        self.len()
    }
}
