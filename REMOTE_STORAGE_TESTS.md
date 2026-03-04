# Remote Storage Test Analysis

This document analyzes the test suites in the `ffi` and `ffi/remote` packages,
evaluating which tests can run through the `ffi.DB` interface (and thus against
either a local or remote backend) versus those that depend on concrete types.

## Interfaces

The `ffi` package defines three interfaces that abstract over backend
implementations:

| Interface | Methods | Description |
|---|---|---|
| `DB` | `Get`, `Update`, `Propose`, `Revision`, `LatestRevision`, `Root`, `Close` | Common database operations |
| `DBProposal` | `Root`, `Commit`, `Drop`, `Get`, `Iter`, `Propose` | Uncommitted proposal |
| `DBRevision` | `Root`, `Get`, `Iter`, `Drop` | Read-only committed revision |
| `DBIterator` | `Next`, `Key`, `Value`, `Err`, `Drop` | Key-value pair iteration |

Two implementations exist:

- **`LocalDB`** (`ffi/db.go`) — wraps the FFI `Database` type; delegates
  directly to the Rust engine via C FFI.
- **`RemoteDB`** (`ffi/remote/db.go`) — wraps a gRPC `Client`; all reads are
  verified via Merkle proofs and all writes via witness-based re-execution.

### Operations NOT on the interfaces (concrete-type only)

| Type | Method | Notes |
|---|---|---|
| `Database` | `Dump()` | DOT-format trie dump for debugging |
| `Proposal` | `Dump()` | DOT-format trie dump for debugging |
| `Revision` | `Dump()` | DOT-format trie dump (Get/Iter/Root/Drop are on `DBRevision`) |
| `Iterator` | `SetBatchSize(int)` | Configure batch fetching |
| `Iterator` | `NextBorrowed()` | Zero-copy iteration mode |

Note: `Database.Revision(root)` was added to the `DB` interface as
`Revision(ctx, root) (DBRevision, error)`. Tests that previously used
`*Revision` directly can now use the `DBRevision` interface.

Tests that use any of the remaining operations above cannot run against the
`DB` interface without further extending it or adding backend-specific
adapters.

## Analysis of `ffi/remote/` Tests

These tests already run against the remote backend via `RemoteDB` or the
lower-level `Client`/`Server` types.

### `ffi/remote/db_test.go` — RemoteDB high-level tests (32 tests)

All tests use the `ffi.DB` / `ffi.DBProposal` / `ffi.DBRevision` /
`ffi.DBIterator` interfaces via `RemoteDB`.

