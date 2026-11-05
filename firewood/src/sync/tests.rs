// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! End-to-end state-sync tests (see `docs/src/designs/2026-06-11-state-sync.md`).
//!
//! These complement the pure-layer tests in `super::state` and the
//! single-submit seam tests in `super::syncer`: everything here drives
//! multi-step flows against real databases — a hostile-proof corpus with a
//! retry-to-success epilogue ([`hostile`]) and shedding with real truncated
//! proofs.

#![expect(
    clippy::arithmetic_side_effects,
    reason = "tests use plain arithmetic on small constants"
)]

use std::collections::VecDeque;
use std::num::NonZeroUsize;

use firewood_macros::hash_mode;
use firewood_storage::EthHash;
use firewood_storage::HashMode;
use firewood_storage::MerkleDbHash;
use firewood_storage::SeededRng;

use super::GetWork;
use super::Submit;
use super::Syncer;
use super::WorkItem;
use super::start_sync;
use crate::api::Db as _;
use crate::api::DbView as _;
use crate::api::FrozenRangeProof;
use crate::api::HashKey;
use crate::db::BatchOp;
use crate::db::Db;
use crate::db::test::TestDb;

/// 32-byte big-endian-indexed key: 24 zero bytes then `i`. Sized like an
/// Ethereum account key so the checker accepts it in both hash modes
/// (`db/tests/concurrent_rebase.rs` precedent).
fn key32(i: u64) -> Vec<u8> {
    let mut k = vec![0u8; 24];
    k.extend_from_slice(&i.to_be_bytes());
    k
}

fn val(i: u64) -> Vec<u8> {
    format!("v{i}").into_bytes()
}

fn limit(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("test limits are nonzero")
}

/// A source database committed with known content; `target` is its root.
struct Source<H: HashMode> {
    db: TestDb<H>,
    target: HashKey,
}

fn source_with<H: HashMode>(kvs: Vec<(Vec<u8>, Vec<u8>)>) -> Source<H> {
    let db = TestDb::<H>::new();
    let batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> = kvs
        .into_iter()
        .map(|(key, value)| BatchOp::Put { key, value })
        .collect();
    db.update(batch).expect("populate the source");
    let target = db.root_hash().expect("source database is non-empty");
    Source { db, target }
}

/// Source of `n` deterministic [`key32`]/[`val`] pairs.
fn source_n<H: HashMode>(n: u64) -> Source<H> {
    source_with::<H>((0..n).map(|i| (key32(i), val(i))).collect())
}

/// Honest fetcher: generate the proof a peer would serve for `item`,
/// optionally truncated to `proof_limit` pairs (the generation-side
/// truncation knob).
fn fetch<H: HashMode>(
    src: &Source<H>,
    item: &WorkItem,
    proof_limit: Option<NonZeroUsize>,
) -> FrozenRangeProof {
    let view = src
        .db
        .revision(src.target.clone())
        .expect("target revision exists");
    view.range_proof(
        item.first_key.as_deref(),
        item.last_key.as_deref(),
        proof_limit,
    )
    .expect("source can prove any requested range")
}

fn to_bytes(proof: &FrozenRangeProof) -> Vec<u8> {
    let mut out = Vec::new();
    proof.write_to_vec(&mut out).expect("serialize the proof");
    out
}

/// Outcome counters from a serial drive.
struct DriveStats {
    /// Total successful submits, including the kv-less absence proofs that
    /// retire empty seed slices and shed chunks.
    submits: usize,
    /// Submits whose proof carried at least one key/value pair. Progress
    /// bounds are asserted on THIS counter: a re-fetch regression re-serves
    /// keys (so kv-bearing submits exceed the `ceil(n / limit)` + tails
    /// floor), while absence proofs legitimately add kv-less submits.
    kv_submits: usize,
}

