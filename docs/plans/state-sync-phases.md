# State sync: phased delivery plan

Implementation phasing for the design in [`state-sync.md`](./state-sync.md)
(referred to below by section).
The design is one effort; this plan sequences it into an MVP and two
follow-on phases. Data structures are shaped end-to-end for the full design
(per-region hash in `coverage` exists *for* Phase 2), so no MVP work is
throwaway.

## MVP — static range-proof sync to a fixed target

Design sections: Core idea, Endpoint key type, Data structures, Hole
selection, Concurrency model, FFI surface, Completion detection, Error
handling, Lifecycle.

Shippable standalone: today every retarget already degrades to full range
proofs (the change-proof stubs return `ErrInsufficientHistory`), so a
fixed-target syncer is no functional regression — it replaces the
avalanchego orchestrator (`database/merkle/sync`) plus the Firewood adapter
(`database/merkle/firewood/syncer`) with the new ownership split.

Suggested commit/PR sequence (stacked, `N/7` titles):

1. **`Endpoint` type + split arithmetic.** `Endpoint::{Key, Max}`,
   `Ord`, the binary-fraction midpoint and N-way slice routines
   (shortest-key-strictly-inside emission, half-bit carry). Add the
   `rangemap` dependency. Pure functions, exhaustive unit tests
   (midpoint of `[a, Max)`, variable-length keys, N-way tiling with no
   gap/overlap for prime N).
2. **`SyncState` + hole selection.** `coverage: RangeMap<Endpoint, Hash>`,
   `requested: RangeSet<Endpoint>`, `work: HashMap<WorkId, InFlight>`,
   seed/refeed logic, cold-space computation
   (`gaps(coverage) − requested`). New module `firewood/src/sync/`.
   No FFI, no I/O — state-machine unit tests drive `get_work`-shaped
   calls directly.
3. **Submit path.** Verify (`verify_range_proof_structure`) → propose →
   `commit_with_rebase` (mutex **not** held across commit) → map update
   under the mutex: retire id, insert `Verified`, `Continue`/`Exhausted`
   decision, large-remainder guard + `wakeup_neighbor`, `InvalidProof`
   keeps the region reserved under the same id. The
   `find_next_key` precondition (`None` at end, else `a < r < b`) is
   relied on, not re-checked.
4. **Completion + finish.** `Done` = latest committed root == target,
   checked on `get_work`. Distinct `CoverageRootMismatch` error code
   (unique, not generic `Err`) + debug assertion. `finish_sync` frees the
   handle and returns the latest root.
5. **FFI surface.** `fw_start_sync` / `fw_get_work` / `fw_submit_work` /
   `fw_finish_sync`, `Box<SyncHandle>` + tagged result enums following
   the existing `ProposalResult` pattern. Contract to preserve in doc
   comments: `fw_get_work` is a pure function of durable `SyncState`,
   never blocks, idempotent to re-call — the Go condvar argument depends
   on it.
6. **Go fixed pool.** `task_limit` goroutines spawned up front, park/wake
   on one `sync.Cond`, `fw_get_work` called **under `mu`** (the
   lost-wakeup argument in the design is load-bearing — implement the
   three listed requirements verbatim). `InvalidProof` → re-fetch same
   range, different peer.
7. **End-to-end tests.** In-process two-database sync harness: generate
   range proofs from a source `Db`, feed them through the orchestration,
   assert final root == target. Cases: truncated proofs, hostile proofs
   (`InvalidProof` retry), large-remainder shedding, `task_limit = 1`,
   restart-from-scratch idempotency. Rust integration tests for the core;
   one Go test through the FFI boundary.

MVP explicitly excludes: pivot, change proofs, `WorkKind`, persisted
coverage, smart `find_next_key`. Restart = start over (full cold-sync
cost with today's naive `find_next_key` — accepted, see Lifecycle).

Open MVP decisions (deferred-for-v1 in the design): `max_length` per
request (fixed vs. `fw_start_sync` parameter) and revision-churn batching.
Pick the simplest (fixed constant, one commit per proof) and revisit with
measurements.

## Phase 2 — pivoting + change proofs

Design sections: Pivoting, Dispatch priority, Multi-pivot, In-flight
requests at pivot time, Network expiry, Pivot FFI.

- `fw_pivot(handle, H_new)`: flips target, `Broadcast`s parked workers,
  `Err` after `Done`.
- Work items gain `WorkKind` (range | change) + the `from` hash for
  change work.
- Dispatch priority at pull points: change-proof upgrade > own range
  continuation > new range hole.
- In-flight `H_old` proofs commit as `Verified(H_old)` then queue as
  upgrade work; multi-pivot (several historical hashes in `coverage`)
  supported.
- `fw_submit_work` gains `ChangeProofUnavailable`: region goes cold,
  re-dispatched as a range proof to the current target (liveness floor).
- Completion check restricted to `target` (Phase-1 "no gaps" form stops
  being sufficient).
- Retires the avalanchego `ErrInsufficientHistory` stubs. The Rust
  change-proof primitives (`Db::change_proof`, `verify_change_proof`,
  `apply_change_proof_to_parent`) already exist — this is dispatch and
  binding work, not trie logic.
- Test focus: deterministic replacements for the skipped-as-flaky
  merkledb retarget tests — pivot-at-completion expects `Err`,
  bounded-churn multi-pivot convergence.

## Phase 3 — durability and optimization

Design sections: Future work, Deferred for v1, Lifecycle (probe floor).

- **Persist `coverage`.** Kills the warm-restart probe floor; the same
  lever enables the known-good `R1 → R2` post-`Done` advance (seed
  durable coverage as one `Verified(R1)` span, run the upgrade pass).
- **Known-good advance** as a user-facing feature (avalanchego behavior
  for this mode TBD).
- **Smart `find_next_key` (#352).** Region-bounded local-diff skip —
  tracked independently; the orchestration relies only on its public
  contract.
- **Metrics** via the existing FFI metrics context: coverage %,
  outstanding count, verification counters.
- **Tuning:** `max_length` selection, batching proofs per commit
  (revision churn), structured `InvalidProof` reasons for peer scoring,
  explicit peer-affinity lineage if the implicit same-goroutine affinity
  proves insufficient.
