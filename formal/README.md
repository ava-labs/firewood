# Formal Verification of Change Proof Verification

TLA+ specifications that formally verify the correctness of Firewood's change
proof verification design using the TLC model checker.

## Background

Firewood's change proof verification uses a novel technique: pre-verified hash
chains enable direct in-range child comparison, avoiding trie reconstruction
and rehash. The correctness of this technique depends on precise boundary
nibble derivation, correct in-range classification, and sound hash chain
verification. These properties have been formally verified by exhaustive model
checking across all small trie instances.

## What This Provides

**1. Proves the novel technique is mathematically sound.**

Firewood's in-range children comparison (instead of trie reconstruction) is not
used by any other blockchain state sync implementation. The informal argument
-- "if in-range children match and the hash chain is valid, the substitution is
a no-op" -- is intuitive but had never been verified against all possible trie
shapes. TLC checked it exhaustively for all 64 possible tries at BF=2 and
confirmed the boundary classification is correct for 20 keys at BF=4. This is
stronger than any amount of unit testing because TLC checks every combination,
not just the ones a developer thought to write.

**2. Caught a real modeling error that reflects a real design subtlety.**

P10 (empty batch\_ops attack detection) initially failed. The model only
checked in-range children, but differences at the proven key itself are caught
by the value check, not the children check. This is exactly the kind of subtle
interaction between two verification mechanisms that is easy to miss in a code
review but impossible to miss in a formal model. The fact that TLC found it
instantly -- while multiple review passes and exhaustive code reading did not
flag it -- demonstrates the value of formal methods.

**3. Confirmed both known bugs were real and their fixes are necessary.**

Two bugs from the git history have been formally reproduced by TLC:

- **Bug 1** (commit `9c43a6cb4`): Before the fix, the start proof's
  inclusion/exclusion result was discarded. `value_digest` was called for hash
  chain verification, but both `Ok(Some)` (inclusion) and `Ok(None)`
  (exclusion) were accepted regardless of the first batch op's type. TLC
  confirms: there EXISTS a (trie, key) combination where the buggy version
  accepts a Put at an absent key that the fixed version correctly rejects.