/// Serial driver: greedily hold every outstanding work item and sweep them
/// to `Done` with honest proofs.
///
/// Panics on `Wait` with nothing held (a stall: this driver holds every
/// outstanding item), on `InvalidProof` (the fetcher is honest), and past
/// `max_submits` total submits (progress bound — converts a stall into a
/// failure). Stops early after exactly `stop_after` successful submits when
/// given, dropping any held items (abandoned in-flight reservations).
fn drive_with<H: HashMode>(
    syncer: &Syncer<'_>,
    src: &Source<H>,
    proof_limit: Option<NonZeroUsize>,
    max_submits: usize,
    stop_after: Option<usize>,
    mut held: VecDeque<WorkItem>,
) -> DriveStats {
    let mut stats = DriveStats {
        submits: 0,
        kv_submits: 0,
    };
    let mut steps = 0usize;
    loop {
        steps += 1;
        assert!(steps < 100_000, "sync driver failed to converge");
        if stop_after == Some(stats.submits) {
            return stats;
        }
        match syncer.get_work().expect("no invariant violations") {
            GetWork::Work(item) => {
                held.push_back(item);
                continue; // greedy: hold every outstanding item
            }
            GetWork::Done => return stats,
            GetWork::Wait => {}
        }
        // Wait while this driver holds every outstanding item means there
        // is genuinely nothing else to hand out; sweep a held item.
        let item = held
            .pop_front()
            .expect("Wait with nothing held: the sync stalled");
        assert!(
            stats.submits < max_submits,
            "progress bound exceeded: more than {max_submits} submits"
        );
        let proof = fetch(src, &item, proof_limit);
        stats.submits += 1;
        if !proof.key_values().is_empty() {
            stats.kv_submits += 1;
        }
        match syncer.submit(item.id, &proof).expect("honest submit") {
            Submit::Continue(next) => held.push_back(next),
            Submit::Exhausted => {}
            Submit::InvalidProof(e) => panic!("honest source proof rejected: {e}"),
        }
    }
}

/// [`drive_with`] from scratch, running to `Done`.
fn drive_serial<H: HashMode>(
    syncer: &Syncer<'_>,
    src: &Source<H>,
    proof_limit: Option<NonZeroUsize>,
    max_submits: usize,
) -> DriveStats {
    drive_with(syncer, src, proof_limit, max_submits, None, VecDeque::new())
}

/// Root equality plus full key/value iteration equality (the iteration
/// gives better failure diagnostics than the root alone — and proves stale
/// destination keys were deleted, not merely out-hashed).
fn assert_synced<H: HashMode>(dst: &Db<H>, src: &Source<H>) {
    let dst_root = dst.root_hash().expect("synced destination is non-empty");
    let src_view = src
        .db
        .revision(src.target.clone())
        .expect("target revision exists");
    let dst_view = dst
        .revision(dst_root.clone())
        .expect("destination revision exists");
    let mut src_iter = src_view
        .iter_option(None::<&[u8]>)
        .expect("iterate the source");
    let mut dst_iter = dst_view
        .iter_option(None::<&[u8]>)
        .expect("iterate the destination");
    loop {
        let expected = src_iter.next().map(|kv| kv.expect("read source kv"));
        let actual = dst_iter.next().map(|kv| kv.expect("read destination kv"));
        if expected.is_none() && actual.is_none() {
            break;
        }
        assert_eq!(expected, actual, "key/value divergence after sync");
    }
    assert_eq!(dst_root, src.target, "destination root must equal target");
}

/// First work item of a fresh `task_limit = 1` syncer: the whole keyspace.
fn whole_keyspace_item(syncer: &Syncer<'_>) -> WorkItem {
    match syncer.get_work().expect("get work") {
        GetWork::Work(item) => {
            assert!(item.first_key.is_none(), "single seed slice is unbounded");
            assert!(item.last_key.is_none(), "single seed slice is unbounded");
            item
        }
        other => panic!("expected work for a fresh sync, got {other:?}"),
    }
}

/// The empty Ethereum trie has a root hash, so an empty destination is already
/// at that target. (The empty MerkleDB trie has no root hash, so it cannot be a
/// target at all.)
#[test]
fn empty_ethereum_target_is_done_for_an_empty_destination() {
    let target = TestDb::<EthHash>::new()
        .root_hash()
        .expect("the empty Ethereum trie has a root hash");
    let dst = TestDb::<EthHash>::new();
    let syncer = start_sync(&*dst, target.clone(), limit(4));
    assert!(matches!(syncer.get_work(), Ok(GetWork::Done)));
    assert_eq!(syncer.finish(), Some(target));
}

