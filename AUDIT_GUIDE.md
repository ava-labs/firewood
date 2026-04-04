# Firewood Proof Code Audit — Getting Started

## Scope

This audit covers the following areas:

- **Single key proofs** — inclusion and exclusion proofs for individual keys
- **Range proofs** — proofs over contiguous key ranges with boundary handling
- **Change proofs** — proofs that a trie transitioned correctly between two revisions
- **FFI/Go layer safety** — correctness and memory safety of the C FFI bindings and Go wrappers

The primary focus should be on Ethereum-compatible hashing (`ethhash` feature),
as this is the production configuration for EVM chains. The default merkledb
hashing mode (SHA-256) should also be reviewed but is secondary.

## Extracting and Building

Clone the repository and check out the audit branch:

```bash
git clone https://github.com/ava-labs/firewood.git
cd firewood
git checkout rkuris/cp-restructure-squashed
```

Follow the build instructions at:
<https://github.com/ava-labs/firewood/blob/main/README.md#build>

## Building the Documentation

Generate HTML documentation including private items:

```bash
cargo doc --document-private-items --no-deps
```

Open the generated docs in your browser:

```bash
open target/doc/firewood/index.html
```

## Key Documentation Entry Points

Start with the proofs module overview:

- **Proofs module** — `target/doc/firewood/proofs/index.html`

From there, the main areas of interest:

### Range Proofs

- `target/doc/firewood/proofs/range/struct.RangeProof.html`
- `target/doc/firewood/proofs/range/struct.RangeProofIter.html`
- Verification: `target/doc/firewood/merkle/fn.verify_range_proof.html`

### Change Proofs

- `target/doc/firewood/proofs/change/struct.ChangeProof.html`
- `target/doc/firewood/proofs/change/struct.ChangeProofVerificationContext.html`
- `target/doc/firewood/proofs/change/fn.verify_change_proof_structure.html`
- Root hash verification: `target/doc/firewood/merkle/fn.verify_change_proof_root_hash.html`

### Core Proof Types

- `target/doc/firewood/proofs/types/enum.ProofError.html`
- `target/doc/firewood/proofs/types/struct.ProofNode.html`
- `target/doc/firewood/proofs/types/struct.Proof.html`
- `target/doc/firewood/proofs/types/trait.ProofCollection.html`
- `target/doc/firewood/proofs/types/enum.ProofType.html`

## FFI Safety Design

The FFI layer bridges Rust and Go through C-compatible bindings. There is no
single design document, but the following strategies are documented inline and
worth understanding before reviewing the code.

### Keep-alive handles (`ffi/keepalive.go`)

Every Go object that holds a reference to a Rust-side resource acquires a
keep-alive handle backed by a `sync.WaitGroup` on the `Database`. Calling
`Database.Close()` blocks until all outstanding handles are released, preventing
use-after-free of the underlying Rust database. Double-free is prevented by
nil-assignment after release.

### Borrowed vs owned data (`ffi/src/value/borrowed.rs`, `ffi/src/value/owned.rs`)

Data crossing the FFI boundary is explicitly typed as either *borrowed* or
*owned*. Borrowed data points into Rust memory and is only valid for the
duration of a single FFI call; owned data is heap-allocated and must be freed
by the caller using the corresponding `free` function. The Rust-side types
carry `PhantomData` lifetimes to document these invariants.

### Go memory pinning (`ffi/memory.go`)

When Go memory is passed to C/Rust, Go's `runtime.Pinner` is used to pin the
backing arrays so the garbage collector cannot relocate them during the call.
Nested structures (e.g., batch operations containing multiple key-value slices)
have all their components pinned before the FFI call and unpinned afterward.

### Thread safety

Go wrappers protect all FFI object access with `sync.RWMutex` (reads) or
`sync.Mutex` (state changes). The Rust-side `BorrowedSlice<T>` types carry
explicit `unsafe impl Send + Sync` with documented safety justifications.

## Source Files

