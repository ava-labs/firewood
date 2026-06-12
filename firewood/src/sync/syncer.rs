// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! [`Syncer`]: the Db-coupled state-sync driver (see
//! `docs/src/designs/2026-06-11-state-sync.md`, "End-to-end flow").
//!
//! Wraps the pure [`SyncState`] coverage machine with the database half of
//! the protocol: handing out proof-request bounds ([`Syncer::get_work`]),
//! verifying and committing submitted range proofs ([`Syncer::submit`]),
//! and detecting completion (the latest committed root equals the target).
//! The caller remains the transport: it fetches each requested proof from a
//! peer, parses it, and submits the result here.

use std::num::NonZeroUsize;
use std::ops::Range;

use parking_lot::Mutex;

use super::endpoint::Endpoint;
use super::endpoint::proof_cut;
use super::state::NextWork;
use super::state::SyncError;
use super::state::SyncState;
use super::state::WorkId;
use super::state::complete::Completed;
use super::state::complete::ProofExtent;
use crate::api::FrozenRangeProof;
use crate::api::HashKey;
use crate::api::OwnedKeyValuePairs;
use crate::api::{self};
use crate::proofs::ProofError;
use crate::proofs::find_next_key_after_range_proof;
use crate::proofs::range::remaining_subtrees_after_range_proof;
use crate::proofs::verify_range_proof_structure;

/// Driver for one sync run toward a fixed target root.
///
/// Created by [`start_sync`]. All methods take `&self`; share one `Syncer`
/// across worker threads.
#[derive(Debug)]
pub struct Syncer<'db> {
    /// The database being synced toward the target root.
    db: &'db dyn api::DynDb,
    /// Held only for map accesses, never across verifying, merging, or
    /// committing. `db.root_hash()` is the one Db call made under it; the
    /// Db never takes this mutex, so the lock order cannot cycle.
    state: Mutex<SyncState>,
}

/// Begin a sync of `db` toward `target` with at most `task_limit`
/// concurrently outstanding work items.
#[must_use]
pub fn start_sync(db: &dyn api::DynDb, target: HashKey, task_limit: NonZeroUsize) -> Syncer<'_> {
    Syncer {
        db,
        state: Mutex::new(SyncState::new(target, task_limit)),
    }
}

/// A region of the keyspace handed to one sync worker. Forward
/// `first_key` and `last_key` to a peer unchanged.
#[derive(Debug)]
pub struct WorkItem {
    /// Correlates this region with its eventual [`Syncer::submit`] call.
    pub id: WorkId,
    /// Inclusive lower bound; `None` means unbounded. Never `Some` of an
    /// empty key.
    pub first_key: Option<Box<[u8]>>,
    /// Inclusive upper bound; `None` means unbounded.
    pub last_key: Option<Box<[u8]>>,
    /// True iff shareable cold work remains right now: the caller should
    /// wake exactly one parked worker.
    pub wakeup_neighbor: bool,
}

/// Map a stored half-open region to inclusive proof-request bounds. A
/// `Key([])` start and a `Max` end both map to `None`: for `Some([])` the
/// generator emits a root start proof, which the verifier rejects under a
/// `None` bound (`UnexpectedStartProof`).
fn request_key_bounds(range: &Range<Endpoint>) -> (Option<&[u8]>, Option<&[u8]>) {
    let start = match &range.start {
        // No key sorts below the empty key, so a `Key([])` start is unbounded.
        Endpoint::Key(k) if k.is_empty() => None,
        Endpoint::Key(k) => Some(&**k),
        Endpoint::Max => unreachable!("a work range never starts at Max"),
    };
    let end = match &range.end {
        Endpoint::Max => None,
        Endpoint::Key(k) => Some(&**k),
    };
    (start, end)
}