#[hash_mode]
#[test]
fn task_limit_one_full_sweep_end_to_end<H: HashMode>() {
    let src = source_n::<H>(100);
    let dst = TestDb::<H>::new();
    let syncer = start_sync(&*dst, src.target.clone(), limit(1));
    // Progress count (guards against re-serving keys forever): each truncated
    // proof serves exactly 8 kvs and the continuation resumes at succ(L),
    // so no key is ever re-served and exactly ceil(100/8) = 13 kv-bearing
    // proofs occur (12 full chunks + a 4-kv tail). Equality matters: a
    // slack bound of +2 would absorb a one-key-per-proof re-serve
    // regression, which lands on exactly 15. One worker is never idle, so
    // nothing is shed.
    let stats = drive_serial(&syncer, &src, Some(limit(8)), 128);
    let expected = 100usize.div_ceil(8);
    assert_eq!(
        stats.kv_submits, expected,
        "keys were re-served or dropped: {} kv-bearing submits for 100 keys at limit 8",
        stats.kv_submits
    );
    assert_eq!(syncer.finish(), Some(src.target.clone()));
    assert_synced(&dst, &src);
}

/// succ(L) continuation boundary: the source contains the prefix-adjacent
/// pair `k` and `k‖0x00` — the minimal strict successor, which is exactly
/// where the continuation resumes after a proof truncates at `k`. A
/// successor regression that skips past the immediate successor (e.g.
/// appending `0x01` instead of `0x00`) either never fetches `k‖0x00` or
/// claims it covered without serving it; both fail the root assertions
/// below. The pure layer pins succ semantics synthetically; this proves the
/// boundary key actually crosses the wire. Keys are 31 and 32 bytes, inside
/// the Ethereum hash-mode cap.
#[hash_mode]
#[test]
fn succ_boundary_adjacent_pair_is_fetched<H: HashMode>() {
    let k = vec![0xaa; 31];
    let mut k0 = k.clone();
    k0.push(0x00);
    // Two filler keys below k so a limit-3 proof truncates exactly at k,
    // leaving k‖0x00 as the sole key of the continuation request.
    let src = source_with::<H>(vec![
        (vec![0x10], b"f1".to_vec()),
        (vec![0x20], b"f2".to_vec()),
        (k, b"vk".to_vec()),
        (k0, b"vk0".to_vec()),
    ]);
    let dst = TestDb::<H>::new();
    let syncer = start_sync(&*dst, src.target.clone(), limit(1));
    let stats = drive_serial(&syncer, &src, Some(limit(3)), 64);
    // Exactly two kv-bearing proofs: [f1, f2, k] then [k‖0x00].
    assert_eq!(
        stats.kv_submits, 2,
        "the boundary key was re-served or dropped"
    );
    assert_eq!(syncer.finish(), Some(src.target.clone()));
    assert_synced(&dst, &src);
}

#[hash_mode]
#[test]
fn shedding_occurs_with_real_truncated_proofs<H: HashMode>() {
    // All 500 keys differ only in their last two bytes, packed at the very
    // left of the LEFT seed slice (the task_limit = 2 seed split is at
    // 0x80), like the storage of one hot account. The right slice is empty,
    // so its worker goes idle at once, and the first truncation of the
    // dense lineage must shed at a cut inside the keys.
    let src = source_n::<H>(500);
    let dst = TestDb::<H>::new();
    let syncer = start_sync(&*dst, src.target.clone(), limit(2));

    let GetWork::Work(left) = syncer.get_work().expect("get work") else {
        panic!("expected the left seed slice");
    };
    let GetWork::Work(right) = syncer.get_work().expect("get work") else {
        panic!("expected the right seed slice");
    };

    // Retire the empty right slice first: a kv-less absence proof.
    let absence = fetch(&src, &right, Some(limit(16)));
    assert!(absence.key_values().is_empty(), "right slice must be empty");
    assert!(matches!(
        syncer.submit(right.id, &absence),
        Ok(Submit::Exhausted)
    ));
    // Nothing cold remains (left lineage in flight, right covered).
    assert!(matches!(syncer.get_work(), Ok(GetWork::Wait)));

    let proof = fetch(&src, &left, Some(limit(16)));
    let Submit::Continue(kept) = syncer.submit(left.id, &proof).expect("honest submit") else {
        panic!("the dense lineage must truncate");
    };
    assert!(
        kept.wakeup_neighbor,
        "an idle worker means the first truncation sheds"
    );
    let GetWork::Work(helper) = syncer.get_work().expect("get work") else {
        panic!("the shed cold chunk must be handed out immediately");
    };
    assert!(
        !fetch(&src, &helper, Some(limit(16)))
            .key_values()
            .is_empty(),
        "the cut must split the dense keys, not land past them"
    );

    // Drive both lineages (and any further sheds) to completion.
    let mut held = VecDeque::new();
    held.push_back(kept);
    held.push_back(helper);
    drive_with(&syncer, &src, Some(limit(16)), 512, None, held);
    assert_eq!(syncer.finish(), Some(src.target.clone()));
    assert_synced(&dst, &src);
}

