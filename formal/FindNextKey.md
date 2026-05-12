# Formal Analysis: `find_next_key` Correctness for Range Proofs

## 1. Problem Setting

A **syncer** replicates a remote Merkle trie by fetching verifiable
subsets of its key-value pairs. The syncer partitions the keyspace into
disjoint ranges and fetches each range independently, possibly from
different remote peers. A single range may require multiple fetches if
the remote peer limits the number of key-value pairs per response.

After each fetch, the syncer must decide whether the range is fully
covered or whether more fetches are needed. Getting this wrong causes:
- **Infinite loops**: continuing to fetch when the range is already covered.
- **Missing data**: stopping early when keys remain unfetched.

The `find_next_key` function makes this decision. This document formalizes
its correctness requirements and analyzes the proposed implementation.

## 2. Definitions

### 2.1 Trie

Let K denote the universe of all possible keys, totally ordered by
lexicographic comparison (≤). Let T be a key-value map over K. T[k]
returns the value associated with key k, or ⊥ if k is not present in T.
Note that ⊥ (absent) is distinct from the empty string ε — a key may
map to ε. Let K(T) = {k ∈ K : T[k] ≠ ⊥} denote the set of keys
present in T. K(T) ⊆ K.

We extend the ordering on K with two sentinels: −∞ (less than all keys)
and +∞ (greater than all keys). These are not valid keys and cannot
appear in T.

### 2.2 Request

A range proof request is a tuple R = (s, e, n) where:
- s ∈ K ∪ {−∞}: start of range
- e ∈ K ∪ {+∞}: end of range
- n ∈ ℕ⁺ ∪ {+∞}: maximum number of key-value pairs to return (+∞ = no limit)

Precondition: s ≤ e.

### 2.3 Merkle Proofs

A **Merkle proof** for a target key k in trie T is a sequence of trie
nodes along the path from the root toward k. The **terminal node** is
the deepest (last) node in this sequence. The **terminal_key** is the
key of the terminal node.

We write **t(π)** for the terminal_key of a proof π.

The relationship between t(π) and the target key k determines the proof type:

- **Inclusion proof**: terminal_key = k (equivalently, k ∈ K(T)). The
  terminal node holds the value T[k].

- **Exclusion proof**: terminal_key ≠ k (equivalently, k ∉ K(T)).
  There are two sub-cases:
  - **Divergent**: Keys are sequences of nibbles (half-bytes).
    terminal_key and k share a common prefix of d ≥ 0 nibbles but
    differ at nibble position d+1. Due to path compression, a
    single trie node spans the nibbles from position d+1 onward
    along terminal_key's path. Since each nibble position has at
    most one child, k cannot exist in T — terminal_key's node
    occupies the only path through the shared prefix. terminal_key
    may be less than or greater than k depending on the divergent
    nibble at position d+1.
  - **Short**: the terminal node has no child in the direction of k's
    next nibble. terminal_key is a strict prefix of k, therefore
    terminal_key < k.

### 2.4 Range Proof Response

A range proof for request (s, e, n) on trie T consists of:

- `key_values`: an ordered sequence ⟨(k₁,v₁), ..., (kₘ,vₘ)⟩ with
  k₁ < k₂ < ... < kₘ, satisfying:
  - Let R = {k ∈ K(T) : s ≤ k ≤ e} be the set of all keys in T within
    the requested range, and let r₁ < r₂ < ... < r|R| be its sorted
    enumeration.
  - m = min(n, |R|) — the response contains either all keys in range,
    or exactly n if the limit is hit first.
  - Completeness: {k₁, ..., kₘ} = {r₁, ..., rₘ} — the returned keys
    are exactly the first m keys in R (no gaps).
  - Correctness: vᵢ = T[kᵢ] for all 1 ≤ i ≤ m — each returned value
    matches the trie.

We say the response is **truncated** when n < |R| (the limit prevented
all keys from being returned), and **exhaustive** when n ≥ |R| (all
keys in range were returned).

- `start_proof`: a Merkle proof (Section 2.3) for target key s.
  Empty if s = −∞.

- `end_proof`: a Merkle proof for the right boundary.
  - If truncated: a Merkle proof for target key kₘ (the last returned
    key). Since kₘ ∈ K(T), this is always an inclusion proof.
  - If exhaustive:
    - If e = +∞: empty.
    - If e ≠ +∞: a Merkle proof for target key e. This is an inclusion
      proof if e ∈ K(T), or an exclusion proof if e ∉ K(T).

*Note: Issue #1989 uses the labels L, E, and P for last_op.key, end_key,
and proven_right_key respectively. In our notation these correspond to
kₘ, e, and t(end_proof).*

### 2.5 Post-Verification Properties

After a range proof for request (s, e, n) has been successfully verified
against a root hash h, the following properties are established. These
are the facts available to `find_next_key`; it need not (and cannot)
re-examine the trie.

