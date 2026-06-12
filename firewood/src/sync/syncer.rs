// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! [`Syncer`]: the Db-coupled state-sync driver (see
//! `docs/plans/state-sync.md`, "End-to-end flow").
//!
//! Wraps the pure [`SyncState`] coverage machine with the database half of
//! the protocol: handing out proof-request bounds ([`Syncer::get_work`]),
//! verifying and committing submitted range proofs ([`Syncer::submit`]),
//! and detecting completion (the latest committed root equals the target).
//! The caller remains the transport: it fetches each requested proof from a
//! peer, parses it, and submits the result here.

use std::num::NonZeroUsize;
use std::ops::Range;

use nonzero_ext::nonzero;
use parking_lot::Mutex;

use crate::api::{self, Db as _, FrozenRangeProof, HashKey};
use crate::db::Db;
use crate::proofs::{ProofError, find_next_key_after_range_proof, verify_range_proof_structure};

use super::endpoint::Endpoint;
use super::state::{
    Completed, NextWork, ProofExtent, SyncError, SyncState, WorkId, request_bounds,
};

/// Maximum number of key/value pairs accepted per submitted proof.
///
/// Sized so the transport's byte budget, not the key count, is the binding
/// limit: network messages are capped at 2 MiB, and the smallest realistic
/// Ethereum pair (a 64-byte storage key plus its RLP value and framing) is
/// roughly 100 bytes, so a maximal response carries at most ~21k pairs.
/// 32768 sits above that, so honest peers always fill the packet and stop
/// on bytes; a proof exceeding this count could not have come from a
/// byte-limited response and is rejected as a peer fault. Fixed for the
/// MVP (a tuning knob per `docs/plans/state-sync.md`, "Deferred for v1").
pub const MAX_PROOF_KEYS: NonZeroUsize = nonzero!(32768usize);

/// Driver for one sync run toward a fixed target root.
///
/// Created by [`start_sync`]. All methods take `&self` and are safe to call
/// from multiple threads: the inner sync state is guarded by a mutex that is
/// held only for brief map accesses — never across proof verification,
/// merging, or committing.
#[derive(Debug)]
pub struct Syncer<'db> {
    /// The database being synced toward the target root.
    db: &'db Db,
    /// The pure coverage/work-handout machine. Lock discipline: held only
    /// inside [`Syncer::get_work`] and for phases 1 and 3 of
    /// [`Syncer::submit`]; `db.root_hash()` is the one permitted Db call
    /// under this mutex (sync-mutex -> revisions-read-lock only; the Db
    /// side never takes the sync mutex, so no cycle).
    state: Mutex<SyncState>,
}

/// Begin a sync of `db` toward `target` with at most `task_limit`
/// concurrently outstanding work items.
#[must_use]
pub fn start_sync(db: &Db, target: HashKey, task_limit: NonZeroUsize) -> Syncer<'_> {
    Syncer {
        db,
        state: Mutex::new(SyncState::new(target, task_limit)),
    }
}

/// A region of the keyspace handed to one sync worker.
///
/// `first_key`/`last_key` are the EXACT inclusive proof-request bounds to
/// forward to a peer (already endpoint-mapped: `None` means unbounded on
/// that side). `None` must be forwarded as the protocol's "no bound", never
/// as a present empty key — the proof generator emits a different (and
/// unverifiable) shape for an explicit empty start key.
///
/// Requests must additionally be capped at [`MAX_PROOF_KEYS`] keys: that
/// limit is enforced at [`Syncer::submit`] (an oversized response is
/// classified as a peer fault), so the transport must ask peers for at most
/// that many keys per request.
#[derive(Debug)]
pub struct WorkItem {
    /// Correlates this region with its eventual [`Syncer::submit`] call.
    pub id: WorkId,
    /// Inclusive lower proof-request bound; `None` means no lower bound.
    /// Never `Some` of an empty key.
    pub first_key: Option<Box<[u8]>>,
    /// Inclusive upper proof-request bound; `None` means unbounded.
    pub last_key: Option<Box<[u8]>>,
    /// True iff shareable cold work remains right now: the caller should
    /// wake exactly one parked worker.
    pub wakeup_neighbor: bool,
}