| Test | Interface Operations | Notes |
|---|---|---|
| `TestNewRemoteDB` | `Root`, `Get`, `Update` | Basic CRUD |
| `TestRemoteDBPropose` | `Propose`, `Root`, `Commit` | Propose-commit cycle |
| `TestRemoteDBProposalGet` | `Propose`, `Get` (proposal) | Read new + pre-existing keys from proposal |
| `TestRemoteDBProposalIter` | `Propose`, `Iter`, `Next`, `Key` | Iterate over proposal state |
| `TestRemoteDBProposalChain` | `Propose` (chained), `Root`, `Commit` | Two-level proposal chain |
| `TestRemoteDBProposeDrop` | `Propose`, `Drop`, `Root` | Drop does not affect DB |
| `TestRemoteDBPrefixDelete` | `Update` with `PrefixDelete` | Verify prefix removal |
| `TestRemoteDBPrefixDeletePropose` | `Propose` with `PrefixDelete`, `Get`, `Commit` | PrefixDelete in proposal |
| `TestRemoteDBPrefixDeleteChained` | `Propose` (chained) with `PrefixDelete` | PrefixDelete across chain |
| `TestRemoteDBPrefixDeleteMixed` | `Update` with mixed `Put`/`PrefixDelete` | Mixed batch operations |
| `TestNewRemoteDBBadRoot` | `NewRemoteDB` with wrong hash | Error handling |
| `TestRemoteDBProposalIterTamperedValue` | `Propose`, `Iter`, gRPC interceptor | Tampered response values ignored; proof pairs used |
| `TestRemoteDBProposalIterMissingProof` | `Propose`, `Iter`, gRPC interceptor | Missing range proof rejected |
| `TestRemoteDBProposalIterTamperedProof` | `Propose`, `Iter`, gRPC interceptor | Corrupted range proof rejected |
| `TestRemoteDBProposalIterMultiBatch` | `Propose`, `Iter` (300+ keys) | Range proof verified across batch boundaries |
| `TestRemoteDBRevisionGet` | `Revision`, `Get` (revision) | Historical read: root1 sees old data, not root2 |
| `TestRemoteDBRevisionIter` | `Revision`, `Iter`, `Next`, `Key`, `Value` | Iterate committed revision, verify order + values |
| `TestRemoteDBRevisionIterStartKey` | `Revision`, `Iter` with start key | Partial iteration from specific key |
| `TestRemoteDBRevisionIterMultiBatch` | `Revision`, `Iter` (300 keys) | Pagination across 256-key batch boundary |
| `TestRemoteDBRevisionBadRoot` | `Revision`, `Get`, `Iter` | Fabricated root; errors on first read |
| `TestRemoteDBRevisionAfterUpdate` | `Revision`, `Update`, `Get` | Revision at root1 still works after Update to root2 |
| `TestRemoteDBRevisionDrop` | `Revision`, `Drop`, `Revision`, `Get` | Drop is no-op; can create another with same root |
| `TestRemoteDBLatestRevision` | `LatestRevision`, `Root`, `Get` | Bootstrap + update, verify root and data |
| `TestRemoteDBLatestRevisionAfterUpdate` | `LatestRevision`, `Update` | Update changes what LatestRevision returns |
| `TestRemoteDBLatestRevisionMatchesRoot` | `LatestRevision`, `Root` | LatestRevision().Root() == db.Root() |
| `TestServerProposalTTLExpiry` | `CreateProposal`, `CommitProposal` | Proposal reaped after TTL; commit returns "not found" |
| `TestServerProposalTTLNotExpired` | `CreateProposal`, `CommitProposal` | Commit before TTL succeeds |
| `TestServerStop` | `CreateProposal`, `Stop()`, `CommitProposal` | Stop() reaps all proposals |
| `TestRemoteDBProposalExpiredOnServer` | `Propose`, `Commit`, `Iter`, `Propose`, `Drop` | Client surfaces clear errors when server GC reaps proposal |
| `TestRemoteDBProposalIterProposalOnlyKey` | `Propose`, `Iter`, `Next`, `Key`, `Value` | Verifies range proof from proposal state includes proposal-only keys |
| `TestRemoteDBIterMidPaginationExpiry` | `Propose`, `Iter`, `Next`, `Err` | Iterator returns error when GC reaps proposal between batches |
| `TestRemoteDBChainedProposalParentReaped` | `Propose` (chained), `Commit` | Chained proposals fail when GC reaps parent |

The tampered/missing/corrupted iterator tests use `startServerWithInterceptor`
— a helper that wraps the gRPC server with a `grpc.UnaryInterceptor` to
simulate adversarial behavior. The multi-batch tests insert 300+ keys (batch
size is 256) to force pagination across batch boundaries with verified handoff.

The 7 revision tests verify the `DBRevision` interface through `RemoteDB`.
Revisions are lightweight (root hash + RPC client only), with no server-side
state. All reads are independently verified via Merkle proofs.

### `ffi/remote/remote_test.go` — Client/Server integration (6 tests)

These test the lower-level `Client` and `Server` types directly (not the `DB`
interface).

| Test | Operations | Portable? |
|---|---|---|
| `TestBootstrapAndGet` | `Client.Bootstrap`, `Client.Get` | No — `Client`-specific |
| `TestUpdateAndVerify` | `Client.Update`, `Client.Get` | No — `Client`-specific |
| `TestMultipleUpdates` | `Client.Update` (sequential) | No — `Client`-specific |
| `TestBootstrapWithWrongHash` | `Client.Bootstrap` with bad hash | No — `Client`-specific |
| `TestCreateProposalReturnsWitness` | Raw gRPC `CreateProposal`/`CommitProposal` | No — tests server RPC directly |
| `TestDeleteOperation` | `Client.Update` with `Delete` | No — `Client`-specific |

