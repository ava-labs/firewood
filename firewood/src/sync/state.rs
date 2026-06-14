// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! [`SyncState`]: the pure state-sync bookkeeping machine (see
//! `docs/plans/state-sync.md`, "Hole selection").
//!
//! No Db, no I/O, no locking — just the coverage/in-flight maps and the
//! work-handout policy: an even `1/task_limit` seed fan-out, contiguous
//! per-lineage sweeps via truncation continuations, a shed guard that
//! re-exposes dense regions to helpers, and largest-cold-hole refeed. The
//! Db-coupled driver (`Syncer`) arrives in a later commit.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::ops::Range;

use firewood_storage::TrieHash;
use rangemap::{RangeMap, RangeSet};

use super::endpoint::{self, Endpoint, Span};

/// Opaque identifier correlating a handed-out work region with its eventual
/// proof submission. Monotonic, starting at 1, never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkId(u64);

impl WorkId {
    /// The raw id value (crosses the FFI as a `u64`).
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl From<u64> for WorkId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

/// Shed the far half of a continuation after this many consecutive
/// truncations of one lineage. Truncation count measures actual observed
/// work (keys served) independent of keyspace width, so it catches the
/// dense-but-narrow region that a span threshold structurally misses.
#[allow(
    dead_code,
    reason = "consumed by the Syncer submit path in a later commit"
)]
pub(crate) const SHED_TRUNCATION_LIMIT: u32 = 4;

/// One outstanding work region.
#[derive(Debug, Clone)]
#[allow(
    dead_code,
    reason = "consumed by the Syncer submit path in a later commit"
)]
pub(crate) struct InFlight {
    /// Exactly the half-open range handed out; the proof request is the
    /// inclusive `range.start ..= range.end` (see [`request_bounds`]).
    pub(crate) range: Range<Endpoint>,
    /// Target hash this item was issued against (Phase-2 ready; always
    /// `SyncState::target` in Phase 1).
    pub(crate) hash: TrieHash,
    /// Consecutive truncations of this lineage since its last shed — drives
    /// the large-remainder guard ([`SHED_TRUNCATION_LIMIT`]). Reset to 0 on
    /// shed.
    pub(crate) truncations: u32,
}

/// Result of [`SyncState::next_work`].
#[derive(Debug, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "consumed by the Syncer submit path in a later commit"
)]
pub(crate) enum NextWork {
    /// A new region to work.
    Work {
        /// Fresh identifier for the reservation.
        id: WorkId,
        /// The half-open region handed out.
        range: Range<Endpoint>,
        /// True iff cold space remains after this reservation — the caller
        /// should wake a parked helper.
        wakeup_neighbor: bool,
    },
    /// Nothing to hand out now; not complete (work in flight elsewhere).
    Wait,
    /// Coverage tiles the whole keyspace and nothing is in flight. The
    /// Db-coupled caller decides Done vs `CoverageRootMismatch`.
    CoverageComplete,
}

/// How far a verified proof reached, derived by the caller from
/// `find_next_key_after_range_proof`: `None` becomes [`ProofExtent::Full`],
/// `Some((l, _))` becomes [`ProofExtent::Truncated`] with `last_key = l`.
#[derive(Debug, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "consumed by the Syncer submit path in a later commit"
)]
pub(crate) enum ProofExtent {
    /// The proof covered the entire requested range.
    Full,
    /// The proof stopped early; `last_key` is the greatest key it proved.
    Truncated {
        /// Greatest key covered by the proof; verifier-guaranteed to lie
        /// inside the requested range.
        last_key: Box<[u8]>,
    },
}

/// Result of [`SyncState::complete`].
#[derive(Debug, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "consumed by the Syncer submit path in a later commit"
)]
pub(crate) enum Completed {
    /// Region fully covered; id retired.
    Exhausted,
    /// Same lineage continues on `range` under fresh `id`.
    Continue {
        /// Fresh identifier for the continuation reservation.
        id: WorkId,
        /// The remaining half-open region this lineage keeps sweeping.
        range: Range<Endpoint>,
        /// True iff a cold chunk was shed — wake a parked helper.
        wakeup_neighbor: bool,
    },
}

/// Errors surfaced by the pure sync state machine.
///
/// Only externally-triggerable conditions are errors; every internal
/// invariant is a `debug_assert!`. The Db-coupled driver (a later commit)
/// extends this enum with its own variants.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
#[allow(
    dead_code,
    reason = "consumed by the Syncer submit path in a later commit"
)]
pub(crate) enum SyncError {
    /// The work id is unknown or already retired (e.g. a duplicate
    /// submission after a timeout re-fetch). Callers drop this.
    #[error("unknown or already-retired work id {}", .0.as_u64())]
    StaleWorkId(WorkId),
    /// The truncation key reported for a work item lies outside its
    /// requested range: the proof/find-next-key layer broke its contract.
    /// Checked in release (not just debug-asserted) because acting on a bad
    /// key would either panic inside the coverage map (reversed range) or
    /// silently claim coverage the proof never proved.
    #[error("truncation key for work id {} lies outside the requested range", .0.as_u64())]
    TruncationKeyOutOfRange(WorkId),
}

/// Map a stored half-open region to the INCLUSIVE proof-request bounds.
///
/// `Key([])` start maps to `None` (no lower bound) and `Max` end maps to
/// `None`. These MUST be `None`, not `Some(&[])`: the proof generator emits
/// an empty start proof for `None`, and the verifier rejects a non-empty
/// start proof under a `None` bound (`UnexpectedStartProof`) — `Some([])`
/// makes the generator emit a root proof instead, which is not
/// wire-compatible.
#[allow(
    dead_code,
    reason = "consumed by the Syncer submit path in a later commit"
)]
pub(crate) fn request_bounds(range: &Range<Endpoint>) -> (Option<&[u8]>, Option<&[u8]>) {
    let start = match &range.start {
        Endpoint::Key(k) if k.is_empty() => None,
        Endpoint::Key(k) => Some(&**k),
        Endpoint::Max => {
            debug_assert!(false, "a work range never starts at Max");
            None
        }
    };
    let end = match &range.end {
        Endpoint::Max => None,
        Endpoint::Key(k) => Some(&**k),
    };
    (start, end)
}