impl WorkItem {
    /// Build the caller-facing item for a handed-out region.
    fn from_range(id: WorkId, range: &Range<Endpoint>, wakeup_neighbor: bool) -> Self {
        let (first_key, last_key) = request_bounds(range);
        Self {
            id,
            first_key: first_key.map(Box::from),
            last_key: last_key.map(Box::from),
            wakeup_neighbor,
        }
    }
}

/// Result of [`Syncer::get_work`].
#[derive(Debug)]
pub enum GetWork {
    /// A region to fetch and submit.
    Work(WorkItem),
    /// Nothing to hand out right now (capacity is full or every coverage
    /// gap is already in flight); park until a wakeup and re-call.
    Wait,
    /// The latest committed root equals the target: the sync is complete.
    Done,
}

/// Result of [`Syncer::submit`] (the non-error outcomes).
#[derive(Debug)]
pub enum Submit {
    /// Same region, next chunk: the lineage continues under a fresh id.
    Continue(WorkItem),
    /// The region is fully covered; call [`Syncer::get_work`] again.
    Exhausted,
    /// Peer-fault rejection: the proof failed verification against the
    /// target root. The region stays reserved under the SAME id with zero
    /// state mutation — re-fetch the same bounds from another peer and
    /// resubmit under the same id.
    InvalidProof(api::Error),
}