### `ffi/remote/cache_test.go` — Read cache (12 tests)

These test the `readCache` and its integration with `RemoteDB`. They are
remote-internal implementation details.

| Test | Portable? |
|---|---|
| `TestCacheLookupMiss` | No — tests `readCache` directly |
| `TestCacheStoreAndLookup` | No |
| `TestCacheNilValue` | No |
| `TestCacheClear` | No |
| `TestCacheEviction` | No |
| `TestCacheInvalidateKey` | No |
| `TestCacheInvalidatePrefix` | No |
| `TestCacheInvalidateBatch` | No |
| `TestCacheInvalidationOnUpdate` | No — tests cache clearing on `RemoteDB.Update` |
| `TestCacheInvalidationOnBootstrap` | No — tests cache clearing on bootstrap |
| `TestCacheInvalidationOnCommit` | No — tests cache invalidation on commit |
| `TestCacheConcurrentGet` | No |

### `ffi/remote/concurrency_test.go` — Thread safety (4 tests)

These test concurrency properties of `RemoteDB` specifically.

| Test | Portable? |
|---|---|
| `TestConcurrentGets` | No — uses `RemoteDB` internal setup |
| `TestConcurrentGetDuringUpdate` | No |
| `TestConcurrentGetDuringClose` | No |
| `TestConcurrentGetAndRoot` | No |

### `ffi/remote/eviction_test.go` — Eviction policies (15 tests)

Pure unit tests for the `evictionStore` implementations (LRU, Clock,
SampleK-LRU, Random). No database involvement; not relevant to interface
portability.

### `ffi/remote/benchmark_test.go` — Performance benchmarks (13 benchmarks)

Remote-specific benchmarks measuring gRPC overhead, proof verification, witness
generation, and caching. Not candidates for interface migration.

## Analysis of `ffi/` Tests

### `ffi/db_test.go` — Interface tests (9 tests)

These already use the `DB` / `DBProposal` / `DBRevision` / `DBIterator`
interfaces via `NewLocalDB`. They are the primary candidates for running
against both backends.

| Test | Interface Operations | Remote-compatible? |
|---|---|---|
| `TestNewLocalDB` | `Root`, `Update`, `Get` | **Yes** |
| `TestLocalDBPropose` | `Propose`, `Root`, `Get`, `Commit` | **Yes** |
| `TestLocalDBProposalChain` | `Propose` (chained), `Get`, `Commit` | **Yes** |
| `TestLocalDBRevision` | `Revision`, `Root`, `Get`, `Iter`, `Drop` | **Yes** |
| `TestLocalDBRevisionNotFound` | `Revision` with bad root | **Yes** |
| `TestLocalDBPrefixDelete` | `Update` with `PrefixDelete`, `Get` | **Yes** |
| `TestLocalDBProposalIter` | `Propose`, `Iter`, `Next`, `Key`, `Err`, `Drop` | **Yes** |
| `TestLocalDBLatestRevision` | `LatestRevision`, `Root`, `Get` | **Yes** |
| `TestLocalDBLatestRevisionEmpty` | `LatestRevision` (empty DB) | **Yes** |

**Migration strategy**: Parameterize the `newLocalDB` helper to accept a `DB`
constructor, then run the same table of tests with both `NewLocalDB` and
`NewRemoteDB`.

### `ffi/firewood_test.go` — Concrete `Database` tests (32 tests)

These use the concrete `*Database` type directly. Each test is assessed for
whether its operations map onto the `DB`/`DBProposal` interfaces.

