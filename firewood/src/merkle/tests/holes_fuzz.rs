// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Ground-truth differential fuzz for post-merge hole detection.
//!
//! Each run builds a random target map `M` and derives a local map `L` from
//! it by dropping keys, altering values, and inserting surplus keys. It
//! takes a range proof from `M` over a random range and verifies it, then
//! overwrites `L`'s content inside the proof's applied range with `M`'s —
//! what merging the proof does — before committing the result to a database
//! and walking the committed revision. The labels are checked against the
//! two maps directly by the oracle in `tests/holes.rs`, which also asserts
//! the global partition of the complement of the applied range. The labels
//! are then applied through the public write API — deletion geometry from
//! the labels' own payloads, replacement content from `M` — and the walk over
//! the committed result must find only agreement.
//!
//! Each run then repeats the exercise through a change proof taken from `M`
//! with `L` as the source — the proof a syncing client would hold — and,
//! when the two proofs apply over the same range and anchor at the same
//! boundary keys, asserts the two walks agree label for label.
//!
//! Under the Ethereum mode the generator also plants well-formed accounts
//! with storage children, so the masked account comparison and the
//! lone-storage-child fold normalization get random coverage.
//!
//! Set `FIREWOOD_TEST_SEED` to replay one run and `FIREWOOD_TEST_SOAK_SECONDS`
//! to run fresh seeds until a deadline. The run logs the node reads each walk
//! made against the local revision; that number is recorded, not asserted.

use std::cell::Cell;
use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::{Duration, Instant};

use firewood_macros::hash_mode;
use firewood_storage::{
    Committed, EthHash, FileBacked, FileIoError, HashMode, HashedNodeReader, LinearAddress,
    MaybePersistedNode, MerkleDbHash, NodeHashAlgorithm, NodeReader, NodeStore, RootReader,
    SeededRng, SharedNode, TrieHash,
};

use super::accounts::{
    account_storage_key, empty_code_hash, rlp_encode_account, rlp_encode_storage,
};
use super::holes::{
    Applied, Map, applied_range, assert_matches_truth, covers, in_applied, new_db, remedy_batch,
    root, trie, values_equal, view_map,
};
use crate::api::{self, BatchOp, Db as _, HashKey, Proposal as _};
use crate::db::Db;
use crate::merkle::holes::{find_holes_after_change_proof, find_holes_after_range_proof};
use crate::merkle::{RightBoundary, right_edge};
use crate::{Hole, VerifiedChangeProof, VerifiedRangeProof};

/// A view that counts the node reads made through it.
struct Counting<'a, T> {
    inner: &'a T,
    reads: Cell<usize>,
}

impl<T: NodeReader> NodeReader for Counting<'_, T> {
    fn read_node(&self, addr: LinearAddress) -> Result<SharedNode, FileIoError> {
        self.reads.set(self.reads.get().saturating_add(1));
        self.inner.read_node(addr)
    }

    fn must_recompute_storage_hash(&self) -> bool {
        self.inner.must_recompute_storage_hash()
    }

    fn node_hash_algorithm(&self) -> NodeHashAlgorithm {
        self.inner.node_hash_algorithm()
    }
}

impl<T: RootReader> RootReader for Counting<'_, T> {
    fn root_node(&self) -> Option<SharedNode> {
        self.inner.root_node()
    }

    fn root_as_maybe_persisted_node(&self) -> Option<MaybePersistedNode> {
        self.inner.root_as_maybe_persisted_node()
    }
}

impl<T: HashedNodeReader> HashedNodeReader for Counting<'_, T> {
    fn root_address(&self) -> Option<LinearAddress> {
        self.inner.root_address()
    }

    fn root_hash(&self) -> Option<TrieHash> {
        self.inner.root_hash()
    }
}

/// An account the generator planted: its key and the fields it can mutate.
#[derive(Clone)]
struct Account {
    key: [u8; 32],
    nonce: u64,
    balance: u64,
}

impl Account {
    fn value(&self, rng: &SeededRng) -> Vec<u8> {
        // The storageRoot placeholder is random on purpose: hashing derives
        // the real field, and the walk and the oracle both ignore it.
        rlp_encode_account(self.nonce, self.balance, &rng.random(), &empty_code_hash()).into_vec()
    }
}

/// The target state and what the generator knows about its shape.
struct Target {
    map: Map,
    accounts: Vec<Account>,
}

