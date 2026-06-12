// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::num::NonZeroUsize;

use firewood::ProofError;
use firewood::api::{self, FrozenRangeProof};
use firewood::sync::{GetWork, Submit, SyncError, Syncer, WorkId};

use crate::{
    BorrowedBytes, DatabaseHandle, GetWorkResult, HashKey, HashResult, Maybe, OwnedBytes,
    SubmitResult, SyncStartResult, VoidResult,
};

/// Maximum key/value pairs per submitted proof; proofs exceeding this are
/// rejected as invalid. Go callers should pass this as the key limit in
/// peer requests. Sized so the transport's byte budget, not the key count,
/// is the binding limit (see `firewood::sync::MAX_PROOF_KEYS`).
pub const FWD_SYNC_MAX_PROOF_KEYS: u32 = 32768;
const _: () = assert!(FWD_SYNC_MAX_PROOF_KEYS as usize == firewood::sync::MAX_PROOF_KEYS.get());

/// An opaque handle to an in-progress state sync toward a fixed target root.
///
/// Created by [`fwd_start_sync`]; freed by [`fwd_finish_sync`] or
/// [`fwd_free_sync`]. Borrows the database handle: the database must not be
/// closed while this handle is live (the Go wrapper enforces this with a
/// keep-alive handle).
#[derive(Debug)]
pub struct SyncHandle<'db> {
    syncer: Syncer<'db>,
    handle: &'db DatabaseHandle,
}

impl SyncHandle<'_> {
    fn get_work(&self) -> Result<GetWork, SyncError> {
        self.syncer.get_work()
    }

    fn submit(&self, id: WorkId, proof: &FrozenRangeProof) -> Result<Submit, SyncError> {
        self.syncer.submit(id, proof)
    }

    fn finish(self) -> Option<api::HashKey> {
        self.syncer.finish()
    }
}

impl crate::MetricsContextExt for SyncHandle<'_> {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        self.handle.metrics_context()
    }
}

/// A region of the keyspace handed to one sync worker. The bounds are the
/// EXACT inclusive proof-request bounds to forward to a peer (unlike
/// `NextKeyRange`, whose end is exclusive — do not copy decode logic between
/// the two).
#[derive(Debug)]
#[repr(C)]
pub struct SyncWorkItem {
    /// Correlates this region with its eventual [`fwd_submit_work`] call.
    pub id: u64,
    /// Inclusive lower bound of the proof request. EMPTY means "no lower
    /// bound" (request from the start of the keyspace). This encoding is
    /// lossless: a work range never has an empty-but-present lower bound.
    ///
    /// The caller MUST translate empty to the protocol's "no start key"
    /// (`Nothing`), never to a present empty key: the proof generator emits
    /// a different (and unverifiable) shape for an explicit empty start key.
    ///
    /// The caller must free with [`fwd_free_owned_bytes`].
    ///
    /// [`fwd_free_owned_bytes`]: crate::fwd_free_owned_bytes
    pub start_key: OwnedBytes,
    /// Inclusive upper bound of the proof request; `None` means unbounded.
    /// If present, the caller must free with [`fwd_free_owned_bytes`].
    ///
    /// [`fwd_free_owned_bytes`]: crate::fwd_free_owned_bytes
    pub end_key: Maybe<OwnedBytes>,
    /// True iff shareable cold work remains right now: the caller should
    /// wake exactly one parked worker (`Signal`, not `Broadcast`).
    pub wakeup_neighbor: bool,
}

impl From<firewood::sync::WorkItem> for SyncWorkItem {
    fn from(item: firewood::sync::WorkItem) -> Self {
        Self {
            id: item.id.as_u64(),
            // None ⇔ empty is bijective here: `request_bounds()` maps the
            // `Key([])` range start to `None` and never emits `Some(empty)`.
            start_key: item.first_key.unwrap_or_default().into(),
            end_key: item.last_key.map(Into::into).into(),
            wakeup_neighbor: item.wakeup_neighbor,
        }
    }
}

/// Validate the caller-provided task limit.
fn checked_task_limit(task_limit: u32) -> Result<NonZeroUsize, api::Error> {
    NonZeroUsize::new(task_limit as usize)
        .ok_or_else(|| crate::handle::invalid_data("task_limit must be nonzero"))
}

