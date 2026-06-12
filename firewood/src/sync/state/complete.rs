// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Retiring work items: recording a verified proof's extent in
//! [`SyncState`] and handing out its continuation.

use std::ops::Range;

use firewood_storage::TrieHash;

use super::InFlight;
use super::SyncError;
use super::SyncState;
use super::WorkId;
use crate::sync::endpoint::Endpoint;

/// How far a verified proof reached, derived by the caller: a proof whose
/// proven right edge falls short of the requested end becomes
/// [`ProofExtent::Truncated`] with `last_key` set to that edge; any other
/// proof becomes [`ProofExtent::Full`].
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ProofExtent {
    /// The proof covered the entire requested range.
    Full,
    /// The proof stopped early; `last_key` is the greatest key it proved.
    Truncated {
        /// The verifier's proven right edge: the greatest key the proof
        /// covers, inside the requested range. Not the resume key from
        /// `find_next_key_after_range_proof`.
        last_key: Box<[u8]>,
        /// Where to split the unfetched rest if a worker is idle (see
        /// [`crate::sync::endpoint::proof_cut`]).
        cut: Option<Endpoint>,
    },
}

/// Result of [`SyncState::complete`].
#[derive(Debug, PartialEq, Eq)]
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

impl SyncState {
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

    /// Retire item `id` and record its verified proof's extent.
    ///
    /// A truncated proof covers `[range.start, last_key]`, inclusive of
    /// `last_key`. The next work item requests `[succ(last_key), range.end)`,
    /// which does not overlap it, or only `[succ(last_key), cut)` when a work
    /// slot would otherwise sit idle; `[cut, range.end)` then goes cold.
    ///
    /// # Errors
    ///
    /// - [`SyncError::StaleWorkId`] iff `id` is not outstanding, e.g. a
    ///   duplicate submission. Nothing changes.
    /// - [`SyncError::TruncationKeyOutOfRange`] iff a [`ProofExtent::Truncated`]
    ///   key falls outside the item's range. The id is retired and its range
    ///   goes back to cold space.
    pub(crate) fn complete(
        &mut self,
        id: WorkId,
        extent: ProofExtent,
    ) -> Result<Completed, SyncError> {
        let InFlight { range, hash } = self.work.remove(&id).ok_or(SyncError::StaleWorkId(id))?;
        self.requested.remove(range.clone());

        match extent {
            ProofExtent::Full => {
                self.coverage.insert(range, hash);
                Ok(Completed::Exhausted)
            }
            ProofExtent::Truncated { last_key, cut } => {
                if !range.contains(&Endpoint::Key(last_key.clone())) {
                    return Err(SyncError::TruncationKeyOutOfRange(id));
                }
                // No key lies between `last_key` and its successor, and
                // `last_key < range.end`, so `resume <= range.end`.
                let resume = Endpoint::Key(crate::proofs::lex_successor(&last_key));
                self.coverage.insert(range.start..resume.clone(), hash);
                if resume == range.end {
                    return Ok(Completed::Exhausted);
                }
                // The continuation takes a slot; any slot left over is an idle
                // worker. A cut at or past `range.end` (a subtree starting at
                // the requested end) leaves nothing to shed.
                let idle_worker = self.work.len().saturating_add(1) < self.task_limit.get();
                let shed_at = cut.filter(|cut| idle_worker && resume < *cut && *cut < range.end);
                let wakeup_neighbor = shed_at.is_some();
                let kept = resume..shed_at.unwrap_or(range.end);
                let id = self.reserve(kept.clone());
                Ok(Completed::Continue {
                    id,
                    range: kept,
                    wakeup_neighbor,
                })
            }
        }
    }
}
