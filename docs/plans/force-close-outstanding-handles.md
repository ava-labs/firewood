# Force-close outstanding proposals/revisions on `Database.Close`

## Problem

Closing a Firewood database from Go currently fails with
`ErrActiveKeepAliveHandles` if any `Proposal`, `Revision`, or `Reconstructed`
view has not been explicitly dropped. Callers want an opt-in to have `Close`
forcibly drop those outstanding handles instead of erroring out.

The Rust side does **not** need changes: `Db::close(self)` (firewood/src/db.rs:398)
delegates to `RevisionManager::close` and does not reject outstanding
proposals/revisions on its own. The error originates in
`Database.Close` (ffi/firewood.go:469), which waits on
`db.outstandingHandles sync.WaitGroup` and times out via the supplied context.

The Go FFI wraps `Proposal`, `Revision`, and `Reconstructed` in `handle[T]`
with a working `Drop()` (ffi/keepalive.go:48); we lack a way to enumerate the
live handles so `Close` can drop them.

**Iterators are a pre-existing gap.** `Iterator` (ffi/iterator.go:36) holds a
raw `*C.IteratorHandle` directly and does **not** participate in the keepalive
`WaitGroup` at all. On the Rust side the iterator holds its own `NodeStore`
Arc so it can outlive its parent `Revision`/`Proposal`, but on the Go/FFI side
nothing prevents `Database.Close` from running while iterators are live —
calling `Iterator.Next` afterwards is a use-after-free. This plan brings
iterators into the same keepalive + live-handle registry as the other
handle types so they are both *waited on* and *force-droppable*.

## Design

### API

Convert `Close` to variadic:

```go
func (db *Database) Close(ctx context.Context, opts ...CloseOption) error
```

Variadic args are backward compatible — every existing `db.Close(ctx)` call
site compiles unchanged.

Add:

```go
type CloseOption func(*closeConfig)

func WithForceCloseHandles() CloseOption
```

When set, `Close` forcibly drops every registered live handle before invoking
`C.fwd_close_db`. Without it, behavior is unchanged (graceful wait on the
`WaitGroup`, returning `ErrActiveKeepAliveHandles` if `ctx` fires first).

### Tracking outstanding handles

`Database` gains a registry alongside the existing `WaitGroup`:

```go
type Database struct {
    // ... existing fields ...
    outstandingHandles sync.WaitGroup           // unchanged

    liveHandlesMu sync.Mutex
    liveHandles   map[*databaseKeepAliveHandle]func() error // value: Drop func
}
```

`createHandle[T]` (ffi/keepalive.go:34) takes the `*Database` (not just the
`WaitGroup`) and:

1. Calls `keepAliveHandle.init(&db.outstandingHandles)` (existing behavior).
2. Registers `(&h.keepAliveHandle, dropFn)` into `db.liveHandles` under
   `liveHandlesMu`.

The registered `dropFn` is the **outer** type's `Drop` method (e.g.
`Iterator.Drop`, `Proposal.Drop`), not `handle.Drop` directly. The closure
captures the typed `free func(T)` so we don't need to track the handle's
type in the registry — `map[*databaseKeepAliveHandle]func() error` is
sufficient. Using the outer `Drop` matters for `Iterator`, which must call
`freeCurrentAllocation()` before freeing its handle (ffi/iterator.go:190);
registering `handle.Drop` directly would leak the borrowed batch on force
close. Registration therefore happens at the call site of `createHandle`,
where the outer `Drop` is in scope, rather than inside `createHandle` itself.

`databaseKeepAliveHandle.disown` (ffi/keepalive.go:106) deregisters from the
map after a successful `Done()`. Pass the `*Database` (or just the map+mutex)
into the keepalive struct via `init`.

Why both a `WaitGroup` and a map? The `WaitGroup` already powers the graceful
`ctx`-bounded wait; preserving it avoids reworking that path. The map is only
read on force close and on debug introspection.

### Force-close semantics

In `Close`:

1. `db.handleLock.Lock()` — already done; prevents new handles being created
   concurrently (constructors take `handleLock.RLock`).
2. If `WithForceCloseHandles()` is set:
   - Acquire `liveHandlesMu`, snapshot the values into a local slice, release.
   - For each `dropFn`, call it; collect errors with `errors.Join`. Each
     `handle.Drop` already takes `keepAliveHandle.mu.Lock`, which serializes
     against any in-flight method on that handle (handles take `mu.RLock` for
     the duration of every C call).
   - The `WaitGroup` counter must reach zero as a side effect of these drops.
3. Run the existing graceful wait on `outstandingHandles` with `ctx`. With
   force close it returns immediately; without it, behavior is unchanged.
4. Acquire `commitLock`, call `C.fwd_close_db`, null out `db.handle`.
5. Return joined errors from step 2 (if any) merged with the close error.

### Concurrent in-flight calls

`handle.keepAliveHandle.mu` (RWMutex) is the existing serialization point:

