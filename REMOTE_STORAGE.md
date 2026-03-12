# Remote Storage: Transparent `ffi.DB` Interface

This document describes all changes made to implement a transparent remote
backend via the `ffi.DB` interface, allowing callers to swap between a local
FFI-backed database and a remote gRPC-backed database with minimal code changes.

## Motivation

Programs using `*ffi.Database` call methods like `Get`, `Update`, `Propose`,
`Root`, `Close`. We want callers to be able to swap the local FFI backend for a
remote gRPC backend by adopting the `ffi.DB` interface type and changing the
constructor. All remote operations are cryptographically verified: reads via
Merkle proofs, writes via witness proofs, iteration via server-side batching.

### Constraints

- **No circular dependency**: `ffi` cannot import `remote`, so interfaces live
  in `ffi` and the remote adapter lives in `remote`.
- **Existing code untouched**: `*ffi.Database`, `*ffi.Proposal`, and
  `*ffi.Iterator` are not modified.
- **`context.Context` on the interface**: Remote calls need context for
  cancellation/timeouts; local wrappers accept but ignore it.

## Files Created

### `ffi/db.go` — Interfaces + LocalDB adapter

Defines three interfaces and their local (FFI-backed) implementations:

- **`DB`** interface: `Get`, `Update`, `Propose`, `Revision`,
  `LatestRevision`, `Root`, `Close` — all with `context.Context` except `Root`.
- **`DBProposal`** interface: `Root`, `Commit`, `Drop`, `Get`, `Iter`,
  `Propose` — supports reading from and chaining on uncommitted proposals.
- **`DBRevision`** interface: `Root`, `Get`, `Iter`, `Drop` — read-only access
  to a committed revision at a specific root hash.
- **`DBIterator`** interface: `Next`, `Key`, `Value`, `Err`, `Drop` — standard
  forward-only iteration.
- **`LocalDB`** struct wrapping `*Database`, **`localProposal`** wrapping
  `*Proposal`, **`localRevision`** wrapping `*Revision`, **`localIterator`**
  wrapping `*Iterator` — thin wrappers that delegate to the underlying FFI
  types and ignore `context.Context`.
- **`NewLocalDB(db *Database) DB`** — constructor returning the interface type.
- Compile-time checks: `var _ DB = (*LocalDB)(nil)`, etc.

### `ffi/remote/db.go` — RemoteDB + remoteProposal + remoteIterator

Implements the same three interfaces over gRPC with cryptographic verification:

- **`RemoteDB`** wraps a `*Client`. Created via `NewRemoteDB(ctx, addr,
  trustedRoot, depth)` which dials, bootstraps (fetches + verifies truncated
  trie), and returns `ffi.DB`.
- **`remoteProposal`** holds `proposalID`, `root`, `newTrie`
  (`*ffi.TruncatedTrie` from witness verification), gRPC client, `parentRoot`
  pointer (for Commit to propagate the new root up), `witness`
  (`*ffi.WitnessProof`, kept alive for cache invalidation at commit time),
  `rc` (`*ffi.RemoteClient`, for `CommitTrie`/`VerifyWitness`), `depth`, `mu`
  (points to Client's RWMutex), and `expectedCumulativeOps`.
  - `Commit` sends `CommitProposal` RPC then calls `rc.CommitTrie(newTrie,
    witness)` which atomically swaps the internal trie, invalidates affected
    cache entries via the witness's batch ops, and returns the new root hash.
    Updates `*parentRoot` to propagate the root up the chain.
  - `Drop` frees `newTrie` and `witness`, then sends `DropProposal` RPC.
    Suppresses gRPC `NotFound` errors since they indicate the server already
    cleaned up (e.g., via GC or a prior Drop).
  - `Get` sends `GetValue` RPC with the proposal's root hash, verifies the
    single-key Merkle proof client-side.
  - `Iter` sends `IterBatch` RPC, returns a `remoteIterator`.
  - `Propose` sends `CreateProposal` RPC with `parent_proposal_id` set,
    verifies witness against `rc` (the committed trie inside the
    `RemoteClient`), returns child `remoteProposal`.
- **`remoteIterator`** holds current batch of `KeyValuePair` from the server,
  cursor index, `hasMore` flag, gRPC client, caller's `context.Context`, and
  verified proposal root hash.
  - `Next` advances cursor; if batch exhausted and `hasMore`, fetches next batch
    via `IterBatch` RPC using the caller's context (start_key = last key +
    `\x00`). Each batch is verified via range proof before being consumed.
  - `Drop` is a no-op (server creates a fresh iterator per `IterBatch` call).

Helper functions:

- `batchOpsToProto` — converts `[]ffi.BatchOp` to `[]*pb.BatchOperation`.
- `verifyWitnessFromResponse` — deserializes + verifies witness proof from a
  `CreateProposalResponse` against a given base trie.

### `ffi/db_test.go` — LocalDB tests

9 tests exercising the `ffi.DB`/`ffi.DBProposal`/`ffi.DBRevision`/
`ffi.DBIterator` interfaces through `LocalDB`:

- `TestNewLocalDB` — create, update, get, verify root changes, get
  non-existent key.
- `TestLocalDBPropose` — propose, read from proposal, verify DB isolation,
  commit, verify root.
- `TestLocalDBProposalChain` — chain two proposals (p1 then p2 on p1), read
  all keys from p2, commit chain in order.
- `TestLocalDBRevision` — insert data, get revision at root, verify Get/Iter/
  Root/Drop, verify non-existent keys return nil.
- `TestLocalDBRevisionNotFound` — revision with fabricated root returns error.
- `TestLocalDBPrefixDelete` — prefix delete via Update.
- `TestLocalDBProposalIter` — iterate from a proposal, verify lexicographic
  order and completeness.
- `TestLocalDBLatestRevision` — insert data, call `LatestRevision`, verify
  root matches `db.Root()`, verify `Get` returns correct data.
- `TestLocalDBLatestRevisionEmpty` — empty DB returns error.

### `ffi/remote/db_test.go` — RemoteDB tests

32 tests exercising the interfaces through `RemoteDB` over a real gRPC
server/client:

- `TestNewRemoteDB` — bootstrap, get, update through `ffi.DB`.
- `TestRemoteDBPropose` — propose, verify root differs, commit, verify root
  updated.
- `TestRemoteDBProposalGet` — read new and pre-existing keys from an
  uncommitted proposal.
- `TestRemoteDBProposalIter` — iterate from an uncommitted proposal, verify
  order.
- `TestRemoteDBProposalChain` — chain two proposals remotely, verify distinct
  roots, commit chain.
- `TestRemoteDBProposeDrop` — propose then drop, verify DB root unchanged.
- `TestRemoteDBPrefixDelete` — prefix delete via Update.
- `TestRemoteDBPrefixDeletePropose` — prefix delete via Propose.
- `TestRemoteDBPrefixDeleteChained` — prefix delete in chained proposal.
- `TestRemoteDBPrefixDeleteMixed` — mixed Put + PrefixDelete batch.
- `TestNewRemoteDBBadRoot` — wrong trusted root fails at construction.
- `TestRemoteDBProposalIterTamperedValue` — tampered response values ignored
  (proof pairs used instead).
