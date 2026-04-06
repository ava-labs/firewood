# Runtime Hash Selection Retrospective

## Summary

This document now serves as an implementation-informed retrospective for making
Firewood's hashing mode truly runtime-selectable. The feature is complete in
this branch on both the Rust side and the Go FFI side.

The implemented end state is:

- one Rust build supports both MerkleDB hashing and Ethereum hashing
- the selected hash mode comes entirely from `NodeHashAlgorithm`
- the selected hash mode is persisted in the database header
- different databases with different hash modes can be opened by the same
  process at the same time
- the Go FFI no longer relies on build-time hash-mode selection
- the Go FFI requires callers to select the hash mode explicitly

## Scope

This document primarily covers the Rust implementation, but it also captures
the Go FFI learnings that were discovered while completing the end-to-end
feature:

- `firewood`
- `firewood-storage`
- `firewood-ffi` Rust bindings
- `ffi` Go wrapper and tests

Out of scope:

- release and migration policy
- downstream consumer migration policy

## Existing Groundwork

The main preparatory change has already landed in `301308d16`:
`feat!: Make NodeHashAlgorithm a required option on storage (#1608)`.

That change already provides:

- `NodeHashAlgorithm` as an explicit runtime value
- `node_hash_algorithm` on `Db::new(...)`, `ConfigManager`, `FileBacked`, and
  `MemStore`
- `ReadableStorage::node_hash_algorithm()`
- persistence of the selected mode in `NodeStoreHeader`

This turned out to be the right foundation. The remaining work was mostly
removing compile-time assumptions that still existed above and below that
plumbing.

One implementation detail that became important later: the FFI Rust ABI already
carried `DatabaseHandleArgs.node_hash_algorithm`, so no new C ABI was required
to complete the Go-side work.

## Final Implemented State

### Rust runtime selection

The Rust implementation now behaves as follows:

- both MerkleDB and Ethereum hashers are compiled into the same build
- `NodeHashAlgorithm` drives hashing, proof encoding, empty-root behavior, and
  database validation at runtime
- node serialization and deserialization are parameterized by the selected hash
  mode
- proof objects carry their hash mode in memory and on the wire
- database open validates the caller-selected mode against the persisted header
  mode rather than against a build flag

### Go FFI runtime selection

The Go FFI now behaves as follows:

- `ffi.New(path, nodeHashAlgorithm, ...)` requires the caller to choose the
  hashing mode explicitly
- tests and compatibility suites choose hash mode explicitly instead of trying
  to infer it from the build
- the generated header and docs describe runtime selection rather than
  compile-time feature matching

### Cargo feature status

The `ethhash` Cargo feature is no longer part of the workspace behavior model.
The implementation no longer depends on it for selecting hashing mode, and the
workspace tooling, docs, and tests have been updated accordingly.

## Original Blockers

### Compile-time validation still rejected the runtime choice

`storage/src/nodestore/hash_algo.rs` currently treats the runtime enum as a
placeholder for a future runtime design. `validate_init()` rejects any
`NodeHashAlgorithm` that does not match `cfg!(feature = "ethhash")`.

That check is enforced during header read and validation in
`storage/src/nodestore/header.rs`.

Result: the runtime enum exists, but the build still hard-locks the actual
algorithm.

### Only one hashing implementation was compiled

`storage/src/hashers/mod.rs` currently compiles exactly one module:

- `merkledb` when `ethhash` is disabled
- `ethhash` when `ethhash` is enabled

Result: even after removing the validation check, the binary still would not
contain both implementations.

### Core hash-related types changed shape at compile time

`storage/src/hashtype.rs` currently changes `HashType` based on the feature:

- MerkleDB build: `HashType = TrieHash`
- Ethereum build: `HashType = HashOrRlp`

This is the largest structural blocker. Runtime selection requires one stable
type layout that can represent both modes in the same build.

### Hashing behavior was selected with `#[cfg]`, not data

Several core paths branch on `feature = "ethhash"`:

- `storage/src/nodestore/hash.rs`
- `storage/src/hashednode.rs`
- `storage/src/checker/mod.rs`
- `storage/src/nodestore/mod.rs`
- `firewood/src/merkle/parallel.rs`
- `firewood/src/api.rs`
- `firewood/src/proofs/mod.rs`
- `firewood/src/proofs/ser.rs`
- `firewood/src/proofs/de.rs`