/// Begin a state sync of the database toward `target` with at most
/// `task_limit` concurrently outstanding work items.
///
/// # Returns
///
/// - [`SyncStartResult::NullHandlePointer`] if `db` is null.
/// - [`SyncStartResult::Ok`] with the sync handle.
/// - [`SyncStartResult::Err`] if `task_limit == 0`.
///
/// # Safety
///
/// The caller must:
/// * ensure that `db` is a valid pointer to a [`DatabaseHandle`].
/// * free the returned handle with [`fwd_finish_sync`] or [`fwd_free_sync`]
///   before closing the database with [`fwd_close_db`].
/// * call [`fwd_free_owned_bytes`] to free the memory associated with a
///   returned error.
///
/// [`fwd_close_db`]: crate::fwd_close_db
/// [`fwd_free_owned_bytes`]: crate::fwd_free_owned_bytes
#[unsafe(no_mangle)]
pub extern "C" fn fwd_start_sync(
    db: Option<&DatabaseHandle>,
    target: HashKey,
    task_limit: u32,
) -> SyncStartResult<'_> {
    crate::invoke_with_handle(db, move |db| {
        let task_limit = checked_task_limit(task_limit)?;
        Ok::<_, api::Error>(SyncHandle {
            syncer: db.start_sync(target.into(), task_limit),
            handle: db,
        })
    })
}

/// Hand the calling worker a new region to sync, or report `Wait`/`Done`.
///
/// # Contract (load-bearing — the Go condvar pool depends on this verbatim)
///
/// The `Work`/`Wait`/`Done` answer is a pure function of the durable
/// `SyncState`: this call never parks (it may briefly block on internal
/// locks — reading the latest root waits out an in-flight commit's rebase
/// window), performs no I/O or network access, and is safe and
/// idempotent to re-call — re-calling after `Wait` or `Done` without an
/// intervening [`fwd_submit_work`] is always legal and returns a consistent
/// answer. Callers therefore may (and the Go pool MUST) evaluate it while
/// holding their own park/wake mutex, in a re-check loop around the wait.
///
/// # Locking (Go-side discipline)
///
/// - `fwd_get_work` IS called while holding the Go pool mutex `mu`.
/// - [`fwd_submit_work`] must NOT be called under `mu`.
///
/// # Returns
///
/// - [`GetWorkResult::NullHandlePointer`] if `sync` is null.
/// - [`GetWorkResult::Work`] with a new region to fetch and submit.
/// - [`GetWorkResult::Wait`] if nothing can be handed out right now.
/// - [`GetWorkResult::Done`] if the latest committed root equals the target.
/// - [`GetWorkResult::CoverageRootMismatch`] on the internal invariant
///   violation (restart the whole sync).
/// - [`GetWorkResult::Err`] on any other internal error.
///
/// # Thread Safety
///
/// Safe to call concurrently with [`fwd_submit_work`] and other
/// `fwd_get_work` calls on the same handle (internal mutex). NOT safe to
/// call concurrently with [`fwd_finish_sync`]/[`fwd_free_sync`] on the same
/// handle.
///
/// # Safety
///
/// The caller must:
/// * ensure that `sync` is a valid pointer to a [`SyncHandle`].
/// * free the key members of a returned [`SyncWorkItem`] with
///   [`fwd_free_owned_bytes`], and likewise any returned error.
///
/// [`fwd_free_owned_bytes`]: crate::fwd_free_owned_bytes
#[unsafe(no_mangle)]
pub extern "C" fn fwd_get_work(sync: Option<&SyncHandle<'_>>) -> GetWorkResult {
    crate::invoke_with_handle(sync, SyncHandle::get_work)
}