/// Pure sync bookkeeping: no Db, no I/O, no locking. The Db-coupled wrapper
/// (`Syncer`) arrives in a later commit.
///
/// Coverage and in-flight space are tracked in two interval maps over
/// [`Endpoint`]; *cold* space (unfetched, not in flight) is implicit:
/// `gaps(coverage) − requested`. The number of outstanding items is always
/// `work.len()` — there is deliberately no separate counter to desync.
#[derive(Debug)]
#[allow(
    dead_code,
    reason = "consumed by the Syncer submit path in a later commit"
)]
pub(crate) struct SyncState {
    /// Root hash being synced to. Fixed in Phase 1.
    target: TrieHash,
    /// Durable verified coverage: range -> hash it was verified against.
    /// Adjacent equal-hash entries coalesce automatically.
    coverage: RangeMap<Endpoint, TrieHash>,
    /// In-flight occupancy index. Invariant: equals the union of
    /// `work[*].range`, and is disjoint from `coverage`'s keys.
    requested: RangeSet<Endpoint>,
    /// Authoritative per-item state, keyed by id. Lookup-only (never
    /// iterated for decisions), so a std `HashMap`; at most `task_limit`
    /// entries.
    work: HashMap<WorkId, InFlight>,
    /// Next id to allocate; starts at 1, monotonic.
    next_id: u64,
    /// Cap on concurrently outstanding work items. `NonZero` by type, so no
    /// zero-task-limit error path exists.
    task_limit: NonZeroUsize,
    /// Even seed slices handed out so far; seed phase while `< task_limit`.
    seeded: usize,
    /// Left edge of the not-yet-seeded tail. During the seed phase,
    /// everything in `requested` and `coverage` lies strictly left of this.
    seed_cursor: Endpoint,
}

#[allow(
    dead_code,
    reason = "consumed by the Syncer submit path in a later commit"
)]
impl SyncState {
    /// Create a fresh sync toward `target` with at most `task_limit`
    /// concurrently outstanding work items.
    pub(crate) fn new(target: TrieHash, task_limit: NonZeroUsize) -> Self {
        Self {
            target,
            coverage: RangeMap::new(),
            requested: RangeSet::new(),
            work: HashMap::new(),
            next_id: 1,
            task_limit,
            seeded: 0,
            seed_cursor: Endpoint::keyspace_start(),
        }
    }

    /// The root hash being synced to.
    pub(crate) const fn target(&self) -> &TrieHash {
        &self.target
    }

    /// Hand out the next region to work, or report why there is none.
    ///
    /// Seed phase: the first `task_limit` calls carve even slices of the
    /// keyspace (for `task_limit = 16` the cuts land exactly on the nibble
    /// boundaries `0x10, 0x20, …, 0xf0`). Steady phase: bisect the largest
    /// remaining cold hole ("help a neighbor").
    pub(crate) fn next_work(&mut self) -> NextWork {
        // Capacity gate: a worker only asks when it holds nothing, but this
        // keeps the machine safe against misuse and is the documented cap.
        if self.work.len() >= self.task_limit.get() {
            return NextWork::Wait;
        }
        if self.seeded < self.task_limit.get() {
            self.next_seed()
        } else {
            self.next_refeed()
        }
    }

    /// Seed phase: carve the next even slice out of `[seed_cursor, Max)`.
    fn next_seed(&mut self) -> NextWork {
        // Seed-phase invariant: every handed-out or covered range lies
        // strictly left of `seed_cursor` (continuations stay inside their
        // own slice; coverage comes only from handed-out slices), so the
        // remaining cold span at seed time is exactly `[seed_cursor, Max)`.
        debug_assert!(
            self.requested.iter().all(|r| r.end <= self.seed_cursor),
            "requested range beyond the seed cursor"
        );
        debug_assert!(
            self.coverage.iter().all(|(r, _)| r.end <= self.seed_cursor),
            "covered range beyond the seed cursor"
        );
        // The caller checked `seeded < task_limit`, so this cannot wrap and
        // `owed >= 1`.
        let owed = self.task_limit.get().wrapping_sub(self.seeded);
        let slice_end = if owed == 1 {
            // The final slice takes the exact remainder.
            Endpoint::Max
        } else {
            // `None` cannot happen for `owed >= 2` over `[seed_cursor, Max)`
            // (the fraction width is always nonzero since `seed_cursor` is a
            // finite key); defensively take the whole remainder if it did.
            endpoint::even_slice_point(&self.seed_cursor, &Endpoint::Max, owed)
                .unwrap_or(Endpoint::Max)
        };
        let range = self.seed_cursor.clone()..slice_end.clone();
        self.seed_cursor = slice_end;
        if range.end == Endpoint::Max {
            // The seed phase ends with the keyspace fully tiled.
            self.seeded = self.task_limit.get();
        } else {
            // `seeded < task_limit` was checked by the caller; cannot wrap.
            self.seeded = self.seeded.wrapping_add(1);
        }
        let id = self.reserve(range.clone(), 0);
        let wakeup_neighbor = self.has_cold();
        NextWork::Work {
            id,
            range,
            wakeup_neighbor,
        }
    }