/// Hostile-proof corpus: every class of tampered or mis-served proof must be
/// rejected without mutating the destination, and the SAME syncer (same
/// reserved id) must then accept a genuine proof and run to completion —
/// the retry-to-success epilogue is part of every corpus test.
///
/// Classification (asserted by the corpus):
///
/// | Failure class | Caught by | Surfaces as |
/// |---|---|---|
/// | truncated wire bytes, trailing bytes, bad header/varint/discriminant | `FrozenRangeProof::from_slice` (`ReadError`) | FFI maps the parse error to `InvalidProof` before `submit` |
/// | wrong root, stale-revision proof, kv tampering, wrong-bounds proof | `verify_range_proof_structure` inside `submit` | `Submit::InvalidProof`, zero state mutation, same id reserved |
/// | bit flip that parses to a semantically identical proof | neither — correctly so | skipped via the `parsed == genuine` guard |
mod hostile {
    use super::*;
    use crate::proofs::verify_range_proof_structure;

    type Kv = (crate::merkle::Key, crate::merkle::Value);

    /// True iff `proof` passes the EXACT verification `Syncer::submit`
    /// runs for `item`'s reserved bounds. Used to pre-screen mutations
    /// that corrupt only encoding verification does not bind (e.g. the
    /// `partial_len` of a zero-nibble edge node under merkledb hashing):
    /// `submit` would legitimately ACCEPT such a proof — retiring the
    /// reservation the corpus reuses — so they are classified out here,
    /// guarded by a kv-equality assert at the call sites (identical kvs
    /// mean an identical merge, i.e. the mutation is semantically inert).
    ///
    /// kv equality also pins the continuation extent: the naive
    /// `find_next_key_after_range_proof` derives it solely from
    /// `key_values().last()`. A find-next implementation that inspects the
    /// proof's structure would need this guard to compare extents too.
    fn verifies<H: HashMode>(src: &Source<H>, item: &WorkItem, proof: &FrozenRangeProof) -> bool {
        verify_range_proof_structure(
            proof,
            src.target.clone(),
            item.first_key.as_deref(),
            item.last_key.as_deref(),
            H::ALGORITHM,
            None,
        )
        .is_ok()
    }

    /// Rebuild a proof from cloned edge proofs and replacement key/values
    /// (no byte surgery needed; `Proof` and `ProofNode` are `Clone`).
    fn rebuilt(genuine: &FrozenRangeProof, kvs: Vec<Kv>) -> FrozenRangeProof {
        FrozenRangeProof::new(
            genuine.start_proof().clone(),
            genuine.end_proof().clone(),
            kvs.into_boxed_slice(),
            genuine.hash_mode(),
        )
    }

    fn with_bit_flipped(bytes: &[u8], idx: usize) -> Box<[u8]> {
        let mut v = bytes.to_vec();
        v[idx] ^= 0x01;
        v.into_boxed_slice()
    }

    /// Retry-to-success epilogue shared by every corpus test: after any
    /// number of rejections, the SAME reserved id accepts a genuine proof
    /// and the same syncer drives to completion.
    fn complete_after_rejections<H: HashMode>(
        syncer: &Syncer<'_>,
        src: &Source<H>,
        dst: &Db<H>,
        item: WorkItem,
        proof_limit: Option<NonZeroUsize>,
    ) {
        let genuine = fetch(src, &item, proof_limit);
        let mut held = VecDeque::new();
        match syncer
            .submit(item.id, &genuine)
            .expect("genuine proof accepted under the same id")
        {
            Submit::Continue(next) => held.push_back(next),
            Submit::Exhausted => {}
            Submit::InvalidProof(e) => panic!("genuine proof rejected after hostile retries: {e}"),
        }
        drive_with(syncer, src, proof_limit, 128, None, held);
        assert_synced(dst, src);
    }