/// Submit serialized range-proof bytes for the region identified by `id`.
///
/// Verifies the proof against the region's bounds and target hash, commits
/// the verified key-values as a new revision (`commit_with_rebase`), and
/// updates the coverage map. See [`SubmitResult`] for the outcomes;
/// [`SubmitResult::InvalidProof`] (peer fault: keep the same `id`, re-fetch
/// from another peer) is distinct from [`SubmitResult::Err`] (internal
/// fault: the region also stays reserved under the same `id`, and a plain
/// resubmit recovers transient failures).
///
/// A stale or unknown `id` (e.g. a duplicate submission) returns
/// [`SubmitResult::Err`]; the earlier submission's commit was idempotent so
/// this is harmless. One corner: unparseable proof bytes are classified as
/// [`SubmitResult::InvalidProof`] before the `id` is consulted, so a stale
/// `id` paired with garbage bytes reports `InvalidProof` even though
/// nothing is reserved under that `id` anymore.
///
/// # Locking (Go-side discipline)
///
/// Must NOT be called while holding the Go pool mutex `mu`: this call does
/// real verification + commit work and would serialize the pool; the
/// lost-wakeup proof only requires [`fwd_get_work`] under `mu`.
///
/// # Returns
///
/// - [`SubmitResult::NullHandlePointer`] if `sync` is null.
/// - [`SubmitResult::Continue`] with the same region's next chunk.
/// - [`SubmitResult::Exhausted`] if the region is fully covered.
/// - [`SubmitResult::InvalidProof`] if the proof was rejected (peer fault).
/// - [`SubmitResult::Err`] on an internal error.
///
/// # Thread Safety
///
/// Safe to call concurrently with [`fwd_get_work`] and other
/// `fwd_submit_work` calls on the same handle. NOT safe concurrently with
/// [`fwd_finish_sync`]/[`fwd_free_sync`].
///
/// # Safety
///
/// The caller must:
/// * ensure that `sync` is a valid pointer to a [`SyncHandle`].
/// * ensure that `proof` is valid for [`BorrowedBytes`].
/// * free the key members of a returned [`SyncWorkItem`] with
///   [`fwd_free_owned_bytes`], and likewise any returned error or rejection
///   detail.
///
/// [`fwd_free_owned_bytes`]: crate::fwd_free_owned_bytes
#[unsafe(no_mangle)]
pub extern "C" fn fwd_submit_work(
    sync: Option<&SyncHandle<'_>>,
    id: u64,
    proof: BorrowedBytes<'_>,
) -> SubmitResult {
    crate::invoke_with_handle(sync, move |sync| {
        // Unparseable bytes are the peer's fault: classify as InvalidProof
        // WITHOUT calling submit, so the region stays reserved under the
        // same id with zero state mutation.
        let proof = match FrozenRangeProof::from_slice(&proof) {
            Ok(proof) => proof,
            Err(err) => {
                let err = api::Error::ProofError(ProofError::Deserialization(err));
                return SubmitResult::InvalidProof(err.to_string().into_bytes().into());
            }
        };
        sync.submit(id.into(), &proof).into()
    })
}

/// Consume the sync handle and return the latest committed root hash for
/// the caller to confirm against the target. Meaningful once
/// [`fwd_get_work`] returned `Done`, but callable anytime — an abandoned
/// sync leaves only ordinary revisions (see `docs/plans/state-sync.md`,
/// Lifecycle). Returns [`HashResult::None`] if the database is empty.
///
/// # Returns
///
/// - [`HashResult::NullHandlePointer`] if `sync` is null.
/// - [`HashResult::None`] if the database is empty.
/// - [`HashResult::Some`] with the latest committed root hash.
/// - [`HashResult::Err`] if the process panics while freeing the handle.
///
/// # Safety
///
/// The caller must:
/// * ensure that `sync` is a valid pointer to a [`SyncHandle`].
/// * not use `sync` after this call, and not call this concurrently with
///   any other function on the same handle.
/// * call [`fwd_free_owned_bytes`] to free the memory associated with a
///   returned error.
///
/// [`fwd_free_owned_bytes`]: crate::fwd_free_owned_bytes
#[unsafe(no_mangle)]
pub extern "C" fn fwd_finish_sync(sync: Option<Box<SyncHandle<'_>>>) -> HashResult {
    crate::invoke_with_handle(sync, |sync| Ok::<_, api::Error>(sync.finish()))
}

