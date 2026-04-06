# Formal Verification of Change Proof Verification

TLA+ specifications that formally verify the correctness of Firewood's change
proof verification algorithm using the TLC model checker.

## Overview

The specifications model the full change proof verification pipeline from
`firewood/src/merkle/mod.rs`: proof extraction, branch reconciliation,
intermediate branch collapsing, outside children classification, and hybrid
root hash computation. The model uses path-compressed tries matching the
Rust implementation's data structures.

The model found **four bugs**, three of which have been fixed. One remains
unfixed.

## Prerequisites

- Java runtime: `brew install openjdk`
- TLA+ tools: `tla2tools.jar` (included in this directory)

## Running

```bash
./run.sh CompressedTrieModel        # Validate compressed trie model (~30s)
./run.sh TrieOperations             # Validate insert/delete (~3m at BF=3)
./run.sh ChangeProofVerification    # Full pipeline check (~6m at BF=2)
./run.sh AdversarialProof           # Adversarial diff enumeration (<1s)
./run.sh RegressionPrefixDelete     # Prefix-delete regression (~30s)

./run-simulate.sh ChangeProofVerification   # Probabilistic BF=3 (~1h)
./run-adversarial-long.sh 2 8              # 8-worker adversarial for 2h
```

## Specifications

### CompressedTrie.tla (shared library, 207 lines)

Pure operators with no state machine. Foundation for all other specs.

**Operators:**

- `Compress(trie)` / `Decompress(ctrie)` — build and extract
  path-compressed tries from flat key-value maps
- `Lookup(ctrie, key)` — key lookup in a compressed trie
- `CompressedHash(node, prefix)` — abstract Merkle hash mirroring the Rust
  preimage structure from `storage/src/hashers/merkledb.rs` (full path,
  value, child hashes as a TLA+ record — injective by construction)
- `WellFormed(node)` — structural invariants: no single-child-no-value
  nodes, leaves always have values
- `CommonPrefixLen`, `SeqLte`, `InRange`, `GetValueAt`, `SetValueAt` —
  utilities

**Design:** A trie is a partial function `[AllKeys -> Values ∪ {NoVal}]`.
`Values == 1..NumValues` (finite set, default 2). Keys are nibble sequences
of length `1..MaxDepth`. The branch factor `BF` determines the nibble
alphabet `0..BF-1`. A compressed trie node is a record
`[pp, value, children]` where `pp` is the partial path (nibble sequence).
The hash includes the full nibble path (`parentPrefix ++ pp`), matching the
Rust implementation where the preimage includes `parent_prefix_path() ++
partial_path()`.

### CompressedTrieModel.tla (66 lines)

Validates the compressed trie model by exhaustive checking.

**State machine:** Each state is one flat trie from `AllTries`. TLC explores
all possible tries and checks invariants on each.

**Properties verified:**

| Property | What it checks |
|----------|---------------|
| RoundTrip | `Decompress(Compress(trie)) = trie` |
| LookupCorrect | `Lookup(Compress(trie), k) = trie[k]` for all keys |
| StructurallyValid | `WellFormed(Compress(trie))` |
| DepthBounded | No node path exceeds MaxDepth |
| Hash injectivity | Different tries produce different root hashes (ASSUME, small configs only) |

**Configuration:** BF=2, MaxDepth=2, NumValues=2 → 729 tries, 29 seconds.

### TrieOperations.tla (175 lines)

Validates that incremental `Insert` and `Delete` operations on a compressed
trie produce the same result as building from scratch via `Compress`.

**State machine:** Starts from empty. Non-deterministically applies
`Insert(key, value)` or `Delete(key)`. A shadow flat map tracks ground
truth.

**Operators:**

- `TrieInsert(ctrie, key, value)` — four cases: exact match (update
  value), descend into child, key-exhausted split, divergence split
- `TrieDelete(ctrie, key)` — navigate to key, remove value, flatten
  single-child branches via `Flatten`

**Invariants:** `ctrie = Compress(shadow)`, `WellFormed(ctrie)`,
`Lookup(ctrie, k) = shadow[k]` — all checked at every reachable state.

**Configuration:** BF=3, MaxDepth=2, NumValues=2 → 531,441 states, 19.1M
transitions, 3 minutes 21 seconds.

### ChangeProofVerification.tla (702 lines)

Models the full change proof verification pipeline and checks that honest
proofs are accepted. Also provides adversarial verification operators.

**Operators modeled (matching Rust code):**