These branches encode more than just "use sha2 vs sha3". They also encode
algorithm-specific trie semantics such as:

- Ethereum account-subtree fake-root handling
- Ethereum key validity rules
- Ethereum empty-root behavior
- proof serialization differences
- whether long values are represented as `ValueDigest::Hash`

## Design Goals

- remove the `ethhash` feature from `firewood` and `firewood-storage`
- support both hash modes in one binary
- keep the database file header as the source of truth for existing databases
- avoid changing the high-level Firewood API shape unless required
- keep MerkleDB behavior unchanged for MerkleDB databases
- keep Ethereum-compatible behavior unchanged for Ethereum databases

## Non-Goals

- redesigning the trie model
- changing proof formats beyond what is required to carry per-proof hash mode
- mixing hash modes inside a single database

## Implemented Design

### 1. Make `NodeHashAlgorithm` the only selector

`NodeHashAlgorithm` became the sole authority for algorithm-specific behavior.

Required changes:

- remove `compile_option()`
- remove `matches_compile_option()`
- remove `validate_init()`
- keep header validation that compares the caller-selected mode against the
  persisted header mode

Implemented result:

- creating a database uses the caller-provided `NodeHashAlgorithm`
- opening a database succeeds only if the caller-provided mode matches the
  header mode
- the build itself no longer constrains which mode may be used

### 2. Compile both hashing implementations unconditionally

`storage/src/hashers/merkledb.rs` and `storage/src/hashers/ethhash.rs` are now
built in all normal builds.

Required changes:

- stop using the `ethhash` feature for behavior in `firewood/Cargo.toml` and
  `storage/Cargo.toml`
- keep a temporary no-op compatibility alias if downstream crates, tests, or
  build scripts still enable `ethhash`
- promote current optional `ethhash` dependencies to normal dependencies where
  needed:
  - `rlp`
  - `sha3`
  - `bytes`
- make `storage/src/hashers/mod.rs` unconditionally expose both implementations

This was a prerequisite for every remaining runtime-dispatch change.

### 3. Give `HashType` one stable representation

`HashType` no longer changes shape at compile time.

Implemented approach:

- use a single enum in all builds, equivalent to the current Ethereum-capable
  representation
- represent both modes as:
  - `Hash(TrieHash)`
  - `Rlp(...)`

Under this design:

- MerkleDB hashing always produces `Hash`
- Ethereum hashing may produce either `Hash` or `Rlp`

Why this turned out to be the right fit:

- it matches the existing Ethereum implementation
- it avoids pervasive generics or trait-object dispatch
- it preserves the current proof and child-hash model
- it lets shared code handle both algorithms with one type

Required follow-up:

- update `IntoHashType`
- update helpers that assume `HashType` is always `TrieHash` in MerkleDB builds
- audit all `into_triehash()` and comparison sites for assumptions about
  `HashType`

### 4. Move hash behavior behind runtime dispatch

Code that previously used `#[cfg(feature = "ethhash")]` for algorithm behavior
now dispatches on `self.storage.node_hash_algorithm()` or an equivalent
explicit `NodeHashAlgorithm` parameter.

In practice, this worked best when done at a few high-leverage boundaries
instead of scattered at every leaf.

Recommended boundary methods:

- `NodeHashAlgorithm::is_ethereum()`
- `NodeHashAlgorithm::empty_root_hash() -> Option<TrieHash>`
- `NodeHashAlgorithm::proof_hash_mode() -> u8`
- `NodeHashAlgorithm::is_valid_stored_key(&Path) -> bool`
- `NodeHashAlgorithm::hash_node(...) -> HashType`
- `NodeHashAlgorithm::hash_subtrie(...) -> Result<(MaybePersistedNode, HashType), FileIoError>`

The exact placement mattered less than keeping call sites simple and explicit.

### 5. Refactor `NodeStore` hashing paths first

`storage/src/nodestore/hash.rs` is the most important implementation site.

This file currently has two different execution models:

- MerkleDB hashing is pure and does not need storage access during recursion
- Ethereum hashing may need storage access because previously hashed children
  sometimes need to be re-read and rehashed

Implemented refactor:

- make `hash_helper()` always take `&self`
- make the recursive helper always use one signature
- branch at runtime inside the helper based on `self.storage.node_hash_algorithm()`
- keep the Ethereum-specific fake-root logic, but only execute it in Ethereum
  mode

This change removes several awkward `#[cfg]` differences in:

- `storage/src/nodestore/hash.rs`
- `storage/src/nodestore/mod.rs`
- `firewood/src/merkle/parallel.rs`

### 6. Make `hashednode` runtime-aware

`storage/src/hashednode.rs` currently bakes algorithm-specific behavior into the
types:

- `ValueDigest::Hash` only exists in non-ethhash builds
- `make_hash()` is compile-time conditional
- verification behavior changes at compile time

This is now runtime behavior.

Implemented direction:

- keep `ValueDigest::Hash` available in all builds
- make `ValueDigest::make_hash()` take a `NodeHashAlgorithm`
- make proof verification and serialization use the algorithm that the proof
  declares

Expected behavior:

- MerkleDB mode may hash long values into `ValueDigest::Hash`
- Ethereum mode keeps the current inline-value behavior

### 7. Make proof format mode explicit per proof

`firewood/src/proofs/mod.rs` previously exposed a compile-time `HASH_MODE`
constant. That could not survive runtime selection.

Required changes:

- remove compile-time `HASH_MODE`
- store the hash mode in the proof header based on the originating database's
  `NodeHashAlgorithm`
- use the header's hash mode during serialization and deserialization
- make `ValueDigest` and `HashType` proof encoding accept both representations

This affected at least:

- `firewood/src/proofs/mod.rs`
- `firewood/src/proofs/ser.rs`
- `firewood/src/proofs/de.rs`

This was also the most compatibility-sensitive area: proof readers must reject
unknown modes, but they can no longer depend on compile-time feature selection.

### 8. Make API defaults mode-aware instead of build-aware

`firewood/src/api.rs` currently makes the default empty root hash depend on the
build:

- MerkleDB build returns `None`
- Ethereum build returns the empty RLP hash

That needed to become mode-aware, and now is.

This likely requires changing where the "default root hash" decision is made.
The current `HashKeyExt::default_root_hash()` trait has no database context, so
it cannot make a correct runtime decision by itself.

Implemented direction:

- move empty-root handling closer to `DbView` / `NodeStore`
- compute the default root from the database's `NodeHashAlgorithm`
- avoid any global "current build hash mode" helper

### 9. Make checker behavior runtime-aware

`storage/src/checker/mod.rs` currently switches key validation and hash checking
with `#[cfg]`.

Required changes:

- validate keys using the storage's `NodeHashAlgorithm`
- compute hash checks using the storage's `NodeHashAlgorithm`
- carry any Ethereum-only traversal metadata, such as `has_peers`, only when
  needed by the selected mode

This was also a good place to centralize runtime helpers for:

- valid key shape
- node hash calculation for verification

### 10. Remove compile-option assumptions from tests, examples, and benches

Many tests and examples previously used `NodeHashAlgorithm::compile_option()`.
That helper needed to disappear, and now has.

Required changes:

- make tests choose the desired algorithm explicitly
- parameterize shared tests over both modes where practical
- update examples and benches to pass `NodeHashAlgorithm::MerkleDB` or
  `NodeHashAlgorithm::Ethereum` directly

This is important because the test suite now verifies that one build can
exercise both modes.

### 11. Keep the Go FFI API mode-aware, not build-aware

The Go wrapper originally carried the hash mode as a required positional
argument while the surrounding tests and docs still assumed build-time feature
selection.

The implemented Go shape is:

- require `ffi.New(path, nodeHashAlgorithm, ...)` to receive the desired hash
  mode explicitly
- keep the Rust/C ABI unchanged by continuing to populate
  `DatabaseHandleArgs.node_hash_algorithm`
- make Go tests choose the desired mode directly instead of probing the build

This turned out to be the right shape for the Go API because:

- callers cannot accidentally rely on a default hash mode when opening an
  existing database
