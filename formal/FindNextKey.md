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

`find_next_key` takes a verified range proof response and the
original end_key e, and returns either:
- **None**: the range [s, e] is fully covered. No more fetches needed.
- **Some(kₘ, e)**: the range was partially covered through kₘ.
  Continue fetching from [kₘ, e]. The end_key e is always echoed
  back unchanged — the function never modifies the upper bound.

The function assumes the proof has already been verified (Section 2.5).
Conditions such as kₘ > e are rejected during verification and cannot
reach `find_next_key`.

### 3.1 Inputs

The function can rely on:
- **From the verified proof**: key_values (and thus m and kₘ),
  start_proof, end_proof, its terminal node, and t(end_proof). These
  are verified against root hash h and can be trusted.
- **From the verifier's own state**: s and e (the range the verifier
  requested). These are known to the verifier, not derived from the
  proof.
- The post-verification properties established in Section 2.5.

The function cannot rely on:
- The trie T itself.
- Whether the prover truncated or exhausted the response.
- The target key the prover used to generate end_proof.
- The prover honoring the requested limit n. The prover may return
  fewer keys than n for any reason (its own limits, network
  constraints, or policy). Therefore, comparing m to n does not
  reliably distinguish truncation from exhaustion.

### 3.2 Unambiguous Cases

From the analysis in Section 2.5, the following cases allow `find_next_key`
to determine the truncation status:

**Case 1: m = 0** (empty key_values).
The response is exhaustive (Section 2.5, property 3).
Return: **None**.

**Case 2: kₘ = e** (last returned key equals requested bound).
The completeness property (Section 2.4) guarantees key_values
contains all keys in [s, kₘ] with no gaps. Since kₘ = e, this
covers the entire requested range [s, e].
Return: **None**.

**Case 3: Terminal node has no value.**
The end_proof is an exclusion proof. An honest truncated response
always produces an inclusion proof of kₘ (since kₘ ∈ K(T)), so
exclusion implies exhaustive (Section 2.5).
Return: **None**.

**Case 4: Terminal node has value, t(end_proof) ≠ kₘ.**
An honest truncated response would have t(end_proof) = kₘ. Since
t(end_proof) ≠ kₘ, the response is exhaustive (Section 2.5).
Return: **None**.

### 3.3 The Ambiguous Case

**Case 5: Terminal node has value, t(end_proof) = kₘ, and kₘ < e.**

This is the truncation ambiguity from Section 2.5. The proof is
consistent with:
- Truncation: the prover hit the limit at kₘ and generated an
  inclusion proof of kₘ. There may be more keys in (kₘ, e].