| Test | Key Operations | Remote-compatible? | Blocker |
|---|---|---|---|
| `TestUpdateSingleKV` | `Update`, `Root` | **Yes** | — |
| `TestNodeHashAlgorithmValidation` | `Update`, `Root` | **Yes** | — |
| `TestUpdateMultiKV` | `Update`, `Root` | **Yes** | — |
| `TestTruncateDatabase` | `ffi.New` with `WithTruncate` | **No** | Config option, not DB interface |
| `TestClosedDatabase` | Operations after `Close` | **No** | Tests concrete error behavior |
| `TestInsert100` | `Update` (100 keys), `Root` | **Yes** | — |
| `TestRangeDelete` | `Update` with `Delete`, `Get` | **Yes** | — |
| `TestInvariants` | `Update`, `Database.Dump` | **No** | Uses `Dump()` (concrete-only) |
| `TestConflictingProposals` | `Propose` (concurrent), `Commit`, error | **Partial** | Tests commit-conflict semantics; may differ |
| `TestDeleteAll` | `Update` with `Delete`, `Get` | **Yes** | — |
| `TestDropProposal` | `Propose`, `Drop`, `Get` | **Yes** | — |
| `TestProposeFromProposal` | `Propose` (chained), `Get`, `Commit` | **Yes** | — |
| `TestDeepPropose` | `Propose` (5+ levels), `Get` | **Yes** | — |
| `TestDropProposalAndCommit` | `Propose`, `Drop`, `Commit` error | **Partial** | Tests specific error from dropped parent |
| `TestProposeSameRoot` | `Propose` (identical ops), `Root` | **Yes** | — |
| `TestRevision` | `Revision(root)`, `Revision.Get` | **Yes** | Now uses `DB.Revision` / `DBRevision` interface |
| `TestRevisionOutlivesProposal` | `Revision`, `Propose`, `Drop` | **Yes** | Now uses `DB.Revision` / `DBRevision` interface |
| `TestCommitWithRevisionHeld` | `Revision`, `Propose`, `Commit` | **Yes** | Now uses `DB.Revision` / `DBRevision` interface |
| `TestRevisionOutlivesReaping` | `Revision`, multiple commits | **Partial** | Uses `WithRevisions()` config; lifecycle semantics may differ |
| `TestInvalidRevision` | `Revision(badRoot)` | **Yes** | Now uses `DB.Revision` / `DBRevision` interface |
| `TestGetNilCases` | `Get` nil key, nil vs empty values | **Yes** | — |
| `TestEmptyProposals` | `Propose` (empty batch), `Commit` | **Yes** | — |
| `TestHandlesFreeImplicitly` | GC/finalizer behavior | **No** | Tests FFI handle cleanup internals |
| `TestFjallStore` | `ffi.New` with `FjallStore` | **No** | Backend config option |
| `TestNilVsEmptyValue` | `Update`, `Get` distinguishing nil/empty | **Yes** | — |
| `TestDeleteKeyWithChildren` | `Update` with `Delete`, `Get` | **Yes** | — |
| `TestCloseWithCancelledContext` | `Close` with cancelled ctx | **No** | Tests concrete close behavior |
| `TestCloseWithMultipleActiveHandles` | `Close` with active handles | **No** | Tests concrete close behavior |
| `TestCloseSucceedsWhenHandlesDroppedInTime` | `Close` with deferred drops | **No** | Tests concrete close behavior |
| `TestDump` | `Update`, `Propose`, `Database.Dump`, `Proposal.Dump` | **No** | Uses `Dump()` |
| `TestProposeOnProposalRehash` | `Propose` (chained), `Root` | **Yes** | — |

### `ffi/iterator_test.go` — Iterator tests (8 tests)

These use the concrete `*Iterator` type with mode combinations (Owned/Borrowed,
Single/Batched). `SetBatchSize` and `NextBorrowed` are not on the `DBIterator`
interface.