| TLA+ operator | Rust function | Description |
|---------------|--------------|-------------|
| `ExtractProof` | `Merkle::prove` | Walk trie, collect proof nodes with child hashes |
| `InsertBranchAt` | `insert_branch_from_nibbles` | Create branch structure, split nodes |
| `ReconcileProofNode` | `reconcile_branch_proof_node` | Resolve value at target only via on_conflict |
| `CollapseStripNode` | `collapse_strip` | Strip non-on-path children, flatten |
| `CollapseRootToPath` | `collapse_root_to_path` | Strip root to match end_root shape |
| `ComputeOutsideChildren` | `compute_outside_children` | ChildMask boundary classification |
| `HybridHash` | `compute_root_hash_with_proofs` | In-range from trie, outside from proof |
| `RightEdgeKey` | `compute_right_edge_key` | Right boundary of proven range |
| `ProofFollowsKey` | `value_digest` branch check | Proof follows correct nibble toward target |

**Structural checks modeled:**

- `ProofFollowsKey` — each non-terminal proof node follows the correct
  branch toward the target key
- `EndProofOpConsistent` / `StartProofOpConsistent` — Put expects
  inclusion, Delete expects exclusion

**State machine:** Each state is `(startTrie, endTrie, startKey, endKey)`.
TLC explores all combinations.

**Invariant:** `HonestProofAccepted` — for valid ranges, the full pipeline
(extract proofs, reconcile, collapse, outside children, hybrid hash)
produces the correct root hash.

**Adversarial operators:**

- `VerifyWithDiff` — runs the pipeline with an externally supplied
  (possibly tampered) diff
- `SpuriousOps` — generates single-key tamperings of the honest diff

**Two modes:**

- Exhaustive (BF=2): enumerate all (startTrie, endTrie, startKey, endKey)
- Probabilistic (BF=3): TLC `-simulate` samples random tuples

**Configuration:** BF=2, MaxDepth=2, NumValues=2 → 531,441 trie pairs ×
range combos. Exhaustive ~6 minutes for honest check.

### AdversarialProof.tla (43 lines)

Exhaustive adversarial testing. Enumerates ALL possible diffs for each
(startTrie, endTrie, startKey, endKey) and checks the security property.

**Operator:**

- `AllPossibleDiffs(sk, ek)` — generates every possible diff (each
  in-range key can be Put with any value, Delete, or absent)

**Invariant:** `OnlyCorrectDiffAccepted` — if verification accepts a diff,
the proposal must agree with endTrie on every in-range key. Also checks
`HonestProofAccepted`.

**Configuration:** BF=2, MaxDepth=1 exhaustive (648 states) or BF=2
MaxDepth=2 simulation.

### RegressionPrefixDelete.tla (59 lines)

Regression test for the prefix-key deletion bug: verification must succeed
when a key that is a prefix of other keys is deleted between start_root and
end_root, and the query range excludes the deleted prefix.

**Invariant:** `PrefixDeleteVerifies` — for all configurations matching the
prefix-delete pattern, `VerifyChangeProof` returns TRUE.

## Configuration Parameters

| Parameter | Description | Typical values |
|-----------|-------------|---------------|
| BF | Branch factor (nibble values: 0..BF-1) | 2 (fast), 3 (divergence parent) |
| MaxDepth | Maximum key length in nibbles | 1 (tiny), 2 (standard) |
| NumValues | Distinct values per key | 2 (tests value-dependent hashing) |
| None | Model value sentinel | Assigned `None = None` in cfg |

**State space:**

| Config | AllKeys | AllTries | Trie pairs |
|--------|---------|----------|-----------|
| BF=2, MaxDepth=1, NV=2 | 2 | 9 | 81 |
| BF=2, MaxDepth=2, NV=2 | 6 | 729 | 531,441 |
| BF=3, MaxDepth=2, NV=2 | 12 | 531,441 | 282 billion |

BF=3 trie pairs are infeasible for exhaustive checking; use `-simulate`.

## Bugs Found

### Bug 1: Wrong-branch proof reuse — FIXED

`value_digest` verified that proof nodes were prefixes of the target key
but never checked that the proof followed the correct branch at each level.
An attacker could reuse a proof for key B (nibble 3) as an exclusion proof
for key C (nibble 5).

**Fix:** Commit `ac341f67e` — check that the nibble toward the next proof
node equals the nibble toward the target key.

**Found by:** `AdversarialProof.tla`

### Bug 2: Borrowed subtree gap — FIXED