/// A random byte key. Lengths favour short keys, so tries branch early and
/// stubs sit at many depths, and never equal 32 bytes: under the Ethereum
/// mode that length is an account, and accounts are planted separately with
/// well-formed values.
fn random_key(rng: &SeededRng) -> Vec<u8> {
    let len = match rng.random_range(0..10_u32) {
        0..=5 => rng.random_range(1..=3_usize),
        6..=7 => rng.random_range(4..=8),
        8 => 20,
        _ => 40,
    };
    (0..len).map(|_| rng.random()).collect()
}

/// A random value. Lengths run past 32 bytes so the MerkleDB digest path,
/// which yields `PointStale` rather than `PointFix`, is reached.
fn random_value(rng: &SeededRng) -> Vec<u8> {
    let len = rng.random_range(1..=48_usize);
    (0..len).map(|_| rng.random()).collect()
}

/// The storage-child count of a planted account, weighted toward one and two
/// so the fold convention straddles between target and local often.
fn storage_child_count(rng: &SeededRng) -> usize {
    match rng.random_range(0..20_u32) {
        0..=3 => 0,
        4..=10 => 1,
        11..=16 => 2,
        _ => rng.random_range(3..=5),
    }
}

fn plant_account(rng: &SeededRng, map: &mut Map, account: &Account, slots: usize) {
    map.insert(account.key.to_vec(), account.value(rng));
    let mut nibbles: BTreeSet<u8> = BTreeSet::new();
    while nibbles.len() < slots {
        nibbles.insert(rng.random_range(0..16_u8));
    }
    for nibble in nibbles {
        map.insert(
            account_storage_key(&account.key, nibble << 4).into_vec(),
            rlp_encode_storage(&rng.random()),
        );
    }
}

fn generate_target<H: HashMode>(rng: &SeededRng) -> Target {
    let mut map = Map::new();
    let count = rng.random_range(5..=60_usize);
    while map.len() < count {
        map.insert(random_key(rng), random_value(rng));
    }
    let mut accounts = Vec::new();
    if H::ALGORITHM.is_ethereum() {
        for _ in 0..rng.random_range(0..=3_u32) {
            let account = Account {
                key: rng.random(),
                nonce: rng.random_range(0..1_000_u64),
                balance: rng.random_range(0..1_000_000_u64),
            };
            plant_account(rng, &mut map, &account, storage_child_count(rng));
            accounts.push(account);
        }
    }
    Target { map, accounts }
}

/// Derive the local state: drop keys, alter values, add surplus keys, and
/// under the Ethereum mode edit account fields and storage slots so both the
/// masked comparison and the fold are exercised.
fn derive_local(rng: &SeededRng, target: &Target) -> Map {
    if rng.random_range(0..10_u32) == 0 {
        // Fully synced: the strongest false-hole detector.
        return target.map.clone();
    }
    let mut local = target.map.clone();
    local.retain(|_, _| rng.random_range(0..4_u32) != 0);
    for value in local.values_mut() {
        if rng.random_range(0..5_u32) == 0 {
            *value = random_value(rng);
        }
    }
    for _ in 0..rng.random_range(0..=8_u32) {
        let key = random_key(rng);
        if !target.map.contains_key(&key) {
            local.insert(key, random_value(rng));
        }
    }
    for account in &target.accounts {
        match rng.random_range(0..4_u32) {
            // Same fields, different storageRoot placeholder: invisible.
            0 => {
                local.insert(account.key.to_vec(), account.value(rng));
            }
            // A field differs: visible through the mask, as a point fix when
            // a boundary probes the account key and otherwise inside a span.
            1 => {
                let edited = Account {
                    nonce: account.nonce.wrapping_add(1),
                    ..account.clone()
                };
                local.insert(account.key.to_vec(), edited.value(rng));
            }
            // A storage-slot write: surplus unless the nibble collides with
            // a planted slot, in which case a value edit. Either way the
            // slot count may cross the fold.
            2 => {
                local.insert(
                    account_storage_key(&account.key, rng.random_range(0..16_u8) << 4).into_vec(),
                    rlp_encode_storage(&rng.random()),
                );
            }
            _ => {}
        }
    }
    local
}

/// A random bound: absent, an existing target key, or a random byte string.
fn random_bound(rng: &SeededRng, keys: &[Vec<u8>]) -> Option<Vec<u8>> {
    match rng.random_range(0..5_u32) {
        0 => None,
        1..=2 => Some(keys[rng.random_range(0..keys.len())].clone()),
        _ => Some(random_key(rng)),
    }
}

struct Stats {
    walks: usize,
    reads_max: usize,
    reads_total: usize,
    time_max: Duration,
    time_total: Duration,
    labels_total: usize,
    /// Runs where the two walks were required to agree label for label.
    equivalence_checks: usize,
}

