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
- **`remoteProposal`** holds `proposalID`, `root`, `newTrie` (verified
  truncated trie from witness), gRPC client, `parentTrie` pointer (for Commit
  to swap), `committedTrie` (for chained witness verification), and `depth`.
  - `Commit` sends `CommitProposal` RPC then replaces the parent's trie pointer
    with the verified `newTrie`.
  - `Drop` frees local `newTrie` and sends `DropProposal` RPC.
  - `Get` sends `GetValue` RPC with the proposal's root hash, verifies the
    single-key Merkle proof client-side.
  - `Iter` sends `IterBatch` RPC, returns a `remoteIterator`.
  - `Propose` sends `CreateProposal` RPC with `parent_proposal_id` set,
    verifies witness against `committedTrie`, returns child `remoteProposal`.
- **`remoteIterator`** holds current batch of `KeyValuePair` from the server,
  cursor index, `hasMore` flag, and gRPC client.
  - `Next` advances cursor; if batch exhausted and `hasMore`, fetches next batch
    via `IterBatch` RPC (start_key = last key + `\x00`).
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

25 tests exercising the interfaces through `RemoteDB` over a real gRPC
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

- `committedRoot ffi.Hash` — the committed revision root this proposal chain
  is based on. For first-level proposals, equals `root_hash` from the request;
  for chained proposals, inherited from the parent.
- `cumulativeOps []ffi.BatchOp` — all batch operations accumulated from the
  chain root to this proposal.

**`CreateProposal` updated**: If `parent_proposal_id` is set, looks up the
parent entry and calls `parent.proposal.Propose(ops)`. Inherits
`committedRoot` from the parent and appends current ops to
`parent.cumulativeOps`. The witness is always generated from `committedRoot`
with `cumulativeOps` — this is necessary because `GenerateWitness` only works
against committed revisions (see the Rust type constraint note above).

**`DropProposal` (new)**: Loads and deletes the proposal from the map, calls
`proposal.Drop()` to free the FFI handle.