- `TestRemoteDBProposalIterMissingProof` — missing proof rejected.
- `TestRemoteDBProposalIterTamperedProof` — corrupted proof bytes rejected.
- `TestRemoteDBProposalIterMultiBatch` — 300+ keys verified across batch
  boundaries.
- `TestRemoteDBRevisionGet` — two commits, revision at root1 sees old data,
  not data from root2.
- `TestRemoteDBRevisionIter` — iterate all keys from a committed revision,
  verify order and values.
- `TestRemoteDBRevisionIterStartKey` — iterate from a specific start key
  within a revision.
- `TestRemoteDBRevisionIterMultiBatch` — insert 300 keys, verify pagination
  across 256-key batch boundary.
- `TestRemoteDBRevisionBadRoot` — revision with fabricated root; Get/Iter
  return server error (creation itself succeeds since it's a no-op).
- `TestRemoteDBRevisionAfterUpdate` — create revision at root1, then Update
  to root2; revision at root1 still works.
- `TestRemoteDBRevisionDrop` — Drop is no-op; can create another revision
  with same root afterward.
- `TestRemoteDBLatestRevision` — bootstrap + update, `LatestRevision` returns
  revision with same root as `db.Root()`, verify `Get` works.
- `TestRemoteDBLatestRevisionAfterUpdate` — update changes what
  `LatestRevision` returns.
- `TestRemoteDBLatestRevisionMatchesRoot` — `LatestRevision().Root()` ==
  `db.Root()`.
- `TestServerProposalTTLExpiry` — proposal reaped after TTL; commit returns
  "not found".
- `TestServerProposalTTLNotExpired` — commit before TTL succeeds.
- `TestServerStop` — `Stop()` reaps all proposals.
- `TestRemoteDBProposalExpiredOnServer` — client surfaces clear errors from
  `Commit`/`Iter`/`Propose` when server GC reaps proposal; `Drop` succeeds
  since the server already cleaned up.
- `TestRemoteDBProposalIterProposalOnlyKey` — range proof from proposal state
  includes keys that exist only in the proposal.
- `TestRemoteDBIterMidPaginationExpiry` — iterator returns error when GC reaps
  proposal between batches.
- `TestRemoteDBChainedProposalParentReaped` — chained proposal operations fail
  when GC reaps the parent.

## Files Modified

### `ffi/src/remote.rs` — Rust FFI fix for `fwd_get_with_proof`

**Change**: In `fwd_get_with_proof` (line ~522), replaced:

```rust
let revision = db.db().revision(api_hash).map_err(|e| e.to_string())?;
```

with:

```rust
let view = db.get_root(root_hash.into()).map_err(|e| e.to_string())?;
```

**Why**: `db.db().revision()` only finds *committed* revisions.
`db.get_root()` calls `db.view()` internally, which checks proposals first,
then committed revisions. This is required for `remoteProposal.Get()` to work:
the client sends `GetValue` with the proposal's root hash, and the server
needs to find the live (uncommitted) proposal to generate the proof.

The `fwd_db_range_proof` function already used `db.get_root()` for the same
reason; this change makes `fwd_get_with_proof` consistent.

**Also removed** the now-unused `DbView as _` import (line 17). The `Db as _`
import remains because `fwd_create_truncated_trie` still calls
`db.db().revision()` (intentionally — truncated tries are only created from
committed revisions for bootstrapping).

**Note**: `fwd_generate_witness` was **not** changed to use `get_root` because
`generate_witness` requires `&T: TrieReader + HashedNodeReader`, which is
satisfied by `NodeStore` (the concrete type from `revision()`) but not by
`dyn DynDbView` (the trait object from `get_root()`). This Rust type constraint
is the reason for the cumulative-ops approach described below.

### `ffi/remote/proto/remote.proto` — New RPCs and messages

Added to the `FirewoodRemote` service:

- **`rpc DropProposal`** — drops a pending proposal without committing, freeing
  server-side FFI handles.
- **`rpc IterBatch`** — paginated iteration over a proposal's key-value pairs.

New/modified messages:

- `CreateProposalRequest`: added `optional uint64 parent_proposal_id = 4` for
  chained proposals.
- `DropProposalRequest` / `DropProposalResponse` (new).
- `IterBatchRequest` (new): `proposal_id`, `start_key`, `batch_size`.
- `KeyValuePair` (new): `key`, `value`.
- `IterBatchResponse` (new): `repeated KeyValuePair pairs`, `bool has_more`.

Go proto code was regenerated after changes (`proto/remote.pb.go` and
`proto/remote_grpc.pb.go`).

### `ffi/remote/server.go` — New RPC handlers + chained proposal support

**`proposalEntry` struct expanded** with:

- `mu sync.Mutex` — protects the proposal FFI handle against concurrent access
  from GC reaping and RPC handlers, preventing use-after-free races.
- `dropped bool` — set under `mu` when the proposal has been dropped or
  committed. Operations that acquire `mu` check this flag and return an error
  if set.
- `committedRoot ffi.Hash` — the committed revision root this proposal chain
  is based on. For first-level proposals, equals `root_hash` from the request;
  for chained proposals, inherited from the parent.
- `cumulativeOps []ffi.BatchOp` — all batch operations accumulated from the
  chain root to this proposal.

**`CreateProposal` updated**: If `parent_proposal_id` is set, looks up the
parent entry, acquires the parent's mutex (checking `dropped` to guard against
GC races), and calls `parent.proposal.Propose(ops)`. Inherits `committedRoot`
from the parent and appends current ops to `parent.cumulativeOps`. The witness
is always generated from `committedRoot` with `cumulativeOps` — this is
necessary because `GenerateWitness` only works against committed revisions
(see the Rust type constraint note above).

**`CommitProposal` updated**: Now acquires the entry's mutex and sets
`dropped = true` before committing, preventing concurrent GC from dropping
the proposal mid-commit. Returns `codes.NotFound` (instead of a plain error)
if the proposal does not exist.

**`DropProposal` (new)**: Loads and deletes the proposal from the map, acquires
the entry's mutex, sets `dropped = true`, and calls `proposal.Drop()` to free
the FFI handle. Returns `codes.NotFound` if the proposal does not exist.

**`IterBatch` (new)**: In proposal mode, looks up the proposal entry by ID,
acquires the entry's mutex (checking `dropped` to guard against GC races),
creates an iterator via `proposal.Iter(startKey)`, and holds the mutex for the
entire batch collection to prevent the GC goroutine from dropping the proposal
while iteration is in progress. In revision mode (`proposal_id == 0`), creates
a temporary `Revision` from `root_hash`. In both modes, collects up to
`batchSize` pairs, checks for more by attempting one extra advance, and returns
`IterBatchResponse` with `has_more` flag.