- both Ethereum and MerkleDB remain explicit and readable
- the constructor signature makes the compatibility-sensitive choice impossible
  to miss
- the ABI boundary stays stable

### 12. Clean up user-facing build assumptions

The implementation was not complete when hashing worked at runtime. It was only
complete once user-facing strings, generated headers, docs, scripts, and tests
stopped describing hash selection as a build-time decision.

That cleanup included:

- removing feature-gated language from docs and generated headers
- updating error wording that still said "feature not supported in this build"
- removing `--features ethhash` from workspace validation and benchmarking
  flows
- updating compatibility tests to match runtime hash-mode selection

## Implementation Learnings

### Node serialization is part of the runtime-selection problem

The original design correctly identified hashing and proofs as blockers, but it
understated one important dependency: node serialization itself is
algorithm-dependent.

In practice:

- MerkleDB node serialization writes child hashes as raw 32-byte hashes
- Ethereum node serialization writes child references as hash-or-inline-RLP
- deserialization must know the database's `NodeHashAlgorithm` to preserve
  on-disk compatibility

This means runtime selection is not only about computing hashes. It also
requires `Node::as_bytes(...)`, `Node::from_reader(...)`, and `HashType`
encoding helpers to accept the selected algorithm explicitly.

This was a critical implementation finding because it affects compatibility
with already-persisted databases.

### Proofs must carry hash mode in memory, not only on the wire

The proof header's hash mode is necessary but not sufficient.

During implementation, it became clear that proof verification and proof
composition also need the mode available after deserialization and during
purely in-memory operations.

That led to the following implementation shape:

- `Proof<T>` carries `NodeHashAlgorithm`
- `Proof::new_with_hash_algorithm(...)` and
  `Proof::empty_with_hash_algorithm(...)` are required
- `RangeProof` and `ChangeProof` must enforce that both boundary proofs use the
  same algorithm
- proof serialization writes the mode from the originating proof object, not
  from a compile-time constant

Without this, the proof header can round-trip correctly while in-memory proof
verification still hashes nodes with the wrong algorithm.

### Test scaffolding must be explicit about hash mode

The implementation also exposed a test-harness trap.

Historically, several tests used shared helpers that selected the hash mode via
`NodeHashAlgorithm::compile_option()`. Once the runtime design removed that
helper, some Ethereum-only tests quietly started building MerkleDB tries unless
their helpers were parameterized explicitly.

The practical consequence is:

- test helpers need an explicit `NodeHashAlgorithm` parameter where mode matters
- MerkleDB-oriented helpers may still default to `NodeHashAlgorithm::MerkleDB`
- Ethereum compatibility tests must construct Ethereum-mode tries explicitly

This should be treated as a dedicated phase in any follow-up work, not as
cleanup.

### The Rust workspace surface is wider than `firewood` and `firewood-storage`

Even though the core refactor lives in those two crates, the implementation had
to touch other Rust crates in the workspace because they depended on the old
compile-time helpers or build-aware empty-root behavior.

The affected Rust consumers included:

- `firewood-ffi` (Rust side only, not the Go wrapper)
- `firewood-replay`
- `firewood-benchmark`
- examples and benches under `firewood`

This is worth planning for up front because it is easy to underestimate the
tail of compile-fix work after the storage and proof refactors are done.

### Full feature deletion still had a distinct rollout tail

Even after the core runtime-selection refactor was complete, deleting the
feature name itself still required a follow-up cleanup pass through manifests,
docs, tests, and scripts.

That turned out to be worth tracking separately because:

- downstream crates and scripts were still passing `--features ethhash`
- several docs still described `ethhash` as a functional toggle
- removing the feature name was mostly compatibility and rollout work, not core
  runtime-selection work

Future phased work should still treat "behavior no longer depends on the
feature" and "feature key is deleted from manifests and tooling" as separate
milestones.

### The Go API did not need a new ABI, but it did need a new shape

The FFI Rust boundary already had the right data field, so the Go-side work did
not require a new C ABI or Rust FFI function signature.

What did need to change was the Go wrapper API shape:

- the default path needed to align with the intended consumer experience
- the override needed to look like every other Go functional option
- tests needed to stop inferring mode from the build and start selecting it
  explicitly

