---
title: "State Sync: Phased Delivery Plan"
status: proposed
category: [sync, ffi]
authors: [rkuris]
---

# State Sync: Phased Delivery Plan

Implementation phasing for the design in [State Sync](2026-06-11-state-sync.md)
(referred to below by section).
The design is one effort; this plan sequences it into Phase 1 and two
follow-on phases. Data structures are shaped end-to-end for the full design
(per-region hash in `coverage` exists *for* Phase 2), so no Phase 1 work is
throwaway.

## Phase 1 — static range-proof sync to a fixed target

Design sections: Core idea, Endpoint key type, Data structures, Hole
selection, Concurrency model, FFI surface, Completion detection, Error
handling, Lifecycle.

Shippable standalone: today every retarget already degrades to full range
proofs (the change-proof stubs return `ErrInsufficientHistory`), so a
fixed-target syncer is no functional regression — it replaces the
avalanchego orchestrator (`database/merkle/sync`) plus the Firewood adapter
(`database/merkle/firewood/syncer`) with the new ownership split.

Suggested commit/PR sequence (stacked, `N/8` titles):

1. **`Endpoint` type + split arithmetic.** `Endpoint::{Key, Max}`,
   `Ord`, the binary-fraction midpoint and N-way slice routines
   (shortest-key-strictly-inside emission, half-bit carry). Pure
   functions, exhaustive unit tests (midpoint of `[a, Max)`,
   variable-length keys, N-way tiling with no gap/overlap for prime N).
2. **`SyncState` + hole selection.** Add the `rangemap` dependency.
   `coverage: RangeMap<Endpoint, Hash>`,
   `requested: RangeSet<Endpoint>`, `work: HashMap<WorkId, InFlight>`,
   seed/refeed logic, cold-space computation
   (`gaps(coverage) − requested`). New module `firewood/src/sync/`.
   No FFI, no I/O — state-machine unit tests drive `get_work`-shaped
   calls directly.
3. **Submit path + completion.** Verify (`verify_range_proof_structure`,
   no per-proof key cap) → merge clamped to the proven right edge →
   `commit_with_rebase` (mutex **not** held across commit) → map update
   under the mutex: retire id, insert `Verified`, `Continue`/`Exhausted`
   decision, large-remainder guard + `wakeup_neighbor`, `InvalidProof`
   keeps the region reserved under the same id. The truncation
   precondition (`a ≤ L < b`) is re-checked in release builds. `Done` =
   latest committed root == target, checked on `get_work`; a distinct
   `CoverageRootMismatch` error (unique, not generic `Err`). `finish`
   returns the latest root.
4. **FFI surface.** `fwd_start_sync` / `fwd_get_work` / `fwd_submit_work` /
   `fwd_finish_sync` / `fwd_free_sync`, `Box<SyncHandle>` + tagged result enums following
   the existing `ProposalResult` pattern. Contract to preserve in doc
   comments: `fwd_get_work` is a pure function of durable `SyncState`,
   never blocks, idempotent to re-call — the Go condvar argument depends
   on it.
5. **Go sync handle.** `Database.StartSync`, `Sync.Finish`, and `Drop`
   over the FFI, holding a keep-alive lease on the database, with
   lifecycle tests (rejected arguments, `Close` blocked by a live sync,
   `Finish`/`Drop` idempotency).
6. **Go fixed pool.** `Sync.Run`: `task_limit` goroutines spawned up
   front, park/wake on one `sync.Cond`, `fwd_get_work` called **under
   `mu`** (the lost-wakeup argument in the design is load-bearing —
   implement the three listed requirements verbatim). `InvalidProof` →
   re-fetch same range, different peer. Each fetch returns a per-response
   `report` callback that the pool calls after the submit (accepted or
   rejected), for peer scoring and code-hash follow-up. Fake-source pool
   tests plus FFI-backed end-to-end tests, including a restart.
7. **Rust hostile-proof tests.** In-process two-database sync harness:
   generate range proofs from a source `Db`, corrupt or substitute them,
   and assert `InvalidProof` with the region still reserved, then retry
   the same id to completion.
8. **Rust restart, concurrency, and random tests.** Restart-from-scratch
   idempotency, concurrent workers under revision reaping,
   large-remainder shedding with real truncated proofs, `task_limit = 1`,
   and randomized round trips asserting final root == target.

Phase 1 explicitly excludes: pivot, change proofs, `WorkKind`, persisted
coverage, smart `find_next_key`. Restart = start over (full cold-sync
cost with today's naive `find_next_key` — accepted, see Lifecycle).

Open Phase 1 decision (deferred to Phase 3 or later in the design): revision-churn
batching. Start with one commit per proof and revisit with measurements.
Verification takes no per-proof key cap, so request size is avalanchego's
choice.

## Phase 2 — pivoting + change proofs

Design sections: Pivoting, Dispatch priority, Multi-pivot, In-flight
requests at pivot time, Network expiry, Pivot FFI.

- `fwd_pivot(handle, H_new)`: flips target, `Broadcast`s parked workers,
  `Err` after `Done`.
- Work items gain `WorkKind` (range | change) + the `from` hash for
  change work.
- Dispatch priority at pull points: change-proof upgrade > own range
  continuation > new range hole.
- In-flight `H_old` proofs commit as `Verified(H_old)` then queue as
  upgrade work; multi-pivot (several historical hashes in `coverage`)
  supported.
- `fwd_submit_work` gains `ChangeProofUnavailable`: region goes cold,
  re-dispatched as a range proof to the current target (liveness floor).
- Completion check restricted to `target` (the Phase 1 "no gaps" form stops
  being sufficient).
- Retires the avalanchego `ErrInsufficientHistory` stubs. The Rust
  change-proof primitives (`Db::change_proof`, `verify_change_proof`,
  `apply_change_proof_to_parent`) already exist — this is dispatch and
  binding work, not trie logic.
- Test focus: deterministic replacements for the skipped-as-flaky
  merkledb retarget tests — pivot-at-completion expects `Err`,
  bounded-churn multi-pivot convergence.

## Phase 3 — durability and optimization

Design sections: Future possibilities, Deferred to Phase 3 or later,
Lifecycle (probe floor).

- **Persist `coverage`.** Kills the warm-restart probe floor; the same
  lever enables the known-good `R1 → R2` post-`Done` advance (seed
  durable coverage as one `Verified(R1)` span, run the upgrade pass).
- **Known-good advance** as a user-facing feature (avalanchego behavior
  for this mode TBD).
- **Smart `find_next_key`.** Region-bounded local-diff skip —
  tracked independently; the orchestration relies only on its public
  contract.
- **Metrics** via the existing FFI metrics context: coverage %,
  outstanding count, verification counters.
- **Tuning:** request size, batching proofs per commit
  (revision churn), structured `InvalidProof` reasons for peer scoring,
  explicit peer-affinity lineage if the implicit same-goroutine affinity
  proves insufficient.