**`ServerOption` / `WithProposalTTL` (new)**: Functional options for `NewServer`.
`WithProposalTTL(ttl)` enables a background GC goroutine that reaps proposals
older than `ttl`. A zero TTL (the default) disables GC, preserving backward
compatibility. The GC ticker fires at `ttl/2` intervals (clamped to a minimum
of 1 second to prevent busy-spinning with very small TTLs).

**`WithContext(ctx)` (new)**: Sets the context used by the GC goroutine. When
the context is cancelled, the GC goroutine exits and all remaining proposals
are reaped — equivalent to calling `Stop()`. This is useful when the server
lifecycle is managed by a parent context. Default is `context.Background()`.

**`Stop()` (new)**: Signals the GC goroutine to exit and waits for it to drain
all remaining proposals (reaps with `maxAge=0`). Safe to call multiple times.
Should be called on server shutdown to release FFI resources from leaked
proposals.

## Key Design Decisions

### 1. Cumulative ops for chained proposal witnesses

The Rust `generate_witness` function requires `&T: TrieReader +
HashedNodeReader`, satisfied by `NodeStore` (from committed revisions) but not
by `dyn DynDbView` (from proposals via `db.view()`). This means witness
generation can only use committed revisions as the base.

For chained proposals (p2 on top of p1), we solve this by:

- **Server side**: accumulating all ops from the chain root (committed
  revision) to the current proposal. For p2, `cumulativeOps = p1_ops ++
  p2_ops`. The witness is generated as `GenerateWitness(committedRoot,
  cumulativeOps, p2_newRoot, depth)`.
- **Client side**: verifying all witnesses against the committed trie inside
  the `RemoteClient` handle, not against the parent proposal's trie. Each
  `remoteProposal` carries an `rc *ffi.RemoteClient` reference for this
  purpose.

### 2. Interface in `ffi`, adapter in `remote`

Avoids circular imports. `ffi` defines `DB`/`DBProposal`/`DBIterator` with no
knowledge of gRPC. `remote` imports `ffi` and provides `RemoteDB` which
satisfies `ffi.DB`.

### 3. `context.Context` on interface methods

Required for remote operations (cancellation, timeouts, metadata). Local
wrappers accept but ignore the context parameter.

### 4. Stateless server-side iteration

The `IterBatch` RPC creates a fresh `Proposal.Iter()` per call rather than
holding a stateful iterator across calls. Pagination is done by the client
setting `start_key` to the last returned key + `\x00`. This avoids server-side
iterator lifecycle management.

### 5. Single client per server

The system is designed for exclusive access: one client operates against one
server at a time. This simplifies several aspects of the design:

- **Proposal IDs**: The server uses a monotonically incrementing `uint64`
  counter. Multiple clients could create conflicting proposals from the same
  base revision, leading to commit failures or undefined state.
- **Proposal chain pointer model**: `remoteProposal` uses a `parentRoot
  *ffi.Hash` pointer so that committing a child proposal propagates the new
  root hash up to the parent. This requires linear chains committed
  leaf-to-root — a constraint naturally satisfied when one client controls all
  proposal ordering.
- **GC is for crash recovery, not isolation**: `WithProposalTTL` exists to
  reclaim leaked proposals if the single client crashes between `CreateProposal`
  and `Commit`/`Drop`. It is not designed to isolate concurrent clients.
- **Cache coherence**: The Rust-side read cache (inside `RemoteClientHandle`)
  assumes it is the only writer. Multiple clients writing through the same
  server would cause stale cache entries with no cross-client invalidation.
- **`Client.mu` serialization**: The client's `RWMutex` serializes mutations
  (`Update`, `Commit`, `Bootstrap`, `Close`) against reads (`Get`, `Root`).
  This works because there is exactly one client; a multi-client system would
  need server-side coordination instead.

## Running Tests

The Go tests auto-detect the compiled hash algorithm at runtime, so they work
regardless of whether the FFI library was built with or without `ethhash`.

```bash
# Rust: clippy + test
cargo clippy --workspace --features ethhash,logger --all-targets
cargo test --workspace --features ethhash,logger --all-targets

# Build FFI library (with ethhash)
cd ffi/src && cargo build --features ethhash,logger
# OR build without ethhash (MerkleDB mode) — Go tests handle both:
cd ffi/src && cargo build --features logger

# Go vet
cd ffi && go vet ./...
cd ffi/remote && go vet ./...

# LocalDB tests
cd ffi && go test -v -count=1 -run "TestLocalDB|TestNewLocalDB" ./...

# RemoteDB tests (all)
cd ffi/remote && go test -v -count=1 ./...

# Iterator verification tests (tampered, missing proof, corrupted, multi-batch)
cd ffi/remote && go test -run "TestRemoteDBProposalIter" -v -count=1

# Cache benchmark (NoCache vs per-policy Cached)
cd ffi/remote && go test -bench BenchmarkGetCached -benchtime=5s -count=1
```

## File Reference

| File | Role |
| ------ | ------ |
| `ffi/db.go` | **NEW** — `DB`, `DBProposal`, `DBRevision`, `DBIterator` interfaces + `LocalDB` adapter |
| `ffi/db_test.go` | **NEW** — LocalDB interface tests (incl. revision) |
| `ffi/remote/db.go` | **NEW** — `RemoteDB`, `remoteProposal`, `remoteIterator` |
| `ffi/remote/db_test.go` | **NEW** — RemoteDB interface tests |
| `ffi/remote_client.go` | **NEW** — Go wrapper for `RemoteClientHandle` (cache + trie in Rust) |
| `ffi/remote/server.go` | **MODIFIED** — `DropProposal`, `CommitProposal`, `IterBatch` handlers, chained proposal support, per-proposal mutex, gRPC status codes |
| `ffi/remote/proto/remote.proto` | **MODIFIED** — new RPCs and messages |
| `ffi/remote/proto/remote.pb.go` | **REGENERATED** |
| `ffi/remote/proto/remote_grpc.pb.go` | **REGENERATED** |
| `ffi/src/remote.rs` | **MODIFIED** — `fwd_get_with_proof` uses `get_root`; added `RemoteClientHandle` + 7 FFI functions |
| `ffi/remote/remote_test.go` | **MODIFIED** — runtime hash algorithm detection in `newTestDB` |
| `ffi/remote/client.go` | **MODIFIED** — `ClientOption`, `WithCacheSize`, `WithCache`; uses `*ffi.RemoteClient` for cache + trie |
| `ffi/remote/benchmark_test.go` | **MODIFIED** — `setupRemoteDB` accepts options, `BenchmarkGetCached` per policy |
| `ffi/src/proofs/range.rs` | **MODIFIED** — added `fwd_range_proof_verify_and_extract` (verify + return KV pairs) |
| `ffi/proofs.go` | **MODIFIED** — added `KeyValue` struct, `VerifyAndExtractRangeProof()`, `bytesPresent` Maybe helper |
| `ffi/firewood.h` | **AUTO-UPDATED** — new C function declaration for `fwd_range_proof_verify_and_extract` |
| `firewood/src/remote/ser.rs` | **NEW** — Binary serialization for `TruncatedTrie` and `WitnessProof` wire format |
| `firewood/src/remote/cache.rs` | **NEW** — Client-side Rust read cache with eviction policies |