| Test | Key Operations | Remote-compatible? | Blocker |
|---|---|---|---|
| `TestIter` | `Iter`, `Next`, `Key`, `Value` | **Partial** | Uses `SetBatchSize`, `NextBorrowed` |
| `TestIterOnRoot` | `Revision.Iter` | **Partial** | Uses `Revision` (now has interface), but also `SetBatchSize`/`NextBorrowed` |
| `TestIterOnProposal` | `Propose`, `Iter` | **Partial** | Uses `SetBatchSize`, `NextBorrowed` |
| `TestIterAfterProposalCommit` | `Propose`, `Commit`, `Iter` | **Partial** | Uses `SetBatchSize`, `NextBorrowed` |
| `TestIterUpdate` | `Iter`, `Update` (iter unchanged) | **Partial** | Uses `Revision.Iter` (now has interface), `SetBatchSize` |
| `TestIterDone` | `Iter` exhaustion safety | **Partial** | Uses `SetBatchSize`, `NextBorrowed` |
| `TestIterOutlivesRevision` | `Revision.Iter`, `Revision.Drop` | **Partial** | Uses `Revision` (now has interface), but also `SetBatchSize`/`NextBorrowed` |
| `TestIterOutlivesProposal` | `Propose`, `Iter`, `Drop` | **Partial** | Uses `SetBatchSize`, `NextBorrowed` |

**Migration strategy**: Each test runs in 4 mode combinations. The
`Owned`+default-batch mode is equivalent to the `DBIterator` interface and could
be extracted into interface-compatible variants. Tests that use `Revision` are
blocked entirely.

### `ffi/metrics_test.go` — Metrics tests (2 tests)

| Test | Remote-compatible? | Blocker |
|---|---|---|
| `TestMetrics` | **No** | Tests Rust-side Prometheus metrics via FFI |
| `TestExpensiveMetrics` | **No** | Tests Rust-side histogram metrics |

## Summary

### Remote-compatible `ffi/` tests (can run against `DB` interface today)

| Source File | Test | Status |
|---|---|---|
| `db_test.go` | `TestNewLocalDB` | Ready — already uses interface |
| `db_test.go` | `TestLocalDBPropose` | Ready — already uses interface |
| `db_test.go` | `TestLocalDBProposalChain` | Ready — already uses interface |
| `db_test.go` | `TestLocalDBRevision` | Ready — already uses interface |
| `db_test.go` | `TestLocalDBRevisionNotFound` | Ready — already uses interface |
| `db_test.go` | `TestLocalDBPrefixDelete` | Ready — already uses interface |
| `db_test.go` | `TestLocalDBProposalIter` | Ready — already uses interface |
| `db_test.go` | `TestLocalDBLatestRevision` | Ready — already uses interface |
| `db_test.go` | `TestLocalDBLatestRevisionEmpty` | Ready — already uses interface |
| `firewood_test.go` | `TestUpdateSingleKV` | Needs refactor to use `DB` |
| `firewood_test.go` | `TestNodeHashAlgorithmValidation` | Needs refactor to use `DB` |
| `firewood_test.go` | `TestUpdateMultiKV` | Needs refactor to use `DB` |
| `firewood_test.go` | `TestInsert100` | Needs refactor to use `DB` |
| `firewood_test.go` | `TestRangeDelete` | Needs refactor to use `DB` |
| `firewood_test.go` | `TestDeleteAll` | Needs refactor to use `DB` |
| `firewood_test.go` | `TestDropProposal` | Needs refactor to use `DB` |
| `firewood_test.go` | `TestProposeFromProposal` | Needs refactor to use `DB` |
| `firewood_test.go` | `TestDeepPropose` | Needs refactor to use `DB` |
| `firewood_test.go` | `TestProposeSameRoot` | Needs refactor to use `DB` |
| `firewood_test.go` | `TestGetNilCases` | Needs refactor to use `DB` |
| `firewood_test.go` | `TestEmptyProposals` | Needs refactor to use `DB` |
| `firewood_test.go` | `TestNilVsEmptyValue` | Needs refactor to use `DB` |
| `firewood_test.go` | `TestDeleteKeyWithChildren` | Needs refactor to use `DB` |
| `firewood_test.go` | `TestProposeOnProposalRehash` | Needs refactor to use `DB` |
| `firewood_test.go` | `TestRevision` | Needs refactor to use `DB.Revision` / `DBRevision` |
| `firewood_test.go` | `TestRevisionOutlivesProposal` | Needs refactor to use `DB.Revision` / `DBRevision` |
| `firewood_test.go` | `TestCommitWithRevisionHeld` | Needs refactor to use `DB.Revision` / `DBRevision` |
| `firewood_test.go` | `TestInvalidRevision` | Needs refactor to use `DB.Revision` / `DBRevision` |