- Exhaustion: the prover returned all keys in [s, e] and the
  end_proof happens to terminate at kₘ (either because e = kₘ,
  which is excluded by kₘ < e, or because an exclusion-divergent
  proof of e landed on kₘ's node).

The verifier cannot distinguish these from the proof alone.

### 3.4 Resolving the Ambiguous Case

The verifier has no information from the proof alone that
distinguishes truncation from exhaustion in Case 5. We consider
three approaches:

We define **succ(k)** as the lexicographic successor of key k — the
smallest key strictly greater than k (e.g., k with a 0x00 byte
appended).

**Option A: Return Some(succ(kₘ), e) in the ambiguous case.**

When the verifier cannot determine the truncation status, it
conservatively assumes truncation and returns Some(succ(kₘ), e).
The next fetch requests [succ(kₘ), e], which excludes kₘ (already
accumulated).

- If the response was actually exhaustive: the next fetch returns
  m = 0 (no keys in (kₘ, e]) → Case 1 → None. Terminates.
- If the response was actually truncated: the next fetch returns
  keys in (kₘ, e]. Progress is made.

*Safety*: kₘ was already included in the current key_values and
verified. Advancing past it loses no data.

*Liveness*: succ(kₘ) > kₘ ≥ k₁ ≥ s, so s' > s strictly. Each
iteration advances the start key, and the range [s, e] is finite,
so the syncer must terminate.

This costs at most one extra round-trip per ambiguous response.

**Option B: Heuristic on proof structure (issue #1989's proposal).**

Use properties of the end_proof's terminal node (e.g., whether kₘ
is a prefix of e, the number of returned keys) to guess whether the
response is truncated or exhaustive.

*Safety*: unsound. As shown in Section 2.5, the terminal node's
properties do not reliably distinguish the cases. The heuristic may
return None when the response is actually truncated, causing keys
in (kₘ, e] to be silently lost (completeness violation).

*Liveness*: the heuristic may return Some(kₘ, e) (with kₘ, not
succ(kₘ)) when the response is exhaustive. If the prover returns
the same response, the syncer loops indefinitely.

Note: comparing m to the requested limit n (as proposed in Ron's
fix fe064255a) has the same issues. The prover may return fewer
than n keys for reasons unrelated to exhaustion (its own limits,
network constraints, or policy). Therefore m < n does not reliably
indicate exhaustion.

### 3.5 Counterexamples to Proposed Heuristics

This section provides concrete counterexamples to the heuristics
proposed in issue #1989 and in Ron's fix fe064255a. All
counterexamples assume the ambiguous case (Case 5): terminal node
has a value, t(end_proof) = kₘ, and kₘ < e.

**Heuristic 1: Compare m to requested limit n.**

If m < n, assume exhaustive (return None). If m = n, assume
truncated (return Some).

*Counterexample (completeness violation)*: The verifier requests
[s, e] with n = 10. The prover has 15 keys in range but applies
its own limit of 5, returning m = 5 keys with an inclusion
end_proof of kₘ. The verifier sees m = 5 < n = 10 and concludes
exhaustive. Returns None. 10 keys in (kₘ, e] are lost.

**Heuristic 2: If the last operation is a Put and m = 1, assume
exhaustive.**

The reasoning: a single Put that matches the end_proof terminal
suggests the prover had only one key to return.

*Counterexample (completeness violation)*: The trie has keys
{k₁, k₂, k₃} in [s, e]. The prover uses limit = 1, returning
only k₁ with an inclusion end_proof of k₁. m = 1, last op is
Put. The heuristic returns None. Keys k₂ and k₃ are lost.

**Heuristic 3: If kₘ is a strict prefix of e, assume truncated.
Otherwise assume exhaustive.**

The reasoning: if kₘ is a prefix of e, the prover likely stopped
short. If not, the proof likely covers through e.

*Counterexample (completeness violation)*: The trie has keys
{\x10, \x10\x10} in [s, \x10\x30]. The prover uses limit = 1,
returning only \x10 with an inclusion end_proof of \x10. kₘ = \x10
is a prefix of e = \x10\x30, so the heuristic returns Some — correct
in this case. But reverse the scenario: the trie has only {\x10} in
[s, \x10\x30], and the prover returns \x10 exhaustively. kₘ = \x10
is still a prefix of e, so the heuristic returns Some — an
unnecessary round-trip, but not a violation.

Now consider: trie has keys {\x10\x30, \x10\x35} in [s, \x10\x40].
The prover uses limit = 1, returning \x10\x30. kₘ = \x10\x30 is NOT
a prefix of e = \x10\x40. The heuristic assumes exhaustive and
returns None. Key \x10\x35 is lost.

**Heuristic 4: Use t(end_proof) > kₘ to detect exhaustion.**

If the end_proof's terminal key exceeds kₘ, assume exhaustive.

*This is Cases 3 and 4 from Section 3.2* — these are sound and
already handled as unambiguous cases. The heuristic is correct here.
The problem is that it provides no help in Case 5 where
t(end_proof) = kₘ, which is the ambiguous case.

### 3.6 Correctness Properties

We state two properties that a correct `find_next_key` must satisfy,
and evaluate them under Option A (return Some(succ(kₘ), e) in the
ambiguous case).

**Liveness**: The syncer terminates after a finite number of fetches.

*Argument under Option A*: The only case returning Some is Case 5,
which returns s' = succ(kₘ). Since kₘ ≥ k₁ ≥ s (key_values covers
[s, ...]) and succ(kₘ) > kₘ, we have s' > s strictly. Each
iteration strictly advances the start key over a finite range [s, e],
so the syncer must eventually reach Case 1 (m = 0) or Case 2
(kₘ = e) and return None.

**Completeness**: When the syncer terminates, the union of all
key_values across all fetches equals K(T) ∩ [s, e]. No keys are
missed.

*Argument under Option A*: find_next_key returns None only in
Cases 1-4. In each of these cases, Section 2.5 establishes the
response is exhaustive, so all keys in [s, e] are covered.
find_next_key never returns None in Case 5 — it always returns
Some(succ(kₘ), e). Therefore None is only returned when exhaustion
is certain. Between iterations, advancing to succ(kₘ) skips only
kₘ itself, which was already accumulated — no keys are lost.

*Under Option B (heuristic)*: find_next_key may return None in
Case 5 when the response is actually truncated — violating
completeness. Keys in (kₘ, e] would be silently lost.

## 4. Why the Ambiguity Cannot Be Resolved by the Prover

A natural question is whether the proof generator can avoid producing
ambiguous proofs, eliminating the need for conservative behavior on
the verifier side.