### Pre-existing files (unchanged, for reference)

| File | Role |
| ------ | ------ |
| `ffi/firewood.go` | `Database` type — `Get`, `Update`, `Propose`, `Root`, `Close` |
| `ffi/proposal.go` | `Proposal` type — `Get`, `Iter`, `Propose`, `Commit`, `Drop`, `Root` |
| `ffi/iterator.go` | `Iterator` type — `Next`, `Key`, `Value`, `Err`, `Drop` |
| `ffi/batch_op.go` | `BatchOp` type — `Put`, `Delete`, `PrefixDelete` |
| `ffi/single_key_proof.go` | `GetWithProof`, `VerifySingleKeyProof` |
| `ffi/truncated_trie.go` | `TruncatedTrie` — `Root`, `RootHash`, `VerifyRootHash`, `VerifyWitness`, `MarshalBinary`, `UnmarshalBinary`, `Free`; `Database.GenerateWitness`, `Database.CreateTruncatedTrie` |
| `ffi/witness_proof.go` | `WitnessProof` — `Free`, `MarshalBinary`, `UnmarshalBinary` |
| `ffi/remote/client.go` | `Client` — `Get`, `Update`, `Bootstrap`, `Propose`, `Revision`, `Root`, `Close`, `ClientOption`, `WithCacheSize`, `WithCache`; uses `*ffi.RemoteClient` |

### `ffi/remote/remote_test.go` — Runtime hash algorithm detection

**Change**: Added `detectHashAlgorithm()` helper and updated `newTestDB` to use
it instead of hardcoding `ffi.EthereumNodeHashing`.

**Problem**: The remote tests in `remote_test.go` hardcoded
`ffi.EthereumNodeHashing` in `newTestDB()`. When the FFI library was built
without the `ethhash` feature (i.e., MerkleDB/SHA-256 mode), every remote test
failed immediately at database creation with:

> node store hash algorithm mismatch: want to initialize with Ethereum,
> but build option is for MerkleDB

**Solution**: Added a `sync.Once`-guarded `detectHashAlgorithm()` function that
tries creating a database with `EthereumNodeHashing` first — if that fails, it
falls back to `MerkleDBNodeHashing`. This matches the existing pattern in
`ffi/firewood_test.go:177-197`. The detection result is cached in a
package-level variable so it only runs once per test binary execution.

```go
var (
    detectedAlgo     ffi.NodeHashAlgorithm
    detectedAlgoOnce sync.Once
)

func detectHashAlgorithm() ffi.NodeHashAlgorithm {
    detectedAlgoOnce.Do(func() { /* try Ethereum, fallback to MerkleDB */ })
    return detectedAlgo
}
```

`newTestDB` was changed from:

```go
db, err := ffi.New(dbFile, ffi.EthereumNodeHashing)
```

to:

```go
db, err := ffi.New(dbFile, detectHashAlgorithm())
```

Added `os` and `sync` imports.

**Why not reuse `ffi/firewood_test.go`'s helper?** It's in a different package
(`ffi` vs `ffi/remote`). The `remote` package imports `ffi` as an external
dependency, so it can't access unexported test helpers. Duplicating the small
detection snippet (~15 lines) is simpler than creating a shared exported test
utility.

## Client-Side Read Cache

### Motivation

Benchmark results show Remote `Get()` is ~28× slower than Local (~254µs vs
~9µs). The cost is: gRPC round-trip + server-side proof generation + client-side
`VerifySingleKeyProof`. A client-side cache eliminates both network and crypto
costs for repeated reads. Blockchain state access is highly skewed (hot accounts,
popular contracts), so hit rates should be good.

### Architecture: Rust `RemoteClientHandle`

The read cache lives entirely in Rust, inside a `RemoteClientHandle` that owns
both the committed `TruncatedTrie` and an optional `ReadCache`. This avoids
reimplementing cache data structures in Go and keeps all trie + cache state
behind a single FFI handle.

```rust
// ffi/src/remote.rs
pub struct RemoteClientHandle {
    trie: RwLock<TruncatedTrie>,   // committed trie (interior mutability)
    cache: Option<ReadCache>,       // ReadCache has internal Mutex
}
```

The `ReadCache` (in `firewood/src/remote/cache.rs`) supports 4 eviction
policies: LRU, Random, Clock, and Sample-K-LRU. It uses a memory budget
(bytes) rather than entry count.

#### FFI Functions

7 C-ABI functions expose the handle to Go:

| Function | Description |
| --- | --- |
| `fwd_create_remote_client` | Create handle with optional cache (max bytes + policy) |
| `fwd_free_remote_client` | Drop handle and all owned resources |
| `fwd_remote_client_bootstrap` | Deserialize trie, verify root hash, clear cache |
| `fwd_remote_client_cache_lookup` | Cache lookup → Miss / HitPresent / HitAbsent |
| `fwd_remote_client_verify_get` | Verify single-key proof against committed root, cache result |
| `fwd_remote_client_verify_witness` | Verify witness against committed trie, return new trie handle |
| `fwd_remote_client_commit_trie` | Swap committed trie, invalidate cache via witness batch ops |

#### Go Wrapper (`ffi/remote_client.go`)

```go
type RemoteClient struct {
    handle  *C.RemoteClientHandle
    cleanup runtime.Cleanup
}

type EvictionPolicy int
const (
    EvictionLRU     EvictionPolicy = 0
    EvictionRandom  EvictionPolicy = 1
    EvictionClock   EvictionPolicy = 2
    EvictionSampleK EvictionPolicy = 3
)
```

Methods: `NewRemoteClient`, `Bootstrap`, `CacheLookup`, `VerifyGet`,
`VerifyWitness`, `CommitTrie`, `Free`.

### Caching Scheme

Each verified `Get()` call (whether the key exists or not) produces a cache
entry in the Rust `ReadCache`:

- Key exists → cached with value (positive result)
- Key does not exist → cached as absent (exclusion proof was validated)

Cache insertion happens inside `fwd_remote_client_verify_get` after successful
proof verification. Cache invalidation happens inside
`fwd_remote_client_commit_trie` using the witness's `batch_ops` field
(`ClientOp::Put` / `ClientOp::Delete`).

### Cache Lifetime and Invalidation

| Event | Cache Action |
| --- | --- |
| `Bootstrap()` | Full clear (trie replaced) |
| `Get()` (verified) | Store result in cache |
| `CommitTrie()` | Invalidate keys from witness batch ops |
| `Free()` | Cache dropped with handle |

### API

The cache is opt-in via functional options:

```go
// No cache (default, existing behavior unchanged):
rdb, err := NewRemoteDB(ctx, addr, root, depth)

// With cache (LRU default via WithCacheSize):
rdb, err := NewRemoteDB(ctx, addr, root, depth, WithCacheSize(32<<20))

// With explicit eviction policy:
rdb, err := NewRemoteDB(ctx, addr, root, depth,
    WithCache(32<<20, ffi.EvictionSampleK))

// Also works with NewClient directly:
client, err := NewClient(addr, depth, WithCache(32<<20, ffi.EvictionClock))
```

