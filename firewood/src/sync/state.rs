// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! [`SyncState`]: the pure state-sync bookkeeping machine (see
//! `docs/src/designs/2026-06-11-state-sync.md`, "Hole selection").
//!
//! No Db, no I/O, no locking — just the coverage/in-flight maps and the
//! work-handout policy: an even `1/task_limit` seed fan-out, contiguous
//! per-lineage sweeps via truncation continuations, a shed at a
//! proof-chosen cut whenever a worker is idle, and largest-cold-hole refeed.
//! The Db-coupled driver is [`super::syncer::Syncer`].

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::ops::Range;

use firewood_storage::TrieHash;
use rangemap::RangeMap;
use rangemap::RangeSet;

use super::endpoint::Endpoint;
use super::endpoint::Width;

pub(crate) mod complete;

/// Opaque identifier correlating a handed-out work region with its eventual
/// proof submission. `SyncState` issues ids monotonically from
/// [`WorkId::default`] (1) and never reuses one, so 0 is never outstanding.
/// Any `u64` converts, so an id arriving over the FFI that was never issued
/// is reported as stale rather than rejected at conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WorkId(u64);

impl Default for WorkId {
    /// The first id issued.
    fn default() -> Self {
        Self(1)
    }
}

impl WorkId {
    /// The raw id value (crosses the FFI as a `u64`).
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// The id after this one. A u64 counter never wraps in practice;
    /// `wrapping_add` satisfies `arithmetic_side_effects`.
    #[must_use]
    const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

impl From<u64> for WorkId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

/// One outstanding work item.
#[derive(Debug)]
struct InFlight {
    /// The half-open range handed out; the proof request includes both ends.
    range: Range<Endpoint>,
    /// The hash the item was issued against; always `SyncState::target` in
    /// Phase 1.
    hash: TrieHash,
}

/// Result of [`SyncState::next_work`].
#[derive(Debug, PartialEq, Eq)]
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

/// Errors surfaced by state sync.
///
/// Every variant is fatal to the sync: abandon it and restart. Only
/// externally-triggerable conditions are errors; every internal invariant
/// is a `debug_assert!`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SyncError {
    /// The work id is unknown or already retired: the caller submitted an id
    /// it no longer holds, such as a duplicate submission.
    #[error("unknown or already-retired work id {}", .0.as_u64())]
    StaleWorkId(WorkId),
    /// The truncation key reported for a work item lies outside its
    /// requested range: the proof/find-next-key layer broke its contract.
    /// Checked in release (not just debug-asserted) because acting on a bad
    /// key would either panic inside the coverage map (reversed range) or
    /// silently claim coverage the proof never proved.
    #[error("truncation key for work id {} lies outside the requested range", .0.as_u64())]
    TruncationKeyOutOfRange(WorkId),
    /// Never a peer fault; see [`super::Syncer::get_work`].
    #[error(
        "committed root {} does not match target {target} after full coverage",
        actual.as_ref().map_or_else(|| "(empty)".to_owned(), ToString::to_string)
    )]
    CoverageRootMismatch {
        /// The fixed root hash the sync was started toward.
        target: TrieHash,
        /// The latest committed root that failed to match it.
        actual: Option<TrieHash>,
    },
    /// Internal database/API failure (never the peer's fault).
    #[error(transparent)]
    Api(#[from] crate::api::Error),
}

/// Pure sync bookkeeping: no Db, no I/O, no locking. The Db-coupled wrapper
/// is [`super::syncer::Syncer`].
///
/// *Cold* space (unfetched and not in flight) is not stored; it is
/// `gaps(coverage) − requested`.
#[derive(Debug)]
pub(crate) struct SyncState {
    /// Root hash being synced to. Fixed in Phase 1.
    target: TrieHash,
    /// Verified ranges, each mapped to the hash it was verified against.
    coverage: RangeMap<Endpoint, TrieHash>,
    /// The union of every `work` item's range; disjoint from `coverage`.
    requested: RangeSet<Endpoint>,
    /// Outstanding items; at most `task_limit`.
    work: HashMap<WorkId, InFlight>,
    /// The next id to hand out. Ids start at 1 and are never reused.
    next_id: WorkId,
    task_limit: NonZeroUsize,
    /// Seed slices handed out so far.
    seeded: usize,
    /// Start of the unseeded tail `[seed_cursor, Max)`; the seed phase lasts
    /// while it is not empty.
    seed_cursor: Endpoint,
}