impl WorkItem {
    fn from_range(id: WorkId, range: &Range<Endpoint>, wakeup_neighbor: bool) -> Self {
        let (first_key, last_key) = request_key_bounds(range);
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
#[non_exhaustive]
pub enum GetWork {
    /// A region to fetch and submit.
    Work(WorkItem),
    /// Nothing to hand out now; park until a wakeup, then call again.
    Wait,
    /// The latest committed root equals the target: the sync is complete.
    Done,
}

/// Result of [`Syncer::submit`] (the non-error outcomes).
#[derive(Debug)]
#[non_exhaustive]
pub enum Submit {
    /// Same region, next chunk: the lineage continues under a fresh id.
    Continue(WorkItem),
    /// The region is fully covered; call [`Syncer::get_work`] again.
    Exhausted,
    /// The proof failed verification: the peer's fault. Nothing changed and
    /// the id is still reserved; fetch the same bounds from another peer and
    /// resubmit under the same id.
    InvalidProof(api::Error),
}

impl Syncer<'_> {
    /// Reserve and hand out the next region, or report [`GetWork::Wait`] or
    /// [`GetWork::Done`].
    ///
    /// Never parks and never takes a caller lock, though it may block
    /// briefly on internal locks; the caller's lost-wakeup argument relies
    /// on this (`docs/src/designs/2026-06-11-state-sync.md`, "Lost-wakeup
    /// correctness").
    ///
    /// `Done` means the latest committed root equals the target, so an
    /// already-synced database is `Done` on the first call. A straggler
    /// [`Syncer::submit`] may still be in flight; its commit is idempotent.
    ///
    /// # Errors
    ///
    /// [`SyncError::CoverageRootMismatch`] iff coverage is complete but the
    /// root is not the target. Every committed proof was verified against
    /// the target, so this means either a commit from outside the syncer,
    /// which the caller must not make during a sync, or an internal bug.
    pub fn get_work(&self) -> Result<GetWork, SyncError> {
        let mut state = self.state.lock();
        // `target` is a committed root, so the hash modes' differing roots
        // for an empty database cannot falsely match it.
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
            NextWork::CoverageComplete => Err(SyncError::CoverageRootMismatch {
                target: state.target().clone(),
                actual: root,
            }),
        }
    }

    /// Verify `proof` against the region reserved under `id`, commit it as
    /// a new revision, and advance the coverage map.
    ///
    /// A proof that fails verification returns [`Submit::InvalidProof`].
    ///
    /// The merge stops at the proof's proven extent, not the requested end:
    /// `merge_key_value_range` deletes every in-range key the proof lacks,
    /// so merging a truncated proof to the requested end would delete keys
    /// past the truncation point, including a boundary key a neighbor has
    /// already proven.
    ///
    /// # Errors
    ///
    /// All fatal (see [`SyncError`]):
    ///
    /// - [`SyncError::StaleWorkId`] iff `id` is not outstanding.
    /// - [`SyncError::Api`] for internal database failures.
    /// - [`SyncError::TruncationKeyOutOfRange`] iff the verifier broke its
    ///   contract. The proof has already been committed by then.
    pub fn submit(&self, id: WorkId, proof: &FrozenRangeProof) -> Result<Submit, SyncError> {
        // Look up without retiring: an invalid proof keeps its reservation.
        let (range, hash) = self
            .state
            .lock()
            .work_item(id)
            .ok_or(SyncError::StaleWorkId(id))?;
        let (first_key, last_key) = request_key_bounds(&range);

        let ctx = match verify_range_proof_structure(
            proof,
            hash,
            first_key,
            last_key,
            self.db.node_hash_algorithm(),
            None,
        ) {
            Ok(ctx) => ctx,
            Err(e) if is_peer_fault(&e) => return Ok(Submit::InvalidProof(e)),
            Err(e) => return Err(SyncError::Api(e)),
        };

        // Truncated only when the proven right edge falls short of the
        // requested end. TODO(@rkuris): these `?`s report internal errors,
        // which holds while find-next cannot fail; a find-next that can
        // surface peer-shaped errors must classify them with `is_peer_fault`.
        let extent = match (
            find_next_key_after_range_proof(proof, &ctx)?,
            ctx.right_edge_key(),
        ) {
            (Some(_), Some(edge)) if last_key.is_none_or(|end| edge < end) => {
                let subtrees = remaining_subtrees_after_range_proof(proof, &ctx)?;
                ProofExtent::Truncated {
                    last_key: edge.into(),
                    cut: proof_cut(&subtrees, &range.end),
                }
            }
            _ => ProofExtent::Full,
        };

        let merge_last: Option<&[u8]> = match &extent {
            ProofExtent::Full => last_key,
            ProofExtent::Truncated { last_key, .. } => Some(last_key),
        };
        // No RevisionNotFound retry: it needs `max_revisions` commits to land
        // during one submit, and the caller restarts the sync if it does.
        let key_values: OwnedKeyValuePairs = proof.key_values().to_vec().into_boxed_slice();
        let proposal = self
            .db
            .merge_key_value_range(first_key, merge_last, key_values)?;
        proposal.commit_with_rebase().map_err(SyncError::Api)?;

        // A duplicate that raced past the lookup is caught here as stale;
        // its commit was idempotent.
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
    /// revisions behind (see `docs/src/designs/2026-06-11-state-sync.md`, "Lifecycle").
    #[must_use]
    pub fn finish(self) -> Option<HashKey> {
        self.db.root_hash()
    }
}

/// Whether a verification error is the peer's fault: every `ProofError`
/// except `ProofError::IO`, which is storage-shaped. Applies only to the
/// verify step of [`Syncer::submit`]; later errors are always internal.
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