**`IterBatch` (new)**: Looks up proposal by ID, calls `proposal.Iter(startKey)`,
collects up to `batchSize` pairs, checks for more by attempting one extra
advance, returns `IterBatchResponse` with `has_more` flag.

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
- **Client side**: verifying all witnesses against the `committedTrie` (the
  client's trie at the committed revision), not against the parent proposal's
  trie. Each `remoteProposal` carries a `committedTrie` reference inherited
  from its parent.

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

# RemoteDB tests (all, including cache)
cd ffi/remote && go test -v -count=1 ./...

# Cache tests only
cd ffi/remote && go test -run TestCache -v -count=1

# Eviction policy tests
cd ffi/remote && go test -run "TestEviction|TestLRU|TestClock|TestSampleK|TestRandom" -v -count=1

# Cache + eviction tests with race detector
cd ffi/remote && go test -race -run "TestCache|TestEviction" -count=1

# Iterator verification tests (tampered, missing proof, corrupted, multi-batch)
cd ffi/remote && go test -run "TestRemoteDBProposalIter" -v -count=1

# Cache benchmark (NoCache vs per-policy Cached)
cd ffi/remote && go test -bench BenchmarkGetCached -benchtime=5s -count=1

# Eviction throughput benchmark per policy
cd ffi/remote && go test -bench BenchmarkCacheEviction -benchtime=5s -count=1
```

## File Reference

| File | Role |
|------|------|
| `ffi/db.go` | **NEW** — `DB`, `DBProposal`, `DBRevision`, `DBIterator` interfaces + `LocalDB` adapter |
| `ffi/db_test.go` | **NEW** — LocalDB interface tests (incl. revision) |
| `ffi/remote/db.go` | **NEW** — `RemoteDB`, `remoteProposal`, `remoteIterator` |
| `ffi/remote/db_test.go` | **NEW** — RemoteDB interface tests |
| `ffi/remote/cache.go` | **NEW** — `readCache` wrapper delegating to `evictionStore` backend |
| `ffi/remote/cache_test.go` | **NEW** — Cache unit tests + integration tests |
| `ffi/remote/eviction.go` | **NEW** — `evictionStore` interface, `EvictionPolicy` enum, factory |
| `ffi/remote/eviction_lru.go` | **NEW** — LRU eviction via doubly-linked list + map |
| `ffi/remote/eviction_random.go` | **NEW** — Random eviction via dense key slice + map |
| `ffi/remote/eviction_clock.go` | **NEW** — Clock (second-chance) eviction via circular list |
| `ffi/remote/eviction_samplek.go` | **NEW** — Sample-K-LRU eviction (Redis-style approximated LRU) |
| `ffi/remote/eviction_test.go` | **NEW** — Parameterized tests across all 4 eviction policies |
| `ffi/remote/server.go` | **MODIFIED** — `DropProposal`, `IterBatch` handlers, chained proposal support |
| `ffi/remote/proto/remote.proto` | **MODIFIED** — new RPCs and messages |
| `ffi/remote/proto/remote.pb.go` | **REGENERATED** |
| `ffi/remote/proto/remote_grpc.pb.go` | **REGENERATED** |
| `ffi/src/remote.rs` | **MODIFIED** — `fwd_get_with_proof` uses `get_root` |
| `ffi/remote/remote_test.go` | **MODIFIED** — runtime hash algorithm detection in `newTestDB` |
| `ffi/remote/client.go` | **MODIFIED** — `ClientOption`, `WithCacheSize`, `WithCache`, cache integration in `Get`/`Update`/`Bootstrap`/`Close`/`Propose` |
| `ffi/remote/benchmark_test.go` | **MODIFIED** — `setupRemoteDB` accepts options, `BenchmarkGetCached` per policy, `BenchmarkCacheEviction` |
| `ffi/src/proofs/range.rs` | **MODIFIED** — added `fwd_range_proof_verify_and_extract` (verify + return KV pairs) |
| `ffi/proofs.go` | **MODIFIED** — added `KeyValue` struct, `VerifyAndExtractRangeProof()`, `bytesPresent` Maybe helper |
| `ffi/firewood.h` | **AUTO-UPDATED** — new C function declaration for `fwd_range_proof_verify_and_extract` |

### Pre-existing files (unchanged, for reference)

| File | Role |
|------|------|
| `ffi/firewood.go` | `Database` type — `Get`, `Update`, `Propose`, `Root`, `Close` |
| `ffi/proposal.go` | `Proposal` type — `Get`, `Iter`, `Propose`, `Commit`, `Drop`, `Root` |
| `ffi/iterator.go` | `Iterator` type — `Next`, `Key`, `Value`, `Err`, `Drop` |
| `ffi/batch_op.go` | `BatchOp` type — `Put`, `Delete`, `PrefixDelete` |
| `ffi/single_key_proof.go` | `GetWithProof`, `VerifySingleKeyProof` |
| `ffi/truncated_trie.go` | `TruncatedTrie` — `VerifyWitness`, `Root`, `Free`, `GenerateWitness` |
| `ffi/remote/client.go` | `Client` — `Get`, `Update`, `Bootstrap`, `Propose`, `Revision`, `Root`, `Close`, `ClientOption`, `WithCacheSize`, `WithCache` |

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

Benchmark results show a **~3,400–3,700× speedup** for cached reads across all
eviction policies:

| Variant | ns/op | Speedup vs NoCache |
|---------|------:|-------------------:|
| NoCache | 247,716 | — |
| LRU | 73 | 3,393× |
| Random | 66 | 3,753× |
| Clock | 68 | 3,643× |
| SampleKLRU | 68 | 3,643× |

Eviction throughput (inserting into a full 1,000-entry cache):

| Policy | ns/op |
|--------|------:|
| LRU | 182 |
| Clock | 194 |
| Random | 210 |
| SampleKLRU | 388 |

### Why Not Reuse Firewood's Rust Cache?

Firewood's Rust-side cache (`storage/src/linear/filebacked.rs`) caches **trie
nodes** keyed by disk offset (`LinearAddress → Arc<Node>`). The Go client needs
**key-value results** (`[]byte → []byte`). These are fundamentally different
abstraction levels — the Rust cache accelerates node deserialization from disk,
while the Go cache needs to skip the entire RPC + proof verification path.

### Caching Scheme

Each `Get()` call that results in a verified response (whether the key exists
or not) produces a cache entry:

```go
key ([]byte) → cacheEntry { value []byte, found bool }
```

- `found=true, value=<data>`: Key exists, value is the verified data.
- `found=true, value=[]byte{}`: Key exists with an empty value.
- `found=false, value=nil`: Key verified to not exist (exclusion proof was
  validated). Equally expensive to prove, so worth caching.

### Cache Lifetime and Invalidation

**Selective invalidation** on `Update()` and `Commit()`:

The client already knows every `BatchOp` in each write. For each op:

| Op Type | Invalidation Action |
|---------|-------------------|
| `Put(key, val)` | Delete `key` from cache |
| `Delete(key)` | Delete `key` from cache |
| `PrefixDelete(prefix)` | Delete all cached keys with that prefix |

**Full invalidation** on `Bootstrap()` and `Close()`:

These replace the entire trie state. No batch ops to examine — clear everything.

### Architecture: Pluggable Eviction Policies

The cache uses a pluggable `evictionStore` interface to decouple the caching
logic (`readCache`) from the eviction strategy. This was introduced to replace
the original simple admission control (which silently dropped new entries when
the cache was full) with proper eviction — ensuring the cache can always accept
new entries by evicting stale ones.

#### `evictionStore` interface (`eviction.go`)

```go
type evictionStore interface {
    get(key string) (cacheEntry, bool) // lookup + update access metadata
    put(key string, entry cacheEntry)  // store + evict if at capacity
    del(key string) bool               // remove specific key
    keys(fn func(key string))          // iterate all keys (for prefix scan)
    clear()
    len() int
}
```

#### `EvictionPolicy` enum (`eviction.go`)

```go
type EvictionPolicy int

const (
    LRU            EvictionPolicy = iota // Doubly-linked list, true LRU
    RandomEviction                       // Uniform random eviction
    Clock                                // Second-chance / clock algorithm
    SampleKLRU                           // Sample K random, evict oldest access
)
```

Factory function `newEvictionStore(policy, maxSize)` creates the appropriate
backend.

#### Policy 1: LRU (`eviction_lru.go`)

**Data structures**: `map[string]*lruNode` + doubly-linked list with sentinel
head/tail nodes.

- `get`: map lookup, move node to front. O(1).
- `put`: if exists, update + move to front. If new at capacity, evict tail
  (least recently used), insert at front. O(1).
- `del`: map lookup, unlink from list. O(1).
- Sentinels eliminate nil checks in link/unlink operations.

**Best for**: Workloads with strong temporal locality (blockchain state access
with hot accounts). Recommended default.

#### Policy 2: Random (`eviction_random.go`)

**Data structures**: `map[string]*randomEntry` + dense `[]string` order slice +
local `*rand.Rand` (seeded from `math/rand/v2`).

- `get`: map lookup only (no metadata to update). O(1).
- `put`: if exists, update in place. If new at capacity, pick random index,
  evict that entry, swap-with-last to maintain density. O(1).
- `del`: swap-with-last in order slice + update swapped entry's index. O(1).

**Best for**: Uniform access patterns or when simplicity matters.

#### Policy 3: Clock (`eviction_clock.go`)

**Data structures**: `map[string]*clockNode` + circular doubly-linked list with
sentinel + hand pointer.

- `get`: map lookup, set `referenced = true`. O(1).
- `put`: if exists, update + set referenced. If new at capacity, sweep from
  hand: if referenced, clear bit and advance; if unreferenced, evict. Insert
  new node before sentinel. Amortized O(1).
- `del`: if node == hand, advance hand first. Unlink + map delete. O(1).

**Best for**: Approximating LRU with lower per-access overhead (no list
reordering on every get).

#### Policy 4: Sample-K-LRU (`eviction_samplek.go`)

**Data structures**: `map[string]*sampleEntry` with monotonic `uint64` access
counter + dense `[]string` order slice + local `*rand.Rand`. Default K=5
(following Redis).

- `get`: map lookup, increment counter, set `lastAccess`. O(1).
- `put`: if exists, update + touch. If new at capacity, sample K random
  distinct entries, evict the one with smallest `lastAccess`. O(K).
- `del`: swap-with-last in order slice. O(1).

**Best for**: Large caches where true LRU's linked-list overhead matters.
Approximates LRU well with K=5.

### Concurrency Model

Each eviction store uses its own `sync.Mutex` internally. This is necessary
because `Get()` calls `lookup`/`store` on the cache under the client's `RLock`,
allowing concurrent cache access from multiple goroutines. The critical sections
are O(1) (no I/O), so contention is minimal.

- **`Get()` (RLock)**: `backend.get()` for lookup, `backend.put()` to cache
  verified results. Both acquire the store's internal mutex briefly.
- **`Update()` / `Commit()` (write Lock)**: Selective invalidation via two-pass
  `invalidatePrefix` (collect keys, then delete). Exclusive access guaranteed by
  the client's write lock.
- **`Bootstrap()` / `Close()` (write Lock)**: `backend.clear()`.

The two-pass `invalidatePrefix` pattern (collect matching keys via `keys()`,
then delete via `del()`) is safe because invalidation only runs under the
client's write lock, so no concurrent `store`/`lookup` calls add new matching
keys between passes.

### API

The cache is opt-in via functional options:

```go
// No cache (default, existing behavior unchanged):
rdb, err := NewRemoteDB(ctx, addr, root, depth)

// With cache (LRU default via WithCacheSize):
rdb, err := NewRemoteDB(ctx, addr, root, depth, WithCacheSize(10_000))

// With explicit eviction policy:
rdb, err := NewRemoteDB(ctx, addr, root, depth,
    WithCache(10_000, SampleKLRU))

// Also works with NewClient directly:
client, err := NewClient(addr, depth, WithCache(10_000, Clock))
```

`WithCacheSize(n)` delegates to `WithCache(n, LRU)` for backward compatibility.

### Files Created

#### `ffi/remote/eviction.go` — Interface + enum + factory

- **`evictionStore`** interface: `get`, `put`, `del`, `keys`, `clear`, `len`.
- **`EvictionPolicy`** enum: `LRU`, `RandomEviction`, `Clock`, `SampleKLRU`.
- **`newEvictionStore(policy, maxSize)`** — factory returning the concrete store.

#### `ffi/remote/eviction_lru.go` — LRU implementation

- **`lruNode`** struct: `key`, `entry`, `prev`, `next` pointers.
- **`lruStore`** struct: `mu sync.Mutex`, `items map`, `head`/`tail` sentinels,
  `maxSize`.
- All operations O(1) via doubly-linked list with sentinel nodes.

#### `ffi/remote/eviction_random.go` — Random implementation

- **`randomEntry`** struct: `entry`, `index` (position in order slice).
- **`randomStore`** struct: `mu sync.Mutex`, `items map`, `order []string`,
  `maxSize`, `rng *rand.Rand`.
- O(1) eviction via random index selection + swap-with-last deletion.

#### `ffi/remote/eviction_clock.go` — Clock implementation

- **`clockNode`** struct: `key`, `entry`, `referenced bool`, `prev`/`next`.
- **`clockStore`** struct: `mu sync.Mutex`, `items map`, `ring` sentinel,
  `hand` pointer, `maxSize`.
- Amortized O(1) eviction via clock hand sweep.

#### `ffi/remote/eviction_samplek.go` — Sample-K-LRU implementation

- **`sampleEntry`** struct: `entry`, `lastAccess uint64`, `index`.
- **`sampleKLRUStore`** struct: `mu sync.Mutex`, `items map`, `order []string`,
  `counter uint64`, `maxSize`, `k`, `rng *rand.Rand`.
- O(K) eviction by sampling K random entries and evicting the oldest.

#### `ffi/remote/cache.go` — readCache wrapper

- **`cacheEntry`** struct: `value []byte`, `found bool`.
- **`readCache`** struct: `backend evictionStore`.
- **`newReadCache(maxSize int, policy EvictionPolicy)`** — constructor.
- **`lookup(key []byte)`** — delegates to `backend.get()`.
- **`store(key []byte, entry cacheEntry)`** — delegates to `backend.put()`.
- **`invalidateKey(key []byte)`** — delegates to `backend.del()`.
- **`invalidatePrefix(prefix []byte)`** — two-pass: collect matching keys via
  `backend.keys()`, then delete via `backend.del()`. O(cache size) per call,
  but `PrefixDelete` is rare in practice.
- **`invalidateBatch(ops []ffi.BatchOp)`** — iterates ops, dispatches to
  `invalidateKey` or `invalidatePrefix` based on op type.
- **`clear()`** — delegates to `backend.clear()`.
- **`len()`** — delegates to `backend.len()`.

#### `ffi/remote/cache_test.go` — Unit and integration tests

Unit tests (pure `readCache`, no DB):

- `TestCacheLookupMiss`
- `TestCacheStoreAndLookup`
- `TestCacheNilValue` (exclusion proof cached)
- `TestCacheClear`
- `TestCacheEviction` (verifies new entry survives, len == maxSize)
- `TestCacheInvalidateKey`
- `TestCacheInvalidatePrefix`
- `TestCacheInvalidateBatch`

Integration tests (with DB + gRPC):

- `TestCacheInvalidationOnUpdate` — selective: untouched keys survive
- `TestCacheInvalidationOnBootstrap` — full clear
- `TestCacheInvalidationOnCommit` — selective via proposal ops
- `TestCacheConcurrentGet` — 20 goroutines × 10 reads each, race-safe

#### `ffi/remote/eviction_test.go` — Eviction policy tests

Parameterized tests via `forEachPolicy` helper (run across all 4 policies):

- `TestEvictionStore_GetMiss`
- `TestEvictionStore_PutAndGet`
- `TestEvictionStore_PutOverwrite` — len stays 1
- `TestEvictionStore_Del` / `TestEvictionStore_DelMissing`
- `TestEvictionStore_Clear`
- `TestEvictionStore_Eviction` — put maxSize+2, len ≤ maxSize
- `TestEvictionStore_EvictionDoesNotLoseNewEntry` — new entry survives eviction
- `TestEvictionStore_Keys` — visits all entries exactly once
- `TestEvictionStore_DelThenInsert` — delete + insert doesn't trigger eviction

Policy-specific tests:

- `TestLRUStore_EvictsLeastRecent` — touch changes eviction order
- `TestLRUStore_EvictsOldestWithoutTouch` — oldest inserted is evicted
- `TestClockStore_EvictsUnreferenced` — unreferenced entries evicted first
- `TestSampleKLRU_EvictsOldAccess` — with K>>n, approximates true LRU
- `TestRandomStore_EvictsOne` — exactly one entry evicted, new entry present

### Files Modified

#### `ffi/remote/client.go`

- Added **`ClientOption`** type (`func(*Client)`) and **`WithCacheSize`**
  constructor option (delegates to `WithCache` with `LRU`).
- Added **`WithCache(maxEntries int, policy EvictionPolicy)`** — explicit
  policy selection.
- Added `cache *readCache` field to **`Client`** struct (nil when disabled).
- **`NewClient`** now accepts `...ClientOption`.
- **`Get()`**: Cache lookup before RPC; cache store after successful proof
  verification.
- **`Bootstrap()`**: `cache.clear()` after trie replacement.
- **`Update()`**: `cache.invalidateBatch(ops)` after trie swap.
- **`Propose()`**: passes `cache` to `remoteProposal`.
- **`Close()`**: `cache.clear()` before connection close.

#### `ffi/remote/db.go`

- **`NewRemoteDB`** accepts `...ClientOption` and forwards to `Client`.
- **`remoteProposal`** has new `cache *readCache` field.
- **`Commit()`**: `cache.invalidateBatch(p.expectedCumulativeOps)` after trie
  swap. Uses `expectedCumulativeOps` (already tracked) which includes all ops
  from the chain root to this proposal — exactly the set of keys that may have
  changed.
- **Chained `Propose()`**: propagates `cache` to child `remoteProposal`.

#### `ffi/remote/benchmark_test.go`

- **`setupRemoteDB`** accepts `...ClientOption` and forwards to `NewRemoteDB`.
- **`BenchmarkGetCached`**: `Cached` sub-bench now parameterized across all 4
  eviction policies using `WithCache(10_000, policy)`.
- Added **`BenchmarkCacheEviction`**: measures eviction throughput per policy
  at capacity (1,000-entry cache with continuous insertions).

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
|---|---|---|
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
|---|---|
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
- All 53 remote tests pass (49 existing + 4 new)
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
|---|---|
| `ffi/db.go` | **MODIFIED** — added `DBRevision` interface, extended `DB` with `Revision`, added `localRevision` + `LocalDB.Revision` |
| `ffi/db_test.go` | **MODIFIED** — added 2 local revision tests |
| `ffi/remote/server.go` | **MODIFIED** — extended `IterBatch` for `proposal_id == 0` revision mode |
| `ffi/remote/db.go` | **MODIFIED** — added `remoteRevision`, `verifiedGet`, `RemoteDB.Revision`, refactored `remoteProposal.Get` |
| `ffi/remote/client.go` | **MODIFIED** — added `Client.Revision` |
| `ffi/remote/db_test.go` | **MODIFIED** — added 7 remote revision tests |

**No changes to**: proto files (no new RPCs/messages — reuses existing
`IterBatch` and `GetValue` RPCs), `ffi/revision.go`, `ffi/firewood.go`,
`ffi/remote/cache.go`, `ffi/proofs.go`, Rust code.

### Verification Results

- `cd ffi && go build ./...` — compiles
- `cd ffi && go vet ./...` — clean
- `cd ffi && go test ./...` — all tests pass (including 2 new revision tests)
- `cd ffi/remote && go test ./...` — all tests pass (including 7 new revision
  tests)
- All existing proposal/iterator/tamper/cache tests unchanged and passing

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

3. **Concurrency contract**: `Client` uses a `sync.RWMutex` to protect its
   `trie` field. Concurrent reads (`Get`, `Root`) are safe alongside mutations
   (`Bootstrap`, `Update`, `Close`, `Commit`). Mutations are serialized by the
   write lock but callers should not rely on this for correctness — concurrent
   mutations (e.g., two Updates) are logic errors. `remoteProposal` holds a
   pointer to the same mutex so `Commit` can write-lock during the parent trie
   swap. `remoteIterator` is single-goroutine only and is not synchronized.

   **`committedTrie` dangling pointer risk**: When a `remoteProposal` is
   created via `Propose()`, it captures a `committedTrie` reference pointing
   to the client's current trie. If `Update()` is called on the client while
   a proposal is outstanding, the old trie is freed and replaced. The
   proposal's `committedTrie` pointer then dangles. Callers must not
   interleave `Propose` and `Update` on the same client — proposals should
   be committed or dropped before calling `Update`.

4. **Server-side proposal cleanup**: When client-side witness verification
   fails after a successful `CreateProposal` RPC, the client sends a
   best-effort `DropProposal` RPC to clean up the server-side entry. This
   prevents permanent leaks in the server's proposal map. Similarly, the
   server's `CreateProposal` handler drops a newly-created proposal if any
   error occurs between proposal creation and storage in the map.

5. **`TruncatedTrie` finalizer safety net**: `TruncatedTrie` now has a
   `runtime.AddCleanup` finalizer that frees the underlying Rust handle if
   the Go object is garbage collected without an explicit `Free()` call.
   Callers should still call `Free()` (or `Drop()` for proposals) for prompt
   resource release — the finalizer is a safety net, not a substitute.

6. **`remoteIterator` uses `context.Background()`**: When fetching subsequent
   batches in `Next()`, the iterator uses `context.Background()` because the
   `Next() bool` signature doesn't accept a context. This means pagination
   fetches cannot be cancelled via the original context.

7. **Cache does not cover `remoteProposal.Get()` or `remoteRevision.Get()`**:
   Only `Client.Get()` uses the cache. Reads through `remoteProposal.Get()`
   and `remoteRevision.Get()` always go to the server. This is intentional —
   proposal/revision reads are less frequent and their state may diverge from
   the committed state that the cache represents.

8. **`remoteRevision` is stateless**: A `remoteRevision` holds only a root
   hash and RPC client — no server-side state persists. Each `Get` or `Iter`
   call is independently verified via Merkle proofs. If the server prunes the
   revision between calls, subsequent calls will fail with a server-side error.
   This is by design — the lightweight approach avoids server-side lifecycle
   management.