`WithCacheSize(n)` delegates to `WithCache(n, ffi.EvictionLRU)`.

### Files Modified for Cache Integration

#### `ffi/remote/client.go`

- **`Client`** struct uses `*ffi.RemoteClient` (owns trie + cache) + `root
  ffi.Hash` (cached root set by bootstrap/commit).
- **`ClientOption`** configures `clientConfig` (max bytes, policy, sample K).
- **`Get()`**: `rc.CacheLookup(key)` → miss → gRPC → `rc.VerifyGet(key,
  value, proof)`.
- **`Update()`**: unmarshal witness → `rc.VerifyWitness` → gRPC commit →
  `rc.CommitTrie(newTrie, witness)` → `witness.Free()`.
- **`Bootstrap()`**: `rc.Bootstrap(trieData, hash)` → clears cache internally.
- **`Close()`**: `rc.Free()`.

#### `ffi/remote/db.go`

- **`remoteProposal`** stores `witness *ffi.WitnessProof` + `rc
  *ffi.RemoteClient` (for `CommitTrie`).
- **`Commit()`**: `rc.CommitTrie(newTrie, witness)` → invalidates cache.
- **`NewRemoteDB`** creates `ffi.RemoteClient` from options.

#### `ffi/remote/benchmark_test.go`

- **`BenchmarkGetCached`**: Parameterized across all 4 eviction policies using
  `WithCache(32<<20, policy)`.

## Range Proof Verification for Remote Iterator

### Motivation

The `remoteIterator` was the only remote operation without cryptographic
verification. A malicious server could return fabricated values, inject fake
keys, omit keys, or reorder results. Every other remote operation already had
verification:

- `Get` → verified via single-key Merkle proofs
- `Update`/`Propose` → verified via witness-based re-execution

Iterator was the remaining security gap.

### Approach: Range Proof Per Batch

Each `IterBatch` RPC now includes a **range proof** covering the batch's key
range. The client verifies the proof and extracts the KV pairs directly from
the verified proof — the response's `pairs` field is never trusted.

**Why range proofs over per-entry single-key proofs:**

| Aspect | Per-entry proofs | Range proof |
| --- | --- | --- |
| Server cost | N `GetWithProof` calls | 1 `RangeProof` call |
| Wire overhead | N proofs (~500-2000 bytes each) | 1 proof |
| **Completeness** | **No** (server can omit keys) | **Yes** |
| Value correctness | Yes | Yes |
| Ordering | Requires explicit check | Implicit (trie order) |

**How it works:**

A `RangeProof` contains start proof (Merkle path for lower boundary), end proof
(Merkle path for upper boundary), and the actual consecutive trie entries.
Verification proves all three components are consistent with the root hash.

**Contiguous batch coverage:** Each batch uses `startKey` with unbounded
`endKey` and `maxLength = batchSize`. Batch 2 starts at `lastKey + "\x00"` from
batch 1, so proofs tile contiguously with no gaps. A malicious server cannot
omit keys (the proof proves all keys in range are included), inject keys
(Merkle paths won't verify), or reorder (entries follow trie order).

**Verification needs only a root hash** — `RangeProof.Verify()` takes only a
`Hash`, no `*Database` or `*TruncatedTrie`. The chain of trust is:

1. Trusted root → bootstrap
2. Truncated trie → verified against trusted root
3. Proposal root → verified via witness-based re-execution
4. Range proof → verified against proposal root (standalone)

### Changes Made

#### 1. Rust FFI — `ffi/src/proofs/range.rs`

Added `fwd_range_proof_verify_and_extract` — a single C function that verifies
the range proof AND extracts the KV pairs in one call:

```rust
#[unsafe(no_mangle)]
pub extern "C" fn fwd_range_proof_verify_and_extract(
    args: VerifyRangeProofArgs,
) -> KeyValueBatchResult {
    // 1. Verify the proof (same logic as fwd_range_proof_verify)
    // 2. Extract key_values() from the proof
    // 3. Convert to KeyValueBatchResult::Some(batch)
}
```

Reuses existing infrastructure:

- `KeyValueBatchResult` enum (`ffi/src/value/results.rs`)
- `From<Result<Vec<(Key, Value)>, Error>> for KeyValueBatchResult`
- `RangeProofContext.proof.key_values()` for direct access to embedded KV pairs

#### 2. Go FFI — `ffi/proofs.go`

Added `VerifyAndExtractRangeProof` — a single function that deserializes,
verifies, and extracts KV pairs from range proof bytes:

```go
type KeyValue struct {
    Key   []byte
    Value []byte
}

func VerifyAndExtractRangeProof(
    proofBytes []byte,
    rootHash Hash,
    startKey, endKey []byte,  // nil = unbounded
    maxLength uint32,
) ([]KeyValue, error)
```

**No `Maybe` types exposed** — `nil` byte slices mean "unbounded range", and
the conversion to Rust's `Maybe_BorrowedBytes` happens inside the function.
The remote package never touches `Maybe`, matching the unverified iterator's
interface style.

**GC correctness**: KV pairs are copied into Go memory via `C.GoBytes()` (same
pattern as `Iterator.Next()` and `getValueFromValueResult`). Rust memory is
freed before returning. Returned `[]KeyValue` is entirely Go-managed.

Also added `bytesPresent` type (implements `Maybe[[]byte]` for present values)
used internally by the function.

#### 3. Proto — `ffi/remote/proto/remote.proto`

Added two fields:

```protobuf
message IterBatchRequest {
  // ... existing fields ...
  bytes root_hash = 4;  // NEW: root hash for proof generation
}

message IterBatchResponse {
  // ... existing fields ...
  bytes range_proof = 3;  // NEW: serialized range proof for this batch
}
```

Backward compatible: if `root_hash` is absent, no proof is generated.
Proto Go code was regenerated.

#### 4. Server — `ffi/remote/server.go`

Modified `IterBatch` to generate a range proof when `root_hash` is provided:

```go
// After collecting pairs and determining hasMore...
if len(req.GetRootHash()) == ffi.RootLength && len(pairs) > 0 {
    rangeProof, err := s.db.RangeProof(
        root,
        toMaybe(req.GetStartKey()),  // nil/empty → None
        nil,                          // unbounded end
        uint32(batchSize),
    )
    // ... marshal and attach to response
}
```

Added `serverMaybe` type and `toMaybe` helper for creating `ffi.Maybe[[]byte]`
values. One `RangeProof` call per batch instead of N `GetWithProof` calls.

#### 5. Client — `ffi/remote/db.go`

**5a.** Added `root ffi.Hash` field to `remoteIterator` — stores the verified
proposal root for range proof generation.

**5b.** Added `verifyIterBatch` helper:

```go
func verifyIterBatch(
    root ffi.Hash,
    startKey []byte,
    batchSize uint32,
    resp *pb.IterBatchResponse,
) ([]*pb.KeyValuePair, error) {
    // Empty batch with no proof → valid (no entries in range)
    // Pairs but no proof → error "missing range proof"
    // Otherwise: call ffi.VerifyAndExtractRangeProof, rebuild pairs
}
```

The response's `pairs` field is **never trusted** — only the proof's embedded
pairs are used. The interface uses plain `[]byte` parameters (no `Maybe` types).

**5c.** Updated `remoteProposal.Iter()` — passes `RootHash` in request, calls
`verifyIterBatch` to extract trusted pairs.

**5d.** Updated `remoteIterator.Next()` — same pattern for subsequent batches.
`lastKey` comes from the previous batch's **verified** pairs (extracted from a
verified proof), so the pagination key is trustworthy.