    use firewood_macros::hash_mode;
    use firewood_storage::EthHash;
    use firewood_storage::HashMode;
    use firewood_storage::MerkleDbHash;
    use firewood_storage::SeededRng;

    use super::*;
    use crate::api::Db as _;
    use crate::api::DbView as _;
    use crate::db::BatchOp;
    use crate::db::Db;
    use crate::db::test::TestDb;

    /// The FFI hands one `&Syncer` to a whole worker pool.
    const fn assert_send_sync<T: Send + Sync>() {}
    const _: () = assert_send_sync::<Syncer<'static>>();

    fn limit(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test limits are nonzero")
    }

    /// Commit `count` random 32-byte-key entries (the ethhash account-key
    /// shape, valid in both hash modes).
    fn populate_random<H: HashMode>(db: &Db<H>, rng: &SeededRng, count: usize) {
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
    fn source_proof<H: HashMode>(
        src: &Db<H>,
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
    fn latest_val<H: HashMode>(db: &Db<H>, key: &[u8]) -> Option<crate::merkle::Value> {
        let root = db.root_hash().expect("database is non-empty");
        let view = db.revision(root).expect("latest revision exists");
        view.val(key).expect("read the key")
    }

    /// Drive a full in-process sync of `dest` toward `src`'s root with a
    /// serial worker loop, asserting convergence.
    fn run_sync<H: HashMode>(
        src: &Db<H>,
        dest: &Db<H>,
        task_limit: usize,
        proof_limit: Option<NonZeroUsize>,
    ) {
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

    #[hash_mode]
    #[test]
    fn sync_loop_converges_task_limit_one<H: HashMode>() {
        let rng = SeededRng::from_env_or_random();
        let src = TestDb::<H>::new();
        populate_random(&src, &rng, 300);
        let dest = TestDb::<H>::new();
        run_sync(&src, &dest, 1, None);
    }

    #[hash_mode]
    #[test]
    fn sync_loop_converges_task_limit_three<H: HashMode>() {
        let rng = SeededRng::from_env_or_random();
        let src = TestDb::<H>::new();
        populate_random(&src, &rng, 300);
        let dest = TestDb::<H>::new();
        run_sync(&src, &dest, 3, None);
    }

    #[hash_mode]
    #[test]
    fn sync_loop_truncation_heavy<H: HashMode>() {
        // A tiny per-proof limit makes every full-size proof truncate,
        // exercising the Continue sweep and the shed/refeed paths.
        let rng = SeededRng::from_env_or_random();
        let src = TestDb::<H>::new();
        populate_random(&src, &rng, 300);
        for task_limit in [1usize, 3] {
            let dest = TestDb::<H>::new();
            run_sync(&src, &dest, task_limit, Some(limit(5)));
        }
    }

    #[hash_mode]
    #[test]
    fn warm_db_get_work_is_done_immediately<H: HashMode>() {
        // A destination already at the target root: the very first
        // get_work reports Done without handing out any work.
        let src = TestDb::<H>::new();
        src.update(fixed_batch(10)).expect("populate source");
        let dest = TestDb::<H>::new();
        dest.update(fixed_batch(10)).expect("populate dest");
        let target = src.root_hash().expect("source database is non-empty");

        let syncer = start_sync(&*dest, target, limit(3));
        assert!(matches!(syncer.get_work(), Ok(GetWork::Done)));
        // Idempotent to re-call.
        assert!(matches!(syncer.get_work(), Ok(GetWork::Done)));
        assert_eq!(syncer.finish(), src.root_hash());
    }

    #[hash_mode]
    #[test]
    fn submit_invalid_proof_keeps_id_and_state<H: HashMode>() {
        let src = TestDb::<H>::new();
        src.update(fixed_batch(20)).expect("populate source");
        // Same keys, one different value: a valid proof against a
        // DIFFERENT root, so verification against the target fails.
        let imposter = TestDb::<H>::new();
        let mut batch = fixed_batch(20);
        batch.push(BatchOp::Put {
            key: vec![1u8; 32],
            value: b"tampered".to_vec(),
        });
        imposter.update(batch).expect("populate imposter");
        let dest = TestDb::<H>::new();
        let target = src.root_hash().expect("source database is non-empty");

        let syncer = start_sync(&*dest, target, limit(1));
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
    fn is_peer_fault_classifies_verify_errors() {
        assert!(is_peer_fault(&api::Error::ProofError(ProofError::Empty)));
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
    }

    #[hash_mode]
    #[test]
    fn get_work_reports_coverage_root_mismatch<H: HashMode>() {
        let src = TestDb::<H>::new();
        src.update(fixed_batch(4)).expect("populate source");
        let dest = TestDb::<H>::new();
        let target = src.root_hash().expect("source database is non-empty");

        let syncer = start_sync(&*dest, target.clone(), limit(1));
        let GetWork::Work(item) = syncer.get_work().expect("get work") else {
            panic!("expected work for a fresh sync");
        };
        let proof = source_proof(&src, &item, None);
        assert!(matches!(
            syncer.submit(item.id, &proof),
            Ok(Submit::Exhausted)
        ));
        // A write outside the syncer breaks the caller obligation: coverage is
        // complete, but the root no longer matches the target.
        dest.update(vec![BatchOp::Put {
            key: vec![0xee; 32],
            value: b"outside".to_vec(),
        }])
        .expect("commit outside the syncer");
        match syncer.get_work() {
            Err(SyncError::CoverageRootMismatch { target: t, actual }) => {
                assert_eq!(t, target);
                assert_eq!(actual, dest.root_hash());
            }
            other => panic!("expected CoverageRootMismatch, got {other:?}"),
        }
    }

    /// A left region's inclusive request end is the right region's first key.
    /// Once the right region has committed that key, a truncated left proof
    /// must not delete it. MerkleDB only: the boundary key is one byte long.
    #[test]
    fn truncated_merge_keeps_neighbor_boundary_key() {
        let boundary = vec![0x80u8];
        let src = TestDb::<MerkleDbHash>::new();
        src.update(
            [0x10u8, 0x20, 0x30, 0x40, 0x80, 0x90]
                .map(|k| BatchOp::Put {
                    key: vec![k],
                    value: vec![k],
                })
                .to_vec(),
        )
        .expect("populate source");
        let dest = TestDb::<MerkleDbHash>::new();
        let target = src.root_hash().expect("source database is non-empty");

        let syncer = start_sync(&*dest, target, limit(2));
        let Ok(GetWork::Work(left)) = syncer.get_work() else {
            panic!("expected the left seed slice");
        };
        let Ok(GetWork::Work(right)) = syncer.get_work() else {
            panic!("expected the right seed slice");
        };
        assert_eq!(left.last_key.as_deref(), Some(boundary.as_slice()));
        assert_eq!(right.first_key.as_deref(), Some(boundary.as_slice()));

        let right_proof = source_proof(&src, &right, None);
        assert!(matches!(
            syncer.submit(right.id, &right_proof),
            Ok(Submit::Exhausted)
        ));
        assert!(latest_val(&dest, &boundary).is_some());

        let truncated = source_proof(&src, &left, Some(limit(2)));
        assert!(matches!(
            syncer.submit(left.id, &truncated),
            Ok(Submit::Continue(_))
        ));
        assert!(
            latest_val(&dest, &boundary).is_some(),
            "a truncated merge must stop at its proven edge, not the request end"
        );
    }

    #[hash_mode]
    #[test]
    fn submit_truncated_sheds_at_the_proof_cut<H: HashMode>() {
        // Keys [i; 32] for i in 1..=10 hang off one branch under nibble 0.
        // A proof truncated at [5; 32] leaves the sibling subtrees 06..0a
        // unfetched, so the cut lands on the middle one, 08.
        let src = TestDb::<H>::new();
        src.update(fixed_batch(10)).expect("populate source");
        let dest = TestDb::<H>::new();
        let target = src.root_hash().expect("source database is non-empty");
        let syncer = start_sync(&*dest, target, limit(2));
        let GetWork::Work(left) = syncer.get_work().expect("get work") else {
            panic!("expected the first seed slice");
        };
        let GetWork::Work(right) = syncer.get_work().expect("get work") else {
            panic!("expected the second seed slice");
        };
        let empty = source_proof(&src, &right, None);
        assert!(matches!(
            syncer.submit(right.id, &empty),
            Ok(Submit::Exhausted)
        ));

        let truncated = source_proof(&src, &left, Some(limit(5)));
        let Submit::Continue(kept) = syncer.submit(left.id, &truncated).expect("submit") else {
            panic!("truncated proof must continue");
        };
        assert!(kept.wakeup_neighbor, "the shed space needs a helper");
        assert_eq!(kept.last_key.as_deref(), Some(&[0x08][..]));
        let GetWork::Work(shed) = syncer.get_work().expect("get work") else {
            panic!("the idle worker takes the shed space");
        };
        assert_eq!(shed.first_key.as_deref(), Some(&[0x08][..]));
        assert_eq!(shed.last_key, left.last_key);

        for item in [kept, shed] {
            let proof = source_proof(&src, &item, None);
            assert!(matches!(
                syncer.submit(item.id, &proof),
                Ok(Submit::Exhausted)
            ));
        }
        assert!(matches!(syncer.get_work(), Ok(GetWork::Done)));
    }

    #[hash_mode]
    #[test]
    fn submit_truncated_clamps_merge_bound<H: HashMode>() {
        let src = TestDb::<H>::new();
        src.update(fixed_batch(10)).expect("populate source");
        // Stale dest keys absent from the source: one inside the slice a
        // truncated proof proves, one beyond the truncation point.
        let stale_low = vec![0x03u8; 31]; // sorts just before [0x03; 32]
        let stale_high = vec![0xeeu8; 32]; // beyond every source key
        let dest = TestDb::<H>::new();
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

        let syncer = start_sync(&*dest, target, limit(1));
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

    #[hash_mode]
    #[test]
    fn submit_duplicate_after_retire_is_stale<H: HashMode>() {
        let src = TestDb::<H>::new();
        src.update(fixed_batch(10)).expect("populate source");
        let dest = TestDb::<H>::new();
        let target = src.root_hash().expect("source database is non-empty");

        let syncer = start_sync(&*dest, target, limit(1));
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
        // rejected in step 1 — before any verify/commit work.
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