- Method calls (`Get`, `Iterator`, etc.) hold `mu.RLock` for the C call.
- `Drop` / `disown` take `mu.Lock`.

Force-dropping while a method runs simply blocks the drop until the in-flight
call returns — same guarantee as a manual `revision.Drop()` race today. Verify
with a stress test, but no new locking is needed.

### Reconstructed views

`Reconstructed` documents an `A < B` lock order for `keepAliveHandle.mu` vs.
`rootMu` (ffi/reconstructed.go:34). Force-close only takes `keepAliveHandle.mu`
via `Drop`, so the existing order holds. Include a `Reconstructed` in the
force-close test.

## Versioning

Bump `firewood-ffi` from `0.4.0` to `0.5.0` (root `Cargo.toml` and
`ffi/Cargo.toml`). Variadic Go signature is source-compatible, but adding a
new public option + behavior change warrants a minor bump on a 0.x crate.
Update `CHANGELOG.md` accordingly.

## Implementation steps

1. **ffi/keepalive.go**: thread `*Database` (or registry handle) through
   `createHandle` and `databaseKeepAliveHandle.init`; register/deregister in
   `init`/`disown`.
2. **ffi/firewood.go**:
   - Add `liveHandlesMu` + `liveHandles` to `Database`; init in `New`.
   - Add `CloseOption`, `closeConfig`, `WithForceCloseHandles`.
   - Change `Close` to variadic; implement force-drop branch.
   - Update doc comment on `Close` and `ErrActiveKeepAliveHandles`.
3. **ffi/proposal.go, revision.go, reconstructed.go**: pass `db` into the
   `createHandle` call sites (already have `db` in scope).
4. **ffi/iterator.go**: convert `Iterator` to use `*handle[*C.IteratorHandle]`
   like the other types, so it participates in `outstandingHandles` and
   `liveHandles`. Iterator construction sites (`Revision.Iter`,
   `Proposal.Iter`) need access to the parent `*Database`; both already hold
   it transitively (`r.db`, `p.db`). Update `Iterator.Drop` to delegate to
   `handle.Drop` after `freeCurrentAllocation`. Add `runtime.AddCleanup` for
   parity with proposal/revision finalizers.
   - Note: `Iterator` outliving its parent `Revision`/`Proposal` is fine —
     the Rust side already holds an independent `NodeStore` Arc. The
     keepalive only ties the iterator to the *database*, not the parent
     handle, which matches today's documented semantics.
5. **ffi/firewood_test.go**: new tests
   - `TestClose_ForceDropsOutstandingProposal`
   - `TestClose_ForceDropsOutstandingRevision`
   - `TestClose_ForceDropsOutstandingReconstructed`
   - `TestClose_ForceDropsOutstandingIterator`
   - `TestClose_WithoutForce_BlocksOnIterator` (regression: today this
     succeeds and is unsafe; after this change it must wait on the iterator)
   - `TestClose_WithoutForce_StillReturnsErrActiveKeepAliveHandles` (regression)
   - `TestClose_ForceWithConcurrentGet` (stress: goroutine doing `Get`
     while `Close(ctx, WithForceCloseHandles())` runs).
6. **CHANGELOG.md, ffi/Cargo.toml, Cargo.toml**: bump to `0.5.0`.
7. Run `cargo fmt`, `cargo nextest run --workspace --features ethhash,logger
   --all-targets`, `cargo clippy --workspace --features ethhash,logger
   --all-targets`, `cargo doc --no-deps`. Run `go test ./...` in `ffi/`.

## Out of scope

- No Rust API change. `Db::close` already accepts being called while
  `Arc`-shared proposals exist; we are only fixing the FFI/Go-layer wait.
- No change to finalizers; `runtime.AddCleanup` paths still trigger
  `handle.Drop`, which now also deregisters from the live-handle map.
- Not exposing the live-handle count as a public API (could be added later
  for debugging if useful).
- **Proofs (`RangeProof`, `ProposedChangeProof`) are not migrated to
  `handle[T]` and are not entered into the live-handle registry.** A bound
  `proof.Free` in the registry would keep the proof reachable and prevent
  its GC finalizer from running (`TestRangeProofFinalizerCleanup` would
  hang). They still increment the keep-alive WaitGroup, so graceful Close
  waits on them; `WithForceCloseHandles` will not auto-drop a
  still-referenced proof. The change-proof family is being redesigned, so
  the handle[T] migration is deferred until that lands.

## Risks

- **API surface change** on `Close`: variadic mitigates source breakage but
  reflective callers (rare) could notice.
- **Drop-while-in-flight**: relies on existing `keepAliveHandle.mu` semantics;
  covered by stress test.
- **Map churn**: every handle creation/drop now touches `liveHandlesMu`. The
  mutex is uncontended in the common case (per-handle lifecycle, not per-op),
  so cost should be negligible, but worth a benchmark spot-check on the
  insert benchmark if numbers regress.