#### 6. Tests — `ffi/remote/db_test.go`

Four new tests using gRPC server-side interceptors for adversarial scenarios:

- **`TestRemoteDBProposalIterTamperedValue`**: Interceptor tampers with a
  value in the `IterBatch` response pairs. Since the client uses proof-extracted
  pairs (not the response pairs), the tampered data is never consumed. Verifies
  the iterator returns correct (untampered) values.

- **`TestRemoteDBProposalIterMissingProof`**: Interceptor strips the range
  proof from the response. Asserts error contains `"missing range proof"`.

- **`TestRemoteDBProposalIterTamperedProof`**: Interceptor corrupts the range
  proof bytes (flips first 10 bytes). Asserts error contains `"range proof"`.

- **`TestRemoteDBProposalIterMultiBatch`**: Inserts 300 keys (batch size is
  256, so forces 2 batches), verifies proof checking works across batch
  boundaries. Expects 301 keys (300 + 1 from the proposal).

Added `startServerWithInterceptor` helper that wraps the gRPC server with a
`grpc.UnaryInterceptor` for tamper tests.

Existing `TestRemoteDBProposalIter` passes unchanged (honest server).

### Files Changed (This Session)

| File | Change |
| --- | --- |
| `ffi/src/proofs/range.rs` | **MODIFIED** — added `fwd_range_proof_verify_and_extract` |
| `ffi/proofs.go` | **MODIFIED** — added `KeyValue`, `VerifyAndExtractRangeProof`, `bytesPresent` |
| `ffi/firewood.h` | **AUTO-UPDATED** — new C function declaration |
| `ffi/remote/proto/remote.proto` | **MODIFIED** — `root_hash` + `range_proof` fields |
| `ffi/remote/proto/remote.pb.go` | **REGENERATED** |
| `ffi/remote/server.go` | **MODIFIED** — range proof generation in `IterBatch`, `serverMaybe`/`toMaybe` |
| `ffi/remote/db.go` | **MODIFIED** — `root` on iterator, `verifyIterBatch`, verified `Iter()`/`Next()` |
| `ffi/remote/db_test.go` | **MODIFIED** — 4 new tests + `startServerWithInterceptor` helper |

### Verification Results

- `cargo clippy -p firewood-ffi` — no warnings
- `cd ffi && go vet ./...` — clean
- `cd ffi/remote && go vet ./...` — clean
- All 69 remote tests pass
- Existing `TestRemoteDBProposalIter` passes unchanged

### Remaining Work

- **Range proof verification is currently a stub**: The Rust `verify()` method
  in `RangeProofContext` (line 127 of `ffi/src/proofs/range.rs`) logs a warning
  `"range proof verification not yet implemented"` and always returns `Ok(())`.
  The plumbing is complete end-to-end — once the Rust implementation is filled
  in, the Go client will automatically get real cryptographic verification. All
  the tests exercise the full pipeline (serialize, deserialize, "verify",
  extract) so they will continue to pass as the stub is replaced.

- **`ffi/remote/proto/remote_grpc.pb.go`** was NOT regenerated in this session
  because `IterBatch` already existed in the service definition — only message
  fields changed, which only affect `remote.pb.go`.

## Revision Support (Read-Only Historical State)

### Motivation

The `DB` interface provided `Get`, `Update`, `Propose`, `Root`, and `Close` but
had no way to read historical state at a specific root hash. The local
`Database` type supports `Revision(root Hash) (*Revision, error)` for
immutable read-only views, but this was not exposed through the `DB` interface
or the remote client. Adding `Revision` enables reading historical state from
committed revisions — important for blockchain applications that need to query
past state (e.g., answering historical balance queries, replaying transactions).

### Key Insight: No Truncated Trie Needed

A remote revision does **not** need its own truncated trie or cache. The trie
is only required for **witness-based verification** (proposals/updates). For
read-only access, all verification is standalone:

- **Get**: `VerifySingleKeyProof(root, key, value, proof)` — needs only root
  hash.
- **Iter**: `VerifyAndExtractRangeProof(proofBytes, root, ...)` — needs only
  root hash.

Both functions are purely cryptographic and take no DB/trie handles. A
`remoteRevision` is therefore very lightweight: just a root hash + RPC client.

### Safety of Bootstrapping from a Known Root Hash

The caller of `Revision(root)` is responsible for providing a trusted root hash
(from consensus, from a previously verified commit, etc.) — the same trust
model as `Bootstrap(trustedRoot)`.

### Changes Made

#### 1. `ffi/db.go` — `DBRevision` Interface + `localRevision`

Added `DBRevision` interface (read-only subset of `DBProposal`):

```go
type DBRevision interface {
    Root() Hash
    Get(ctx context.Context, key []byte) ([]byte, error)
    Iter(ctx context.Context, key []byte) (DBIterator, error)
    Drop() error
}
```

Extended `DB` interface with:

```go
Revision(ctx context.Context, root Hash) (DBRevision, error)
```

Added `localRevision` wrapping `*Revision` — delegates `Root`, `Get`, `Iter`,
`Drop` directly to the FFI `Revision` type. Added `LocalDB.Revision` method.
Added compile-time check: `_ DBRevision = (*localRevision)(nil)`.

#### 2. `ffi/remote/server.go` — `IterBatch` Extended for Revision Mode

When `proposal_id == 0` (proto3 default — never a valid proposal ID since
`nextID.Add(1)` starts at 1), the server uses `root_hash` to iterate a
committed revision instead of looking up a proposal:

- Creates a `Revision` from the root hash via `s.db.Revision(root)`.
- Iterates the revision using the same `*ffi.Iterator` type as proposals.
- The revision is created and dropped within each `IterBatch` call — no
  server-side state persists between paginated calls.
- Range proof generation is shared between both paths (unchanged).

#### 3. `ffi/remote/db.go` — `remoteRevision` + `verifiedGet` Helper

**`remoteRevision`** implements `ffi.DBRevision`:

```go
type remoteRevision struct {
    root ffi.Hash
    rpc  pb.FirewoodRemoteClient
}
```

- `Root()` — returns the stored root hash.
- `Get(ctx, key)` — calls `verifiedGet` (shared with `remoteProposal.Get`).
- `Iter(ctx, startKey)` — sends `IterBatch` with `proposal_id: 0` (revision
  mode), verifies range proof, returns `remoteIterator`.