1. **Key-value correctness**: The key_values in the proof are present
   in the trie with root hash h, with correct values, in order, and
   with no gaps up to kₘ (as defined in Section 2.4).

2. **Left boundary**: For m > 0, the start_proof confirms that k₁ is
   the first key in K(T) at or after s. If m = 0 and e ≠ +∞, the
   combination of start_proof and end_proof confirms no key exists
   in [s, e].

3. **Right boundary**: If m = 0, the response is exhaustive (truncation
   requires n < |R|, but n ≥ 1 and |R| = 0 when m = 0, so n ≥ |R|).
   The end_proof, if present, is an exclusion proof of e confirming no
   keys exist in the range. The analysis below applies when m > 0.

   When m > 0, the verifier can compute t(end_proof) from the proof,
   but does not know the target key the prover used to generate the
   end_proof — that is, whether the target was kₘ (truncated) or e
   (exhaustive). This creates ambiguity in determining the proof type
   on the right edge.

   If the proof type and target key were known, verification would
   establish:

   - **Inclusion proof of kₘ (truncated)**: t(end_proof) = kₘ. The
     proof confirms the value at kₘ. Coverage extends through kₘ,
     but there may be additional keys in (kₘ, e].

   - **Inclusion proof of e (exhaustive, e ∈ K(T))**: t(end_proof) = e.
     The proof confirms the value at e. All keys in [s, e] have been
     returned.

   - **Exclusion proof of e (exhaustive, e ∉ K(T))**:
     t(end_proof) ≠ e. The proof confirms e does not exist in the
     trie (via divergence or missing child, as defined in Section 2.3).
     All keys in [s, e] have been returned.

   The verifier cannot determine which of these cases applies. It
   knows t(end_proof), kₘ, and e, but the same values of these can
   arise from different cases. For example, t(end_proof) = kₘ with
   the terminal node holding a value could be:
   - Truncated: inclusion proof of kₘ.
   - Exhaustive: inclusion proof of e, when e = kₘ.
   - Exhaustive: exclusion-divergent proof of e, where the proof for
     target e diverged and terminated at kₘ's node (possible when kₘ
     and e share a child slot but the compressed path matches kₘ,
     not e).

   Note that the third case is an exclusion proof whose terminal node
   holds a value. The value belongs to kₘ, not to the target key e —
   the node is the terminal because the path toward e diverged at
   that node. **The presence of a value on the terminal node does not
   imply the proof is an inclusion proof.** Only knowledge of the target
   key can distinguish inclusion from exclusion, and the verifier does
   not have the target key.

   **Determining proof type (inclusion vs exclusion):**

   The verifier can inspect the terminal node of end_proof:
   - If the terminal node has no value: the end_proof is an exclusion
     proof regardless of the target key.
   - If the terminal node has a value: the end_proof could be an
     inclusion proof (target = terminal_key) or an exclusion-divergent
     proof that diverged onto a valued node incidentally (target
     ≠ terminal_key, as shown in the third example above). The verifier
     cannot distinguish these cases without knowing the target key.

   **Proof type ambiguity**: when the terminal node has a value, the
   verifier cannot determine whether the end_proof is an inclusion
   proof or an exclusion-divergent proof.

   **Determining truncation status:**

   Given the proof type determination above:
   - **Exclusion proof** (terminal has no value): the response must be
     exhaustive. A truncated response produces an inclusion proof of
     kₘ (since kₘ ∈ K(T)), so exclusion is incompatible with truncation.
   - **Terminal has value, t(end_proof) ≠ kₘ**: the response must be
     exhaustive. A truncated response would have t(end_proof) = kₘ.
     The proof type ambiguity does not matter here — regardless of
     whether the proof is inclusion or exclusion-divergent, the
     truncation status is determined.
   - **Terminal has value, t(end_proof) = kₘ**: this is consistent with
     truncation (inclusion proof of kₘ) and with exhaustion (inclusion
     proof of e when e = kₘ, or exclusion-divergent proof of e that
     landed on kₘ).

   **Truncation ambiguity**: when the terminal node has a value and
   t(end_proof) = kₘ, the verifier cannot determine whether the
   response is truncated or exhaustive. This is a direct consequence
   of the proof type ambiguity: if the verifier could distinguish
   inclusion from exclusion-divergent, it could resolve truncation
   (inclusion of kₘ when kₘ < e implies truncation; inclusion of kₘ
   when kₘ = e implies exhaustion; exclusion-divergent landing on kₘ
   implies exhaustion). Because the proof type is
   unknown, the truncation status is unknown.

   `find_next_key` must operate correctly in the presence of these
   ambiguities.


## 3. Decision Function

```
find_next_key(L, E, P, batch_ops) → None | Some(L, E)
```