The important lesson is that "runtime-configurable at the ABI layer" does not
automatically mean "runtime-configurable at the user API layer." Those are
separate design problems.

### User-facing defaults are part of the feature, not just ergonomics

The feature did not feel complete until the default behavior was chosen and
made explicit.

For the Go FFI, callers now pass the desired mode directly to
`ffi.New(path, nodeHashAlgorithm, ...)`.

That default required coordinated updates to:

- constructor docs
- README examples
- compatibility tests
- generated headers

In future phased work, the default behavior should be decided early rather than
left as a late wrapper cleanup.

### Runtime selection changes error wording and support expectations

Once both modes are available in one build, error messages that blame the build
become wrong even when the underlying operation is still unsupported for a
particular mode.

That surfaced in the proof/code-hash path, where the correct framing became:

- "operation not supported for this hash mode"

instead of:

- "feature not supported in this build"

This is a small detail, but it affects how users reason about supportability and
misconfiguration.

## Completion Status

The completed implementation established the following:

- one Rust build now supports both MerkleDB and Ethereum hashing
- `NodeHashAlgorithm` is now the runtime selector for storage, hashing, proof
  verification, proof serialization, and empty-root behavior
- database open still validates the caller-selected mode against the persisted
  header mode
- the `ethhash` feature has now been removed from the Rust workspace manifests
- workspace tooling now builds and tests with `--features logger` rather than
  `--features ethhash,logger`
- the Go FFI requires an explicit hash-mode argument
- the generated FFI header and Go docs now describe runtime hash selection

## Suggested Reimplementation Order

### Phase 1: plumbing cleanup

- remove compile-time validation in `hash_algo.rs`
- keep header compatibility checks
- compile both hasher modules
- make Cargo dependencies unconditional

### Phase 2: stable shared types

- make `HashType` unconditional
- make `ValueDigest` unconditional
- update helper traits and conversions

### Phase 3: hashing path conversion

- refactor `nodestore/hash.rs` to runtime dispatch
- update `nodestore/mod.rs` and `merkle/parallel.rs`
- update reconstructed-view hashing

### Phase 4: correctness surfaces

- update checker
- update empty-root behavior
- update proof serialization and deserialization

### Phase 5: API and consumer cleanup

- update the Go FFI constructor shape and default behavior
- keep FFI ABI plumbing stable if possible
- update tests to select the hash mode explicitly
- update generated headers and public docs

### Phase 6: downstream cleanup

- remove the `ethhash` feature from manifests, docs, and scripts
- update tests, examples, benches, and scripts that still pass the feature
- update downstream consumers and packaging

## Risks

### Type churn around `HashType`

Making `HashType` unconditional will touch a large part of the storage and proof
code. This is the most invasive part of the refactor.

### Proof compatibility regressions

Proof encoding currently assumes a compile-time mode. Refactoring this without
breaking verification will require careful test coverage.

### Subtle Ethereum hashing regressions

Ethereum account-subtree behavior is the least conventional part of the hashing
logic. Runtime dispatch must preserve current fake-root and rehash behavior
exactly.

### Hidden compile-time assumptions in tests

The current test suite contains many mode-specific compile-time branches. Those
can mask regressions until the suite is made explicitly dual-mode.

## Performance Considerations

The goal of this refactor is to make hash mode runtime-selectable without a
meaningful performance regression.

### Expected cost of runtime selection

The runtime selection itself should be cheap.

In the hot path, the expensive work is:

- computing `Sha256` or `Keccak256`
- RLP encoding in Ethereum mode
- Ethereum-specific child rehash and fake-root handling
- node traversal and storage reads

Compared to that work, a single `match` on `NodeHashAlgorithm` at the start of
hashing a node or subtrie should be negligible.

This should remain true if we follow two rules:

- branch once at a high-leverage boundary
- call dedicated algorithm-specific implementations after that branch

The design should avoid repeatedly branching inside the innermost loops when the
mode is already known for the current database.

### Main performance risk

The main risk is not runtime dispatch itself. The main risk is the type
generalization required to support both modes in one build.

The most important examples are:

- `HashType` becoming an unconditional type that can represent both hash-only
  and hash-or-RLP values
- `ValueDigest` becoming unconditional across both modes

If those changes are implemented carelessly, MerkleDB mode may pay for:

- larger values moved through hot code paths
- additional branching on enum variants
- reduced compiler simplification in MerkleDB-only paths

That is the area most likely to cause measurable regressions.

### Implementation guidance

To minimize performance impact:

- prefer enum dispatch over trait objects in hot code
- do not box hashers or hashing strategies
- keep MerkleDB and Ethereum hashing logic in separate concrete functions
- dispatch once per operation, then stay in the selected implementation
- preserve MerkleDB fast paths where values are known to be normal hashes
- avoid repeated conversion between `TrieHash` and generalized hash wrappers

In practice, this means code should look more like:

- `match alg { MerkleDB => hash_merkledb(...), Ethereum => hash_ethereum(...) }`

and less like:

- repeated per-field branching deep inside shared helpers
- trait-object based "strategy" calls in tight loops

### Benchmarking expectations

This refactor should be treated as performance-sensitive and verified with
benchmarks before and after the change.

Minimum benchmark coverage should include:

- MerkleDB proposal creation and commit
- Ethereum proposal creation and commit
- trie hashing for both modes
- reconstructed view root hashing for both modes
- proof serialization and deserialization for both modes

The existing hash and proposal benches are a starting point, but they should be
updated to run both algorithms in the same build.

### Acceptance criteria

The target is:

- no meaningful regression attributable to runtime dispatch itself
- any measurable regression must come from shared-type generalization and be
  justified or optimized away

If benchmarking shows MerkleDB regressions after the refactor, the first place
to inspect should be generalized `HashType` and `ValueDigest` handling, not the
top-level `NodeHashAlgorithm` branch.

## Validation During Implementation

The completed implementation was validated with:

- `cargo fmt`
- `cargo build -p firewood-ffi`
- `cargo nextest run --workspace --features logger --all-targets --profile ci`
- `cargo clippy --workspace --features logger --all-targets`
- `cargo doc --no-deps`
- `markdownlint-cli2 ffi/README.md docs/plans/runtime-hash-selection-rust.md`
- local Go validation with `go test ./...` in `ffi`

## Testing Plan

The refactor should be verified with both algorithms in the same build.

Minimum expectations:

- unit tests for both `NodeHashAlgorithm` modes in `firewood-storage`
- unit tests for both modes in `firewood`
- proof round-trip tests for both modes
- checker tests for both modes
- reconstructed-view hash tests for both modes
- parallel proposal tests for both modes
- database reopen tests that verify header-mode enforcement

Additional targeted tests:

- opening a MerkleDB database and an Ethereum database in the same process
- empty-trie root behavior for both modes
- Ethereum account single-child fake-root cases
- long-value proof encoding in MerkleDB mode
- absence of `ValueDigest::Hash` in Ethereum proofs

## Remaining Questions

### Should `HashType` remain public in its current form?

If `HashType` becomes an unconditional enum, the public API surface of
`firewood-storage` becomes more Ethereum-aware even in MerkleDB-only use cases.
That is likely acceptable, but it should be an intentional decision.

### Should MerkleDB-only fast paths remain specialized?

After the refactor, some hot paths may still deserve internal fast paths for
MerkleDB mode. Those optimizations should be added only after the runtime design
is correct and well tested.

### Should other user-facing APIs adopt different defaults?

The Go FFI now requires an explicit hash mode, while other surfaces such as
internal Rust tests and some tooling still default to MerkleDB-oriented
configuration.

That split is intentional today, but future product-facing APIs should make
their defaults explicit and justified rather than inheriting old assumptions.

## Deliverable

The refactor is complete when all of the following are true:

- one Rust build supports both hash modes
- `NodeHashAlgorithm` is selected entirely at runtime
- all hash-mode-sensitive behavior depends on database state, not build flags
- the test suite exercises both modes in the same build
- `ethhash` is removed from the Rust workspace as a Cargo feature
- the Go FFI exposes runtime hash selection without relying on build-time
  feature matching
- user-facing docs, generated headers, and error messages describe runtime
  selection accurately
