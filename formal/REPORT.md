# Formal Verification Report: Change Proof Verification

Date: 2026-03-31

## Summary

We formally verified 18 design properties and 3 protocol properties of
Firewood's change proof verification algorithm using TLA+ and the TLC model
checker. All properties pass exhaustive model checking. No errors were found.

## Motivation

Firewood's change proof verification uses a technique not found in other
blockchain state sync implementations: pre-verified hash chains enable direct
in-range child comparison, avoiding trie reconstruction and full Merkle rehash.
This technique is more performant than AvalancheGo's reconstruct-and-compare
approach but relies on precise boundary nibble derivation at each proof node
depth. A confirmed exploitable bug in the boundary derivation (the
`end_nibbles` fallback issue) motivated formal verification of the design.

## Approach

We modeled the verification algorithm in TLA+ at two levels:

1. **Design properties** (`ChangeProofVerification.tla`): Models a Merkle trie
   with configurable branch factor and bounded depth. Verifies boundary
   classification correctness, hash chain soundness, structural validation,
   operation consistency, truncated proof handling, exclusion proof behavior,
   and the empty batch\_ops attack scenario.

2. **Protocol properties** (`SyncProtocol.tla`): Models the multi-round sync
   protocol as a state machine with both honest and adversarial provers.
   Verifies safety (correct state on termination) and state equality.

TLC exhaustively checks these properties for all trie instances, key
combinations, and protocol executions within the configured bounds.

## Results

```
ChangeProofVerification.tla

Property                              | BF=2 D=2 | BF=4 D=2
--------------------------------------|----------|----------
P1:  Hash chain soundness             | pass     | skip
P1:  Tamper detection                 | pass     | skip
P2:  Boundary end proof               | pass     | pass
P2:  Boundary start proof             | pass     | pass
P2:  Boundary divergence parent       | pass     | pass
P3:  Derivation matches formula       | pass     | pass
P5:  Inverted range rejected          | pass     | pass
P5:  Missing end proof rejected       | pass     | pass
P5:  Unsorted keys rejected           | pass     | pass
P6:  Operation consistency            | pass     | skip
P7:  end_nibbles fallback             | pass     | pass
P7:  Exploit scenario confirmed       | pass     | pass
P8:  Truncated end_nibbles correct    | pass     | pass
P8:  end_key would over-reach         | pass     | pass
P9:  Exclusion proof boundary (end)   | pass     | pass
P9:  Exclusion proof boundary (start) | pass     | pass
P9:  Inclusion proof last node        | pass     | pass
P10: Empty batch_ops attack detected  | pass     | skip

"skip" means the check is guarded by BF <= 3 because it enumerates
all possible tries (2^|AllKeys|), which is infeasible at larger BF.


SyncProtocol.tla (BF=2, MaxDepth=2)

Property      | Result | States
--------------|--------|-------------------
Safety        | pass   | 1,019,456 total
StrongSafety  | pass   | 190,528 distinct
TypeOK        | pass   | depth 5

Checked for all 4,096 (startTrie, endTrie) pairs under both honest
and adversarial provers.
```

## Key Findings

### The boundary classification formula is correct

P2 verifies that for every key in a trie with up to 4-way branching and 2
levels of depth, at every proof node depth, every nibble classified as
"in-range" by the formula (`n < boundary` for end proofs, `n > boundary` for
start proofs, `start_bn < n < end_bn` for divergence parents) has a subtree
where ALL keys are within the claimed range. This is checked against a ground
truth function that recursively enumerates every key in the subtree.

### The boundary derivation from proof structure is equivalent to the formula

P3 verifies that deriving the boundary nibble from the proof's internal
structure (next node's key at current depth, with fallback to external key at
the last node) always produces the same result as the simple formula
`key[depth + 1]`. This closes the gap between how the spec describes the
boundary and how the Rust implementation computes it.

### Truncated proofs require last\_op\_key for end\_nibbles

P8 verifies that using `last_op_key` for end\_nibbles (when batch\_ops is
non-empty) produces a boundary that only checks children within the proposal's
modification range. P8 also confirms that using `end_key` instead would
over-reach — it would check children beyond what the proposal has modified,
causing false mismatches for truncated proofs.

### The empty batch\_ops attack is always detected

P10 verifies that for any two tries differing within the proven range, the
verification detects the difference through either a value mismatch or an
in-range child hash mismatch. The initial formulation of P10 only checked
children and failed — differences at the proven key itself are caught by the
value check (`verify_change_proof_node_value`), not the children check. After
including value comparison, P10 passes. This confirms the verification has no
blind spots for the empty batch\_ops attack.

### The sync protocol is safe under adversarial provers

The sync protocol model allows adversarial provers to send arbitrary subsets of
the real changes in each round. Despite this, if the verifier ever sets
`synced = TRUE` (committed root hash matches target), the verifier's trie is
identical to the target trie. This holds for all 4,096 possible starting
configurations at BF=2.

### The exploit scenario is confirmed

P7 explicitly verifies that the exploit scenario (no end\_nibbles fallback,
boundary is None at the last end proof node) produces `IsInRange(n, None,
TRUE) = FALSE` for all nibbles — no children are checked. This confirms the
bug was real and the fix (falling back to end\_key when batch\_ops is empty)
is necessary.

## Limitations

1. **Model vs implementation gap**: The TLA+ spec models the verification
   DESIGN (boundary formulas, hash chain arguments, protocol logic), not the
   Rust IMPLEMENTATION. The Rust code could have bugs that the TLA+ model
   doesn't capture (e.g., cursor misalignment, off-by-one in nibble indexing,
   path compression edge cases). Closing this gap requires a tool like Kani
   that verifies Rust code directly.

2. **Bounded model checking**: Properties are verified for specific BF and
   MaxDepth values, not for all possible values. The boundary classification
   formula is depth-independent (it only depends on the nibble at the current
   depth), so results generalize by inspection. But TLC does not produce a
   mathematical proof of this generalization.

3. **Abstract hash function**: The hash function is modeled as injective by
   construction (the hash IS the canonical node content). Real collision
   resistance of SHA-256 or Keccak-256 is assumed, not verified.

4. **No path compression**: The model uses flat key-value sets without trie
   path compression. The cursor miss behavior (defaulting to all-None
   children when the proposal has no node at a given depth) is not modeled.

5. **Enumeration limits**: Properties P1, P6, and P10 enumerate all possible
   tries (`SUBSET AllKeys`), which grows as `2^(BF + BF^2 + ...)`. These are
   only feasible at BF <= 3 (64 tries for BF=2 D=2). P2, P3, P5, P7, P8,
   and P9 only enumerate keys (not tries) and scale to BF=4 and beyond.