- `Drop()` — no-op (no server-side state to clean up).

**`verifiedGet`** — extracted shared helper that fetches a value from the
server and verifies its single-key Merkle proof:

```go
func verifiedGet(
    ctx context.Context,
    rpc pb.FirewoodRemoteClient,
    root ffi.Hash,
    key []byte,
) ([]byte, error) { ... }
```

`remoteProposal.Get` was refactored to call `verifiedGet`, eliminating ~15
lines of duplicated proof verification logic.

**`RemoteDB.Revision`** — no server round-trip on creation; errors surface on
first `Get`/`Iter` if the root is invalid or has been pruned.

Added compile-time check: `_ ffi.DBRevision = (*remoteRevision)(nil)`.

#### 4. `ffi/remote/client.go` — `Client.Revision`

Convenience method returning a lightweight `ffi.DBRevision`. Independent of
client trie state — no lock needed:

```go
func (c *Client) Revision(root ffi.Hash) ffi.DBRevision {
    return &remoteRevision{root: root, rpc: c.rpc}
}
```

#### 5. Tests

**`ffi/db_test.go`** (2 new tests):

- `TestLocalDBRevision` — insert data, get revision, verify Get/Iter/Root/Drop.
- `TestLocalDBRevisionNotFound` — revision with non-existent root returns error.

**`ffi/remote/db_test.go`** (7 new tests):

- `TestRemoteDBRevisionGet` — two commits; revision at root1 sees old data.
- `TestRemoteDBRevisionIter` — iterate all keys, verify order and values.
- `TestRemoteDBRevisionIterStartKey` — iterate from a specific start key.
- `TestRemoteDBRevisionIterMultiBatch` — 300 keys, pagination across 256-key
  batch boundary.
- `TestRemoteDBRevisionBadRoot` — fabricated root; Get/Iter return error.
- `TestRemoteDBRevisionAfterUpdate` — revision at root1 still works after
  Update to root2.
- `TestRemoteDBRevisionDrop` — Drop is no-op; can create another revision.

### Files Changed (Revision Session)

| File | Change |
| --- | --- |
| `ffi/db.go` | **MODIFIED** — added `DBRevision` interface, extended `DB` with `Revision`, added `localRevision` + `LocalDB.Revision` |
| `ffi/db_test.go` | **MODIFIED** — added 2 local revision tests |
| `ffi/remote/server.go` | **MODIFIED** — extended `IterBatch` for `proposal_id == 0` revision mode |
| `ffi/remote/db.go` | **MODIFIED** — added `remoteRevision`, `verifiedGet`, `RemoteDB.Revision`, refactored `remoteProposal.Get` |
| `ffi/remote/client.go` | **MODIFIED** — added `Client.Revision` |
| `ffi/remote/db_test.go` | **MODIFIED** — added 7 remote revision tests |

**No changes to**: proto files (no new RPCs/messages — reuses existing
`IterBatch` and `GetValue` RPCs), `ffi/revision.go`, `ffi/firewood.go`,
`ffi/proofs.go`, Rust code.

### Verification Results

- `cd ffi && go build ./...` — compiles
- `cd ffi && go vet ./...` — clean
- `cd ffi && go test ./...` — all tests pass (including 2 new revision tests)
- `cd ffi/remote && go test ./...` — all tests pass (including 7 new revision
  tests)
- All existing proposal/iterator/tamper/cache tests unchanged and passing

## Concurrency and Error Handling Fixes

This section documents fixes for several concurrency and error-handling issues
identified during a bug review of the remote storage feature.

### Trie ownership moved to Rust (Bug 1 fix)

**Problem**: `Update()` frees the old trie and replaces it, but outstanding
`remoteProposal` objects hold a raw `committedTrie` pointer to the old trie.
After `Update()`, the proposal's `committedTrie` dangles.

**Fix**: The committed trie is now owned by the Rust `RemoteClientHandle`
(behind a `parking_lot::RwLock`). Proposals hold only a `*ffi.TruncatedTrie`
for their verified new state and an `*ffi.RemoteClient` reference for witness
verification and commit. The `RemoteClientHandle` atomically swaps its internal
trie during `CommitTrie`, so there are no dangling trie pointers.

### Iterator uses caller's context (Bug 3 fix)

**Problem**: `remoteIterator.Next()` used `context.Background()` for
pagination RPCs instead of the caller's context, making subsequent batch
fetches uncancellable.

**Fix**: `remoteIterator` now stores the caller's `context.Context` (passed
via `Iter()`) and uses it for all subsequent `IterBatch` RPCs in `Next()`.

### Per-proposal mutex for GC race prevention (Bug B fix)

**Problem**: The GC goroutine (`reapExpiredProposals`) can `LoadAndDelete` a
proposal and call `proposal.Drop()` between the time an RPC handler (`IterBatch`,
`CreateProposal`) calls `proposals.Load()` and uses the proposal's FFI handle.
This is a TOCTOU race that can cause use-after-free in native code.

**Fix**: Added `mu sync.Mutex` and `dropped bool` to `proposalEntry`. All
code paths that access the proposal's FFI handle acquire the mutex first:

- **`reapExpiredProposals`**: Lock, set `dropped = true`, drop, unlock.
- **`IterBatch`** (proposal mode): Lock, check `dropped`, iterate. The mutex
  is held for the entire batch collection (released via the `cleanup` function)
  to prevent GC from dropping the proposal while the iterator is in use.
- **`CreateProposal`** (parent access): Lock, check `dropped`, call
  `Propose()`, unlock.
- **`CommitProposal`**: Lock, set `dropped = true`, commit, unlock.
- **`DropProposal`**: Lock, set `dropped = true`, drop, unlock.

If `dropped` is true when checked, the operation returns an appropriate error.

### gRPC status codes and NotFound suppression (Bug A fix)

**Problem**: `DropProposal` and `CommitProposal` returned plain `fmt.Errorf`
for "proposal not found", which maps to gRPC `Unknown`. On the client side,
`remoteProposal.Drop()` could not distinguish "proposal already cleaned up by
GC" from real errors, leading to misleading error returns.

**Fix (server)**: Both `CommitProposal` and `DropProposal` now return
`status.Errorf(codes.NotFound, ...)` for missing proposals.

**Fix (client)**: `remoteProposal.Drop()` suppresses gRPC `NotFound` errors
from the `DropProposal` RPC, since they indicate the server already cleaned up
the proposal. Real errors (network failures, etc.) are still returned.

### Range proof with proposal roots (Bug D investigation)

**Investigated**: `IterBatch` generates range proofs using
`s.db.RangeProof(root, ...)` where `root` may be an uncommitted proposal's
root hash. Investigation of the Rust implementation confirmed this is **not a
bug**: `RevisionManager::view()` (`firewood/src/manager.rs`) performs a
two-phase lookup — first searching active proposals by root hash, then falling
back to committed revisions. Proposal roots are resolvable as long as the
proposal exists in the active proposals cache.