/// Replace the database's content with `m` in one batch: a `DeleteRange`
/// over the empty prefix clears every key, then the puts follow in order.
fn reset_to<H: HashMode>(db: &Db<H>, m: &Map) -> HashKey {
    let batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> =
        std::iter::once(BatchOp::DeleteRange { prefix: Vec::new() })
            .chain(m.iter().map(|(k, v)| BatchOp::Put {
                key: k.clone(),
                value: v.clone(),
            }))
            .collect();
    db.propose(batch).unwrap().commit().unwrap();
    db.root_hash().unwrap()
}

/// A random range request: the bounds the proof is verified for, the start
/// bound the proof is generated with, and the limit.
struct Request {
    start: Option<Vec<u8>>,
    end: Option<Vec<u8>>,
    /// Differs from `start` in the empty-start shape: a proof generated for
    /// an unbounded start, verified for a requested start key at or below the
    /// first target key (and not above the end key).
    generated_start: Option<Vec<u8>>,
    limit: Option<NonZeroUsize>,
}

fn random_request(rng: &SeededRng, keys: &[Vec<u8>]) -> Request {
    let mut start = random_bound(rng, keys);
    let mut end = random_bound(rng, keys);
    if let (Some(s), Some(e)) = (&start, &end)
        && s > e
    {
        std::mem::swap(&mut start, &mut end);
    }
    let limit = if rng.random_range(0..2_u32) == 0 {
        None
    } else {
        NonZeroUsize::new(rng.random_range(1..=10_usize))
    };
    let below_first = keys[0][..rng.random_range(0..=keys[0].len())].to_vec();
    let empty_start = start.is_some()
        && rng.random_range(0..6_u32) == 0
        && end.as_ref().is_none_or(|e| below_first <= *e);
    let generated_start = if empty_start {
        start = Some(below_first);
        None
    } else {
        start.clone()
    };
    Request {
        start,
        end,
        generated_start,
        limit,
    }
}

/// Every `Synced` span really holds equal content, by the oracle's own
/// comparison; a false match would be a hash collision.
fn assert_synced_spans_equal<H: HashMode>(
    target: &Map,
    local: &Map,
    holes: &[Hole],
    locator: &str,
) {
    for hole in holes {
        let Hole::Synced(span) = hole else {
            continue;
        };
        let (lower, upper) = span.as_key_range();
        let under = |m: &Map| -> Vec<(Vec<u8>, Vec<u8>)> {
            m.range::<[u8], _>((
                std::ops::Bound::Included(&*lower),
                upper
                    .as_deref()
                    .map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded),
            ))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
        };
        let (t_under, l_under) = (under(target), under(local));
        assert!(
            t_under.len() == l_under.len()
                && t_under
                    .iter()
                    .zip(&l_under)
                    .all(|((tk, tv), (lk, lv))| tk == lk && values_equal::<H>(tk, tv, lv)),
            "false match under {:?} ({locator})",
            span.prefix()
        );
    }
}

/// The local state after a proof over `range` was applied: the target's
/// content inside the range, the local content outside it. That is what
/// merging a range proof does, and the state a client hands the walk.
fn merged(target: &Map, local: &Map, range: &Applied) -> Map {
    let mut out: Map = local
        .iter()
        .filter(|(k, _)| !in_applied(k, range))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    out.extend(
        target
            .iter()
            .filter(|(k, _)| in_applied(k, range))
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    out
}

type LocalRevision<H> = NodeStore<Committed, FileBacked, H>;

/// What one leg walks.
struct LegInput<'a> {
    /// The post-merge local state.
    local: &'a Map,
    /// The oracle's scope: the range the proof applied over.
    applied: &'a Applied,
    /// Local keys the walk must say nothing about and the remedies must
    /// leave alone: those below a start key whose start proof is empty.
    unlabelled: &'a [Vec<u8>],
}