/// Free the sync handle without reporting the root (drop-without-finish).
/// Used by the Go cleanup path; equivalent to [`fwd_finish_sync`] with the
/// result ignored.
///
/// # Returns
///
/// - [`VoidResult::NullHandlePointer`] if `sync` is null.
/// - [`VoidResult::Ok`] if the handle was successfully freed.
/// - [`VoidResult::Err`] if the process panics while freeing the memory.
///
/// # Safety
///
/// The caller must:
/// * ensure that `sync` is a valid pointer to a [`SyncHandle`].
/// * not use `sync` after this call, and not call this concurrently with
///   any other function on the same handle.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_free_sync(sync: Option<Box<SyncHandle<'_>>>) -> VoidResult {
    crate::invoke_with_handle(sync, drop)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn work_item(
        id: u64,
        first_key: Option<&[u8]>,
        last_key: Option<&[u8]>,
        wakeup_neighbor: bool,
    ) -> firewood::sync::WorkItem {
        firewood::sync::WorkItem {
            id: id.into(),
            first_key: first_key.map(Box::from),
            last_key: last_key.map(Box::from),
            wakeup_neighbor,
        }
    }

    /// `None` bounds flatten losslessly: `first_key: None` becomes an EMPTY
    /// `start_key` (empty-means-unbounded) and `last_key: None` becomes
    /// `Maybe::None`.
    #[test]
    fn work_item_none_bounds_flatten_to_empty_and_maybe_none() {
        let item: SyncWorkItem = work_item(7, None, None, false).into();
        assert_eq!(item.id, 7);
        assert!(
            item.start_key.as_slice().is_empty(),
            "unbounded start must cross as an empty start_key"
        );
        assert!(
            item.end_key.is_none(),
            "unbounded end must cross as Maybe::None"
        );
        assert!(!item.wakeup_neighbor);
    }

    /// Present (non-empty) bounds pass through byte-for-byte.
    #[test]
    fn work_item_present_bounds_pass_through() {
        let item: SyncWorkItem = work_item(9, Some(&[1, 2, 3]), Some(&[4, 5]), true).into();
        assert_eq!(item.id, 9);
        assert_eq!(item.start_key.as_slice(), &[1, 2, 3]);
        match item.end_key {
            Maybe::Some(ref end) => assert_eq!(end.as_slice(), &[4, 5]),
            Maybe::None => panic!("present end bound must cross as Maybe::Some"),
        }
        assert!(item.wakeup_neighbor);
    }

    /// Mixed bounds: each side is mapped independently.
    #[test]
    fn work_item_mixed_bounds_map_independently() {
        let item: SyncWorkItem = work_item(1, None, Some(&[0xee]), true).into();
        assert!(item.start_key.as_slice().is_empty());
        assert!(item.end_key.is_some());

        let item: SyncWorkItem = work_item(2, Some(&[0x10]), None, false).into();
        assert_eq!(item.start_key.as_slice(), &[0x10]);
        assert!(item.end_key.is_none());
    }

    /// `task_limit == 0` is rejected at the FFI boundary. The end-to-end
    /// extern-fn check (through `fwd_start_sync` against a real database)
    /// is covered by the Go test suite: the ffi crate has no `tempfile`
    /// dev-dependency to construct a `DatabaseHandle` here.
    #[test]
    fn zero_task_limit_is_rejected() {
        let err = checked_task_limit(0).expect_err("zero task_limit must be rejected");
        assert!(
            err.to_string().contains("task_limit must be nonzero"),
            "unexpected error message: {err}"
        );
        assert_eq!(
            checked_task_limit(1).expect("nonzero task_limit is accepted"),
            NonZeroUsize::new(1).expect("1 is nonzero")
        );
    }

    /// `SyncError::CoverageRootMismatch` is the ONLY error routed to the
    /// typed [`GetWorkResult::CoverageRootMismatch`] variant; every other
    /// error (here: a stale id) lands in `Err`. Pins the `From` routing in
    /// `results.rs` against future refactors.
    #[test]
    fn coverage_root_mismatch_routes_to_typed_variant() {
        let mismatch = SyncError::CoverageRootMismatch {
            target: api::HashKey::from([0xab; 32]),
            actual: None,
        };
        let routed: GetWorkResult = Err::<GetWork, _>(mismatch).into();
        assert!(matches!(routed, GetWorkResult::CoverageRootMismatch(_)));

        let stale: GetWorkResult =
            Err::<GetWork, _>(SyncError::StaleWorkId(WorkId::from(9))).into();
        assert!(matches!(stale, GetWorkResult::Err(_)));
    }
}