- **Bug 2** (end\_nibbles exploit): Before the fix, when `batch_ops` was empty,
  `end_nibbles` was derived from `last_op_key` (which doesn't exist), producing
  None. TLC confirms: (a) `IsInRange(n, None, TRUE) = FALSE` for all nibbles --
  zero children are checked, and (b) there EXISTS (startTrie, endTrie, endKey)
  where the tries differ within range but the buggy code detects nothing.

These bug reproductions serve as regression tests: if someone removed the start
proof consistency check or the end\_key fallback, TLC would report an error.

**4. No new bugs found.**

Beyond the two known bugs, all 35 design checks and 3 protocol checks passed
on the first correct formulation. No new bugs were discovered in the change
proof verification design.

**What this does NOT provide:**

It does not verify the Rust implementation. The TLA+ spec models the design --
the formulas, the protocol, the attack scenarios. A bug in the Rust code that
correctly implements the wrong formula, or that has an off-by-one in nibble
indexing, or that mishandles trie compression, would not be caught. That gap
requires Kani or similar tools that operate on the actual code.

## Prerequisites

- Java runtime: `brew install openjdk`
- TLA+ tools: `tla2tools.jar` (already in this directory)

## Running

```bash
./run.sh                   # ChangeProofVerification spec
./run.sh SyncProtocol      # Multi-round sync protocol spec
```

Both should report `Model checking completed. No error has been found.`

## Configuration

Edit `ChangeProofVerification.cfg` to change the branch factor and depth:

```text
CONSTANTS
    BF = 4
    MaxDepth = 2
    None = None
```

- `BF`: Branch factor. Use 2 for fast runs (checks all properties including
  P1/P6/P10/P14 which enumerate all possible tries). Use 4 for thorough
  boundary checks (43 seconds). Properties that enumerate `SUBSET AllKeys`
  are guarded by `BF <= 3` and skip automatically at larger branch factors.
- `MaxDepth`: Maximum trie depth. Use 2 for fast runs, 3 for deeper coverage.

## Specifications

### ChangeProofVerification.tla

Models a hexary Merkle trie with configurable branch factor and bounded depth.
Verifies 35 properties as ASSUME checks evaluated by TLC. These cover hash
chain soundness, boundary classification, structural validation, operation
consistency, truncated proofs, exclusion proofs, attack detection, deferred
mismatch scenarios, and reproduction of two known bugs.

### SyncProtocol.tla

Models the multi-round sync protocol as a state machine. The verifier syncs
state from a prover by fetching change proofs in successive rounds. TLC
enumerates all possible (startTrie, endTrie) pairs and all possible sync
executions (including adversarial provers that send arbitrary subsets of
changes). Verifies 3 invariants: Safety, StrongSafety, and TypeOK.

## Properties Verified

### P1: Hash Chain Soundness

Verified at BF=2.

- A proof extracted from a trie always has a valid hash chain to the root
  (each node's hash matches the parent's child pointer at the on-path nibble).
- Flipping any proof node's value breaks the hash chain (tamper detection).

Corresponds to: `test_wrong_end_root_boundary_check`,
Go `"end root mismatch"`.

### P2: Boundary Nibble Classification

Verified at BF=2 and BF=4.

The boundary nibble derivation correctly classifies every child as in-range or
out-of-range for all three verification cases:

- **End proof** (Case 2a): children with nibble < boundary have subtrees
  entirely within the range.
- **Start proof** (Case 2b): children with nibble > boundary have subtrees
  entirely within the range.
- **Divergence parent** (Case 2c): children with nibble strictly between
  start\_bn and end\_bn have subtrees entirely within the range.

This is the non-trivial property where a confirmed exploitable bug previously
existed (the `end_nibbles` fallback issue).

Corresponds to: `test_root_hash_single_end_proof`,
`test_root_hash_single_start_proof`, `test_root_hash_two_proofs`.

### P3: Boundary Derivation from Proof Structure

Verified at BF=2 and BF=4.

The implementation derives boundary nibbles from the proof's internal structure
(next node's key at current depth, with fallback to external key at the last
node). This derivation always produces the same result as the simple formula
`key[depth + 1]` for all valid proofs.

Corresponds to: `test_generator_uses_last_op_key_for_end_proof`.

### P5: Structural Validation

Verified at BF=2 and BF=4.

Spot checks that the structural validation predicate correctly rejects:

- Inverted ranges (start\_key > end\_key)
- Missing end proof when batch\_ops is non-empty
- Unsorted batch\_ops keys

Corresponds to: `test_inverted_range_rejected`, `test_missing_end_proof`,
`test_keys_not_sorted`, Go `"inverted range"`, Go `"duplicate keys"`.

### P6: Operation Consistency

Verified at BF=2.

The Put/Delete vs inclusion/exclusion consistency check
(`StartProofOperationMismatch` / `EndProofOperationMismatch`) correctly
identifies:

- Put at a key absent from end\_root as inconsistent (caught)
- Delete at a key present in end\_root as inconsistent (caught)
- Put at a key present in end\_root as consistent
- Delete at a key absent from end\_root as consistent

Corresponds to: `test_spurious_put_at_start_key_boundary`,
`test_spurious_delete_at_start_key_boundary`.

### P7: end\_nibbles Fallback Mechanism

Verified at BF=2 and BF=4.

- When batch\_ops is non-empty, end\_nibbles uses last\_op\_key and always
  provides a boundary nibble.
- When batch\_ops is empty, end\_nibbles uses end\_key and always provides a
  boundary nibble.
- The exploit scenario (no fallback, boundary is None, no children checked) is
  confirmed to produce None, validating that the fix is necessary.

This is the only known bug confirmed by the model checker.

### P8: Truncated Proofs

Verified at BF=2 and BF=4.

- Using last\_op\_key for end\_nibbles produces a boundary that only checks
  children whose subtrees are within the proposal's modification range
  (keys up to last\_op\_key).
- Using end\_key instead would over-reach, checking children beyond what the
  proposal has modified (confirmed by the `TruncatedEndKeyWouldOverreach`
  existence check).

Corresponds to: `test_truncated_proof_round_trip`.

### P9: Exclusion Proof Boundary Behavior

Verified at BF=2 and BF=4.

- For exclusion proofs (last proof node at a shallower depth than the proven
  key), the fallback nibble from the external key correctly classifies
  in-range children.
- For inclusion proofs (key exhausted at the last node), the boundary is None.
  End proofs check no children (correct: all children are after the proven
  key). Start proofs check all children (correct: all children are after the
  proven key, which is the in-range direction).

Corresponds to: `test_start_proof_exclusion_for_deleted_key`,
`test_start_proof_inclusion_with_children_below`,
`test_end_proof_inclusion_with_children_below`.

### P10: Empty batch\_ops Attack Detection

Verified at BF=2.

For any two tries that differ on at least one key within the proven range, the
verification detects the difference through either a value mismatch or an
in-range child hash mismatch at some depth along the end proof path. This
confirms that the empty batch\_ops attack (prover claims "no changes" when
end\_root differs from start\_root) is always caught.

Corresponds to: `test_empty_batch_ops_with_nonempty_proofs`,
Go `"mismatched base state"`.

### P11: Complete Structural Validation

Verified at BF=2 and BF=4.

Exhaustive check that the structural validation predicate rejects all invalid
proof shapes and accepts all valid ones:

- start\_key > first\_op\_key rejected
- end\_key < last\_op\_key rejected
- max\_length exceeded rejected
- DeleteRange rejected
- Unexpected end proof rejected
- Boundary proof unverifiable rejected
- Valid proof shapes accepted (completeness)

Corresponds to: `test_start_key_larger_than_first_key`,
`test_end_key_less_than_last_key`, `test_proof_larger_than_max_length`,
`test_delete_range_rejected`, `test_unexpected_end_proof`,
`test_boundary_proof_unverifiable`, Go `"start key out of bounds"`,
Go `"end key out of bounds"`, Go `"exceeds max length"`,
Go `DeleteRange rejected`, Go `"unexpected end proof"`,
Go `"boundary proof unverifiable"`.

### P12: Divergence at Depth Zero

Verified at BF=2 and BF=4.

When start\_key and end\_key have different first nibbles, the divergence
depth is 0. Since there is no shared parent node above depth 0, the
implementation correctly rejects with `BoundaryProofsDivergeAtRoot`. The
model verifies that in-range children DO exist between the two first nibbles
(justifying the rejection -- there is no parent to check them on).

Corresponds to: `test_divergence_at_depth_zero`.

### P13: Deferred Boundary Mismatch Detection

Verified at BF=2 and BF=4.

A mismatch at a key between start\_key and end\_key might not be detected in a
truncated round (because the mismatch is beyond the truncation point), but
WILL be detected in a subsequent round. The model verifies:

1. Round 1's boundary (at last\_op\_key) does NOT cover the mismatch key
   (correctly excluded from truncated check).
2. Round 2's range (starting from last\_op\_key) DOES cover the mismatch key.

Corresponds to: Go `TestChangeProofBoundaryValueMismatchDeferred`.

### P14: Empty End Root Detection

Verified at BF=2.

When end\_root is the empty trie hash, the only trie producing that hash is
the empty set. Any non-empty trie has a different root hash, so the hash chain
check will fail. This confirms that a prover cannot claim an empty end state
when the target revision is non-empty.

Corresponds to: Go `"empty end root complete"`, Go `"empty end root partial"`.

### P15: Delete as Last Op in Truncated Proof

Verified at BF=2 and BF=4.

When the last batch op is a Delete, the end proof is an exclusion proof. The
boundary nibble at the last end proof node comes from the deleted key's
nibbles. The model verifies this still correctly classifies in-range children.

Corresponds to: `test_truncated_proof_with_delete_last_op`.

### P16: Asymmetric Depth

Verified at BF=2 and BF=4.

When start\_key is shorter than end\_key (e.g., start=`<<0>>`,
end=`<<0,0,3>>`), the divergence point and boundary nibble derivation must
handle the depth asymmetry correctly. The model verifies that boundary
classification remains correct for the divergence parent, the start proof
(shorter key), and the end proof (longer key).

Corresponds to: Go `TestChangeProofAsymmetricDepth`.

### Bug 1 Reproduction: Start Proof Consistency Check

Verified at BF=2.

Models the buggy code from before commit `9c43a6cb4`, where the start proof's
inclusion/exclusion result was accepted unconditionally. The model defines a
`BuggyOpConsistent` that always returns TRUE (no check) and verifies:

- There EXISTS a (trie, key) where the buggy version accepts a Put at an
  absent key (exclusion proof, but Put expects inclusion) that the fixed
  version rejects with `StartProofOperationMismatch`.

This serves as a TLC regression test: if the consistency check were removed,
this ASSUME would fail.

### Bug 2 Reproduction: end\_nibbles Exploit

Verified at BF=2.

Models the buggy code where `end_nibbles` was not falling back to `end_key`
when `batch_ops` was empty. The model defines a `BuggyEndNibblesSource` that
returns an empty sequence (instead of `end_key`) and verifies:

- With the buggy fallback, `IsInRange(n, None, TRUE) = FALSE` for ALL nibbles
  -- zero children are checked at the last end proof node.
- There EXISTS (startTrie, endTrie, endKey) where the tries differ within
  range but the buggy code checks nothing, allowing the difference to go
  completely undetected.

This serves as a TLC regression test: if the end\_key fallback were removed,
this ASSUME would fail.

### Sync Protocol: Safety, Strong Safety

Verified at BF=2.

For all 4,096 possible (startTrie, endTrie) pairs:

- **Safety**: If the verifier terminates (synced = TRUE), the committed root
  hash matches the target end\_root.
- **Strong Safety**: If the verifier terminates, the verifier's trie is
  identical to the target trie (state equality, not just hash equality).
- **TypeOK**: All state variables remain in expected types throughout
  execution.

These hold even under adversarial sync rounds where the prover sends arbitrary
subsets of the real changes. TLC explored 1,019,456 states (190,528 distinct)
to depth 5.

Corresponds to: Go `TestMultiRoundChangeProof` (all 3 sub-cases),
`test_iterative_sync_converges`.

## Key Findings

**The boundary classification formula is correct.** P2 checks each in-range
nibble against a ground truth function that recursively enumerates every key in
the subtree and verifies they are all within the claimed range. This is not a
spot check — TLC exhaustively verified every nibble at every depth for every
key in the model.

**The boundary derivation from proof structure is equivalent to the formula.**
P3 closes the gap between the spec (which uses `key[depth + 1]` directly) and
the implementation (which derives the boundary from the next proof node's key
at the current depth). For all valid proofs, the derivation produces the same
result.

**Truncated proofs require last\_op\_key for end\_nibbles.** P8 verifies that
using `last_op_key` limits the boundary to the proposal's modification range.
It also confirms that using `end_key` instead would over-reach — checking
children beyond what the proposal has modified, causing false mismatches.

**The empty batch\_ops attack is always detected.** P10 verifies detection
through EITHER a value mismatch or an in-range child hash mismatch. The
initial P10 formulation only checked children and failed — differences at the
proven key itself are caught by the value check, not the children check.

**The sync protocol is safe under adversarial provers.** The protocol model
allows provers to send arbitrary subsets of real changes. Despite this, synced
implies state equality for all 4,096 starting configurations.

**Both known bugs are formally confirmed.** TLC finds concrete witnesses for
both: a (trie, key) where Bug 1's missing consistency check accepts an
inconsistent proof, and a (startTrie, endTrie, endKey) where Bug 2's missing
fallback allows a difference to go undetected.

## Test Coverage

Of 68 total Rust and Go test cases for change proof verification, 47 (69%)
have corresponding model-checked properties. The remaining 21 are
implementation-specific (FFI lifecycle, cursor traversal, serialization) and
are not suitable for design-level model checking.

## Findings

All 35 design properties and 3 protocol properties pass exhaustive model
checking. No new bugs were found. Two known bugs from the git history were
formally reproduced by TLC, confirming both the vulnerable behavior and the
necessity of their fixes:

1. **Start proof consistency check** (commit `9c43a6cb4`): TLC finds a
   concrete (trie, key) witness where the pre-fix code accepts an inconsistent
   proof that the fixed code rejects.

2. **end\_nibbles fallback** (end\_key exploit): TLC finds a concrete
   (startTrie, endTrie, endKey) witness where the pre-fix code fails to detect
   a difference between two tries that the fixed code catches.

Both bug reproductions serve as regression tests within the TLA+ spec. If
either fix were reverted, TLC would report an assumption violation.

The P10 check initially failed during model development because the TLA+ spec
only modeled the children comparison, not the value comparison. Differences at
the proven key itself (e.g., value at endKey changed) are caught by
`verify_change_proof_node_value`, not the in-range children check. After
including value comparison in the model, P10 passes. This was a modeling error,
not a design bug.

## Limitations

- **Model vs implementation gap**: The TLA+ spec models the verification
  DESIGN (boundary formulas, hash chain arguments, protocol logic), not the
  Rust IMPLEMENTATION. The Rust code could have bugs that the model doesn't
  capture (e.g., cursor misalignment, off-by-one in nibble indexing, path
  compression edge cases). Closing this gap requires a tool like Kani that
  verifies Rust code directly.
- **Bounded model checking**: Properties are verified for specific BF and
  MaxDepth values, not for all possible values. The boundary classification
  formula is depth-independent (it only depends on the nibble at the current
  depth), so results generalize by inspection. But TLC does not produce a
  mathematical proof of this generalization.
- **Enumeration limits**: Properties that enumerate all possible tries (P1,
  P6, P10, P14, Bug 1, Bug 2 end-to-end) are only feasible at BF <= 3 due
  to the 2^|AllKeys| state space. Properties that only enumerate keys (P2,
  P3, P5, P7, P8, P9, P11-P13, P15-P16) scale to BF=4 and beyond.
- **No path compression**: The model uses flat key-value sets without trie
  path compression. The cursor miss behavior (defaulting to all-None children
  when the proposal has no node at a given depth) is not modeled.
- **Abstract hash function**: The hash function is modeled as injective by
  construction (the hash IS the canonical node content). Real collision
  resistance of SHA-256 or Keccak-256 is assumed, not verified.