impl SyncState {
    /// Create a fresh sync toward `target` with at most `task_limit`
    /// concurrently outstanding work items.
    pub(crate) fn new(target: TrieHash, task_limit: NonZeroUsize) -> Self {
        Self {
            target,
            coverage: RangeMap::new(),
            requested: RangeSet::new(),
            work: HashMap::new(),
            next_id: WorkId::default(),
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
    /// boundaries `0x10, 0x20, …, 0xf0`). Steady phase: the widest cold hole,
    /// whole ("help a neighbor"); a shed already cut it where its proof showed
    /// the keys divide, so splitting it again here would only guess.
    pub(crate) fn next_work(&mut self) -> NextWork {
        // Workers ask only when idle, so this guards against misuse.
        if self.work.len() >= self.task_limit.get() {
            return NextWork::Wait;
        }
        let range = if self.seed_cursor < Endpoint::Max {
            self.next_seed()
        } else {
            match self.widest_cold_hole() {
                Some(hole) => hole,
                None if self.coverage_complete() => return NextWork::CoverageComplete,
                None => return NextWork::Wait,
            }
        };
        let id = self.reserve(range.clone());
        NextWork::Work {
            id,
            range,
            wakeup_neighbor: self.has_cold_hole(),
        }
    }

    /// The next even slice of the unseeded tail.
    fn next_seed(&mut self) -> Range<Endpoint> {
        // Coverage, reservations, and shed space all come from handed-out
        // slices, so everything left of the cursor stays left of it.
        debug_assert!(
            self.requested.iter().all(|r| r.end <= self.seed_cursor),
            "requested range beyond the seed cursor"
        );
        debug_assert!(
            self.coverage.iter().all(|(r, _)| r.end <= self.seed_cursor),
            "covered range beyond the seed cursor"
        );
        let slices_left = NonZeroUsize::new(self.task_limit.get().wrapping_sub(self.seeded))
            .expect("the last of the task_limit slices ends at Max");
        let slice_end = if slices_left == NonZeroUsize::MIN {
            Endpoint::Max
        } else {
            self.seed_cursor
                .first_cut(&Endpoint::Max, slices_left)
                .expect("a nonempty tail split two or more ways has a key strictly inside")
        };
        self.seeded = self.seeded.wrapping_add(1);
        let range = self.seed_cursor.clone()..slice_end.clone();
        self.seed_cursor = slice_end;
        range
    }

    /// True iff coverage has no gaps over the whole keyspace.
    pub(crate) fn coverage_complete(&self) -> bool {
        let complete = self
            .coverage
            .gaps(&Endpoint::whole_keyspace())
            .next()
            .is_none();
        debug_assert!(
            !complete || (self.work.is_empty() && self.requested.is_empty()),
            "`requested` is disjoint from complete coverage, so nothing is in flight"
        );
        complete
    }

    /// Reserve `range` under a fresh id. The range must be disjoint from
    /// both maps.
    fn reserve(&mut self, range: Range<Endpoint>) -> WorkId {
        debug_assert!(
            !self.requested.overlaps(&range),
            "reservation overlaps in-flight space"
        );
        debug_assert!(
            !self.coverage.overlaps(&range),
            "reservation overlaps verified coverage"
        );
        let id = self.next_id;
        self.next_id = id.next();
        self.requested.insert(range.clone());
        self.work.insert(
            id,
            InFlight {
                range,
                hash: self.target.clone(),
            },
        );
        id
    }

    /// The widest cold hole (coverage gaps minus in-flight ranges); ties go
    /// to the leftmost.
    fn widest_cold_hole(&self) -> Option<Range<Endpoint>> {
        let mut widest: Option<(Width, Range<Endpoint>)> = None;
        for gap in self.coverage.gaps(&Endpoint::whole_keyspace()) {
            for hole in self.requested.gaps(&gap) {
                let hole_width = width(&hole);
                if widest.as_ref().is_none_or(|(w, _)| hole_width > *w) {
                    widest = Some((hole_width, hole));
                }
            }
        }
        widest.map(|(_, hole)| hole)
    }

    /// True iff some space is neither verified nor in flight.
    fn has_cold_hole(&self) -> bool {
        self.coverage
            .gaps(&Endpoint::whole_keyspace())
            .any(|gap| self.requested.gaps(&gap).next().is_some())
    }

    /// Every cold hole, in key order, for asserting the exact cold space.
    /// Production code asks only for [`Self::widest_cold_hole`] or
    /// [`Self::has_cold_hole`], which avoid allocating. The coverage gaps
    /// are collected because each `requested.gaps(gap)` borrows its gap.
    #[cfg(test)]
    fn cold_holes(&self) -> Vec<Range<Endpoint>> {
        let whole = Endpoint::whole_keyspace();
        let gaps: Vec<Range<Endpoint>> = self.coverage.gaps(&whole).collect();
        gaps.iter()
            .flat_map(|gap| self.requested.gaps(gap))
            .collect()
    }
}

/// The width of `range`; zero for an empty or reversed range.
fn width(range: &Range<Endpoint>) -> Width {
    (&range.end - &range.start).unwrap_or_default()
}

#[cfg(test)]
mod tests;
