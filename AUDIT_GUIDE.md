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

**Change proof adversarial fuzz** (mutates generated proofs to model adversarial
conditions):

```bash
cargo nextest run --profile ci -p firewood test_slow_adversarial_change_proof_fuzz -- --exact
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

## Formal Methods

TLA+ model checking was used to verify the correctness of Firewood’s change proof verification approach. It provides more comprehensive coverage of both valid and invalid proofs even compared to fuzz testing, with the limitation that the verified tries are smaller in both their branching factors and their depth. This limitation is due to the computational requirements of exhaustive model checking. To verify larger configurations, the `-simulate` flag was used to not exhaustively enumerate all states but instead randomly sample states from the state space.

Model checking was effective at identifying several edge case security and correctness bugs. All of these bugs have since been fixed. With each bug fix, the models were updated to reflect the updated implementation, and regression specifications were added to ensure that the fixed bugs were not later reintroduced.

### Models

There are four main models in our testing framework that build on top of each other:

1. **CompressedTrieModel:** Models Firewood’s Merkle trie including path compression that reduces trie depth at the cost of increased complexity. Modeling path compression is critical as it is the source of many corner cases that must be verified. This model exhaustively enumerates through the entire key space for a specific maximum trie size, and generates a flat key-value map for each one. Keys are restricted to even nibble lengths, matching Firewood's byte-level key representation where each byte is two nibbles.

   The main use of this model is to determine the correctness of the compressed trie operators (Compress/Decompress/Lookup/Hash) by verifying a number of properties of the trie. These include **RoundTrip** (compress/decompress is lossless), **LookupCorrect**, **StructurallyValid** (follows rules such as a node with no children must have a value), **DepthBounded**, **Hash injectivity** (no two different flat maps produce the same root hash), etc.

2. **TrieOperations**: Models Insert and Delete on the trie. The state transition involves performing Inserts and Deletes to explore all reachable states. The main properties that it verifies are **TrieMatchesShadow**, where the incrementally maintained compressed trie is identical to one built from scratch, **TrieWellFormed**, and **LookupMatchesShadow**.

3. **ChangeProofVerification:** Models the full change proof generation and verification pipeline. Its purpose is to verify that honest proofs are always accepted. Its state machine consists of 4 variables: *startTrie*, *endTrie*, *startKey*, and *endKey*. They are used to represent two trie revisions, and a query range.

   The model checker enumerates through these variable values and runs each through a pipeline consisting of operations that closely match that of the Rust implementation. This includes computing and applying the difference between tries, reconciling proof nodes, collapsing consecutive proof node pairs, performing a hybrid hash where in-range children are hashed from the trie while out-of-range children use hashes from the proof nodes, and comparing the root hashes.

4. **AdversarialProof**: Models every possible combination of Put/Delete/Absent per in-range key in the trie diff, not just combinations that an honest generator can produce. It inherits the verification pipeline from ChangeProofVerification and verifies the **OnlyCorrectDiffAccepted** state property. The property requires that whenever verification accepts a change proof, the proposal agrees with the endTrie.

### Results

| Spec | Config | Mode | Property | Traces/States | Violations |
|------|--------|------|----------|---------------|-----------|
| CompressedTrieModel | BF=2, MaxDepth=2 | Exhaustive | Trie correctness | 81 states | 0 |
| CompressedTrieModel | BF=3, MaxDepth=2 | Exhaustive | Trie correctness | 19,683 states | 0 |
| TrieOperations | BF=3, MaxDepth=2 | Exhaustive | Insert/Delete correctness | 531,441 states | 0 |
| TrieOperations | BF=3, MaxDepth=4 | Simulate | Insert/Delete correctness | ~14,000 traces | 0 |
| ChangeProofVerification | BF=2, MaxDepth=2 | Simulate | Honest proofs always accepted | ~650,000 traces | 0 |
| ChangeProofVerification | BF=3, MaxDepth=2 | Simulate | Honest proofs always accepted | ~280,000 traces | 0 |
| ChangeProofVerification | BF=3, MaxDepth=4 | Simulate | Honest proofs always accepted | ~9,700 traces | 0 |
| AdversarialProof | BF=2, MaxDepth=2 | Simulate | Tampered diffs rejected | ~95,000 traces | 0 |
| AdversarialProof | BF=3, MaxDepth=2 | Simulate | Tampered diffs rejected | ~10,000 traces | 0 |
| AdversarialProof | BF=3, MaxDepth=4 | Simulate | Single-key tampered diffs rejected | ~4,000 traces | 0 |


**BF** (Branch Factor): The number of children per trie node. Firewood uses BF=16 (hex nibbles). The models use smaller values (2–3) for tractability.

**MaxDepth**: The maximum key length in nibbles. Keys are restricted to even lengths (matching Firewood's byte-level keys where each byte is two nibbles). MaxDepth=2 corresponds to 1-byte keys. MaxDepth=4 corresponds to 1–2 byte keys.