    #[hash_mode]
    #[test]
    fn hostile_bit_flips_rejected_then_sync_completes<H: HashMode>() {
        let src = source_n::<H>(100);
        let dst = TestDb::<H>::new();
        let syncer = start_sync(&*dst, src.target.clone(), limit(1));
        let item = whole_keyspace_item(&syncer);
        let genuine = fetch(&src, &item, Some(limit(16)));
        let bytes = to_bytes(&genuine);
        let root_before = dst.root_hash();

        let (mut parse_rejects, mut verify_rejects, mut equivalent) = (0usize, 0usize, 0usize);
        // Deterministic corpus: flip bit (pos % 8) of every byte position.
        for pos in 0..bytes.len() {
            let mut tampered = bytes.clone();
            tampered[pos] ^= 1 << (pos % 8);
            match FrozenRangeProof::from_slice(&tampered) {
                // The FFI layer maps parse errors to InvalidProof before
                // submit is ever reached.
                Err(_) => parse_rejects += 1,
                // Encoding-equivalent flip: the only non-flaky way to
                // tolerate it.
                Ok(parsed) if parsed == genuine => equivalent += 1,
                // Semantically inert flip (verifies; see [`verifies`]): the
                // kv-equality assert is the cryptographic guard — a flip
                // that verifies with DIFFERENT kvs would be a real break.
                Ok(parsed) if verifies(&src, &item, &parsed) => {
                    assert_eq!(
                        parsed.key_values(),
                        genuine.key_values(),
                        "flip at byte {pos} verifies with different key/values"
                    );
                    equivalent += 1;
                }
                Ok(parsed) => {
                    match syncer.submit(item.id, &parsed) {
                        Ok(Submit::InvalidProof(_)) => verify_rejects += 1,
                        other => panic!("flip at byte {pos} was not rejected: {other:?}"),
                    }
                    assert_eq!(
                        dst.root_hash(),
                        root_before,
                        "rejected proof must not commit (flip at byte {pos})"
                    );
                }
            }
        }
        assert_eq!(
            parse_rejects + verify_rejects + equivalent,
            bytes.len(),
            "every byte position must be classified"
        );
        assert!(parse_rejects > 0, "some flips must break the wire format");
        assert!(verify_rejects > 0, "some flips must reach the verifier");

        complete_after_rejections(&syncer, &src, &dst, item, Some(limit(16)));
    }

    #[hash_mode]
    #[test]
    fn hostile_truncated_wire_bytes_unparseable<H: HashMode>() {
        // The trailing-bytes rule makes the full length the unique parse
        // point, so EVERY strict prefix of valid proof bytes fails to
        // parse. This class is owned entirely by the (FFI) parse layer;
        // `submit` never sees it — which is also why this is the one
        // corpus test without the retry-to-completion epilogue: nothing
        // reaches a syncer, so there is no reserved id to retry.
        let src = source_n::<H>(64);
        let view = src
            .db
            .revision(src.target.clone())
            .expect("target revision exists");
        let proof = view
            .range_proof(None::<&[u8]>, None::<&[u8]>, Some(limit(16)))
            .expect("generate the genuine proof");
        let bytes = to_bytes(&proof);
        for cut in 0..bytes.len() {
            assert!(
                FrozenRangeProof::from_slice(&bytes[..cut]).is_err(),
                "strict prefix of length {cut} must not parse"
            );
        }
    }

