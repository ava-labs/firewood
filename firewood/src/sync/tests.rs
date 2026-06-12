// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! End-to-end state-sync tests (see `docs/plans/state-sync.md`).
//!
//! These complement the pure-layer tests in `super::state` and the
//! single-submit seam tests in `super::syncer`: everything here drives
//! multi-step flows against real databases — a hostile-proof corpus with a
//! retry-to-success epilogue ([`hostile`]), restart-from-scratch idempotency
//! ([`restart`]), concurrent worker pools under reap pressure
//! ([`concurrent`]), shedding with real truncated proofs, and a randomized
//! round-trip property test ([`random`]).
//!
//! Keys are capped at 32 bytes when `ethhash` is enabled — see
//! [`max_key_len`] for the known bug this designs around.

#![expect(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    reason = "tests index small fixed-size buffers and use plain arithmetic on small constants"
)]

use std::collections::VecDeque;
use std::num::NonZeroUsize;

use firewood_storage::{CheckOpt, CheckerError, SeededRng};

use crate::api::{self, Db as _, DbView as _, FrozenRangeProof, HashKey};
use crate::db::test::TestDb;
use crate::db::{BatchOp, Db, DbConfig};
use crate::manager::RevisionManagerConfig;

use super::state::SHED_TRUNCATION_LIMIT;
use super::{GetWork, Submit, SyncError, Syncer, WorkId, WorkItem, start_sync};

/// 32-byte big-endian-indexed key: 24 zero bytes then `i`. Sized like an
/// Ethereum account key so the checker accepts it on both feature axes
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

/// Maximum length of generated trie keys.
///
/// Under `ethhash` this is capped at 32 bytes: truncated
/// `range_proof(None, None, limit)` proofs over tries with keys LONGER than
/// 32 bytes currently fail verification against their own root (the
/// fake-root single-child account hash diverges from
/// `ProofNode::from(PathIterItem)`), and the `task_limit = 1` sync issues
/// exactly that proof shape.
///
/// TODO: extend to 64-byte keys under `ethhash` once that bug is fixed;
/// until then the 64-byte axis runs on default features only.
const fn max_key_len() -> usize {
    if cfg!(feature = "ethhash") { 32 } else { 64 }
}

/// A source database committed with known content; `target` is its root.
struct Source {
    db: TestDb,
    target: HashKey,
}

fn source_with(kvs: Vec<(Vec<u8>, Vec<u8>)>) -> Source {
    let db = TestDb::new();
    let batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> = kvs
        .into_iter()
        .map(|(key, value)| BatchOp::Put { key, value })
        .collect();
    db.update(batch).expect("populate the source");
    let target = db.root_hash().expect("source database is non-empty");
    Source { db, target }
}

/// Source of `n` deterministic [`key32`]/[`val`] pairs.
fn source_n(n: u64) -> Source {
    source_with((0..n).map(|i| (key32(i), val(i))).collect())
}

/// Honest fetcher: generate the proof a peer would serve for `item`,
/// optionally truncated to `proof_limit` pairs (the generation-side
/// truncation knob).
fn fetch(src: &Source, item: &WorkItem, proof_limit: Option<NonZeroUsize>) -> FrozenRangeProof {
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
    proof.write_to_vec(&mut out);
    out
}

/// Read a key from the latest committed revision of `db`.
fn latest_val(db: &Db, key: &[u8]) -> Option<crate::merkle::Value> {
    let root = db.root_hash().expect("database is non-empty");
    let view = db.revision(root).expect("latest revision exists");
    view.val(key).expect("read the key")
}

