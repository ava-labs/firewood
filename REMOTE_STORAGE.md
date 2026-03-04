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

- **`DB`** interface: `Get`, `Update`, `Propose`, `Root`, `Close` — all with
  `context.Context` except `Root`.
- **`DBProposal`** interface: `Root`, `Commit`, `Drop`, `Get`, `Iter`,
  `Propose` — supports reading from and chaining on uncommitted proposals.
- **`DBIterator`** interface: `Next`, `Key`, `Value`, `Err`, `Drop` — standard
  forward-only iteration.
- **`LocalDB`** struct wrapping `*Database`, **`localProposal`** wrapping
  `*Proposal`, **`localIterator`** wrapping `*Iterator` — thin wrappers that
  delegate to the underlying FFI types and ignore `context.Context`.
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

4 tests exercising the `ffi.DB`/`ffi.DBProposal`/`ffi.DBIterator` interfaces
through `LocalDB`:

- `TestNewLocalDB` — create, update, get, verify root changes, get
  non-existent key.
- `TestLocalDBPropose` — propose, read from proposal, verify DB isolation,
  commit, verify root.
- `TestLocalDBProposalChain` — chain two proposals (p1 then p2 on p1), read
  all keys from p2, commit chain in order.
- `TestLocalDBProposalIter` — iterate from a proposal, verify lexicographic
  order and completeness.

### `ffi/remote/db_test.go` — RemoteDB tests

7 tests exercising the interfaces through `RemoteDB` over a real gRPC
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
- `TestNewRemoteDBBadRoot` — wrong trusted root fails at construction.

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

# RemoteDB tests
cd ffi/remote && go test -v -count=1 ./...
```

## File Reference

| File | Role |
|------|------|
| `ffi/db.go` | **NEW** — `DB`, `DBProposal`, `DBIterator` interfaces + `LocalDB` adapter |
| `ffi/db_test.go` | **NEW** — LocalDB interface tests |
| `ffi/remote/db.go` | **NEW** — `RemoteDB`, `remoteProposal`, `remoteIterator` |
| `ffi/remote/db_test.go` | **NEW** — RemoteDB interface tests |
| `ffi/remote/server.go` | **MODIFIED** — `DropProposal`, `IterBatch` handlers, chained proposal support |
| `ffi/remote/proto/remote.proto` | **MODIFIED** — new RPCs and messages |
| `ffi/remote/proto/remote.pb.go` | **REGENERATED** |
| `ffi/remote/proto/remote_grpc.pb.go` | **REGENERATED** |
| `ffi/src/remote.rs` | **MODIFIED** — `fwd_get_with_proof` uses `get_root` |
| `ffi/remote/remote_test.go` | **MODIFIED** — runtime hash algorithm detection in `newTestDB` |

### Pre-existing files (unchanged, for reference)

| File | Role |
|------|------|
| `ffi/firewood.go` | `Database` type — `Get`, `Update`, `Propose`, `Root`, `Close` |
| `ffi/proposal.go` | `Proposal` type — `Get`, `Iter`, `Propose`, `Commit`, `Drop`, `Root` |
| `ffi/iterator.go` | `Iterator` type — `Next`, `Key`, `Value`, `Err`, `Drop` |
| `ffi/batch_op.go` | `BatchOp` type — `Put`, `Delete`, `PrefixDelete` |
| `ffi/single_key_proof.go` | `GetWithProof`, `VerifySingleKeyProof` |
| `ffi/truncated_trie.go` | `TruncatedTrie` — `VerifyWitness`, `Root`, `Free`, `GenerateWitness` |
| `ffi/remote/client.go` | `Client` — `Get`, `Update`, `Bootstrap`, `Root`, `Close` |

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

## Known Limitations and Future Work

1. **Range proof verification for iteration**: Currently, `remoteIterator`
   fetches key-value pairs from the server without cryptographic verification
   of the iteration results. A future enhancement could use range proofs to
   verify that the server returned complete and correct iteration results.

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

4. **`remoteIterator` uses `context.Background()`**: When fetching subsequent
   batches in `Next()`, the iterator uses `context.Background()` because the
   `Next() bool` signature doesn't accept a context. This means pagination
   fetches cannot be cancelled via the original context.