    #[hash_mode]
    #[test]
    fn hostile_wrong_root_proof_rejected_same_id_retry<H: HashMode>() {
        // Source committed twice: revision 1 is a stale revision of the
        // SAME database; the target is revision 2's root.
        let src_db = TestDb::<H>::new();
        let old_batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> = (0..100u64)
            .map(|i| BatchOp::Put {
                key: key32(i),
                value: format!("old{i}").into_bytes(),
            })
            .collect();
        let stale_root = src_db
            .update(old_batch)
            .expect("populate revision 1")
            .expect("non-empty commit");
        src_db
            .update((0..100u64).map(|i| BatchOp::Put {
                key: key32(i),
                value: val(i),
            }))
            .expect("populate revision 2");
        let target = src_db.root_hash().expect("source database is non-empty");
        let src = Source { db: src_db, target };
        // A second valid source with the same keys but different values.
        let imposter = source_with::<H>(
            (0..100u64)
                .map(|i| (key32(i), format!("wrong{i}").into_bytes()))
                .collect(),
        );

        let dst = TestDb::<H>::new();
        let syncer = start_sync(&*dst, src.target.clone(), limit(1));
        let item = whole_keyspace_item(&syncer);
        let root_before = dst.root_hash();

        let wrong = fetch(&imposter, &item, Some(limit(16)));
        let stale = src
            .db
            .revision(stale_root)
            .expect("revision 1 is retained")
            .range_proof(
                item.first_key.as_deref(),
                item.last_key.as_deref(),
                Some(limit(16)),
            )
            .expect("revision 1 can prove the range");

        // The canonical "different peer, same range" loop: three rejected
        // attempts under the same id (alternating both hostile shapes),
        // then the honest peer.
        for attempt in 0..3u32 {
            let hostile = if attempt % 2 == 0 { &wrong } else { &stale };
            match syncer.submit(item.id, hostile) {
                Ok(Submit::InvalidProof(_)) => {}
                other => panic!("attempt {attempt}: expected InvalidProof, got {other:?}"),
            }
            assert_eq!(
                dst.root_hash(),
                root_before,
                "attempt {attempt}: rejected proof must not commit"
            );
            // The region stays reserved under the same id (task_limit is 1,
            // so a freed slot would hand out work instead of Wait).
            assert!(matches!(syncer.get_work(), Ok(GetWork::Wait)));
        }

        complete_after_rejections(&syncer, &src, &dst, item, Some(limit(16)));
    }

    #[hash_mode]
    #[test]
    fn hostile_mismatched_range_proof_rejected<H: HashMode>() {
        // A valid proof answering the WRONG question: bounds (k20, k40)
        // against the (None, None) reservation. The verifier sees a
        // non-empty start proof under a None bound and rejects.
        let src = source_n::<H>(100);
        let dst = TestDb::<H>::new();
        let syncer = start_sync(&*dst, src.target.clone(), limit(1));
        let item = whole_keyspace_item(&syncer);
        let root_before = dst.root_hash();

        let mismatched = src
            .db
            .revision(src.target.clone())
            .expect("target revision exists")
            .range_proof(Some(key32(20)), Some(key32(40)), None)
            .expect("generate the mismatched-bounds proof");
        match syncer.submit(item.id, &mismatched) {
            Ok(Submit::InvalidProof(_)) => {}
            other => panic!("expected InvalidProof for mismatched bounds, got {other:?}"),
        }
        assert_eq!(dst.root_hash(), root_before, "rejection must not commit");
        assert!(matches!(syncer.get_work(), Ok(GetWork::Wait)));

        complete_after_rejections(&syncer, &src, &dst, item, Some(limit(16)));
    }

    #[hash_mode]
    #[test]
    fn hostile_tampered_key_values_rejected<H: HashMode>() {
        let src = source_n::<H>(100);
        let dst = TestDb::<H>::new();
        let syncer = start_sync(&*dst, src.target.clone(), limit(1));
        let item = whole_keyspace_item(&syncer);
        let genuine = fetch(&src, &item, Some(limit(16)));
        let root_before = dst.root_hash();

        let kvs: Vec<Kv> = genuine.key_values().to_vec();
        assert_eq!(kvs.len(), 16, "fixture: a full truncated proof");
        let mid = kvs.len() / 2;

        let mut value_flipped = kvs.clone();
        value_flipped[mid].1 = with_bit_flipped(&value_flipped[mid].1, 0);
        let mut key_flipped = kvs.clone();
        key_flipped[mid].0 = with_bit_flipped(&key_flipped[mid].0, 20);
        let mut dropped = kvs.clone();
        dropped.remove(mid);
        let mut duplicated = kvs.clone();
        let dup = duplicated[mid].clone();
        duplicated.insert(mid, dup);
        let mut swapped = kvs.clone();
        let tmp = swapped[mid].clone();
        swapped[mid] = swapped[mid + 1].clone();
        swapped[mid + 1] = tmp;
        let mut appended = kvs.clone();
        // A key past the proven boundary (L = key32(15) for this fixture).
        appended.push((key32(99).into(), val(99).into()));

        let tampered: Vec<(&str, FrozenRangeProof)> = vec![
            ("value byte changed", rebuilt(&genuine, value_flipped)),
            ("key byte changed", rebuilt(&genuine, key_flipped)),
            ("middle kv dropped", rebuilt(&genuine, dropped)),
            ("kv duplicated", rebuilt(&genuine, duplicated)),
            ("adjacent kvs swapped", rebuilt(&genuine, swapped)),
            (
                "kv appended past proven boundary",
                rebuilt(&genuine, appended),
            ),
            (
                "kvs cleared, edge proofs remain",
                rebuilt(&genuine, Vec::new()),
            ),
        ];
        for (label, proof) in &tampered {
            match syncer.submit(item.id, proof) {
                Ok(Submit::InvalidProof(_)) => {}
                other => panic!("{label}: expected InvalidProof, got {other:?}"),
            }
            assert_eq!(
                dst.root_hash(),
                root_before,
                "{label}: rejection must not commit"
            );
        }

        complete_after_rejections(&syncer, &src, &dst, item, Some(limit(16)));
    }