**Total: 28 tests** (9 ready, 19 need refactoring to use interface)

### Partially compatible tests (need interface extensions or test changes)

| Source File | Test | Blocker |
|---|---|---|
| `firewood_test.go` | `TestConflictingProposals` | Tests commit-conflict error semantics |
| `firewood_test.go` | `TestDropProposalAndCommit` | Tests specific error from dropped parent |
| `firewood_test.go` | `TestRevisionOutlivesReaping` | Uses `WithRevisions()` config; lifecycle semantics may differ |
| `iterator_test.go` | `TestIter` | `SetBatchSize`/`NextBorrowed` modes |
| `iterator_test.go` | `TestIterOnRoot` | `SetBatchSize`/`NextBorrowed` modes (Revision now has interface) |
| `iterator_test.go` | `TestIterOnProposal` | `SetBatchSize`/`NextBorrowed` modes |
| `iterator_test.go` | `TestIterAfterProposalCommit` | `SetBatchSize`/`NextBorrowed` modes |
| `iterator_test.go` | `TestIterUpdate` | `SetBatchSize` (Revision now has interface) |
| `iterator_test.go` | `TestIterDone` | `SetBatchSize`/`NextBorrowed` modes |
| `iterator_test.go` | `TestIterOutlivesRevision` | `SetBatchSize`/`NextBorrowed` modes (Revision now has interface) |
| `iterator_test.go` | `TestIterOutlivesProposal` | `SetBatchSize`/`NextBorrowed` modes |

**Total: 11 tests** (could be made compatible with limited changes)

### Not compatible (concrete-type or implementation-specific)

| Source File | Tests | Reason |
|---|---|---|
| `firewood_test.go` | `TestTruncateDatabase` | Backend config option |
| `firewood_test.go` | `TestClosedDatabase` | Concrete close error behavior |
| `firewood_test.go` | `TestInvariants` | Uses `Dump()` |
| `firewood_test.go` | `TestHandlesFreeImplicitly` | FFI handle GC internals |
| `firewood_test.go` | `TestFjallStore` | Backend config option |
| `firewood_test.go` | `TestCloseWith*` (3 tests) | Concrete close behavior |
| `firewood_test.go` | `TestDump` | Uses `Dump()` |
| `metrics_test.go` | `TestMetrics`, `TestExpensiveMetrics` | Rust-side metrics |

**Total: 9 tests** (inherently implementation-specific)

Note: The 5 `Revision` tests (`TestRevision` through `TestInvalidRevision`)
previously listed here have been moved to the "Remote-compatible" or "Partially
compatible" sections now that `DBRevision` exists on the `DB` interface.

### Counts by file

| File | Total | Compatible | Partial | Not Compatible |
|---|---|---|---|---|
| `ffi/db_test.go` | 9 | 9 | 0 | 0 |
| `ffi/firewood_test.go` | 31 | 19 | 3 | 9 |
| `ffi/iterator_test.go` | 8 | 0 | 8 | 0 |
| `ffi/metrics_test.go` | 2 | 0 | 0 | 2 |
| **ffi/ total** | **50** | **28** | **11** | **11** |
| `ffi/remote/db_test.go` | 32 | — | — | — |
| `ffi/remote/remote_test.go` | 6 | — | — | — |
| `ffi/remote/cache_test.go` | 12 | — | — | — |
| `ffi/remote/concurrency_test.go` | 4 | — | — | — |
| `ffi/remote/eviction_test.go` | 15 | — | — | — |
| **ffi/remote/ total** | **69** | — | — | — |