/// One leg of a run: commit the local state, walk it with `walk`, check the
/// labels against the maps, apply them through the write API, and assert the
/// walk over the committed result finds only agreement. Returns the first
/// walk's labels.
fn leg<H: HashMode>(
    db: &Db<H>,
    target: &Map,
    input: LegInput<'_>,
    locator: &str,
    stats: &mut Stats,
    walk: impl Fn(&Counting<'_, LocalRevision<H>>) -> Result<Vec<Hole>, api::Error>,
) -> Vec<Hole> {
    let LegInput {
        local,
        applied,
        unlabelled,
    } = input;
    let local_root = reset_to(db, local);
    let revision = db.revision(local_root).unwrap();
    let counting = Counting {
        inner: &*revision,
        reads: Cell::new(0),
    };
    let began = Instant::now();
    let holes = walk(&counting).unwrap_or_else(|e| panic!("walk failed ({locator}): {e}"));
    let elapsed = began.elapsed();
    stats.walks = stats.walks.saturating_add(1);
    stats.reads_max = stats.reads_max.max(counting.reads.get());
    stats.reads_total = stats.reads_total.saturating_add(counting.reads.get());
    stats.time_max = stats.time_max.max(elapsed);
    stats.time_total = stats.time_total.saturating_add(elapsed);
    stats.labels_total = stats.labels_total.saturating_add(holes.len());

    assert_matches_truth::<H>(target, local, applied, &holes);
    for key in unlabelled {
        assert!(
            holes.iter().all(|h| !covers(h, key)),
            "{key:02x?} below an unauthenticated start is labelled by {holes:?} ({locator})"
        );
    }
    if local == target {
        assert!(
            holes.iter().all(|h| matches!(h, Hole::Synced(_))),
            "fully synced local emitted {holes:?} ({locator})"
        );
    }
    assert_synced_spans_equal::<H>(target, local, &holes, locator);

    // Convergence through the write API.
    db.propose(remedy_batch(target, &holes))
        .unwrap()
        .commit()
        .unwrap();
    let repaired = db.revision(db.root_hash().unwrap()).unwrap();
    let again = walk(&Counting {
        inner: &*repaired,
        reads: Cell::new(0),
    })
    .unwrap();
    assert!(
        again.iter().all(|h| matches!(h, Hole::Synced(_))),
        "after remedies: {again:?} ({locator})"
    );
    // The repaired state is checked against the maps too, so a residual
    // difference the second walk failed to label still surfaces.
    let repaired_map = view_map(&*repaired);
    assert_matches_truth::<H>(target, &repaired_map, applied, &again);
    for key in unlabelled {
        assert_eq!(
            repaired_map.get(key),
            local.get(key),
            "{key:02x?} below an unauthenticated start was touched by the remedies ({locator})"
        );
    }
    holes
}

fn run_one<H: HashMode>(run: usize, seed: u64, db: &Db<H>, stats: &mut Stats) {
    eprintln!(
        "run {run} ({:?}): seed={seed} (export FIREWOOD_TEST_SEED={seed} to reproduce)",
        H::ALGORITHM
    );
    let rng = SeededRng::new(seed);
    let target = generate_target::<H>(&rng);
    let local = derive_local(&rng, &target);
    let keys: Vec<Vec<u8>> = target.map.keys().cloned().collect();
    let Request {
        start,
        end,
        generated_start,
        limit,
    } = random_request(&rng, &keys);
    let empty_start = generated_start.is_none() && start.is_some();
    let locator = format!(
        "{:?}, seed={seed}, run={run}, start={start:02x?}, end={end:02x?}, limit={limit:?}",
        H::ALGORITHM
    );

    // Range-proof leg.
    let t = trie::<H>(&target.map);
    let proof = t
        .range_proof(generated_start.as_deref(), end.as_deref(), limit)
        .unwrap();
    let verified = VerifiedRangeProof::verify(
        Arc::new(proof),
        root(&t),
        start.as_deref(),
        end.as_deref(),
        H::ALGORITHM,
        limit,
    )
    .unwrap_or_else(|e| panic!("verify failed ({locator}): {e}"));
    // The merge writes `[start_key, right_edge_key]` from the verification
    // context. The oracle's scope is `applied_range`, which drops the lower
    // bound when the start proof is empty: the walk labels nothing below the
    // start key then, and the local keys there must survive untouched.
    let ctx = verified.verification();
    let merge_range: Applied = (
        ctx.start_key().map(<[u8]>::to_vec),
        ctx.right_edge_key().map(<[u8]>::to_vec),
    );
    let applied = applied_range(&verified);
    let local_range = merged(&target.map, &local, &merge_range);
    let unlabelled_below: Vec<Vec<u8>> = match (empty_start, &merge_range.0) {
        (true, Some(s)) => local_range.keys().filter(|k| *k < s).cloned().collect(),
        _ => Vec::new(),
    };
    let range_holes = leg(
        db,
        &target.map,
        LegInput {
            local: &local_range,
            applied: &applied,
            unlabelled: &unlabelled_below,
        },
        &locator,
        stats,
        |view| find_holes_after_range_proof::<H, _>(&verified, view),
    );

    // Change-proof leg: the proof a syncing client would hold, from the
    // target with the client's own state as the source.
    let source = trie::<H>(&local);
    let change = t
        .change_proof(start.as_deref(), end.as_deref(), source.nodestore(), limit)
        .unwrap();
    let verified_change = VerifiedChangeProof::verify(
        Arc::new(change),
        root(&t),
        start.as_deref(),
        end.as_deref(),
        H::ALGORITHM,
        limit,
    )
    .unwrap_or_else(|e| panic!("change verify failed ({locator}): {e}"));
    let applied_change: Applied = (
        start.clone(),
        verified_change
            .verification()
            .right_edge_key()
            .map(<[u8]>::to_vec),
    );
    // Applying a change proof writes only its operations, which here are the
    // local-to-target diff over the applied range, so `merged` over that
    // range is the state the proposal produces.
    let change_holes = leg(
        db,
        &target.map,
        LegInput {
            local: &merged(&target.map, &local, &applied_change),
            applied: &applied_change,
            unlabelled: &[],
        },
        &locator,
        stats,
        |view| find_holes_after_change_proof::<H, _>(&verified_change, view),
    );

    // When both proofs apply over the same range and walk the same boundary
    // keys, the two walks must agree label for label. Two things break that
    // and are excluded. The applied ranges differ whenever a limit truncates
    // either proof, and also without one, since a change proof's right edge
    // narrows to its last operation's key whenever the request has no end
    // key, or when the end proof resolves that key consistently with the
    // last operation (`compute_right_edge_key`); the empty-start shape also
    // lands here, since the range side's applied range then has no lower
    // bound (it labels nothing below the start key) while the change side
    // walks a real start proof. The range side takes its out-of-range arm
    // when the end proof's terminal is a real key past the edge: it
    // decomposes the open interval up to that key and walks from there,
    // where the change side walks from the edge itself — the same key space,
    // partitioned differently.
    let range_in_range = matches!(
        right_edge(
            verified.proof().end_proof().as_ref(),
            verified.proof().key_values().last().map(|(k, _)| &**k),
            end.as_deref(),
        ),
        RightBoundary::InRange(_)
    );
    if applied == applied_change && range_in_range {
        stats.equivalence_checks = stats.equivalence_checks.saturating_add(1);
        assert_eq!(change_holes, range_holes, "{locator}");
    }
}

#[hash_mode]
#[test]
fn test_slow_holes_differential_fuzz<H: HashMode>() {
    let mut stats = Stats {
        walks: 0,
        reads_max: 0,
        reads_total: 0,
        time_max: Duration::ZERO,
        time_total: Duration::ZERO,
        labels_total: 0,
        equivalence_checks: 0,
    };
    // One database for the whole run, reset per iteration: every `Db` owns a
    // thread pool, and a long soak that opened one per iteration ran the
    // process out of threads.
    let (db, _dir) = new_db::<H>();

    if let Ok(s) = std::env::var("FIREWOOD_TEST_SEED") {
        // Replay one seed. Takes precedence over the soak knob.
        run_one::<H>(
            0,
            s.parse().expect("FIREWOOD_TEST_SEED must be a u64"),
            &db,
            &mut stats,
        );
    } else if let Ok(s) = std::env::var("FIREWOOD_TEST_SOAK_SECONDS") {
        let secs: u64 = s.parse().expect("FIREWOOD_TEST_SOAK_SECONDS must be a u64");
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(secs))
            .expect("soak deadline fits in an Instant");
        let soak_rng = SeededRng::from_random();
        for run in 0.. {
            if Instant::now() >= deadline {
                break;
            }
            run_one::<H>(run, soak_rng.next_u64(), &db, &mut stats);
        }
    } else {
        // Debug assertions slow each run considerably; keep the default
        // count modest there.
        let iterations = if cfg!(debug_assertions) { 30 } else { 150 };
        let outer_rng = SeededRng::from_random();
        for run in 0..iterations {
            run_one::<H>(run, outer_rng.next_u64(), &db, &mut stats);
        }
    }

    // A single replayed seed may legitimately never compare the two walks;
    // every other mode runs enough shapes that zero checks means the
    // equivalence condition has drifted shut.
    if std::env::var_os("FIREWOOD_TEST_SEED").is_none() {
        assert!(
            stats.equivalence_checks > 0,
            "no run compared the range and change walks"
        );
    }

    let walks = u32::try_from(stats.walks.max(1)).unwrap_or(u32::MAX);
    eprintln!(
        "holes fuzz ({:?}): {} walks, {} labels, {} range/change equivalence checks; \
         node reads per walk max {} mean {}; walk time max {:?} mean {:?}",
        H::ALGORITHM,
        stats.walks,
        stats.labels_total,
        stats.equivalence_checks,
        stats.reads_max,
        stats.reads_total.checked_div(walks as usize).unwrap_or(0),
        stats.time_max,
        stats.time_total.checked_div(walks).unwrap_or_default(),
    );
}