The verifier unconditionally used `last_op_key` as the right boundary
(`effective_end`), creating a gap between `last_op_key` and `end_key` where
subtree hashes were borrowed from proof nodes rather than recomputed. An
attacker could hide changes in this gap.

**Fix:** Commit `148fb9d6b` — replace `effective_end` with
`right_edge_key` computed by `compute_right_edge_key`, which only shortens
the range when truncation is detected.

**Found by:** `AdversarialProof.tla`

### Bug 3: Root structure incompatibility — FIXED

Honest proofs were rejected when out-of-range key deletions changed
end_root's root compression. The proposal retained the old root shape,
and nothing reconciled the gap between the proposal root and the first
proof node.

**Fix:** Commit `d94527f5f` — `collapse_root_to_path` strips non-on-path
children from the proving trie root so its shape matches end_root.

**Found by:** `ChangeProofVerification.tla` `HonestProofAccepted` invariant

**Rust test:** `test_out_of_range_root_structure_change`

### Bug 4: State injection via root collapse — UNFIXED

The fix for Bug 3 introduced a new attack vector. `collapse_root_to_path`
strips ALL non-on-path children from the root, including keys injected by
an attacker. An attacker adds a spurious key at a different first nibble
than the proof path; the collapse strips it, hiding the injected key from
the hash computation. The verification accepts, but the verifier's trie
contains a key that doesn't exist in end_root.

Works with unbounded range [None, None] — no boundary or multi-round
involvement needed. The attacker just injects a key at an off-path nibble.

**Found by:** `AdversarialProof.tla` `OnlyCorrectDiffAccepted` invariant

**Rust test:** `test_collapse_root_hides_spurious_key`

## Limitations

### Model vs implementation gap

The TLA+ spec models the verification DESIGN (the pipeline, the formulas,
the hash computation), not the Rust IMPLEMENTATION. A bug in the Rust code
that correctly implements the modeled algorithm would not be caught.
Examples: off-by-one in nibble indexing, cursor misalignment, path
compression edge cases in the storage layer. Closing this gap requires a
tool like Kani that verifies Rust code directly.

### Bounded model checking

Properties are verified for specific BF and MaxDepth values, not for all
possible values. At BF=2, the trie has only 2-way branching and no
divergence parent case (no children strictly between two boundary nibbles).
BF=3 covers the divergence parent but requires probabilistic checking for
trie-pair properties. Results at BF=2-3 are believed to generalize because
the algorithms are parametric in BF, but TLC does not produce a
mathematical proof of this generalization.

### No truncation modeling

The model does not include a `max_length` parameter. All diffs contain the
complete set of changes — no truncated proofs. The truncation detection
logic (`compute_right_edge_key`) is modeled for the non-truncated case only
(always returns `end_key` when present). Truncated proof handling should be
verified separately.

### No path compression edge cases

The model uses the same node record type for all nodes. The Rust code
distinguishes `BranchNode` and `LeafNode` at the storage layer, with
specific handling for persisted vs in-memory children (`AddressWithHash`,
`MaybePersisted`, `Child::Node`). The model does not capture these
storage-level distinctions.

### Abstract hash function

The hash function is modeled as injective by construction (the hash IS the
canonical node content as a TLA+ record). Real collision resistance of
SHA-256 or Keccak-256 is assumed, not verified.

### Empty trie edge case

The model finds honest-proof violations when endTrie is completely empty
(all keys deleted). This case is not reachable through the Rust API in
non-ethhash mode because `change_proof()` requires both roots to be valid
revisions, and an empty trie has no stored root hash. With `ethhash`, an
empty trie has a well-defined hash and may be reachable.

## Test Coverage

Of the operators in `firewood/src/merkle/mod.rs` related to change proof
verification, the model covers:

| Function | Modeled | Notes |
|----------|---------|-------|
| `verify_change_proof_root_hash` | Yes | Full pipeline |
| `compute_outside_children` | Yes | All 4 terminal cases |
| `compute_root_hash_with_proofs` | Yes | Hybrid hash |
| `reconcile_branch_proof_node` | Yes | Target-only value resolution |
| `collapse_branch_to_path` | Yes | Between consecutive proof nodes |
| `collapse_root_to_path` | Yes | Root reshaping |
| `insert_branch_from_nibbles` | Yes | Structure creation |
| `Proof::value_digest` | Partial | ProofFollowsKey + op consistency |
| `verify_change_proof_structure` | Partial | Op consistency, range checks |
| `DiffMerkleNodeStream` | No | Diff computed directly from flat maps |
| `verify_range_proof` | No | Range proofs not modeled |