    #[hash_mode]
    #[test]
    fn test_slow_hostile_random_mutation_soak<H: HashMode>() {
        let rng = SeededRng::from_env_or_random();
        let src = source_n::<H>(300);
        let dst = TestDb::<H>::new();
        let syncer = start_sync(&*dst, src.target.clone(), limit(1));
        let mut item = whole_keyspace_item(&syncer);
        let mut genuine = fetch(&src, &item, Some(limit(8)));
        let mut bytes = to_bytes(&genuine);

        for iteration in 1..=500u32 {
            if iteration.is_multiple_of(50) {
                // Advance real coverage so the corpus runs against MOVING
                // coverage, not only the first region. 300 keys at limit 8
                // need 38 truncated chunks; only 10 genuine submissions
                // happen here, so exhaustion is unreachable.
                match syncer.submit(item.id, &genuine).expect("genuine submit") {
                    Submit::Continue(next) => {
                        item = next;
                        genuine = fetch(&src, &item, Some(limit(8)));
                        bytes = to_bytes(&genuine);
                    }
                    other => panic!("iteration {iteration}: expected Continue, got {other:?}"),
                }
                continue;
            }
            let root_before = dst.root_hash();
            let mut tampered = bytes.clone();
            match rng.random_range(0..4u32) {
                0 => {
                    // Flip 1..=8 random bits.
                    for _ in 0..rng.random_range(1..=8usize) {
                        let pos = rng.random_range(0..tampered.len());
                        tampered[pos] ^= 1 << rng.random_range(0..8u32);
                    }
                }
                1 => {
                    // Splice: copy a random run over another offset.
                    let len = rng.random_range(1..=tampered.len().min(64));
                    let from = rng.random_range(0..=tampered.len() - len);
                    let to = rng.random_range(0..=tampered.len() - len);
                    let chunk: Vec<u8> = tampered[from..from + len].to_vec();
                    tampered.splice(to..to + len, chunk);
                }
                2 => {
                    // Truncate a random suffix: always a strict prefix, so
                    // always a parse reject (trailing-bytes rule).
                    tampered.truncate(rng.random_range(0..tampered.len()));
                }
                _ => {
                    // Overwrite a random run with random bytes.
                    let len = rng.random_range(1..=tampered.len().min(32));
                    let at = rng.random_range(0..=tampered.len() - len);
                    rng.fill_bytes(&mut tampered[at..at + len]);
                }
            }
            match FrozenRangeProof::from_slice(&tampered) {
                Err(_) => {}                          /* parse reject: the FFI layer owns this class */
                Ok(parsed) if parsed == genuine => {} // mutation round-tripped
                Ok(parsed) if verifies(&src, &item, &parsed) => {
                    // Semantically inert mutation (see [`verifies`]):
                    // identical kvs mean an identical merge.
                    assert_eq!(
                        parsed.key_values(),
                        genuine.key_values(),
                        "iteration {iteration}: mutation verifies with different key/values"
                    );
                }
                Ok(parsed) => {
                    match syncer.submit(item.id, &parsed) {
                        Ok(Submit::InvalidProof(_)) => {}
                        other => {
                            panic!("iteration {iteration}: mutation not rejected: {other:?}")
                        }
                    }
                    assert_eq!(
                        dst.root_hash(),
                        root_before,
                        "iteration {iteration}: rejection must not commit"
                    );
                }
            }
        }

        complete_after_rejections(&syncer, &src, &dst, item, Some(limit(8)));
    }
}