The relevant source code lives in:

| Area | Path |
| ------ | ------ |
| Proof types & traits | `firewood/src/proofs/types.rs` |
| Range proofs | `firewood/src/proofs/range.rs` |
| Change proofs | `firewood/src/proofs/change.rs` |
| Serialization | `firewood/src/proofs/ser.rs` |
| Deserialization | `firewood/src/proofs/de.rs` |
| Proof headers | `firewood/src/proofs/header.rs` |
| Proof stream reader | `firewood/src/proofs/reader.rs` |
| Range proof verification | `firewood/src/merkle/mod.rs` (`verify_range_proof`) |
| Change proof verification | `firewood/src/merkle/mod.rs` (`verify_change_proof_root_hash`) |
| FFI proof bindings (Rust) | `ffi/src/proofs.rs`, `ffi/src/proofs/range.rs`, `ffi/src/proofs/change.rs` |
| Go proof wrapper | `ffi/proofs.go` |
| Go memory management | `ffi/memory.go`, `ffi/keepalive.go` |

## Hashing Modes

Firewood supports two hashing modes controlled by the `ethhash` feature flag:

- **Default (merkledb)** — SHA-256 hashing, compatible with AvalancheGo's
  merkledb. This is what you get when building without feature flags.
- **`ethhash`** — Keccak-256 hashing, compatible with Ethereum and EVM chains.
  This mode also understands "account" nodes at specific trie depths with
  RLP-encoded values, and computes the account trie hash as the actual root.
  See `storage/src/hashers/ethhash.rs` for the implementation.

Most commands in this guide pass `--features ethhash` to build and test in
Ethereum-compatible mode. To test the default merkledb mode, omit the feature:

```bash
cargo nextest run --workspace --features logger --all-targets
```

## Running the Tests

```bash
cargo nextest run --workspace --features ethhash,logger --all-targets
```

## Fuzz Tests

There are two categories of fuzz tests relevant to the proof code.

### Rust Randomized Tests (proof-specific)

These are randomized property tests that build random tries, generate proofs,
and verify them across multiple boundary scenarios (existing keys, non-existent
keys, unbounded). They use a seeded RNG and print the seed on each iteration
so failures are reproducible.

**Change proof fuzz** (fixed 32-byte keys):

```bash
cargo nextest run --profile ci -p firewood test_slow_change_proof_fuzz -- --exact
```

**Change proof fuzz — variable-length keys** (1–4096 byte keys, exercises
shallow trie structures, prefix key relationships, and divergent exclusion
proofs):

```bash
cargo nextest run --profile ci -p firewood test_slow_change_proof_fuzz_varlen -- --exact
```

**Range proof fuzz**:

```bash
cargo nextest run --profile ci -p firewood test_range_proof_fuzz -- --exact
```

If a fuzz test fails, the output includes a seed value for reproduction. See
`target/doc/firewood_storage/test_utils/struct.SeededRng.html` for details.

> **Note:** These tests are prefixed `test_slow_` and are excluded from the
> default nextest profile. The `--profile ci` flag is required to run them.

### Go Differential Fuzz Tests (FFI)

These use Go's built-in `go test -fuzz` and exercise the FFI layer by
comparing Firewood's behavior against reference implementations. They require
building the FFI library first (see the FFI section in
[CLAUDE.md](CLAUDE.md#ffi) or the main README).

**Ethereum differential fuzz** (ethhash mode — compares against go-ethereum):

```bash
cd ffi/tests/eth
go test -race -fuzz=FuzzFirewoodTree -fuzztime=1m
go test -race -fuzz=FuzzRangeProofCreation -fuzztime=1m
```

**MerkleDB differential fuzz** (default hash mode — compares against AvalancheGo merkledb):

```bash
cd ffi/tests/firewood
go test -race -fuzz=FuzzTree -fuzztime=1m
```

Failing inputs are saved under `testdata/` in the respective test directory
and will be replayed on subsequent runs.