The prover knows whether it truncated. In the exhaustive case, it
generates end_proof by calling prove(e). In the truncated case, it
calls prove(kₘ). The ambiguity arises when prove(e) and prove(kₘ)
produce identical proofs — which happens in the exclusion-divergent
case where the path toward e diverges at kₘ's node.

In this case, both prove(e) and prove(kₘ) follow the exact same
trie path: they enter through the same child slots at each branch
and terminate at the same node (kₘ's node). The resulting proof
node sequences are byte-identical. The prover cannot produce a
structurally different proof for prove(e) because there is no
alternative path to follow — the trie structure dictates the proof.

Possible mitigations and why they fail:

- **Use a different anchor** (e.g., succ(kₘ) instead of e): the
  path toward succ(kₘ) may also terminate at kₘ's node. If kₘ is
  a leaf, succ(kₘ) enters through kₘ's partial path, finds no
  child for the next nibble, and stops at kₘ — same terminal, same
  ambiguity.

- **Include additional proof nodes beyond the divergence**: there
  are no additional nodes to include. The divergence point is where
  the trie path ends — the child slot needed by e's next nibble
  does not exist.

The ambiguity is therefore a fundamental property of Merkle proofs
in compressed tries: two different target keys can produce
identical proofs when the trie path diverges at a valued node. No
change to the proof generator can eliminate this without extending
the proof format.

This confirms that the verifier must handle the ambiguity. Option A
(conservative continuation with succ(kₘ)) is the sound approach
that requires no format changes.

## 5. Model Checking Results

The arguments in Section 3 are verified by TLA+ model checking. The
models use BF=2 (binary trie), MaxDepth=2 (keys of length 2 nibbles),
and NumValues=1. The prover's truncation limit is nondeterministic —
TLC exhaustively explores all possible truncation choices at each step.

### 4.1 Model Structure

The TLA+ spec (`FindNextKey.tla`) models:

- **Trie**: all possible tries over the key space (from CompressedTrie.tla).
- **Prover**: generates honest range proofs for [cursor, e] but may
  truncate at any limit from 1 to |keys_in_range|. The truncation
  limit is a nondeterministic variable that TLC explores exhaustively.
- **Syncer loop**: a state machine with variables (cursor, accumulated,
  done). Each step fetches a proof, accumulates key_values, and calls
  find_next_key to decide the next cursor or termination.
- **find_next_key**: implements the case analysis from Section 3.2-3.4.

The key operator:

```
FindNextKeyOptionA(proof, endKey) ==
    IF m = 0 THEN None                        \* Case 1
    ELSE IF lastKv = endKey THEN None          \* Case 2
    ELSE IF ~TerminalHasValue THEN None        \* Case 3
    ELSE IF terminalKey ≠ lastKv THEN None     \* Case 4
    ELSE Succ(lastKv)                          \* Case 5: ambiguous
```

Properties checked:
- **Completeness** (invariant): done ⇒ accumulated = K(T) ∩ [s, e].
- **Liveness** (temporal): ◇done (the syncer eventually terminates).

### 4.2 Option A: Verified Correct

| Parameter   | Value |
|-------------|-------|
| BF          | 2     |
| MaxDepth    | 2     |
| NumValues   | 1     |
| States      | 1,224 distinct |
| Result      | **No violations.** Completeness and liveness hold. |

A larger model (MaxDepth=4, 20 possible keys) is running to increase
confidence.

### 4.3 Heuristic Counterexamples

Each heuristic is modeled by replacing FindNextKeyOptionA with a
variant that implements the heuristic in the ambiguous case (Case 5).
TLC finds a completeness violation for each.

**All three heuristics fail on the same counterexample:**

| Variable    | Value |
|-------------|-------|
| Trie        | K(T) = {⟨1,0⟩, ⟨1,1⟩} |
| Range       | [⟨0,0⟩, ⟨1,1⟩] |
| Prover limit | 1 (returns only ⟨1,0⟩) |
| kₘ          | ⟨1,0⟩ |
| Expected    | {⟨1,0⟩, ⟨1,1⟩} |
| Accumulated | {⟨1,0⟩} — key ⟨1,1⟩ lost |

The prover honestly returns the first key in range with limit=1.
Each heuristic incorrectly concludes the response is exhaustive:

- **Heuristic 1** (m < n → exhaustive): m=1 < requestedN=2 → None.
  Fails because the prover used its own limit (1), not the requested
  limit (2).
- **Heuristic 2** (m=1 → exhaustive): m=1 → None. Fails because
  limit=1 always produces m=1, whether truncated or exhaustive.
- **Heuristic 3** (kₘ not prefix of e → exhaustive): ⟨1,0⟩ is not a
  prefix of ⟨1,1⟩ → None. Fails because the prefix relationship
  between kₘ and e is unrelated to truncation status.