impl Syncer<'_> {
    /// Hand out the next region to work, report that everything is in
    /// flight, or report completion.
    ///
    /// The result is a pure function of the durable sync state (plus the
    /// latest committed root); this method never parks waiting for new
    /// work and is idempotent to re-call — though it may briefly block on
    /// internal locks (reading the latest root waits out any in-flight
    /// commit's rebase window). The caller's lost-wakeup argument depends
    /// on this contract (see `docs/plans/state-sync.md`, "Lost-wakeup
    /// correctness").
    ///
    /// Completion is the root check, not coverage: [`GetWork::Done`] is
    /// returned exactly when the latest committed root equals the target —
    /// including on the very first call against an already-synced database
    /// (the warm fast path). `Done` can be observed while a straggler
    /// [`Syncer::submit`] is still in flight elsewhere; that window is
    /// benign — the straggler's commit is idempotent and its map update
    /// still lands.
    ///
    /// # Errors
    ///
    /// [`SyncError::CoverageRootMismatch`] iff coverage tiles the whole
    /// keyspace with nothing in flight, yet the committed root differs
    /// from the target. With a fixed target and the database receiving no
    /// non-syncer commits during the sync (a caller obligation), this is
    /// unreachable (every committed proof was verified against the
    /// target), so it indicates an internal invariant violation; callers
    /// should abandon and restart the sync. Debug builds additionally
    /// assert as an intentional misuse tripwire.
    pub fn get_work(&self) -> Result<GetWork, SyncError> {
        let mut state = self.state.lock();
        // Completion + warm fast path. `db.root_hash()` is permitted under
        // the sync mutex (see the `state` field docs). Both sides of the
        // comparison use the `or_default_root_hash` convention, so the
        // empty-db divergence between default and ethhash builds cannot
        // skew this check.
        let root = self.db.root_hash();
        if root.as_ref() == Some(state.target()) {
            return Ok(GetWork::Done);
        }
        match state.next_work() {
            NextWork::Work {
                id,
                range,
                wakeup_neighbor,
            } => Ok(GetWork::Work(WorkItem::from_range(
                id,
                &range,
                wakeup_neighbor,
            ))),
            NextWork::Wait => Ok(GetWork::Wait),
            NextWork::CoverageComplete => {
                // The root was checked above and did not match: invariant
                // broken.
                let target = state.target().clone();
                debug_assert!(
                    false,
                    "coverage complete but root {root:?} != target {target}"
                );
                Err(SyncError::CoverageRootMismatch {
                    target,
                    actual: root,
                })
            }
        }
    }

    /// Verify `proof` against the region reserved under `id`, commit it as
    /// a new revision, and advance the coverage map.
    ///
    /// Three phases; the sync mutex is held only for the brief map accesses
    /// in phases 1 and 3, never across verification, merging, or
    /// committing:
    ///
    /// 1. **Lookup** the reserved region and its target hash. The
    ///    reservation is NOT retired here: an invalid proof must leave the
    ///    region reserved under the same id, and a duplicate submission
    ///    that races past this lookup is caught in phase 3.
    /// 2. **Verify, merge, commit.** Verification failures are classified
    ///    by `is_peer_fault`. The merge upper bound is CLAMPED to the
    ///    proven extent: `merge_key_value_range` deletes every in-range key
    ///    absent from the proof (restart idempotency depends on this), so
    ///    a truncated proof merges only `first_key..=last_covered_key` —
    ///    merging to the requested bound would delete target-state keys
    ///    beyond the truncation point (including a boundary key a neighbor
    ///    may already have proven) only to re-add them later.
    /// 3. **Complete**: apply the proof's extent to the coverage map and
    ///    hand back the continuation, if any.
    ///
    /// A verification failure is the peer's fault and is reported as
    /// [`Submit::InvalidProof`] — an `Ok` value with zero state mutation:
    /// the region stays reserved under the same `id` so the same bounds can
    /// be re-fetched from another peer and resubmitted.
    ///
    /// # Errors
    ///
    /// - [`SyncError::StaleWorkId`] iff `id` is unknown or already retired
    ///   (e.g. a duplicate submission after a timeout re-fetch); harmless
    ///   to drop — the earlier submission already covered the region.
    /// - [`SyncError::Api`] for internal database failures (never the
    ///   peer's fault). The region stays reserved under the same `id`;
    ///   the caller chooses whether and how to retry (a plain resubmit
    ///   recovers transient failures such as `RevisionNotFound`).
    /// - [`SyncError::TruncationKeyOutOfRange`] iff the find-next-key
    ///   layer broke its contract (internal; the region stays reserved).
    pub fn submit(&self, id: WorkId, proof: &FrozenRangeProof) -> Result<Submit, SyncError> {
        // ── Phase 1: locked lookup (no retirement). ──
        let (range, hash) = self
            .state
            .lock()
            .work_item(id)
            .ok_or(SyncError::StaleWorkId(id))?;
        let (first_key, last_key) = request_bounds(&range);

        // ── Phase 2: Db section — the sync mutex is NOT held. ──
        // 2a. Verify (pure). Failures here are classified per
        //     `is_peer_fault`; every error the pure verifier can emit is a
        //     `ProofError`, i.e. a peer fault.
        let ctx = match verify_range_proof_structure(
            proof,
            hash,
            first_key,
            last_key,
            Some(MAX_PROOF_KEYS),
        ) {
            Ok(ctx) => ctx,
            Err(e) if is_peer_fault(&e) => return Ok(Submit::InvalidProof(e)),
            Err(e) => return Err(SyncError::Api(e)),
        };

        // 2b. Resume point (pure; computed before the commit so phase 3 is
        //     pure map surgery). The returned key is the LAST key the proof
        //     covered, which `SyncState::complete` records inclusively.
        let extent = match find_next_key_after_range_proof(proof, &ctx)? {
            None => ProofExtent::Full,
            Some((last_covered, _)) => ProofExtent::Truncated {
                last_key: last_covered,
            },
        };

        // 2c. Apply + commit, with the merge upper bound CLAMPED to the
        //     proven extent (see the method docs):
        //       Full         -> first_key..=last_key  [request bounds]
        //       Truncated{L} -> first_key..=L         [proven slice only]
        let merge_last: Option<&[u8]> = match &extent {
            ProofExtent::Full => last_key,
            ProofExtent::Truncated { last_key } => Some(last_key),
        };
        // No RevisionNotFound retry loop here, deliberately. That failure
        // needs the proposal's parent revision to be reaped during this
        // verify→commit window, i.e. `max_revisions` whole commits landing
        // while one worker is mid-submit. It is only reachable with an
        // artificially tight revision window and more committers than cores
        // (the shape db/tests/concurrent_rebase.rs constructs on purpose);
        // production minimums (64–128 revisions, ≤8–16 committers scaled to
        // CPUs) are orders of magnitude away. If it ever fires, the region
        // stays reserved under the same id and a plain resubmit recovers.
        let proposal = self
            .db
            .merge_key_value_range(first_key, merge_last, proof)?;
        proposal.commit_with_rebase().map_err(SyncError::Api)?;

        // ── Phase 3: locked map update (short). `complete` re-checks the
        //    id: a duplicate submission that raced us past phase 1 finds
        //    the id retired and gets StaleWorkId (harmless — its commit
        //    was idempotent). ──
        let completed = self.state.lock().complete(id, extent)?;
        match completed {
            Completed::Exhausted => Ok(Submit::Exhausted),
            Completed::Continue {
                id,
                range,
                wakeup_neighbor,
            } => Ok(Submit::Continue(WorkItem::from_range(
                id,
                &range,
                wakeup_neighbor,
            ))),
        }
    }

    /// Free the sync state and return the latest committed root for the
    /// caller to confirm against the target.
    ///
    /// Meaningful once [`Syncer::get_work`] returned [`GetWork::Done`];
    /// callable anytime — an abandoned sync leaves only ordinary committed
    /// revisions behind (see `docs/plans/state-sync.md`, "Lifecycle").
    #[must_use]
    pub fn finish(self) -> Option<HashKey> {
        self.db.root_hash()
    }
}