    /// Steady phase: bisect the largest cold hole, or report
    /// [`NextWork::Wait`]/[`NextWork::CoverageComplete`].
    fn next_refeed(&mut self) -> NextWork {
        let Some(hole) = self
            .cold_holes()
            .into_iter()
            .max_by_key(|hole| endpoint::span(&hole.start, &hole.end))
        else {
            return if self.coverage_complete() {
                NextWork::CoverageComplete
            } else {
                NextWork::Wait
            };
        };
        // Bisect ("help a neighbor"): hand out the LEFT half so the sweep
        // runs left-to-right — EXCEPT for narrow holes, which are handed out
        // WHOLE. Two reasons a hole must go out whole:
        //
        // * Liveness floor: always bisecting would hand out the left half
        //   and leave the right half cold, so the cold frontier would only
        //   converge toward the hole's right edge without ever reaching it
        //   (the fraction width halves forever but never hits zero). Holes
        //   at or below `min_split_span()` therefore go out whole, bounding
        //   the number of handouts needed to retire any hole.
        // * Zero-width holes: a hole with `span == Span::default()` can
        //   still be lexicographically NON-empty — e.g.
        //   `[Key([0x10]), Key([0x10, 0x00]))` contains exactly the key
        //   `[0x10]` (keys of the form `a ++ 0x00^k` add no fraction width).
        //   Such holes must be handed out whole — never split (impossible:
        //   `midpoint` is `None`) and never dropped, or sync could not
        //   complete. The floor comparison covers them (zero span is the
        //   `Span` minimum).
        let range = if endpoint::span(&hole.start, &hole.end) <= min_split_span() {
            hole
        } else {
            match endpoint::midpoint(&hole.start, &hole.end) {
                Some(mid) => hole.start.clone()..mid,
                // Defensive: a hole wider than the (nonzero) floor always
                // has a midpoint; hand it out whole if not.
                None => hole,
            }
        };
        let id = self.reserve(range.clone(), 0);
        let wakeup_neighbor = self.has_cold();
        NextWork::Work {
            id,
            range,
            wakeup_neighbor,
        }
    }

    /// Snapshot of an outstanding item: `(range, hash)`; `None` for a stale
    /// id.
    ///
    /// Does NOT retire the id: an invalid proof (peer fault) must keep the
    /// reservation alive under the same id so the same bounds can be
    /// re-fetched from another peer.
    pub(crate) fn work_item(&self, id: WorkId) -> Option<(Range<Endpoint>, TrieHash)> {
        self.work
            .get(&id)
            .map(|item| (item.range.clone(), item.hash.clone()))
    }

    /// Apply a verified proof's extent to the maps.
    ///
    /// A truncated proof covers `[range.start, successor(last_key))` —
    /// inclusive of the last proven key — and the lineage continues at
    /// `successor(last_key)`. Because `successor(last_key)` is strictly
    /// greater than the previous request's covered ceiling, the in-flight
    /// start strictly advances on every [`Completed::Continue`]; an
    /// exhausted tail re-requested as `[successor(L), b]` comes back with
    /// empty key/values, which the caller preflights to
    /// [`ProofExtent::Full`], yielding [`Completed::Exhausted`]. Strict
    /// forward progress in every case — including against the naive
    /// `find_next_key` — is what prevents the bug-#1989 stall.
    ///
    /// # Errors
    ///
    /// - [`SyncError::StaleWorkId`] iff `id` is not outstanding (raced
    ///   retirement, e.g. a duplicate submission after a timeout re-fetch).
    /// - [`SyncError::TruncationKeyOutOfRange`] iff a [`ProofExtent::Truncated`]
    ///   key falls outside the item's requested range. Neither error mutates
    ///   any state; the item (if any) stays reserved under the same id.
    pub(crate) fn complete(
        &mut self,
        id: WorkId,
        extent: ProofExtent,
    ) -> Result<Completed, SyncError> {
        let Some(inflight) = self.work.get(&id) else {
            return Err(SyncError::StaleWorkId(id));
        };
        if let ProofExtent::Truncated { last_key } = &extent {
            // Verifier/find-next-key contract: range.start <= Key(L) <
            // range.end. Re-checked in release — see
            // [`SyncError::TruncationKeyOutOfRange`]. Checked BEFORE any
            // mutation so the error leaves the item reserved.
            let key = Endpoint::Key(last_key.clone());
            if key < inflight.range.start || inflight.range.end <= key {
                return Err(SyncError::TruncationKeyOutOfRange(id));
            }
        }
        let InFlight {
            range,
            hash,
            truncations,
        } = self
            .work
            .remove(&id)
            .expect("present: looked up immediately above");
        // Non-empty by the reservation invariant; `remove` would panic on an
        // empty range.
        debug_assert!(range.start < range.end, "in-flight ranges are non-empty");
        self.requested.remove(range.clone());

        match extent {
            ProofExtent::Full => {
                // Non-empty by the reservation invariant asserted above.
                self.coverage.insert(range, hash);
                Ok(Completed::Exhausted)
            }
            ProofExtent::Truncated { last_key } => {
                // range.start <= Key(L) < range.end was guarded above.
                // Proven-inclusive of L: cover [start, succ(L)). succ(L) has
                // no byte string strictly between it and L, so succ(L) <= end
                // always (L < end). Non-empty: start <= Key(L) < Key(succ(L)).
                let resume = Endpoint::Key(endpoint::successor(&last_key));
                let covered = range.start.clone()..resume.clone();
                debug_assert!(covered.start < covered.end, "start <= Key(L) < succ(L)");
                self.coverage.insert(covered, hash);

                if resume >= range.end {
                    // succ(L) == end: the truncation landed exactly on the
                    // region's right edge; the remainder is empty.
                    return Ok(Completed::Exhausted);
                }
                Ok(self.continue_or_shed(resume..range.end, truncations))
            }
        }
    }

    /// Continue a truncated lineage on `remainder`, shedding the far half
    /// cold once the lineage has truncated [`SHED_TRUNCATION_LIMIT`]
    /// consecutive times.
    ///
    /// With the naive find-next-key every full-size proof truncates, so `K`
    /// consecutive truncations approximate `K × max_length` keys actually
    /// served from one region — a direct density signal that fires exactly
    /// when one worker is grinding a hot region serially, which a span-ratio
    /// rule would never catch on a dense-but-narrow tail.
    fn continue_or_shed(&mut self, remainder: Range<Endpoint>, truncations: u32) -> Completed {
        let truncations = truncations.saturating_add(1);
        if truncations >= SHED_TRUNCATION_LIMIT
            && let Some(mid) = endpoint::midpoint(&remainder.start, &remainder.end)
        {
            // Keep [resume, mid) for this lineage (counter reset — it is a
            // fresh, halved assignment); [mid, end) goes COLD (simply not
            // reserved; cold space is implicit). Wake a parked helper.
            let cont = remainder.start.clone()..mid;
            let id = self.reserve(cont.clone(), 0);
            return Completed::Continue {
                id,
                range: cont,
                wakeup_neighbor: true,
            };
        }
        // Normal sweep: whole remainder, same lineage, counter carried.
        // (An unsplittable remainder also lands here regardless of count.)
        let id = self.reserve(remainder.clone(), truncations);
        Completed::Continue {
            id,
            range: remainder,
            wakeup_neighbor: false,
        }
    }