### 3.1 Preconditions
- L ≤ E (if both defined)
- L ≤ P (the last key can't exceed what the proof covers)

### 3.2 Cases

**Case 1: Empty batch_ops** (no data in range)
- If end_proof is empty → Done (empty trie or no keys in range)
- If end_proof is non-empty → Done (proof confirms range is empty)
- Return: `None`

**Case 2: L = E** (last key equals requested bound)
- The proof reached the requested end. No more keys to fetch.
- Return: `None`

**Case 3: P > L** (proof structurally reaches past last key)
- The end_proof was generated for a key beyond L.
- This means the proof covers [s, P) or [s, P], and all keys between L and P
  are accounted for (either absent from the trie or explicitly included).
- Return: `None`

**Case 4: P = L** (proof terminal equals last key)
- The end_proof is an inclusion proof of L. This is the truncation signature.
- However, it could also occur when L happens to be the last key in the trie
  within the requested range (and the prover generates an inclusion proof at L
  because there's nothing beyond L to walk to within [s, E]).
- **Ambiguity**: truncated vs. coincidentally exhaustive at L.

### 3.3 Resolving Ambiguity in Case 4

When P = L and L < E, we cannot determine from the proof alone whether:
(a) The prover was truncated at L and there are more keys in (L, E], or
(b) There are no more keys in (L, E] and the prover's end_proof happens
    to anchor at L.

**Proposed heuristic** (from issue #1989):
- If the last op is a Put and |batch_ops| = 1 → assume exhaustive (Done)
- Otherwise → assume truncated (Continue)

**Limitation**: This is a heuristic, not a proof. It can be wrong in both
directions:
- A limit=1 request always returns |batch_ops|=1, so it looks exhaustive
  even when truncated.
- A coincidentally single-op proof looks exhaustive when the range has more data.

### 3.4 Structural Solution

The ambiguity exists because the proof format does not encode whether
truncation occurred. Two possible fixes:

**Option A: Add a truncation flag to the proof.**
The prover sets `truncated = true` when it hits the limit. The verifier
passes this through to `find_next_key`. No heuristics needed.

**Option B: Use `max_length` as the signal.**
If the caller knows the limit `n` that was used, and `|batch_ops| < n`,
the proof is exhaustive. If `|batch_ops| = n`, it may be truncated.
This is what Ron's fix (fe064255a) does for range proofs.

Option B has the "coincidental match" problem: `|batch_ops| = n` doesn't
guarantee truncation (there might be exactly `n` keys in range). But it's
conservative — it returns Continue, which is safe (extra request, no data loss).

## 4. Correctness Properties

### Property 1: Progress
If `find_next_key` returns `Some(s', e')`, then s' > s. The next request
covers a strictly smaller range. This prevents infinite loops.

**Proof**: s' = L ≥ s (since L is in [s, E]). If L = s, then no keys were
returned in (s, E], which means batch_ops is empty → Case 1 → returns None.
Contradiction. So L > s, and s' = L > s. ∎

### Property 2: Completeness
If `find_next_key` returns `None`, then the union of all batch_ops from all
requests covers every key in the original requested range [s₀, E].

**Sketch**: Each request covers [sᵢ, E]. The proof for request i is verified
against the trie root hash, confirming the returned keys are correct for [sᵢ, E].
When find_next_key returns None (Case 1, 2, or 3), the proof confirms no more
keys exist in (L, E]. Combined with the returned keys in [sᵢ, L], the range
[sᵢ, E] is fully covered.

Case 4 (P = L, heuristic) may incorrectly return None when there are more keys.
This is the gap in the correctness argument.

### Property 3: Safety
If `find_next_key` returns `Some(L, E)`, no keys in [s, L] are missed. They
were all included in the current proof's batch_ops (verified by the Merkle proof).

## 5. Formal Model (TLA+)

A TLA+ model could verify these properties by:
1. Modeling the trie as a set of key-value pairs
2. Modeling the prover as generating honest proofs (possibly truncated by limit)
3. Modeling the syncer loop: request → verify → find_next_key → repeat
4. Checking that the syncer eventually terminates and covers all keys

The state space is the cross product of:
- All sparse tries (up to MaxPopulated keys)
- All (start, end, limit) request parameters
- The syncer's accumulated key set

The invariant is:
- If the syncer terminates, its accumulated keys = trie's keys in [s₀, E]
- The syncer always terminates (no infinite loops)

## 6. Open Questions

1. **Is the heuristic in Case 4 acceptable?** Option B (using max_length) avoids
   the heuristic entirely for range proofs. For change proofs, max_length is not
   currently available at the find_next_key call site.

2. **Should `find_next_key` receive `max_length` as input?** This would let it
   apply Option B uniformly. The current FFI signature doesn't pass it.

3. **Is there a case where the prover generates P = L for a non-truncated proof
   when L < E?** If so, Option B (|batch_ops| < n → exhaustive) would incorrectly
   return Continue, which is safe but wasteful. If not, P = L always means
   truncation, and no heuristic is needed.