/// Peer-fault classification for errors out of the VERIFY phase (2a of
/// [`Syncer::submit`]) only.
///
/// Classification is positional: phases 2b–2c (resume-point computation,
/// merge, commit) never produce peer faults, so their errors are returned
/// as [`SyncError::Api`] without consulting this table.
///
/// | `api::Error` variant | classified as |
/// |---|---|
/// | `ProofError(_)` except `ProofError::IO` | peer fault → [`Submit::InvalidProof`] |
/// | `ProofError(ProofError::IO(_))` | internal (storage-shaped; cannot arise from the pure verifier — classified defensively) |
/// | everything else (`FileIO`, `IO`, `RevisionNotFound`, …) | internal → [`SyncError::Api`] |
const fn is_peer_fault(error: &api::Error) -> bool {
    match error {
        api::Error::ProofError(proof_error) => !matches!(proof_error, ProofError::IO(_)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::arithmetic_side_effects,
        reason = "tests may use plain arithmetic on small constants"
    )]

    use std::collections::VecDeque;
    use std::ops::Deref;

    use firewood_storage::SeededRng;

    use super::*;
    use crate::api::DbView as _;
    use crate::db::{BatchOp, Db, DbConfig};

    /// `Syncer` must remain shareable across worker threads (the FFI step
    /// hands one `&Syncer` to a whole pool).
    const fn assert_send_sync<T: Send + Sync>() {}
    const _: () = assert_send_sync::<Syncer<'static>>();

    /// A temp-dir-backed database that is closed (persisted) on drop.
    struct TestDb {
        db: Option<Db>,
        _tmpdir: tempfile::TempDir,
    }

    impl TestDb {
        fn new() -> Self {
            let tmpdir = tempfile::tempdir().expect("create a temporary directory");
            let db =
                Db::new(tmpdir.path(), DbConfig::builder().build()).expect("create a database");
            Self {
                db: Some(db),
                _tmpdir: tmpdir,
            }
        }
    }

    impl Deref for TestDb {
        type Target = Db;
        fn deref(&self) -> &Self::Target {
            self.db.as_ref().expect("database is open until drop")
        }
    }

    impl Drop for TestDb {
        fn drop(&mut self) {
            if let Some(db) = self.db.take() {
                db.close().expect("close the database");
            }
        }
    }

    fn limit(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test limits are nonzero")
    }

    /// Commit `count` random 32-byte-key entries (the ethhash account-key
    /// shape, valid on both feature axes).
    fn populate_random(db: &Db, rng: &SeededRng, count: usize) {
        let batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> = (0..count)
            .map(|_| {
                let key: [u8; 32] = rng.random();
                let value: [u8; 8] = rng.random();
                BatchOp::Put {
                    key: key.to_vec(),
                    value: value.to_vec(),
                }
            })
            .collect();
        db.update(batch).expect("populate the database");
    }

    /// Deterministic source content: keys `[i; 32]` for `i in 1..=n`.
    fn fixed_batch(n: u8) -> Vec<BatchOp<Vec<u8>, Vec<u8>>> {
        (1..=n)
            .map(|i| BatchOp::Put {
                key: vec![i; 32],
                value: vec![i; 8],
            })
            .collect()
    }

    /// Generate an honest range proof from `src` for the item's request
    /// bounds, optionally truncated to `proof_limit` pairs.
    fn source_proof(
        src: &Db,
        item: &WorkItem,
        proof_limit: Option<NonZeroUsize>,
    ) -> FrozenRangeProof {
        let root = src.root_hash().expect("source database is non-empty");
        let view = src.revision(root).expect("source root revision exists");
        view.range_proof(
            item.first_key.as_deref(),
            item.last_key.as_deref(),
            proof_limit,
        )
        .expect("source can prove any requested range")
    }

    /// Read a key from the latest committed revision of `db`.
    fn latest_val(db: &Db, key: &[u8]) -> Option<crate::merkle::Value> {
        let root = db.root_hash().expect("database is non-empty");
        let view = db.revision(root).expect("latest revision exists");
        view.val(key).expect("read the key")
    }

    /// Drive a full in-process sync of `dest` toward `src`'s root with a
    /// serial worker loop, asserting convergence.
    fn run_sync(src: &Db, dest: &Db, task_limit: usize, proof_limit: Option<NonZeroUsize>) {
        let target = src.root_hash().expect("source database is non-empty");
        let syncer = start_sync(dest, target, limit(task_limit));
        let mut queue: VecDeque<WorkItem> = VecDeque::new();
        let mut steps = 0usize;
        loop {
            steps += 1;
            assert!(steps < 100_000, "sync failed to converge");
            match syncer.get_work().expect("no invariant violations") {
                GetWork::Work(item) => {
                    assert!(
                        queue.len() < task_limit,
                        "more work outstanding than task_limit"
                    );
                    queue.push_back(item);
                    continue;
                }
                GetWork::Done => break,
                GetWork::Wait => {}
            }
            let item = queue
                .pop_front()
                .expect("Wait implies queued work in a serial driver");
            let proof = source_proof(src, &item, proof_limit);
            match syncer.submit(item.id, &proof).expect("honest submit") {
                Submit::Continue(next) => queue.push_back(next),
                Submit::Exhausted => {}
                Submit::InvalidProof(e) => panic!("honest source proof rejected: {e}"),
            }
        }
        assert_eq!(
            syncer.finish(),
            src.root_hash(),
            "dest root must equal source root after sync"
        );
    }

    #[test]
    fn sync_loop_converges_task_limit_one() {
        let rng = SeededRng::from_env_or_random();
        let src = TestDb::new();
        populate_random(&src, &rng, 300);
        let dest = TestDb::new();
        run_sync(&src, &dest, 1, None);
    }

    #[test]
    fn sync_loop_converges_task_limit_three() {
        let rng = SeededRng::from_env_or_random();
        let src = TestDb::new();
        populate_random(&src, &rng, 300);
        let dest = TestDb::new();
        run_sync(&src, &dest, 3, None);
    }

    #[test]
    fn sync_loop_truncation_heavy() {
        // A tiny per-proof limit makes every full-size proof truncate,
        // exercising the Continue sweep and the shed/refeed paths.
        let rng = SeededRng::from_env_or_random();
        let src = TestDb::new();
        populate_random(&src, &rng, 300);
        for task_limit in [1usize, 3] {
            let dest = TestDb::new();
            run_sync(&src, &dest, task_limit, Some(limit(5)));
        }
    }

    #[test]
    fn warm_db_get_work_is_done_immediately() {
        // A destination already at the target root: the very first
        // get_work reports Done without handing out any work.
        let src = TestDb::new();
        src.update(fixed_batch(10)).expect("populate source");
        let dest = TestDb::new();
        dest.update(fixed_batch(10)).expect("populate dest");
        let target = src.root_hash().expect("source database is non-empty");

        let syncer = start_sync(&dest, target, limit(3));
        assert!(matches!(syncer.get_work(), Ok(GetWork::Done)));
        // Idempotent to re-call.
        assert!(matches!(syncer.get_work(), Ok(GetWork::Done)));
        assert_eq!(syncer.finish(), src.root_hash());
    }

    #[test]
    fn submit_invalid_proof_keeps_id_and_state() {
        let src = TestDb::new();
        src.update(fixed_batch(20)).expect("populate source");
        // Same keys, one different value: a valid proof against a
        // DIFFERENT root, so verification against the target fails.
        let imposter = TestDb::new();
        let mut batch = fixed_batch(20);
        batch.push(BatchOp::Put {
            key: vec![1u8; 32],
            value: b"tampered".to_vec(),
        });
        imposter.update(batch).expect("populate imposter");
        let dest = TestDb::new();
        let target = src.root_hash().expect("source database is non-empty");

        let syncer = start_sync(&dest, target, limit(1));
        let GetWork::Work(item) = syncer.get_work().expect("get work") else {
            panic!("expected work for a fresh sync");
        };
        let root_before = dest.root_hash();

        let bad = source_proof(&imposter, &item, None);
        match syncer.submit(item.id, &bad) {
            Ok(Submit::InvalidProof(api::Error::ProofError(_))) => {}
            other => panic!("expected InvalidProof(ProofError), got {other:?}"),
        }
        // Zero state mutation: nothing was committed...
        assert_eq!(
            dest.root_hash(),
            root_before,
            "invalid proof must not commit"
        );
        // ...and the region stays reserved under the same id (task_limit
        // is 1, so a free slot would hand out work instead of Wait).
        assert!(matches!(syncer.get_work(), Ok(GetWork::Wait)));

        // A same-id retry with an honest proof succeeds.
        let good = source_proof(&src, &item, None);
        assert!(matches!(
            syncer.submit(item.id, &good),
            Ok(Submit::Exhausted)
        ));
        assert!(matches!(syncer.get_work(), Ok(GetWork::Done)));
        assert_eq!(syncer.finish(), src.root_hash());
    }

    #[test]
    fn submit_classifies_internal_vs_peer() {
        // The classifier itself (positional: only verify-phase errors are
        // ever candidates — see `is_peer_fault`).
        assert!(is_peer_fault(&api::Error::ProofError(ProofError::Empty)));
        assert!(is_peer_fault(&api::Error::ProofError(
            ProofError::ProofIsLargerThanMaxLength
        )));
        assert!(is_peer_fault(&api::Error::ProofError(
            ProofError::NonMonotonicIncreaseRange
        )));
        // ProofError::IO is storage-shaped: internal, defensively.
        let io = firewood_storage::FileIoError::from_generic_no_file(
            std::io::Error::other("boom"),
            "classification test",
        );
        assert!(!is_peer_fault(&api::Error::ProofError(ProofError::IO(io))));
        // Non-proof errors are always internal.
        assert!(!is_peer_fault(&api::Error::RevisionNotFound {
            provided: None
        }));
        assert!(!is_peer_fault(&api::Error::SendErrorToWorker));

        // End-to-end: an oversized (but otherwise honest) proof is a peer
        // fault — Ok(InvalidProof), id retained, nothing committed.
        let rng = SeededRng::from_env_or_random();
        let src = TestDb::new();
        populate_random(&src, &rng, MAX_PROOF_KEYS.get() + 8);
        let dest = TestDb::new();
        let target = src.root_hash().expect("source database is non-empty");

        let syncer = start_sync(&dest, target, limit(1));
        let GetWork::Work(item) = syncer.get_work().expect("get work") else {
            panic!("expected work for a fresh sync");
        };
        let oversized = source_proof(&src, &item, None);
        assert!(oversized.key_values().len() > MAX_PROOF_KEYS.get());
        match syncer.submit(item.id, &oversized) {
            Ok(Submit::InvalidProof(api::Error::ProofError(
                ProofError::ProofIsLargerThanMaxLength,
            ))) => {}
            other => panic!("expected ProofIsLargerThanMaxLength, got {other:?}"),
        }
        assert!(
            dest.root_hash() != src.root_hash(),
            "oversized proof must not commit"
        );
        // Same id still live: a right-sized proof for the same bounds lands.
        let good = source_proof(&src, &item, Some(MAX_PROOF_KEYS));
        assert!(matches!(
            syncer.submit(item.id, &good),
            Ok(Submit::Continue(_) | Submit::Exhausted)
        ));
    }

    #[test]
    fn submit_truncated_clamps_merge_bound() {
        let src = TestDb::new();
        src.update(fixed_batch(10)).expect("populate source");
        // Stale dest keys absent from the source: one inside the slice a
        // truncated proof proves, one beyond the truncation point.
        let stale_low = vec![0x03u8; 31]; // sorts just before [0x03; 32]
        let stale_high = vec![0xeeu8; 32]; // beyond every source key
        let dest = TestDb::new();
        dest.update(vec![
            BatchOp::Put {
                key: stale_low.clone(),
                value: b"stale".to_vec(),
            },
            BatchOp::Put {
                key: stale_high.clone(),
                value: b"stale".to_vec(),
            },
        ])
        .expect("pre-populate dest with stale keys");
        let target = src.root_hash().expect("source database is non-empty");

        let syncer = start_sync(&dest, target, limit(1));
        let GetWork::Work(item) = syncer.get_work().expect("get work") else {
            panic!("expected work for a fresh sync");
        };
        assert!(item.first_key.is_none(), "single seed slice is unbounded");
        assert!(item.last_key.is_none(), "single seed slice is unbounded");

        // Truncate after 5 pairs: the proof proves only up to L = [5; 32].
        let truncated = source_proof(&src, &item, Some(limit(5)));
        let Submit::Continue(next) = syncer.submit(item.id, &truncated).expect("submit") else {
            panic!("truncated proof must continue");
        };
        // The continuation resumes at succ(L) = L ++ 0x00.
        let mut expected_resume = vec![5u8; 32];
        expected_resume.push(0);
        assert_eq!(next.first_key.as_deref(), Some(expected_resume.as_slice()));

        // The merge bound was clamped to L: the stale key inside the
        // proven slice is gone, the stale key beyond L SURVIVES until the
        // continuation lands.
        assert!(
            latest_val(&dest, &stale_low).is_none(),
            "stale key within the proven slice must be deleted"
        );
        assert!(
            latest_val(&dest, &stale_high).is_some(),
            "stale key beyond the truncation point must survive the clamped merge"
        );

        // The continuation (full) merges to the unbounded request end and
        // finally deletes the far stale key.
        let rest = source_proof(&src, &next, None);
        assert!(matches!(
            syncer.submit(next.id, &rest),
            Ok(Submit::Exhausted)
        ));
        assert!(
            latest_val(&dest, &stale_high).is_none(),
            "stale key must be deleted once the continuation lands"
        );
        assert!(matches!(syncer.get_work(), Ok(GetWork::Done)));
        assert_eq!(syncer.finish(), src.root_hash());
    }

    #[test]
    fn submit_duplicate_after_retire_is_stale() {
        let src = TestDb::new();
        src.update(fixed_batch(10)).expect("populate source");
        let dest = TestDb::new();
        let target = src.root_hash().expect("source database is non-empty");

        let syncer = start_sync(&dest, target, limit(1));
        let GetWork::Work(item) = syncer.get_work().expect("get work") else {
            panic!("expected work for a fresh sync");
        };
        let proof = source_proof(&src, &item, None);
        assert!(matches!(
            syncer.submit(item.id, &proof),
            Ok(Submit::Exhausted)
        ));
        let root_after = dest.root_hash();

        // A duplicate submission (e.g. after a timeout re-fetch) is
        // rejected in phase 1 — before any verify/commit work.
        match syncer.submit(item.id, &proof) {
            Err(SyncError::StaleWorkId(stale)) => assert_eq!(stale, item.id),
            other => panic!("expected StaleWorkId, got {other:?}"),
        }
        assert_eq!(
            dest.root_hash(),
            root_after,
            "duplicate submission must not commit"
        );
        assert!(matches!(syncer.get_work(), Ok(GetWork::Done)));
    }
}