    /// True iff coverage has no gaps over the whole keyspace.
    pub(crate) fn coverage_complete(&self) -> bool {
        let whole = Endpoint::whole_keyspace();
        let complete = self.coverage.gaps(&whole).next().is_none();
        // `requested` is always disjoint from coverage, so full coverage
        // forces both in-flight structures empty.
        debug_assert!(
            !complete || (self.work.is_empty() && self.requested.is_empty()),
            "complete coverage must imply nothing in flight"
        );
        complete
    }

    /// Allocate the next monotonic work id.
    const fn alloc_id(&mut self) -> WorkId {
        let id = WorkId(self.next_id);
        // A u64 cannot realistically wrap; wrapping_add documents intent
        // without an arithmetic_side_effects expect.
        self.next_id = self.next_id.wrapping_add(1);
        id
    }

    /// Reserve `range` for a new in-flight item under a fresh id.
    ///
    /// Every caller guarantees `range` is non-empty by construction and
    /// disjoint from both maps; `RangeSet::insert` would panic on an empty
    /// range.
    fn reserve(&mut self, range: Range<Endpoint>, truncations: u32) -> WorkId {
        debug_assert!(range.start < range.end, "reserved ranges are non-empty");
        debug_assert!(
            !self.requested.overlaps(&range),
            "reservation overlaps in-flight space"
        );
        debug_assert!(
            !self.coverage.overlaps(&range),
            "reservation overlaps verified coverage"
        );
        let id = self.alloc_id();
        self.requested.insert(range.clone());
        let previous = self.work.insert(
            id,
            InFlight {
                range,
                hash: self.target.clone(),
                truncations,
            },
        );
        debug_assert!(previous.is_none(), "work ids are never reused");
        id
    }

    /// Cold space: coverage gaps minus in-flight ranges, in key order.
    ///
    /// The outer gaps are collected first because `RangeSet::gaps` borrows
    /// its outer range — the single-expression
    /// `gaps(..).flat_map(|g| requested.gaps(&g))` form does not compile
    /// (E0515).
    fn cold_holes(&self) -> Vec<Range<Endpoint>> {
        let whole = Endpoint::whole_keyspace();
        let outer: Vec<Range<Endpoint>> = self.coverage.gaps(&whole).collect();
        outer
            .iter()
            .flat_map(|gap| self.requested.gaps(gap))
            .collect()
    }

    /// True iff any cold hole exists (early-exit form of
    /// `!cold_holes().is_empty()`).
    fn has_cold(&self) -> bool {
        let whole = Endpoint::whole_keyspace();
        self.coverage
            .gaps(&whole)
            .any(|gap| self.requested.gaps(&gap).next().is_some())
    }
}

