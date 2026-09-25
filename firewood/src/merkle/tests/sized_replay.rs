// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Replay-log harness for [`Merkle::range_proof_sized`] /
//! [`Merkle::change_proof_sized`] against a real C-Chain snapshot.
//!
//! Ignored unless `SIZED_DB_DIR` and `SIZED_REPLAY_LOG` are set. The
//! database directory is mutated (the log is replayed into it), so point it
//! at a scratch copy. Env knobs:
//!
//! - `SIZED_DB_DIR`        directory holding `firewood.db` (a scratch copy)
//! - `SIZED_REPLAY_LOG`    FFI replay log recorded from that snapshot
//! - `SIZED_LOCAL_COMMIT`  change-proof base snapshot (default 8192)
//! - `SIZED_TARGET_COMMIT` commit proofs are generated at (default 16384)
//! - `SIZED_BUDGETS`       comma list of wire budgets (default: 256K..4M)
//! - `SIZED_RANGE_STARTS`  random range start keys per budget (default 16)
//! - `SIZED_RANGE_CHUNKS`  consecutive chunks paged per start (default 4)
//! - `SIZED_STRATEGIES`    `sized`, `bisect`, or both (default both)
//! - `SIZED_ROUNDS`        repeat the whole sweep N times (default 1); records
//!   carry `round` so a warm-cache round can be selected
//! - `SIZED_OUT`           JSONL output path (one record per chunk)
//!
//! Two sizing strategies run on the identical fixture: the streaming sizer
//! under test, and the doubling-probe-plus-bisection on `max_length` that
//! the current sync client uses, so bytes, messages and CPU compare directly.

#![expect(
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "harness-only arithmetic on bounded measurement values"
)]

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Write};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use firewood_metrics::{MetricsContext, set_metrics_context};
use metrics::{
    Counter, Gauge, Histogram, HistogramFn, Key as MetricKey, KeyName, Metadata, Recorder,
    SharedString, Unit,
};
use serde::Deserialize;
use serde_bytes::ByteBuf;

use crate::api::{self, Db as _, FrozenChangeProof, FrozenRangeProof, HashKey, Proposal as _};
use crate::db::{BatchOp, Db, DbConfig, Proposal};
use crate::manager::RevisionManagerConfig;
use crate::merkle::sized::SizingHint;
use crate::merkle::{Key, Merkle, Value};
use crate::proofs::ProofError;
use firewood_storage::{Committed, DefaultHashMode, FileBacked, HashMode, NodeStore};

type Rev = Arc<NodeStore<Committed, FileBacked, DefaultHashMode>>;

// ---------------------------------------------------------------------------
// configuration