### Files Changed

| File | Change |
| --- | --- |
| `ffi/remote/server.go` | **MODIFIED** — added `mu`/`dropped` to `proposalEntry`, mutex protection in all proposal operations, gRPC `codes.NotFound` status codes, `google.golang.org/grpc/codes` and `status` imports |
| `ffi/remote/db.go` | **MODIFIED** — `Drop()` suppresses `codes.NotFound` RPC errors, added `google.golang.org/grpc/codes` and `status` imports |
| `ffi/remote/db_test.go` | **MODIFIED** — updated `TestRemoteDBProposalExpiredOnServer` to expect `Drop()` to succeed when server has already GC'd the proposal |

### Verification Results

- `cd ffi/remote && go vet ./...` — clean
- `cd ffi/remote && go test -count=1 -race ./...` — all tests pass with race
  detector enabled

## Known Limitations and Future Work

1. **Range proof verification is a Rust stub**: The `RangeProofContext::verify()`
   method (line 127 of `ffi/src/proofs/range.rs`) logs a warning and returns
   `Ok(())` without performing real cryptographic checks. The full end-to-end
   plumbing (serialize → deserialize → verify → extract) is complete, so once
   the Rust implementation is filled in, all existing tests will continue to
   pass and real security will be active.

2. **Rust `generate_witness` type constraints**: The `generate_witness` function
   requires `TrieReader + HashedNodeReader` which `dyn DynDbView` doesn't
   implement. If these trait bounds were relaxed (or `DynDbView` gained these
   impls), chained proposals could generate witnesses directly from the parent
   proposal state instead of using cumulative ops from the committed root.

3. **Concurrency contract (single-client)**: The system assumes one client per
   server (see Key Design Decision #5). `Client` uses a `sync.RWMutex` to
   protect its `root` field: concurrent reads (`Get`, `Root`) are safe
   alongside mutations (`Bootstrap`, `Update`, `Close`, `Commit`), but
   concurrent mutations are logic errors. `remoteProposal` holds a pointer to
   the same mutex so `Commit` can write-lock during the parent root update.
   The proposal chain uses `parentRoot *ffi.Hash` pointers — committing a
   child writes the new root back to the parent, requiring linear chains
   committed leaf-to-root. These constraints are naturally enforced by
   single-client control. `remoteIterator` is single-goroutine only and is not
   synchronized.

   **Trie lifetime**: The committed trie is owned by `RemoteClientHandle` in
   Rust (behind `parking_lot::RwLock`). Proposals hold only their own
   `*ffi.TruncatedTrie` (verified new state) and an `*ffi.RemoteClient`
   reference. `CommitTrie` atomically swaps the Rust-side trie, so
   `Update()` on the client while a proposal is outstanding does not cause
   dangling pointers.

4. **Server-side proposal cleanup**: Multiple layers prevent leaked proposals:

   - **Client-side best-effort cleanup**: When client-side witness verification
     fails after a successful `CreateProposal` RPC, the client sends a
     best-effort `DropProposal` RPC to clean up the server-side entry.
     `remoteProposal.Drop()` suppresses `codes.NotFound` errors from the RPC
     since they indicate the server already cleaned up.
   - **Server-side creation rollback**: The `CreateProposal` handler drops a
     newly-created proposal if any error occurs between proposal creation and
     storage in the map.
   - **TTL-based garbage collection**: `NewServer(db, WithProposalTTL(ttl))`
     starts a background goroutine that reaps proposals older than `ttl`. This
     handles the case where a client crashes after `CreateProposal` but before
     `Commit` or `Drop`. The default TTL is **zero** (GC disabled), so callers
     must opt in. The GC goroutine exits when `Stop()` is called or when the
     context passed via `WithContext(ctx)` is cancelled; both paths reap all
     remaining proposals.
   - **Per-proposal mutex**: Each `proposalEntry` has a `sync.Mutex` that
     protects its FFI handle. The GC goroutine acquires the mutex and sets
     `dropped = true` before calling `proposal.Drop()`. RPC handlers
     (`IterBatch`, `CreateProposal`) acquire the mutex and check `dropped`
     before using the proposal, preventing use-after-free races between GC
     and concurrent operations.

5. **`TruncatedTrie` finalizer safety net**: `TruncatedTrie` now has a
   `runtime.AddCleanup` finalizer that frees the underlying Rust handle if
   the Go object is garbage collected without an explicit `Free()` call.
   Callers should still call `Free()` (or `Drop()` for proposals) for prompt
   resource release — the finalizer is a safety net, not a substitute.

6. **Cache does not cover `remoteProposal.Get()` or `remoteRevision.Get()`**:
   Only `Client.Get()` uses the cache. Reads through `remoteProposal.Get()`
   and `remoteRevision.Get()` always go to the server. This is intentional —
   proposal/revision reads are less frequent and their state may diverge from
   the committed state that the cache represents.

7. **`remoteRevision` is stateless**: A `remoteRevision` holds only a root
   hash and RPC client — no server-side state persists. Each `Get` or `Iter`
   call is independently verified via Merkle proofs. If the server prunes the
   revision between calls, subsequent calls will fail with a server-side error.
   This is by design — the lightweight approach avoids server-side lifecycle
   management.

8. **Crash between server commit and client trie swap**: `Commit()` sends the
   `CommitProposal` RPC before acquiring the client mutex and swapping the
   local truncated trie. This is intentional — holding the mutex during a
   network round-trip would block all concurrent reads. However, if the
   process crashes after the RPC succeeds but before the local swap completes,
   the server's committed state advances while the client's truncated trie
   remains stale. **Recovery**: call `Client.Bootstrap` with the server's
   current root hash (obtained out-of-band or from a trusted source). The
   server's committed state is always the source of truth; the client's trie
   is a verifiable cache that can be reconstructed from any trusted root.

9. **O(n) prefix invalidation in cache**: `invalidatePrefix` scans all cache
   entries to find keys matching a prefix. This is O(n) in cache size and is
   triggered by every `PrefixDelete` operation. For workloads with frequent
   PrefixDelete operations and large caches, this could become a bottleneck.
   Possible improvements (in order of complexity):

   - **Clear-on-prefix-delete**: Call `cache.clear()` on any PrefixDelete.
     Simplest fix. The cache repopulates on subsequent reads. Cost: temporary
     cache miss storm after each PrefixDelete.
   - **Sorted slice index**: Maintain a sorted `[]string` of cached keys in
     parallel. Prefix lookup becomes O(log n + k) via binary search to find
     the first matching key, then linear scan of k matches. Cost: O(n)
     insertion to maintain sorted order.
   - **Trie index**: A radix tree over cached keys gives O(p + k) prefix
     lookup where p is prefix length and k is the number of matching keys.
     Cost: additional memory and insertion overhead.
   - **Bloom filter guard**: Track which prefixes have cached keys using a
     Bloom filter. Skip the scan entirely on filter miss. Cost: false
     positives cause unnecessary scans, but the common case (no matching
     keys) becomes O(1).