/// Fraction-width floor for refeed bisection: holes at or below this width
/// are handed out whole (see the comment in [`SyncState::next_refeed`]).
///
/// One ulp at two bytes — `2^-16` of the keyspace — so retiring a hole of
/// width `w` takes at most about `log2(w · 2^16) + 1` handouts, while
/// helper-parallelism granularity stays far finer than any realistic
/// `task_limit`.
#[allow(
    dead_code,
    reason = "consumed by the Syncer submit path in a later commit"
)]
fn min_split_span() -> Span {
    endpoint::span(
        &Endpoint::Key([0x00, 0x00].into()),
        &Endpoint::Key([0x00, 0x01].into()),
    )
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::unwrap_used,
        clippy::arithmetic_side_effects,
        reason = "tests may unwrap and use plain arithmetic on small constants"
    )]

    use super::*;
    use firewood_storage::SeededRng;

    fn ep(bytes: &[u8]) -> Endpoint {
        Endpoint::Key(bytes.into())
    }

    fn th(n: u8) -> TrieHash {
        TrieHash::from([n; 32])
    }

    fn limit(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).unwrap()
    }

    fn state(task_limit: usize) -> SyncState {
        SyncState::new(th(7), limit(task_limit))
    }

    fn take_work(state: &mut SyncState) -> (WorkId, Range<Endpoint>, bool) {
        match state.next_work() {
            NextWork::Work {
                id,
                range,
                wakeup_neighbor,
            } => (id, range, wakeup_neighbor),
            other => panic!("expected Work, got {other:?}"),
        }
    }

    fn take_continue(completed: Completed) -> (WorkId, Range<Endpoint>, bool) {
        match completed {
            Completed::Continue {
                id,
                range,
                wakeup_neighbor,
            } => (id, range, wakeup_neighbor),
            Completed::Exhausted => panic!("expected Continue, got Exhausted"),
        }
    }

    fn coverage_entries(state: &SyncState) -> Vec<(Range<Endpoint>, TrieHash)> {
        state
            .coverage
            .iter()
            .map(|(range, hash)| (range.clone(), hash.clone()))
            .collect()
    }

    /// `(coverage entries, requested ranges, outstanding item count)`.
    type Snapshot = (
        Vec<(Range<Endpoint>, TrieHash)>,
        Vec<Range<Endpoint>>,
        usize,
    );

    fn snapshot(state: &SyncState) -> Snapshot {
        (
            coverage_entries(state),
            state.requested.iter().cloned().collect(),
            state.work.len(),
        )
    }

    fn ranges_overlap(a: &Range<Endpoint>, b: &Range<Endpoint>) -> bool {
        a.start < b.end && b.start < a.end
    }

    /// Invariants 1-5 from the implementation blueprint (§7).
    fn assert_invariants(state: &SyncState) {
        let whole = Endpoint::whole_keyspace();
        // 1. `requested` is disjoint from coverage.
        for range in state.requested.iter() {
            assert!(
                !state.coverage.overlaps(range),
                "requested {range:?} overlaps coverage"
            );
        }
        // 2. `requested` equals the union of work ranges; all non-empty.
        let mut union = RangeSet::new();
        for item in state.work.values() {
            assert!(
                item.range.start < item.range.end,
                "empty in-flight range {:?}",
                item.range
            );
            union.insert(item.range.clone());
        }
        assert!(
            union.iter().eq(state.requested.iter()),
            "requested set diverged from the work map"
        );
        // 3. Outstanding item count is capped.
        assert!(
            state.work.len() <= state.task_limit.get(),
            "task limit exceeded"
        );
        // 4. Seed phase: everything occupied lies strictly left of the
        //    seed cursor.
        if state.seeded < state.task_limit.get() {
            assert!(state.seed_cursor < Endpoint::Max);
            for range in state.requested.iter() {
                assert!(range.end <= state.seed_cursor, "request beyond seed cursor");
            }
            for (range, _) in state.coverage.iter() {
                assert!(
                    range.end <= state.seed_cursor,
                    "coverage beyond seed cursor"
                );
            }
        }
        // 5. Complete coverage implies nothing in flight.
        if state.coverage.gaps(&whole).next().is_none() {
            assert!(state.work.is_empty(), "complete coverage with live work");
            assert!(
                state.requested.is_empty(),
                "complete coverage with requests"
            );
        }
    }

    // ── 1 ──────────────────────────────────────────────────────────────
    #[test]
    fn seed_sixteen_lands_on_nibble_boundaries() {
        let mut state = state(16);
        let mut expected_start = ep(&[]);
        for i in 1..=16u8 {
            let (_, range, _) = take_work(&mut state);
            assert_eq!(range.start, expected_start, "slice {i} must abut");
            let expected_end = if i == 16 {
                Endpoint::Max
            } else {
                ep(&[i * 0x10])
            };
            assert_eq!(range.end, expected_end, "slice {i} boundary");
            expected_start = range.end.clone();
            // The stored cursor matches the derived rightmost cold gap.
            let holes = state.cold_holes();
            if i < 16 {
                let tail = holes.last().unwrap();
                assert_eq!(tail.end, Endpoint::Max);
                assert_eq!(state.seed_cursor, tail.start);
            } else {
                assert!(holes.is_empty());
                assert_eq!(state.seed_cursor, Endpoint::Max);
            }
            assert_invariants(&state);
        }
    }

    // ── 2 ──────────────────────────────────────────────────────────────
    #[test]
    fn seed_prime_task_limit_tiles_exactly() {
        let mut state = state(5);
        let mut previous_end = ep(&[]);
        for i in 1..=5usize {
            let (_, range, _) = take_work(&mut state);
            assert_eq!(range.start, previous_end, "slice {i} must abut");
            assert!(range.start < range.end, "slice {i} must be non-empty");
            previous_end = range.end.clone();
            assert_invariants(&state);
        }
        assert_eq!(previous_end, Endpoint::Max, "last slice ends at Max");
        assert!(state.cold_holes().is_empty(), "no gap and no overlap");
    }

    // ── 3 ──────────────────────────────────────────────────────────────
    #[test]
    fn seed_wakeup_truth() {
        let mut state = state(16);
        for i in 1..=16usize {
            let (_, _, wakeup) = take_work(&mut state);
            assert_eq!(wakeup, i < 16, "handout {i} wakeup");
        }
    }

    // ── 4 ──────────────────────────────────────────────────────────────
    #[test]
    fn full_extent_coalesces_adjacent_coverage() {
        let mut state = state(2);
        let (id_a, range_a, _) = take_work(&mut state);
        let (id_b, range_b, _) = take_work(&mut state);
        assert_eq!(range_a.end, range_b.start, "seed slices abut");

        assert!(matches!(
            state.complete(id_a, ProofExtent::Full),
            Ok(Completed::Exhausted)
        ));
        assert!(matches!(
            state.complete(id_b, ProofExtent::Full),
            Ok(Completed::Exhausted)
        ));
        assert_eq!(
            coverage_entries(&state),
            vec![(Endpoint::whole_keyspace(), th(7))],
            "abutting equal-hash coverage coalesces to one entry"
        );
        // Both ids are retired.
        assert!(matches!(
            state.complete(id_a, ProofExtent::Full),
            Err(SyncError::StaleWorkId(_))
        ));
        assert!(matches!(state.next_work(), NextWork::CoverageComplete));
        assert_invariants(&state);
    }

    // ── 5 ──────────────────────────────────────────────────────────────
    #[test]
    fn truncated_records_succ_inclusive_coverage() {
        let mut state = state(1);
        let (id0, range0, _) = take_work(&mut state);
        assert_eq!(range0, Endpoint::whole_keyspace());

        let completed = state
            .complete(
                id0,
                ProofExtent::Truncated {
                    last_key: Box::from(&[0x40][..]),
                },
            )
            .unwrap();
        let (id1, range1, wakeup) = take_continue(completed);
        assert_ne!(id1, id0, "continuation gets a fresh id");
        assert!(!wakeup);
        assert_eq!(range1, ep(&[0x40, 0x00])..Endpoint::Max);
        assert_eq!(
            coverage_entries(&state),
            vec![(ep(&[])..ep(&[0x40, 0x00]), th(7))],
            "coverage is inclusive of the last proven key"
        );
        assert!(state.work_item(id0).is_none(), "old id is stale");
        assert!(state.work_item(id1).is_some());
        assert!(matches!(
            state.complete(id0, ProofExtent::Full),
            Err(SyncError::StaleWorkId(_))
        ));
        assert_invariants(&state);
    }

    // ── 6 ──────────────────────────────────────────────────────────────
    #[test]
    fn truncation_on_right_edge_is_exhausted() {
        // Seed slicing never produces a right edge of the form `L ++ 0x00`,
        // so reserve the region directly (legitimate: `complete` is the unit
        // under test).
        let mut state = state(1);
        state.seeded = 1;
        state.seed_cursor = Endpoint::Max;
        let range = ep(&[0x10])..ep(&[0x10, 0x40, 0x00]);
        let id = state.reserve(range.clone(), 0);

        let completed = state
            .complete(
                id,
                ProofExtent::Truncated {
                    last_key: Box::from(&[0x10, 0x40][..]),
                },
            )
            .unwrap();
        // succ(L) == range.end: empty remainder, no dangling reservation,
        // and no empty rangemap insert was attempted.
        assert_eq!(completed, Completed::Exhausted);
        assert_eq!(coverage_entries(&state), vec![(range, th(7))]);
        assert!(state.requested.is_empty());
        assert!(state.work.is_empty());
        assert_invariants(&state);
    }

    // ── 7 ──────────────────────────────────────────────────────────────
    #[test]
    fn exhausted_tail_rerequest_makes_progress() {
        // The bug-#1989 shape: a truncated region whose tail holds no more
        // keys. The continuation start must strictly advance every step so
        // the empty re-request (preflighted to Full) exhausts the lineage.
        let mut state = state(1);
        let (id0, range0, _) = take_work(&mut state);
        let mut previous_start = range0.start;

        let (id1, range1, _) = take_continue(
            state
                .complete(
                    id0,
                    ProofExtent::Truncated {
                        last_key: Box::from(&[0x55][..]),
                    },
                )
                .unwrap(),
        );
        assert!(range1.start > previous_start, "start must strictly advance");
        previous_start = range1.start.clone();

        let (id2, range2, _) = take_continue(
            state
                .complete(
                    id1,
                    ProofExtent::Truncated {
                        last_key: Box::from(&[0x60][..]),
                    },
                )
                .unwrap(),
        );
        assert!(range2.start > previous_start, "start must strictly advance");

        // Tail is exhausted: the re-request returns no keys -> Full.
        assert!(matches!(
            state.complete(id2, ProofExtent::Full),
            Ok(Completed::Exhausted)
        ));
        assert!(matches!(state.next_work(), NextWork::CoverageComplete));
        assert_invariants(&state);
    }

    // ── 8 ──────────────────────────────────────────────────────────────
    #[test]
    fn lineage_sheds_after_k_truncations() {
        let mut state = state(2);
        let (id_a, range_a, _) = take_work(&mut state);
        let (id_b, _, _) = take_work(&mut state);
        assert_eq!(range_a, ep(&[])..ep(&[0x80]));
        assert!(matches!(
            state.complete(id_b, ProofExtent::Full),
            Ok(Completed::Exhausted)
        ));

        // First K-1 truncations: whole remainder, no wakeup.
        let mut id = id_a;
        for last_key in [&[0x10][..], &[0x20], &[0x30]] {
            let (next, range, wakeup) = take_continue(
                state
                    .complete(
                        id,
                        ProofExtent::Truncated {
                            last_key: last_key.into(),
                        },
                    )
                    .unwrap(),
            );
            assert_eq!(range.end, ep(&[0x80]), "no shed: whole remainder");
            assert!(!wakeup);
            id = next;
            assert_invariants(&state);
        }

        // Kth truncation sheds: continuation clipped at the midpoint, far
        // half cold, helper woken.
        let (id_shed, range_shed, wakeup) = take_continue(
            state
                .complete(
                    id,
                    ProofExtent::Truncated {
                        last_key: Box::from(&[0x40][..]),
                    },
                )
                .unwrap(),
        );
        assert_eq!(range_shed, ep(&[0x40, 0x00])..ep(&[0x60]));
        assert!(wakeup, "shed must wake a parked helper");
        assert_eq!(state.cold_holes(), vec![ep(&[0x60])..ep(&[0x80])]);

        // The shed chunk is findable by a subsequent next_work (bisected).
        let (_, helper_range, helper_wakeup) = take_work(&mut state);
        assert_eq!(helper_range, ep(&[0x60])..ep(&[0x70]));
        assert!(helper_wakeup, "right half of the bisected hole stays cold");
        assert_invariants(&state);

        // Counter reset: the post-shed lineage takes another K truncations.
        let mut id = id_shed;
        for last_key in [&[0x44][..], &[0x48], &[0x4c]] {
            let (next, range, wakeup) = take_continue(
                state
                    .complete(
                        id,
                        ProofExtent::Truncated {
                            last_key: last_key.into(),
                        },
                    )
                    .unwrap(),
            );
            assert_eq!(range.end, ep(&[0x60]), "no shed: whole remainder");
            assert!(!wakeup);
            id = next;
        }
        let (_, range, wakeup) = take_continue(
            state
                .complete(
                    id,
                    ProofExtent::Truncated {
                        last_key: Box::from(&[0x50][..]),
                    },
                )
                .unwrap(),
        );
        assert_eq!(range, ep(&[0x50, 0x00])..ep(&[0x58]), "second shed");
        assert!(wakeup);
        assert_invariants(&state);
    }

    // ── 9 ──────────────────────────────────────────────────────────────
    #[test]
    fn refeed_bisects_largest_cold_hole() {
        // Steady-state cold space arises only from shedding; build the maps
        // directly to keep the scenario small.
        let mut state = state(4);
        state.seeded = 4;
        state.seed_cursor = Endpoint::Max;
        state.coverage.insert(ep(&[0x10])..ep(&[0x80]), th(7));
        state.coverage.insert(ep(&[0xc0])..Endpoint::Max, th(7));
        // Holes: [[], [0x10]) (width 0x10) and [[0x80], [0xc0]) (width 0x40).
        let (_, range, wakeup) = take_work(&mut state);
        assert_eq!(
            range,
            ep(&[0x80])..ep(&[0xa0]),
            "left half of the larger hole"
        );
        assert!(wakeup, "its right half and the smaller hole stay cold");
        assert_eq!(
            state.cold_holes(),
            vec![ep(&[])..ep(&[0x10]), ep(&[0xa0])..ep(&[0xc0])]
        );
        assert_invariants(&state);
    }

    // ── 10 ─────────────────────────────────────────────────────────────
    #[test]
    fn refeed_whole_hole_when_unsplittable() {
        // [Key([0x10]), Key([0x10, 0x00])) has zero fraction width
        // (span == Span::default(), midpoint == None) yet is
        // lexicographically NON-empty: it contains exactly the key [0x10].
        // It must be handed out WHOLE — never split, never dropped.
        let mut state = state(1);
        state.seeded = 1;
        state.seed_cursor = Endpoint::Max;
        state.coverage.insert(ep(&[])..ep(&[0x10]), th(7));
        state
            .coverage
            .insert(ep(&[0x10, 0x00])..Endpoint::Max, th(7));
        let hole = ep(&[0x10])..ep(&[0x10, 0x00]);
        assert_eq!(endpoint::span(&hole.start, &hole.end), Span::default());

        let (id, range, wakeup) = take_work(&mut state);
        assert_eq!(range, hole, "zero-width hole handed out whole");
        assert!(!wakeup);
        assert_invariants(&state);

        assert!(matches!(
            state.complete(id, ProofExtent::Full),
            Ok(Completed::Exhausted)
        ));
        assert!(matches!(state.next_work(), NextWork::CoverageComplete));
    }

    // ── 11 ─────────────────────────────────────────────────────────────
    #[test]
    fn cold_holes_is_gaps_minus_requested() {
        let mut state = state(4);
        state.seeded = 4;
        state.seed_cursor = Endpoint::Max;
        state.coverage.insert(ep(&[0x20])..ep(&[0x40]), th(7));
        state.coverage.insert(ep(&[0x80])..ep(&[0xa0]), th(7));
        // An in-flight island strictly inside the middle coverage gap.
        state.reserve(ep(&[0x50])..ep(&[0x60]), 0);
        assert_eq!(
            state.cold_holes(),
            vec![
                ep(&[])..ep(&[0x20]),
                ep(&[0x40])..ep(&[0x50]),
                ep(&[0x60])..ep(&[0x80]),
                ep(&[0xa0])..Endpoint::Max,
            ],
            "cold space is exactly gaps(coverage) minus requested"
        );
        assert_invariants(&state);
    }

    // ── 12 ─────────────────────────────────────────────────────────────
    #[test]
    fn wait_when_everything_in_flight() {
        let mut state = state(2);
        let (_, _, _) = take_work(&mut state);
        let (id_b, _, _) = take_work(&mut state);
        // Capacity full.
        assert!(matches!(state.next_work(), NextWork::Wait));
        // Capacity free, but the only coverage gap is fully in flight.
        assert!(matches!(
            state.complete(id_b, ProofExtent::Full),
            Ok(Completed::Exhausted)
        ));
        assert!(!state.coverage_complete());
        assert!(matches!(state.next_work(), NextWork::Wait));
        assert_invariants(&state);
    }

    // ── 13 ─────────────────────────────────────────────────────────────
    #[test]
    fn coverage_complete_detection() {
        let mut state = state(2);
        let (id_a, _, _) = take_work(&mut state);
        let (id_b, _, _) = take_work(&mut state);

        // Mixed Full/Truncated completions tile the keyspace.
        let (cont_a, _, _) = take_continue(
            state
                .complete(
                    id_a,
                    ProofExtent::Truncated {
                        last_key: Box::from(&[0x10][..]),
                    },
                )
                .unwrap(),
        );
        assert!(matches!(
            state.complete(cont_a, ProofExtent::Full),
            Ok(Completed::Exhausted)
        ));
        let (cont_b, _, _) = take_continue(
            state
                .complete(
                    id_b,
                    ProofExtent::Truncated {
                        last_key: Box::from(&[0xc0][..]),
                    },
                )
                .unwrap(),
        );
        assert!(matches!(
            state.complete(cont_b, ProofExtent::Full),
            Ok(Completed::Exhausted)
        ));

        assert!(state.coverage_complete());
        assert!(matches!(state.next_work(), NextWork::CoverageComplete));
        assert!(state.work.is_empty());
        assert!(state.requested.is_empty());
        assert_invariants(&state);
    }

    // ── 14 ─────────────────────────────────────────────────────────────
    #[test]
    fn stale_id_rejected_without_mutation() {
        let mut state = state(2);
        let (id_a, _, _) = take_work(&mut state);
        let (_, _, _) = take_work(&mut state);

        // Never-issued id.
        let before = snapshot(&state);
        assert!(matches!(
            state.complete(WorkId::from(9999), ProofExtent::Full),
            Err(SyncError::StaleWorkId(id)) if id.as_u64() == 9999
        ));
        assert_eq!(snapshot(&state), before, "stale complete must not mutate");

        // Already-retired id.
        assert!(matches!(
            state.complete(id_a, ProofExtent::Full),
            Ok(Completed::Exhausted)
        ));
        let after_retire = snapshot(&state);
        assert!(matches!(
            state.complete(id_a, ProofExtent::Full),
            Err(SyncError::StaleWorkId(_))
        ));
        assert_eq!(snapshot(&state), after_retire);
        assert_invariants(&state);
    }

    // ── 14b ────────────────────────────────────────────────────────────
    #[test]
    fn truncation_key_out_of_range_rejected_without_mutation() {
        let mut state = state(2);
        let (id, range, _) = take_work(&mut state);

        let before = snapshot(&state);
        // Below the range start (start is Key([]) for the first seed slice
        // only when task_limit == 1; with task_limit == 2 the second slice
        // starts mid-keyspace — use a key past the END instead, which is out
        // of range for every slice, then a below-start key for the second).
        let past_end = match &range.end {
            Endpoint::Key(end) => Some(endpoint::successor(end)),
            Endpoint::Max => None, // no key is past Max
        };
        if let Some(bad) = past_end {
            assert!(matches!(
                state.complete(id, ProofExtent::Truncated { last_key: bad }),
                Err(SyncError::TruncationKeyOutOfRange(bad_id)) if bad_id == id
            ));
            assert_eq!(
                snapshot(&state),
                before,
                "out-of-range truncation must not mutate"
            );
        }

        // The item stays reserved under the same id: a well-formed
        // completion afterward still succeeds.
        assert!(matches!(
            state.complete(id, ProofExtent::Full),
            Ok(Completed::Exhausted)
        ));
        assert_invariants(&state);

        // Below-start shape: the second slice starts strictly inside the
        // keyspace, so the empty key is below it.
        let (id_b, range_b, _) = take_work(&mut state);
        assert!(
            range_b.start > Endpoint::keyspace_start(),
            "second slice starts mid-keyspace"
        );
        let before_b = snapshot(&state);
        assert!(matches!(
            state.complete(
                id_b,
                ProofExtent::Truncated {
                    last_key: Box::default()
                }
            ),
            Err(SyncError::TruncationKeyOutOfRange(bad_id)) if bad_id == id_b
        ));
        assert_eq!(snapshot(&state), before_b);
        assert_invariants(&state);
    }

    // ── 15 ─────────────────────────────────────────────────────────────
    #[test]
    fn invalid_proof_reservation_survives() {
        // The whole InvalidProof story at the pure layer: look the item up,
        // do NOT call complete. The reservation must survive untouched.
        let mut state = state(2);
        let (id_a, range_a, _) = take_work(&mut state);

        let (looked_range, looked_hash) = state.work_item(id_a).unwrap();
        assert_eq!(looked_range, range_a);
        assert_eq!(looked_hash, th(7));

        // The reserved region is never handed out again...
        let (id_b, range_b, _) = take_work(&mut state);
        assert_ne!(id_b, id_a);
        assert!(!ranges_overlap(&range_a, &range_b));
        assert!(matches!(state.next_work(), NextWork::Wait));

        // ...the id stays valid, and a later complete succeeds.
        assert!(state.work_item(id_a).is_some());
        assert!(matches!(
            state.complete(id_a, ProofExtent::Full),
            Ok(Completed::Exhausted)
        ));
        assert_invariants(&state);
    }

    // ── 16 ─────────────────────────────────────────────────────────────
    #[test]
    fn task_limit_one_serial_sweep() {
        let mut state = state(1);
        let (mut id, range, wakeup) = take_work(&mut state);
        assert_eq!(range, Endpoint::whole_keyspace(), "single seed = whole");
        assert!(!wakeup);
        let mut previous_start = range.start;

        for last_key in [&[0x30][..], &[0x90], &[0xd0]] {
            // Never more than one outstanding item.
            assert!(matches!(state.next_work(), NextWork::Wait));
            let (next, range, wakeup) = take_continue(
                state
                    .complete(
                        id,
                        ProofExtent::Truncated {
                            last_key: last_key.into(),
                        },
                    )
                    .unwrap(),
            );
            assert!(!wakeup);
            assert_eq!(range.end, Endpoint::Max, "whole remainder, no shed");
            assert!(range.start > previous_start);
            previous_start = range.start.clone();
            id = next;
            assert!(state.work.len() <= 1);
            assert_invariants(&state);
        }

        assert!(matches!(
            state.complete(id, ProofExtent::Full),
            Ok(Completed::Exhausted)
        ));
        assert!(matches!(state.next_work(), NextWork::CoverageComplete));
        assert_invariants(&state);
    }

    // ── 17 ─────────────────────────────────────────────────────────────
    fn assert_no_overlap(active: &[(WorkId, Range<Endpoint>)], range: &Range<Endpoint>) {
        for (id, reserved) in active {
            assert!(
                !ranges_overlap(reserved, range),
                "handed-out {range:?} overlaps live {id:?} ({reserved:?})"
            );
        }
    }

    /// A random truncation key within the item's request bounds:
    /// `range.start <= Key(L) < range.end`.
    fn draw_truncation_key(rng: &SeededRng, range: &Range<Endpoint>) -> Box<[u8]> {
        let Endpoint::Key(base) = &range.start else {
            panic!("work ranges never start at Max");
        };
        let mut candidates: Vec<Box<[u8]>> = vec![base.clone()];
        for extra in 1..=2usize {
            let candidate: Box<[u8]> = base
                .iter()
                .copied()
                .chain((0..extra).map(|_| rng.random()))
                .collect();
            if Endpoint::Key(candidate.clone()) < range.end {
                candidates.push(candidate);
            }
        }
        let pick = rng.random_range(0..candidates.len());
        candidates.swap_remove(pick)
    }

    fn run_random_ops(rng: &SeededRng, ops: usize) {
        let task_limit = rng.random_range(1..=8usize);
        let mut state = SyncState::new(th(7), limit(task_limit));
        // Mirror of the live reservations, for overlap checking: no two
        // ever-handed-out ranges may overlap unless the earlier one was
        // retired first.
        let mut active: Vec<(WorkId, Range<Endpoint>)> = Vec::new();

        for _ in 0..ops {
            if active.is_empty() || rng.random_range(0..2u8) == 0 {
                match state.next_work() {
                    NextWork::Work { id, range, .. } => {
                        assert_no_overlap(&active, &range);
                        active.push((id, range));
                    }
                    NextWork::Wait => {}
                    NextWork::CoverageComplete => break,
                }
            } else {
                let pick = rng.random_range(0..active.len());
                let (id, range) = active.swap_remove(pick);
                let extent = if rng.random_range(0..3u8) == 0 {
                    ProofExtent::Full
                } else {
                    ProofExtent::Truncated {
                        last_key: draw_truncation_key(rng, &range),
                    }
                };
                match state.complete(id, extent).unwrap() {
                    Completed::Exhausted => {}
                    Completed::Continue { id, range, .. } => {
                        assert_no_overlap(&active, &range);
                        active.push((id, range));
                    }
                }
            }
            assert_invariants(&state);
            assert_eq!(active.len(), state.work.len(), "mirror diverged");
        }

        // Drive to completion with Full completions. Terminates thanks to
        // the refeed split floor (see `next_refeed`).
        let mut guard = 0usize;
        loop {
            guard += 1;
            assert!(guard < 200_000, "sync failed to converge");
            if let Some((id, _)) = active.pop() {
                assert!(matches!(
                    state.complete(id, ProofExtent::Full),
                    Ok(Completed::Exhausted)
                ));
                assert_invariants(&state);
                continue;
            }
            match state.next_work() {
                NextWork::Work { id, range, .. } => {
                    assert_no_overlap(&active, &range);
                    active.push((id, range));
                }
                NextWork::Wait => panic!("Wait with nothing outstanding"),
                NextWork::CoverageComplete => break,
            }
            assert_invariants(&state);
        }
        assert!(state.coverage_complete());
        assert_invariants(&state);
    }

    #[test]
    fn random_ops_uphold_invariants() {
        let rng = SeededRng::from_env_or_random();
        for _ in 0..10 {
            run_random_ops(&rng, 300);
        }
    }
}