struct Config {
    db_dir: PathBuf,
    log: PathBuf,
    local_commit: u64,
    target_commit: u64,
    budgets: Vec<usize>,
    range_starts: usize,
    range_chunks: usize,
    strategies: Vec<Strategy>,
    rounds: usize,
    out: PathBuf,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Strategy {
    Sized,
    Bisect,
}

/// One measured arm.
#[derive(Clone)]
struct Arm {
    strategy: Strategy,
    name: String,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn env_num<T: std::str::FromStr>(name: &str, default: T) -> T {
    env(name).and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn parse_size(s: &str) -> usize {
    let s = s.trim();
    let (num, mult) = match s.chars().last() {
        Some('K' | 'k') => (&s[..s.len() - 1], 1024),
        Some('M' | 'm') => (&s[..s.len() - 1], 1024 * 1024),
        _ => (s, 1),
    };
    num.trim().parse::<usize>().expect("budget") * mult
}

impl Config {
    fn from_env() -> Option<Self> {
        let db_dir = PathBuf::from(env("SIZED_DB_DIR")?);
        let log = PathBuf::from(env("SIZED_REPLAY_LOG")?);
        let budgets = env("SIZED_BUDGETS").map_or_else(
            || {
                vec![
                    256 * 1024,
                    512 * 1024,
                    1024 * 1024,
                    2 * 1024 * 1024 - 4 * 1024,
                    4 * 1024 * 1024,
                ]
            },
            |v| v.split(',').map(parse_size).collect(),
        );
        let strategies = env("SIZED_STRATEGIES").map_or_else(
            || vec![Strategy::Sized, Strategy::Bisect],
            |v| {
                v.split(',')
                    .map(|s| match s.trim() {
                        "sized" => Strategy::Sized,
                        "bisect" => Strategy::Bisect,
                        other => panic!("unknown strategy {other}"),
                    })
                    .collect()
            },
        );
        Some(Self {
            db_dir,
            log,
            local_commit: env_num("SIZED_LOCAL_COMMIT", 8192),
            target_commit: env_num("SIZED_TARGET_COMMIT", 16384),
            budgets,
            range_starts: env_num("SIZED_RANGE_STARTS", 16),
            range_chunks: env_num("SIZED_RANGE_CHUNKS", 4),
            strategies,
            rounds: env_num("SIZED_ROUNDS", 1),
            out: PathBuf::from(env("SIZED_OUT").unwrap_or_else(|| "sized-replay.jsonl".into())),
        })
    }
}

// ---------------------------------------------------------------------------
// lenient replay-log decoding
//
// Every op is a one-key msgpack map keyed by the operation name, and structs
// are maps, so a struct of optional fields decodes any op: unknown operations
// (the read ops that dominate this log) leave every field `None`.

#[derive(Deserialize)]
struct Segment {
    operations: Vec<Op>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Op {
    #[serde(rename = "Batch")]
    batch: Option<Pairs>,
    #[serde(rename = "ProposeOnDB")]
    propose_on_db: Option<Propose>,
    #[serde(rename = "ProposeOnProposal")]
    propose_on_proposal: Option<ProposeOnProposal>,
    #[serde(rename = "Commit")]
    commit: Option<Commit>,
}

#[derive(Deserialize)]
struct Pair {
    key: ByteBuf,
    #[serde(default)]
    value: Option<ByteBuf>,
    #[serde(default)]
    delete_exact: bool,
}

#[derive(Deserialize)]
struct Pairs {
    pairs: Vec<Pair>,
}

#[derive(Deserialize)]
struct Propose {
    pairs: Vec<Pair>,
    #[serde(default)]
    returned_proposal_id: Option<u64>,
}

#[derive(Deserialize)]
struct ProposeOnProposal {
    proposal_id: u64,
    pairs: Vec<Pair>,
    #[serde(default)]
    returned_proposal_id: Option<u64>,
}

#[derive(Deserialize)]
struct Commit {
    proposal_id: u64,
}

fn to_batch(pairs: Vec<Pair>) -> Vec<BatchOp<Key, Value>> {
    pairs
        .into_iter()
        .map(|p| {
            let key: Key = p.key.into_vec().into_boxed_slice();
            match p.value {
                Some(v) => BatchOp::Put {
                    key,
                    value: v.into_vec().into_boxed_slice(),
                },
                None if p.delete_exact => BatchOp::Delete { key },
                None => BatchOp::DeleteRange { prefix: key },
            }
        })
        .collect()
}

struct Fixture {
    local_root: HashKey,
    target_root: HashKey,
    commits: u64,
    replay: Duration,
}

/// Replays write operations from `log` into `db` until `target_commit`
/// commits have landed, capturing the root after `local_commit`.
fn replay(db: &Db<DefaultHashMode>, cfg: &Config) -> Fixture {
    let started = Instant::now();
    let mut reader = BufReader::with_capacity(1 << 20, File::open(&cfg.log).expect("open log"));
    let mut proposals: HashMap<u64, Proposal<'_, DefaultHashMode>> = HashMap::new();
    let mut commits = 0u64;
    let mut local_root = None;

    let on_commit = |commits: u64, local_root: &mut Option<HashKey>| -> bool {
        if commits == cfg.local_commit {
            *local_root = Some(api::Db::root_hash(db).expect("root after local commit"));
        }
        if commits.is_multiple_of(1024) {
            eprintln!(
                "  replayed {commits} commits ({:.0?} elapsed)",
                started.elapsed()
            );
        }
        commits >= cfg.target_commit
    };

    'outer: loop {
        let mut len_buf = [0u8; 8];
        if let Err(e) = reader.read_exact(&mut len_buf) {
            assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof, "{e}");
            break;
        }
        let len = u64::from_le_bytes(len_buf) as usize;
        if len == 0 {
            continue;
        }
        let mut buf = vec![0u8; len];
        reader.read_exact(&mut buf).expect("segment payload");
        let seg: Segment = rmp_serde::from_slice(&buf).expect("decode segment");

        for op in seg.operations {
            if let Some(b) = op.batch {
                let prop = db.propose(to_batch(b.pairs)).expect("batch propose");
                prop.commit().expect("batch commit");
                commits += 1;
                if on_commit(commits, &mut local_root) {
                    break 'outer;
                }
            }
            if let Some(p) = op.propose_on_db {
                let prop = db.propose(to_batch(p.pairs)).expect("propose on db");
                if let Some(id) = p.returned_proposal_id {
                    proposals.insert(id, prop);
                }
            }
            if let Some(p) = op.propose_on_proposal {
                let parent = proposals
                    .get(&p.proposal_id)
                    .expect("known parent proposal");
                let prop = parent
                    .propose(to_batch(p.pairs))
                    .expect("propose on proposal");
                if let Some(id) = p.returned_proposal_id {
                    proposals.insert(id, prop);
                }
            }
            if let Some(c) = op.commit {
                let prop = proposals.remove(&c.proposal_id).expect("known proposal");
                prop.commit().expect("commit");
                commits += 1;
                if on_commit(commits, &mut local_root) {
                    break 'outer;
                }
            }
        }
    }
    assert_eq!(commits, cfg.target_commit, "replay log too short");
    Fixture {
        local_root: local_root.expect("local root captured"),
        target_root: api::Db::root_hash(db).expect("target root"),
        commits,
        replay: started.elapsed(),
    }
}

// ---------------------------------------------------------------------------
// measurement records

#[derive(Default)]
struct Rec {
    kind: &'static str,
    strategy: String,
    round: usize,
    budget: usize,
    start: usize,
    chunk: usize,
    items: usize,
    wire: usize,
    body: usize,
    natural_end: bool,
    /// Candidates built and serialized to arrive at the chunk.
    builds: usize,
    /// Edge and prefix body bytes the chunk measured (sized only).
    edges: usize,
    ratio_scaled: u32,
    hint_scaled: Option<u32>,
    size_ns: u128,
    marshal_ns: u128,
    decode_ns: u128,
    roundtrip_ok: bool,
    edge_nodes: usize,
}

impl Rec {
    fn write_json(&self, out: &mut impl Write) {
        let hint = self
            .hint_scaled
            .map_or_else(|| "null".to_owned(), |h| h.to_string());
        writeln!(
            out,
            concat!(
                r#"{{"kind":"{}","strategy":"{}","round":{},"budget":{},"start":{},"chunk":{},"items":{},"#,
                r#""wire":{},"body":{},"natural_end":{},"builds":{},"edges":{},"#,
                r#""ratio_scaled":{},"hint_scaled":{},"size_ns":{},"marshal_ns":{},"#,
                r#""decode_ns":{},"roundtrip_ok":{},"edge_nodes":{}}}"#
            ),
            self.kind,
            self.strategy,
            self.round,
            self.budget,
            self.start,
            self.chunk,
            self.items,
            self.wire,
            self.body,
            self.natural_end,
            self.builds,
            self.edges,
            self.ratio_scaled,
            hint,
            self.size_ns,
            self.marshal_ns,
            self.decode_ns,
            self.roundtrip_ok,
            self.edge_nodes,
        )
        .expect("write record");
    }
}

/// Canonical (uncompressed) body length, marshal/decode timings, and a
/// decode round-trip check for a range proof.
fn measure_range(proof: &FrozenRangeProof, wire: &[u8], rec: &mut Rec) {
    let mut body = Vec::new();
    proof.write_body_to_vec(&mut body);
    rec.body = body.len();
    let t = Instant::now();
    let mut again = Vec::new();
    proof.write_to_vec(&mut again).expect("marshal");
    rec.marshal_ns = t.elapsed().as_nanos();
    let t = Instant::now();
    let decoded = FrozenRangeProof::from_slice(wire).expect("decode");
    rec.decode_ns = t.elapsed().as_nanos();
    rec.roundtrip_ok = decoded == *proof && again == wire;
    rec.edge_nodes = proof.start_proof().len() + proof.end_proof().len();
}

fn measure_change(proof: &FrozenChangeProof, wire: &[u8], rec: &mut Rec) {
    let mut body = Vec::new();
    proof.write_body_to_vec(&mut body);
    rec.body = body.len();
    let t = Instant::now();
    let mut again = Vec::new();
    proof.write_to_vec(&mut again).expect("marshal");
    rec.marshal_ns = t.elapsed().as_nanos();
    let t = Instant::now();
    let decoded = FrozenChangeProof::from_slice(wire).expect("decode");
    rec.decode_ns = t.elapsed().as_nanos();
    rec.roundtrip_ok = decoded == *proof && again == wire;
    rec.edge_nodes = proof.start_proof().len() + proof.end_proof().len();
}

fn next_key(last: &[u8]) -> Vec<u8> {
    let mut next = last.to_vec();
    next.push(0);
    next
}

// ---------------------------------------------------------------------------
// the current client's sizing: doubling probe then bisection on max_length

struct Fit<P> {
    proof: P,
    wire: Vec<u8>,
    probes: usize,
    natural: bool,
}

/// `probe(max_len)` returns the proof, its wire bytes (None when the body
/// exceeds the serializer's cap, i.e. certainly over budget) and whether the
/// proof ran out of items before `max_len`.
fn fit_budget<P>(
    budget: usize,
    mut probe: impl FnMut(usize) -> (P, Option<Vec<u8>>, bool),
) -> Fit<P> {
    const ACCEPT_FLOOR_PERCENT: usize = 95;
    let floor = budget * ACCEPT_FLOOR_PERCENT / 100;
    let mut probes = 0usize;
    let mut cap = 1024usize;
    let mut last_fit: Option<(usize, P, Vec<u8>)> = None;
    loop {
        probes += 1;
        let (proof, wire, natural) = probe(cap);
        match wire {
            Some(wire) if wire.len() <= budget => {
                if natural || wire.len() >= floor {
                    return Fit {
                        proof,
                        wire,
                        probes,
                        natural,
                    };
                }
                last_fit = Some((cap, proof, wire));
                cap *= 2;
            }
            _ => break,
        }
    }
    let (mut lo, hi_init) = match &last_fit {
        Some((len, _, _)) => (*len, cap),
        None => (0, cap),
    };
    let mut hi = hi_init;
    let mut best = last_fit;
    for _ in 0..12 {
        if hi - lo <= 1 {
            break;
        }
        let mid = lo + (hi - lo) / 2;
        probes += 1;
        let (proof, wire, _) = probe(mid);
        match wire {
            Some(wire) if wire.len() <= budget => {
                lo = mid;
                let full = wire.len() >= floor;
                best = Some((mid, proof, wire));
                if full {
                    break;
                }
            }
            _ => hi = mid,
        }
    }
    let (_, proof, wire) = best.unwrap_or_else(|| {
        // Even one entry overflows: mirror the sizer and return it alone.
        probes += 1;
        let (proof, wire, _) = probe(1);
        (1, proof, wire.expect("single entry serializes"))
    });
    Fit {
        proof,
        wire,
        probes,
        natural: false,
    }
}

fn wire_or_too_large<F: FnOnce(&mut Vec<u8>) -> Result<(), ProofError>>(f: F) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    match f(&mut out) {
        Ok(()) => Some(out),
        Err(ProofError::BodyTooLarge { .. }) => None,
        Err(e) => panic!("serialize: {e}"),
    }
}

// ---------------------------------------------------------------------------
// sweeps

fn xorshift(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}

fn range_starts(n: usize) -> Vec<Option<Vec<u8>>> {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    std::iter::once(None)
        .chain((1..n).map(|_| {
            Some(
                (0..32)
                    .map(|_| (xorshift(&mut seed) & 0xFF) as u8)
                    .collect(),
            )
        }))
        .collect()
}

fn sweep_range(
    merkle: &Merkle<Rev>,
    cfg: &Config,
    arm: &Arm,
    round: usize,
    budget: usize,
    out: &mut impl Write,
) {
    let strategy = arm.strategy;
    for (si, start0) in range_starts(cfg.range_starts).into_iter().enumerate() {
        let mut start = start0;
        let mut hint: Option<SizingHint> = None;
        for ci in 0..cfg.range_chunks {
            let mut rec = Rec {
                kind: "range",
                strategy: arm.name.clone(),
                round,
                budget,
                start: si,
                chunk: ci,
                hint_scaled: hint.map(|h| h.ratio.scaled()),
                ..Rec::default()
            };
            let (proof, wire, natural) = match strategy {
                Strategy::Sized => {
                    let t = Instant::now();
                    let sized = merkle
                        .range_proof_sized(start.as_deref(), budget, hint)
                        .expect("sized range proof");
                    rec.size_ns = t.elapsed().as_nanos();
                    rec.ratio_scaled = sized.hint.ratio.scaled();
                    rec.edges = sized.hint.edges.unwrap_or_default();
                    rec.builds = last_probes("range");
                    hint = Some(sized.hint);
                    (sized.proof, sized.wire, sized.natural_end)
                }
                Strategy::Bisect => {
                    let t = Instant::now();
                    let fit = fit_budget(budget, |max_len| {
                        let proof = merkle
                            .range_proof(start.as_deref(), None, NonZeroUsize::new(max_len))
                            .expect("range proof");
                        let natural = proof.key_values().len() < max_len;
                        let wire = wire_or_too_large(|o| proof.write_to_vec(o));
                        (proof, wire, natural)
                    });
                    rec.size_ns = t.elapsed().as_nanos();
                    rec.builds = fit.probes;
                    (fit.proof, fit.wire, fit.natural)
                }
            };
            rec.items = proof.key_values().len();
            rec.wire = wire.len();
            rec.natural_end = natural;
            measure_range(&proof, &wire, &mut rec);
            rec.write_json(out);
            if natural || rec.items == 0 {
                break;
            }
            start = proof.key_values().last().map(|(k, _)| next_key(k));
        }
    }
}

fn sweep_change(
    target: &Merkle<Rev>,
    source: &Merkle<Rev>,
    expected: &[BatchOp<Key, Value>],
    arm: &Arm,
    round: usize,
    budget: usize,
    out: &mut impl Write,
) -> usize {
    let strategy = arm.strategy;
    let mut start: Option<Vec<u8>> = None;
    let mut hint: Option<SizingHint> = None;
    let mut covered = 0usize;
    let mut chunks = 0usize;
    loop {
        let mut rec = Rec {
            kind: "change",
            strategy: arm.name.clone(),
            round,
            budget,
            start: 0,
            chunk: chunks,
            hint_scaled: hint.map(|h| h.ratio.scaled()),
            ..Rec::default()
        };
        let (proof, wire, natural) = match strategy {
            Strategy::Sized => {
                let t = Instant::now();
                let sized = target
                    .change_proof_sized(source.nodestore(), start.as_deref(), budget, hint)
                    .expect("sized change proof");
                rec.size_ns = t.elapsed().as_nanos();
                rec.ratio_scaled = sized.hint.ratio.scaled();
                rec.edges = sized.hint.edges.unwrap_or_default();
                rec.builds = last_probes("change");
                hint = Some(sized.hint);
                (sized.proof, sized.wire, sized.natural_end)
            }
            Strategy::Bisect => {
                let t = Instant::now();
                let fit = fit_budget(budget, |max_len| {
                    let proof = target
                        .change_proof(
                            start.as_deref(),
                            None,
                            source.nodestore(),
                            NonZeroUsize::new(max_len),
                        )
                        .expect("change proof");
                    let natural = proof.batch_ops().len() < max_len;
                    let wire = wire_or_too_large(|o| proof.write_to_vec(o));
                    (proof, wire, natural)
                });
                rec.size_ns = t.elapsed().as_nanos();
                rec.builds = fit.probes;
                (fit.proof, fit.wire, fit.natural)
            }
        };
        let ops = proof.batch_ops();
        for (i, op) in ops.iter().enumerate() {
            let want = &expected[covered + i];
            assert_eq!(
                op.key(),
                want.key(),
                "chunk {chunks} op {i} key out of sequence"
            );
            assert_eq!(
                op.value(),
                want.value(),
                "chunk {chunks} op {i} value differs"
            );
        }
        covered += ops.len();
        rec.items = ops.len();
        rec.wire = wire.len();
        rec.natural_end = natural;
        measure_change(&proof, &wire, &mut rec);
        rec.write_json(out);
        chunks += 1;
        if covered >= expected.len() {
            break;
        }
        assert!(!natural, "natural end before covering the diff");
        assert!(chunks < 100_000, "runaway paging");
        start = ops.last().map(|op| next_key(op.key()));
    }
    assert_eq!(
        covered,
        expected.len(),
        "sync must cover the whole diff exactly"
    );
    chunks
}

/// The measured arms, one per requested strategy.
fn arms_for(cfg: &Config) -> Vec<Arm> {
    cfg.strategies
        .iter()
        .map(|&strategy| Arm {
            strategy,
            name: match strategy {
                Strategy::Bisect => "bisect".to_owned(),
                Strategy::Sized => "sized".to_owned(),
            },
        })
        .collect()
}

// ---------------------------------------------------------------------------

/// The last value recorded into one histogram.
#[derive(Default)]
struct LastValue(AtomicU64);

impl HistogramFn for LastValue {
    fn record(&self, value: f64) {
        self.0.store(value as u64, Ordering::Relaxed);
    }
}

/// A metrics recorder that keeps only the sizer's expensive
/// `firewood_sized_proof_probes` histogram, per proof kind, and drops every
/// other metric. Recording elsewhere (the node-read counters on the hot
/// path, for one) would otherwise distort the timings this harness measures.
#[derive(Default)]
struct ProbesRecorder {
    range: Arc<LastValue>,
    change: Arc<LastValue>,
}

impl Recorder for ProbesRecorder {
    fn describe_counter(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_gauge(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn describe_histogram(&self, _: KeyName, _: Option<Unit>, _: SharedString) {}
    fn register_counter(&self, _: &MetricKey, _: &Metadata<'_>) -> Counter {
        Counter::noop()
    }
    fn register_gauge(&self, _: &MetricKey, _: &Metadata<'_>) -> Gauge {
        Gauge::noop()
    }
    fn register_histogram(&self, key: &MetricKey, _: &Metadata<'_>) -> Histogram {
        if key.name() != crate::registry::SIZED_PROOF_PROBES {
            return Histogram::noop();
        }
        let kind = key
            .labels()
            .find(|l| l.key() == "kind")
            .map(metrics::Label::value);
        match kind {
            Some("range") => Histogram::from_arc(Arc::clone(&self.range)),
            Some("change") => Histogram::from_arc(Arc::clone(&self.change)),
            _ => Histogram::noop(),
        }
    }
}

static PROBES: OnceLock<ProbesRecorder> = OnceLock::new();

/// Probes the most recent sized `kind` chunk took, from the recorded metric.
fn last_probes(kind: &str) -> usize {
    let recorder = PROBES.get().expect("metrics recorder installed");
    let last = match kind {
        "range" => &recorder.range,
        _ => &recorder.change,
    };
    last.0.load(Ordering::Relaxed) as usize
}

#[test]
#[ignore = "needs a C-Chain snapshot copy and replay log; see module docs"]
fn replay_sized_proof_sweep() {
    let recorder = PROBES.get_or_init(ProbesRecorder::default);
    metrics::set_global_recorder(ProbesRecorder {
        range: Arc::clone(&recorder.range),
        change: Arc::clone(&recorder.change),
    })
    .expect("install metrics recorder");
    let _expensive_metrics = set_metrics_context(Some(MetricsContext::new(true)));
    let Some(cfg) = Config::from_env() else {
        eprintln!("SIZED_DB_DIR / SIZED_REPLAY_LOG not set; nothing to do");
        return;
    };
    assert!(cfg.local_commit < cfg.target_commit);

    let manager = RevisionManagerConfig::builder()
        .max_revisions((cfg.target_commit - cfg.local_commit) as usize + 1024)
        .build();
    let db_cfg = DbConfig::builder()
        .node_hash_algorithm(DefaultHashMode::ALGORITHM)
        .create_if_missing(false)
        .truncate(false)
        .manager(manager)
        .build();
    let db = Db::<DefaultHashMode>::new_with_hash_mode(&cfg.db_dir, db_cfg).expect("open db");
    eprintln!(
        "opened {} (hash mode {:?}); replaying {} commits from {}",
        cfg.db_dir.display(),
        DefaultHashMode::ALGORITHM,
        cfg.target_commit,
        cfg.log.display()
    );

    let fx = replay(&db, &cfg);
    eprintln!(
        "fixture ready: {} commits in {:.1?}; local {} target {}",
        fx.commits,
        fx.replay,
        hex::encode(&fx.local_root),
        hex::encode(&fx.target_root)
    );

    let target: Merkle<Rev> =
        Merkle::from(api::Db::revision(&db, fx.target_root.clone()).expect("target rev"));
    let source: Merkle<Rev> =
        Merkle::from(api::Db::revision(&db, fx.local_root.clone()).expect("local rev"));

    let t = Instant::now();
    let full = target
        .change_proof(None, None, source.nodestore(), None)
        .expect("full diff");
    let expected = full.batch_ops();
    eprintln!(
        "full diff: {} ops over {} commits ({:.1?})",
        expected.len(),
        cfg.target_commit - cfg.local_commit,
        t.elapsed()
    );

    let mut out = std::io::BufWriter::new(File::create(&cfg.out).expect("create output"));
    writeln!(
        out,
        r#"{{"fixture":{{"local_commit":{},"target_commit":{},"local_root":"{}","target_root":"{}","replay_secs":{:.1},"diff_ops":{},"budgets":{:?},"range_starts":{},"range_chunks":{}}}}}"#,
        cfg.local_commit,
        cfg.target_commit,
        hex::encode(&fx.local_root),
        hex::encode(&fx.target_root),
        fx.replay.as_secs_f64(),
        expected.len(),
        cfg.budgets,
        cfg.range_starts,
        cfg.range_chunks
    )
    .expect("write fixture");

    let arms = arms_for(&cfg);
    for round in 1..=cfg.rounds {
        for &budget in &cfg.budgets {
            for arm in &arms {
                let t = Instant::now();
                sweep_range(&target, &cfg, arm, round, budget, &mut out);
                eprintln!(
                    "r{round} range  budget={budget:>8} {:<10} done in {:.1?}",
                    arm.name,
                    t.elapsed()
                );
                let t = Instant::now();
                let chunks = sweep_change(&target, &source, expected, arm, round, budget, &mut out);
                eprintln!(
                    "r{round} change budget={budget:>8} {:<10} {chunks} messages in {:.1?}",
                    arm.name,
                    t.elapsed()
                );
                out.flush().expect("flush");
            }
        }
    }
    drop(out);
    db.close().expect("close db");
    eprintln!("wrote {}", cfg.out.display());
}