/// Outcome counters from a serial drive.
struct DriveStats {
    /// Total successful submits, including the kv-less absence proofs that
    /// retire shed cold chunks and refeed bisection slices.
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
/// given, dropping any held items (abandoned in-flight reservations — the
/// restart tests depend on this).
fn drive_with(
    syncer: &Syncer<'_>,
    src: &Source,
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
fn drive_serial(
    syncer: &Syncer<'_>,
    src: &Source,
    proof_limit: Option<NonZeroUsize>,
    max_submits: usize,
) -> DriveStats {
    drive_with(syncer, src, proof_limit, max_submits, None, VecDeque::new())
}

/// [`drive_with`] from scratch, stopping after exactly `stop_after`
/// successful submits (held items dropped).
fn drive_serial_partial(
    syncer: &Syncer<'_>,
    src: &Source,
    proof_limit: Option<NonZeroUsize>,
    stop_after: usize,
) -> DriveStats {
    drive_with(
        syncer,
        src,
        proof_limit,
        usize::MAX,
        Some(stop_after),
        VecDeque::new(),
    )
}

/// Root equality plus full key/value iteration equality (the iteration
/// gives better failure diagnostics than the root alone — and proves stale
/// destination keys were deleted, not merely out-hashed).
fn assert_synced(dst: &Db, src: &Source) {
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

/// Run `db.check()` and assert no errors other than `AreaLeaks`.
/// `tolerate_unpersisted` allows callers running against in-memory state
/// (the persist worker may trail the latest committed root) to also ignore
/// `UnpersistedRoot`; after a reopen, pass `false`.
///
/// Local copy of the `db/tests/concurrent_rebase.rs` helper — deliberate
/// duplication; this commit does not refactor `db/tests`.
fn assert_check_clean(db: &Db, label: &str, tolerate_unpersisted: bool) {
    let report = db.check(CheckOpt {
        hash_check: true,
        progress_bar: None,
    });
    let real_errors: Vec<_> = report
        .errors
        .iter()
        .filter(|e| !matches!(e, CheckerError::AreaLeaks(_)))
        .filter(|e| !(tolerate_unpersisted && matches!(e, CheckerError::UnpersistedRoot)))
        .collect();
    assert!(real_errors.is_empty(), "{label}: {real_errors:?}");
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

#[test]
fn task_limit_one_full_sweep_end_to_end() {
    let src = source_n(100);
    let dst = TestDb::new();
    let syncer = start_sync(&dst, src.target.clone(), limit(1));
    // Progress count (a bug-#1989-shaped regression guard): each truncated
    // proof serves exactly 8 kvs and the continuation resumes at succ(L),
    // so no key is ever re-served and exactly ceil(100/8) = 13 kv-bearing
    // proofs occur (12 full chunks + a 4-kv tail). Equality matters: a
    // slack bound of +2 would absorb a one-key-per-proof re-serve
    // regression, which lands on exactly 15. Shed cold chunks and refeed
    // bisection retire EMPTY regions with kv-less absence proofs (every
    // key32 key sits far left of every shed midpoint), which do not count
    // here; the total submit count is still stall-guarded by `max_submits`.
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
/// the ethhash cap.
#[test]
fn succ_boundary_adjacent_pair_is_fetched() {
    let k = vec![0xaa; 31];
    let mut k0 = k.clone();
    k0.push(0x00);
    // Two filler keys below k so a limit-3 proof truncates exactly at k,
    // leaving k‖0x00 as the sole key of the continuation request.
    let src = source_with(vec![
        (vec![0x10], b"f1".to_vec()),
        (vec![0x20], b"f2".to_vec()),
        (k, b"vk".to_vec()),
        (k0, b"vk0".to_vec()),
    ]);
    let dst = TestDb::new();
    let syncer = start_sync(&dst, src.target.clone(), limit(1));
    let stats = drive_serial(&syncer, &src, Some(limit(3)), 64);
    // Exactly two kv-bearing proofs: [f1, f2, k] then [k‖0x00].
    assert_eq!(
        stats.kv_submits, 2,
        "the boundary key was re-served or dropped"
    );
    assert_eq!(syncer.finish(), Some(src.target.clone()));
    assert_synced(&dst, &src);
}

#[test]
fn shedding_occurs_with_real_truncated_proofs() {
    // All 500 keys packed densely at the very left of the LEFT seed slice
    // (key32 keys start with a zero byte; the task_limit = 2 seed split is
    // at 0x80). The right slice is empty, so its worker exhausts
    // immediately, and the dense left lineage must truncate
    // SHED_TRUNCATION_LIMIT consecutive times and then shed — this is the
    // pure-layer shed test re-proven with real proof extents instead of
    // synthetic `Truncated { L }`.
    let src = source_n(500);
    let dst = TestDb::new();
    let syncer = start_sync(&dst, src.target.clone(), limit(2));

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

    // Sweep the dense lineage until the shed fires.
    let mut item = left;
    let mut truncations = 0u32;
    let cont = loop {
        let proof = fetch(&src, &item, Some(limit(16)));
        match syncer.submit(item.id, &proof).expect("honest submit") {
            Submit::Continue(next) => {
                truncations += 1;
                if next.wakeup_neighbor {
                    break next; // the shed: a cold chunk was published
                }
                assert!(
                    truncations < SHED_TRUNCATION_LIMIT,
                    "no shed after {truncations} consecutive truncations"
                );
                item = next;
            }
            other => panic!("dense lineage must keep truncating before the shed: {other:?}"),
        }
    };
    assert_eq!(
        truncations, SHED_TRUNCATION_LIMIT,
        "the shed must fire exactly at the truncation limit"
    );
    // The shed cold chunk is immediately fetchable: Work, not Wait.
    let GetWork::Work(helper) = syncer.get_work().expect("get work") else {
        panic!("the shed cold chunk must be handed out immediately");
    };

    // Drive both lineages (and any further sheds) to completion.
    let mut held = VecDeque::new();
    held.push_back(cont);
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
/// | wrong root, stale-revision proof, kv tampering, wrong-bounds proof, oversized proof | `verify_range_proof_structure` inside `submit` | `Submit::InvalidProof`, zero state mutation, same id reserved |
/// | bit flip that parses to a semantically identical proof | neither — correctly so | skipped via the `parsed == genuine` guard |
///
/// There is deliberately NO oversized-proof test here: the landed seam test
/// `submit_classifies_internal_vs_peer` (`super::syncer`) already proves
/// oversized → `InvalidProof` end-to-end at the real `MAX_PROOF_KEYS`
/// constant; duplicating it with a >32768-key corpus buys nothing.
mod hostile {
    use crate::proofs::verify_range_proof_structure;
    use crate::sync::MAX_PROOF_KEYS;

    use super::*;

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
    /// kv equality also pins the continuation extent today: the naive
    /// `find_next_key_after_range_proof` derives it solely from
    /// `key_values().last()`. If #352's structural implementation lands,
    /// extend this guard to compare extents too.
    fn verifies(src: &Source, item: &WorkItem, proof: &FrozenRangeProof) -> bool {
        verify_range_proof_structure(
            proof,
            src.target.clone(),
            item.first_key.as_deref(),
            item.last_key.as_deref(),
            Some(MAX_PROOF_KEYS),
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
    fn complete_after_rejections(
        syncer: &Syncer<'_>,
        src: &Source,
        dst: &Db,
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

    #[test]
    fn hostile_bit_flips_rejected_then_sync_completes() {
        let src = source_n(100);
        let dst = TestDb::new();
        let syncer = start_sync(&dst, src.target.clone(), limit(1));
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

    #[test]
    fn hostile_truncated_wire_bytes_unparseable() {
        // The trailing-bytes rule makes the full length the unique parse
        // point, so EVERY strict prefix of valid proof bytes fails to
        // parse. This class is owned entirely by the (FFI) parse layer;
        // `submit` never sees it — which is also why this is the one
        // corpus test without the retry-to-completion epilogue: nothing
        // reaches a syncer, so there is no reserved id to retry.
        let src = source_n(64);
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

    #[test]
    fn hostile_wrong_root_proof_rejected_same_id_retry() {
        // Source committed twice: revision 1 is a stale revision of the
        // SAME database; the target is revision 2's root.
        let src_db = TestDb::new();
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
        let imposter = source_with(
            (0..100u64)
                .map(|i| (key32(i), format!("wrong{i}").into_bytes()))
                .collect(),
        );

        let dst = TestDb::new();
        let syncer = start_sync(&dst, src.target.clone(), limit(1));
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

    #[test]
    fn hostile_mismatched_range_proof_rejected() {
        // A valid proof answering the WRONG question: bounds (k20, k40)
        // against the (None, None) reservation. The verifier sees a
        // non-empty start proof under a None bound and rejects.
        let src = source_n(100);
        let dst = TestDb::new();
        let syncer = start_sync(&dst, src.target.clone(), limit(1));
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

    #[test]
    fn hostile_tampered_key_values_rejected() {
        let src = source_n(100);
        let dst = TestDb::new();
        let syncer = start_sync(&dst, src.target.clone(), limit(1));
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

    #[test]
    fn test_slow_hostile_random_mutation_soak() {
        let rng = SeededRng::from_env_or_random();
        let src = source_n(300);
        let dst = TestDb::new();
        let syncer = start_sync(&dst, src.target.clone(), limit(1));
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
                Err(_) => {}                          // parse reject: the FFI layer owns this class
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

/// Restart-from-scratch idempotency: dropping a syncer loses all in-memory
/// coverage bookkeeping; a fresh syncer over the same database must converge
/// from committed revisions alone, correcting divergent values and deleting
/// stale keys as proofs re-cover their ranges.
mod restart {
    use test_case::test_case;

    use super::*;

    /// A stale key absent from the source, scattered across the keyspace
    /// via its first byte (source keys all start with 0x00).
    fn stale_key(i: u64) -> Vec<u8> {
        const SCATTER: [u8; 5] = [0x20, 0x55, 0x90, 0xcc, 0xee];
        let mut k = vec![0u8; 24];
        k[0] = SCATTER[(i % 5) as usize];
        k.extend_from_slice(&i.to_be_bytes());
        k
    }

    // Drop-point rationale: 0 = pure restart (degenerate), 1 = after the
    // first commit, 3/7 = mid-sweep (reservation dropped mid-lineage),
    // 12 = deep into the sweep — much of the keyspace covered in syncer
    // #1's head, none of it durable as bookkeeping, only as committed
    // revisions; the restart must converge from revisions alone.
    #[test_case(0)]
    #[test_case(1)]
    #[test_case(3)]
    #[test_case(7)]
    #[test_case(12)]
    fn restart_completes_after_partial_sync(drop_point: usize) {
        let src = source_n(200);
        let dst = TestDb::new();
        // Pre-populate divergent state: 50 overlapping keys with WRONG
        // values plus 30 stale keys absent from the source. Completion
        // must correct the former and delete the latter.
        let mut divergent: Vec<BatchOp<Vec<u8>, Vec<u8>>> = (0..50u64)
            .map(|i| BatchOp::Put {
                key: key32(i * 4),
                value: b"divergent".to_vec(),
            })
            .collect();
        let stale_keys: Vec<Vec<u8>> = (0..30u64).map(stale_key).collect();
        divergent.extend(stale_keys.iter().map(|k| BatchOp::Put {
            key: k.clone(),
            value: b"stale".to_vec(),
        }));
        dst.update(divergent).expect("pre-populate the destination");

        // Syncer #1: partial drive, then dropped mid-flight (in-memory
        // coverage lost; any in-flight reservation abandoned).
        {
            let syncer = start_sync(&dst, src.target.clone(), limit(1));
            let stats = drive_serial_partial(&syncer, &src, Some(limit(16)), drop_point);
            assert_eq!(stats.submits, drop_point);
        }
        // Syncer #2: fresh bookkeeping, same destination and target.
        {
            let syncer = start_sync(&dst, src.target.clone(), limit(1));
            drive_serial(&syncer, &src, Some(limit(16)), 256);
            assert_eq!(syncer.finish(), Some(src.target.clone()));
        }
        assert_synced(&dst, &src);
        for (i, k) in stale_keys.iter().enumerate() {
            assert!(
                latest_val(&dst, k).is_none(),
                "stale key {i} must be deleted by completion"
            );
        }
        assert_eq!(
            latest_val(&dst, &key32(4)).as_deref(),
            Some(val(4).as_slice()),
            "overlapping key must carry the source value"
        );
        // Warm-start tail assertion: a third syncer over the now-complete
        // destination reports Done on the FIRST get_work, with zero fetches.
        {
            let syncer = start_sync(&dst, src.target.clone(), limit(1));
            assert!(matches!(syncer.get_work(), Ok(GetWork::Done)));
        }
        // Commit-heavy test: check, reopen, recheck (concurrent_rebase
        // pattern).
        assert_check_clean(&dst, "post-restart", true);
        let dst = dst.reopen();
        assert_synced(&dst, &src);
        assert_check_clean(&dst, "post-reopen", false);
    }

    #[test]
    fn restart_deletes_stale_keys_in_covered_range() {
        // Sharper merge-stale-deletion shape: sync fully once, inject
        // garbage directly into the destination (simulating
        // crash-recovered divergence inside already-covered ranges), then
        // re-sync to the same target — completion must DELETE the garbage
        // (the merge deletes in-range keys absent from the proof).
        let src = source_n(100);
        let dst = TestDb::new();
        {
            let syncer = start_sync(&dst, src.target.clone(), limit(1));
            drive_serial(&syncer, &src, Some(limit(16)), 256);
            assert_eq!(syncer.finish(), Some(src.target.clone()));
        }
        let garbage_new = key32(1_000_000);
        let garbage_scattered = stale_key(3);
        dst.update(vec![
            BatchOp::Put {
                key: key32(7),
                value: b"clobbered".to_vec(),
            },
            BatchOp::Put {
                key: garbage_new.clone(),
                value: b"garbage".to_vec(),
            },
            BatchOp::Put {
                key: garbage_scattered.clone(),
                value: b"garbage".to_vec(),
            },
        ])
        .expect("inject garbage");
        assert_ne!(
            dst.root_hash(),
            Some(src.target.clone()),
            "garbage must diverge the root"
        );

        {
            let syncer = start_sync(&dst, src.target.clone(), limit(1));
            drive_serial(&syncer, &src, Some(limit(16)), 256);
            assert_eq!(syncer.finish(), Some(src.target.clone()));
        }
        assert_synced(&dst, &src);
        assert!(
            latest_val(&dst, &garbage_new).is_none(),
            "in-range garbage key must be deleted"
        );
        assert!(
            latest_val(&dst, &garbage_scattered).is_none(),
            "scattered garbage key must be deleted"
        );
        assert_eq!(
            latest_val(&dst, &key32(7)).as_deref(),
            Some(val(7).as_slice()),
            "clobbered key must be restored to the source value"
        );
    }
}

/// Concurrent worker pools driving one shared `Syncer` (the FFI pool shape),
/// per the `db/tests/concurrent_rebase.rs` thread-scope precedent. No
/// wall-clock waits anywhere: `get_work` is non-blocking and idempotent, so
/// workers yield-spin on `Wait` with a bounded counter that converts a real
/// deadlock into a deterministic failure.
mod concurrent {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    const SPIN_LIMIT: u64 = 1_000_000;

    /// Resubmit on `RevisionNotFound`, bounded and counted.
    ///
    /// `submit` deliberately has NO internal retry. Under the slow soak's
    /// deliberately pathological config (`max_revisions = 16`, 8 workers),
    /// `Err(SyncError::Api(RevisionNotFound))` MAY genuinely surface when
    /// the proposal's parent revision is reaped mid-submit; the documented
    /// recovery is that the region stays reserved under the same id and a
    /// plain resubmit recovers. This harness validates exactly that
    /// contract — production code must NOT gain a retry loop.
    fn submit_with_resubmit_on_reap(
        syncer: &Syncer<'_>,
        id: WorkId,
        proof: &FrozenRangeProof,
        reap_resubmits: &AtomicUsize,
    ) -> Submit {
        let mut failures = 0u32;
        loop {
            match syncer.submit(id, proof) {
                Ok(outcome) => return outcome,
                Err(SyncError::Api(api::Error::RevisionNotFound { .. })) => {
                    failures += 1;
                    // failures = 1 initial submit + (failures - 1) resubmits.
                    assert!(
                        failures <= 16,
                        "RevisionNotFound persisted through {} resubmits",
                        failures - 1
                    );
                    reap_resubmits.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => panic!("unexpected submit error: {e}"),
            }
        }
    }

    /// Honest worker loop: acquire, sweep to exhaustion, repeat until Done.
    fn honest_worker(syncer: &Syncer<'_>, src: &Source, proof_limit: Option<NonZeroUsize>) {
        let mut waits = 0u64;
        loop {
            match syncer.get_work().expect("no invariant violations") {
                GetWork::Done => return,
                GetWork::Wait => {
                    waits += 1;
                    assert!(waits < SPIN_LIMIT, "deadlock suspected after {waits} waits");
                    std::thread::yield_now();
                }
                GetWork::Work(mut item) => loop {
                    let proof = fetch(src, &item, proof_limit);
                    match syncer.submit(item.id, &proof).expect("honest submit") {
                        Submit::Continue(next) => item = next,
                        Submit::Exhausted => break,
                        Submit::InvalidProof(e) => panic!("honest proof rejected: {e}"),
                    }
                },
            }
        }
    }

    #[test]
    fn concurrent_workers_complete_sync() {
        const THREADS: usize = 4;
        let src = source_n(1000);
        let dst = TestDb::new();
        let syncer = start_sync(&dst, src.target.clone(), limit(THREADS));
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..THREADS)
                .map(|_| s.spawn(|| honest_worker(&syncer, &src, Some(limit(32)))))
                .collect();
            for handle in handles {
                handle.join().expect("worker panicked");
            }
        });
        // Every worker exited via Done; the syncer agrees.
        assert!(matches!(syncer.get_work(), Ok(GetWork::Done)));
        assert_eq!(syncer.finish(), Some(src.target.clone()));
        assert_synced(&dst, &src);
        assert_check_clean(&dst, "post-concurrency", true);
        let dst = dst.reopen();
        assert_synced(&dst, &src);
        assert_check_clean(&dst, "post-reopen", false);
    }

    /// Hostile worker for the slow soak: serves a valid-but-wrong-root
    /// proof on every 7th sweep iteration (which must be absorbed as
    /// `InvalidProof` under concurrency), then the genuine one. Returns
    /// `(sweep_iterations, invalids_absorbed)`.
    fn hostile_worker(
        syncer: &Syncer<'_>,
        src: &Source,
        imposter: &Source,
        proof_limit: Option<NonZeroUsize>,
        reap_resubmits: &AtomicUsize,
    ) -> (u64, u64) {
        let mut waits = 0u64;
        let mut iterations = 0u64;
        let mut invalids = 0u64;
        loop {
            match syncer.get_work().expect("no invariant violations") {
                GetWork::Done => return (iterations, invalids),
                GetWork::Wait => {
                    waits += 1;
                    assert!(waits < SPIN_LIMIT, "deadlock suspected after {waits} waits");
                    std::thread::yield_now();
                }
                GetWork::Work(mut item) => loop {
                    iterations += 1;
                    if iterations.is_multiple_of(7) {
                        let bad = fetch(imposter, &item, proof_limit);
                        match submit_with_resubmit_on_reap(syncer, item.id, &bad, reap_resubmits) {
                            Submit::InvalidProof(_) => invalids += 1,
                            other => panic!("wrong-root proof accepted: {other:?}"),
                        }
                    }
                    let proof = fetch(src, &item, proof_limit);
                    match submit_with_resubmit_on_reap(syncer, item.id, &proof, reap_resubmits) {
                        Submit::Continue(next) => item = next,
                        Submit::Exhausted => break,
                        Submit::InvalidProof(e) => panic!("honest proof rejected: {e}"),
                    }
                },
            }
        }
    }

    #[test]
    fn test_slow_concurrent_workers_reap_and_hostile_soak() {
        const THREADS: usize = 8;
        // Tight enough that reaping fires during the soak (the
        // concurrent_rebase floor: the persist worker cannot keep up below
        // ~16). The resulting RevisionNotFound pressure on submit is
        // OPPORTUNISTIC: on fast hardware the reap race may never win the
        // verify->commit window (0 resubmits observed locally), and the
        // harness asserts nothing about the count — but whenever CI
        // contention does produce the error, `submit_with_resubmit_on_reap`
        // validates the documented same-id resubmit recovery and fails
        // loudly if it ever stops converging.
        const MAX_REVISIONS: usize = 16;
        const KEYS: u64 = 20_000;

        let src = source_n(KEYS);
        // Same keys, different values: every node differs, so any proof
        // from this source fails verification against the target.
        let imposter = source_with(
            (0..KEYS)
                .map(|i| (key32(i), format!("wrong{i}").into_bytes()))
                .collect(),
        );
        let dst = TestDb::new_with_config(
            DbConfig::builder()
                .manager(
                    RevisionManagerConfig::builder()
                        .max_revisions(MAX_REVISIONS)
                        .build(),
                )
                .build(),
        );
        let syncer = start_sync(&dst, src.target.clone(), limit(THREADS));
        let reap_resubmits = AtomicUsize::new(0);

        let results: Vec<(u64, u64)> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..THREADS)
                .map(|_| {
                    s.spawn(|| {
                        hostile_worker(&syncer, &src, &imposter, Some(limit(64)), &reap_resubmits)
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("worker panicked"))
                .collect()
        });

        for (idx, (iterations, invalids)) in results.iter().enumerate() {
            assert!(
                *iterations < 7 || *invalids >= 1,
                "worker {idx} ran {iterations} sweep iterations but absorbed no InvalidProof"
            );
        }
        let total_invalid: u64 = results.iter().map(|&(_, invalids)| invalids).sum();
        assert!(total_invalid >= 1, "no hostile injection fired");
        eprintln!(
            "soak: {total_invalid} InvalidProof absorptions, {} RevisionNotFound resubmits",
            reap_resubmits.load(Ordering::Relaxed)
        );

        assert!(matches!(syncer.get_work(), Ok(GetWork::Done)));
        assert_eq!(syncer.finish(), Some(src.target.clone()));
        assert_synced(&dst, &src);
        assert_check_clean(&dst, "post-soak", true);
        let dst = dst.reopen();
        assert_synced(&dst, &src);
        assert_check_clean(&dst, "post-reopen", false);
    }
}

/// Randomized end-to-end property test: random source shapes, generation
/// limits, task limits, and a randomly interleaved driver exploring
/// refeed/wakeup orderings the deterministic tests cannot. The oracle is
/// [`assert_synced`] (root equality plus full iteration); `db.check()` is
/// deliberately skipped here — variable-length keys upset the ethhash
/// checker, and root-vs-source equality is the cryptographic oracle.
mod random {
    use super::*;

    #[derive(Debug, Clone, Copy)]
    enum KeyShape {
        Dense32,
        UniformRandom32,
        SharedPrefixClusters,
        VariableLen,
    }

    fn make_kvs(rng: &SeededRng, shape: KeyShape, n: u64) -> Vec<(Vec<u8>, Vec<u8>)> {
        // Shared 16-byte prefixes for `SharedPrefixClusters`.
        let prefixes: Vec<[u8; 16]> = (0..4).map(|_| rng.random()).collect();
        let mut map = std::collections::BTreeMap::new();
        for i in 0..n {
            let key = match shape {
                KeyShape::Dense32 => key32(i),
                KeyShape::UniformRandom32 => rng.random::<[u8; 32]>().to_vec(),
                KeyShape::SharedPrefixClusters => {
                    let mut k = prefixes[rng.random_range(0..prefixes.len())].to_vec();
                    let tail: [u8; 16] = rng.random();
                    k.extend_from_slice(&tail);
                    k
                }
                KeyShape::VariableLen => {
                    let len = rng.random_range(1..=max_key_len());
                    let mut k = vec![0u8; len];
                    rng.fill_bytes(&mut k);
                    k
                }
            };
            // Random keys may collide; the map dedups (last value wins), so
            // the source simply ends up slightly smaller than `n`.
            map.insert(key, val(i));
        }
        map.into_iter().collect()
    }

    fn run_random_case(rng: &SeededRng, case: usize, max_n: u64) {
        let n = match rng.random_range(0..4u32) {
            0 => 0,
            1 => 1,
            2 => 2,
            _ => rng.random_range(3..=max_n),
        };
        let shapes = [
            KeyShape::Dense32,
            KeyShape::UniformRandom32,
            KeyShape::SharedPrefixClusters,
            KeyShape::VariableLen,
        ];
        let shape = shapes[rng.random_range(0..shapes.len())];
        // 1 is the worst-case truncation: every full-size proof continues.
        let gen_limits = [Some(1usize), Some(2), Some(7), Some(33), Some(256), None];
        let gen_limit = gen_limits[rng.random_range(0..gen_limits.len())].map(limit);
        let task_limits = [1usize, 2, 5];
        let task_limit = task_limits[rng.random_range(0..task_limits.len())];
        // The SeededRng banner printed at construction is the repro seed;
        // this diagnostic pins which case within the run failed.
        let diag = format!(
            "case {case}: n={n} shape={shape:?} gen_limit={gen_limit:?} task_limit={task_limit}"
        );

        let kvs = make_kvs(rng, shape, n);
        let src_db = TestDb::new();
        if !kvs.is_empty() {
            let batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> = kvs
                .iter()
                .cloned()
                .map(|(key, value)| BatchOp::Put { key, value })
                .collect();
            src_db.update(batch).expect("populate the source");
        }
        let Some(target) = src_db.root_hash() else {
            // Default-features empty source: there is no root hash to sync
            // toward, so the protocol cannot express this case. (Under
            // ethhash the empty-root constant exists; see below.)
            return;
        };
        let dst = TestDb::new();
        let syncer = start_sync(&dst, target.clone(), limit(task_limit));
        if kvs.is_empty() {
            // ethhash empty source: the destination is already at the
            // empty root, so the warm path reports Done with zero fetches.
            assert!(matches!(syncer.get_work(), Ok(GetWork::Done)), "{diag}");
            assert_eq!(syncer.finish(), Some(target), "{diag}");
            return;
        }
        let src = Source { db: src_db, target };

        // Interleaved driver: at each step randomly either acquire (when
        // below capacity) or submit a randomly chosen held item.
        let mut held: Vec<WorkItem> = Vec::new();
        let mut steps = 0u32;
        loop {
            steps += 1;
            assert!(steps < 10_000, "{diag}: exceeded the step cap");
            let want_acquire =
                held.len() < task_limit && (held.is_empty() || rng.random_range(0..2u32) == 0);
            if want_acquire {
                match syncer.get_work().expect("no invariant violations") {
                    GetWork::Work(item) => {
                        held.push(item);
                        continue;
                    }
                    GetWork::Done => break,
                    GetWork::Wait => {
                        // Nothing to hand out while this driver holds every
                        // outstanding item: fall through and submit one.
                        assert!(!held.is_empty(), "{diag}: Wait with nothing held");
                    }
                }
            }
            let item = held.swap_remove(rng.random_range(0..held.len()));
            let proof = fetch(&src, &item, gen_limit);
            match syncer.submit(item.id, &proof).expect("honest submit") {
                Submit::Continue(next) => held.push(next),
                Submit::Exhausted => {}
                Submit::InvalidProof(e) => panic!("{diag}: honest proof rejected: {e}"),
            }
        }
        // Done can be observed while stragglers are still held (the root
        // already matches); dropping them abandons benign reservations.
        drop(held);
        assert_eq!(syncer.finish(), Some(src.target.clone()), "{diag}");
        assert_synced(&dst, &src);
    }

    #[test]
    fn random_sync_round_trips() {
        let rng = SeededRng::from_env_or_random();
        for case in 0..6 {
            run_random_case(&rng, case, 400);
        }
    }

    #[test]
    fn test_slow_random_sync_round_trips() {
        // n up to 3000 for scale (truncation-heavy sweeps over real tries);
        // honest generation limits stay far below MAX_PROOF_KEYS, so no
        // oversized classification is in play here.
        let rng = SeededRng::from_env_or_random();
        for case in 0..40 {
            run_random_case(&rng, case, 3_000);
        }
    }
}
