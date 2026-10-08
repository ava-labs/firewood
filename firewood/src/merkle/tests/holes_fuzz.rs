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
    EthHash, FileIoError, HashMode, HashedNodeReader, LinearAddress, MaybePersistedNode,
    MerkleDbHash, NodeHashAlgorithm, NodeReader, RootReader, SeededRng, SharedNode, TrieHash,
};

use super::accounts::{
    account_storage_key, empty_code_hash, rlp_encode_account, rlp_encode_storage,
};
use super::holes::{
    Map, applied_range, assert_matches_truth, covers, in_applied, new_db, remedy_batch, root, trie,
    values_equal, view_map,
};
use crate::api::{BatchOp, Db as _, HashKey, Proposal as _};
use crate::db::Db;
use crate::merkle::holes::find_holes_after_range_proof;
use crate::{Hole, VerifiedRangeProof};

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

/// The post-merge local state: the proof's range has already been applied,
/// so the target's content is written over `[start_key, right_edge_key]` and
/// the local keys there are dropped, which is what merging the proof does.
/// The walk then sees the state a client would hand it. Also returns the
/// local keys below a start key whose start proof is empty: the walk labels
/// nothing there (the oracle's `applied_range` reports no lower bound for
/// such a proof), so those keys must come through unlabelled and untouched.
fn merged(target: &Map, local: Map, verified: &VerifiedRangeProof) -> (Map, Vec<Vec<u8>>) {
    let ctx = verified.verification();
    let range = (
        ctx.start_key().map(<[u8]>::to_vec),
        ctx.right_edge_key().map(<[u8]>::to_vec),
    );
    let mut local: Map = local
        .into_iter()
        .filter(|(k, _)| !in_applied(k, &range))
        .collect();
    local.extend(
        target
            .iter()
            .filter(|(k, _)| in_applied(k, &range))
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    let unlabelled_below = match (verified.proof().start_proof().is_empty(), &range.0) {
        (true, Some(s)) => local.keys().filter(|k| *k < s).cloned().collect(),
        _ => Vec::new(),
    };
    (local, unlabelled_below)
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
    let locator = format!(
        "{:?}, seed={seed}, run={run}, start={start:02x?}, end={end:02x?}, limit={limit:?}",
        H::ALGORITHM
    );

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

    let (local, unlabelled_below) = merged(&target.map, local, &verified);
    let applied = applied_range(&verified);

    let local_root = reset_to(db, &local);
    let revision = db.revision(local_root).unwrap();
    let counting = Counting {
        inner: &*revision,
        reads: Cell::new(0),
    };
    let began = Instant::now();
    let holes = find_holes_after_range_proof::<H, _>(&verified, &counting)
        .unwrap_or_else(|e| panic!("walk failed ({locator}): {e}"));
    let elapsed = began.elapsed();
    stats.walks = stats.walks.saturating_add(1);
    stats.reads_max = stats.reads_max.max(counting.reads.get());
    stats.reads_total = stats.reads_total.saturating_add(counting.reads.get());
    stats.time_max = stats.time_max.max(elapsed);
    stats.time_total = stats.time_total.saturating_add(elapsed);
    stats.labels_total = stats.labels_total.saturating_add(holes.len());

    assert_matches_truth::<H>(&target.map, &local, &applied, &holes);
    for key in &unlabelled_below {
        assert!(
            holes.iter().all(|h| !covers(h, key)),
            "{key:02x?} below an unauthenticated start is labelled by {holes:?} ({locator})"
        );
    }
    if local == target.map {
        assert!(
            holes.iter().all(|h| matches!(h, Hole::Synced(_))),
            "fully synced local emitted {holes:?} ({locator})"
        );
    }
    assert_synced_spans_equal::<H>(&target.map, &local, &holes, &locator);

    // Convergence through the write API.
    db.propose(remedy_batch(&target.map, &holes))
        .unwrap()
        .commit()
        .unwrap();
    let repaired = db.revision(db.root_hash().unwrap()).unwrap();
    let again = find_holes_after_range_proof::<H, _>(&verified, &*repaired).unwrap();
    assert!(
        again.iter().all(|h| matches!(h, Hole::Synced(_))),
        "after remedies: {again:?} ({locator})"
    );
    let repaired_map = view_map(&*repaired);
    assert_matches_truth::<H>(&target.map, &repaired_map, &applied, &again);
    for key in &unlabelled_below {
        assert_eq!(
            repaired_map.get(key),
            local.get(key),
            "{key:02x?} below an unauthenticated start was touched by the remedies ({locator})"
        );
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

    let walks = u32::try_from(stats.walks.max(1)).unwrap_or(u32::MAX);
    eprintln!(
        "holes fuzz ({:?}): {} walks, {} labels; node reads per walk max {} mean {}; \
         walk time max {:?} mean {:?}",
        H::ALGORITHM,
        stats.walks,
        stats.labels_total,
        stats.reads_max,
        stats.reads_total.checked_div(walks as usize).unwrap_or(0),
        stats.time_max,
        stats.time_total.checked_div(walks).unwrap_or_default(),
    );
}
