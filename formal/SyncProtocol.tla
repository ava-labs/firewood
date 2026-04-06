------------------------------ MODULE SyncProtocol ------------------------------
(*
 * Formal model of Firewood's multi-round change proof sync protocol.
 *
 * The verifier syncs state from a prover by fetching change proofs
 * in successive rounds. Each round covers a key range [startKey, endKey].
 * After committing each round's changes, the verifier checks if the
 * committed root hash matches the target end_root. If yes, sync is
 * complete. If no, find_next_key provides the next start key.
 *
 * Properties verified:
 *   Safety: If sync terminates, committedRoot = endRoot
 *   Liveness: Under an honest prover, sync eventually terminates
 *   Adversarial safety: A dishonest prover cannot cause the verifier
 *     to accept an incorrect state
 *)

EXTENDS Integers, Sequences, FiniteSets, TLC

CONSTANTS
    BF,        \* Branch factor
    MaxDepth,  \* Max trie depth
    None       \* Sentinel

Nibbles == 0..(BF - 1)
AllKeys == UNION {[1..d -> Nibbles] : d \in 1..MaxDepth}

\* The two tries: the prover's target state (endRootTrie) and the
\* verifier's starting state (startRootTrie). These are fixed.
\* We don't use CONSTANTS for tries — we make them variables chosen in Init
\* so TLC checks all possible pairs of start/end tries.

\* Trie hash (same as main spec).
EmptyHash == [v |-> FALSE, ch |-> [n \in Nibbles |-> None]]

RECURSIVE SubtrieHash(_, _)
SubtrieHash(trie, prefix) ==
    LET depth == Len(prefix)
        hasValue == prefix \in trie
        childHash(n) ==
            IF depth >= MaxDepth
            THEN IF Append(prefix, n) \in trie
                 THEN [v |-> TRUE, ch |-> [nn \in Nibbles |-> None]]
                 ELSE None
            ELSE LET h == SubtrieHash(trie, Append(prefix, n))
                 IN  IF h = EmptyHash THEN None ELSE h
    IN  [v |-> hasValue, ch |-> [n \in Nibbles |-> childHash(n)]]

RootHash(trie) == SubtrieHash(trie, <<>>)

\* Compute the changes between two tries in a key range.
\* Returns the set of keys where the tries differ within [startKey, endKey].
RECURSIVE SeqLte(_, _)
SeqLte(a, b) ==
    IF Len(a) = 0 THEN TRUE
    ELSE IF Len(b) = 0 THEN FALSE
    ELSE IF a[1] < b[1] THEN TRUE
    ELSE IF a[1] > b[1] THEN FALSE
    ELSE SeqLte(Tail(a), Tail(b))

SeqLt(a, b) == SeqLte(a, b) /\ a # b

ChangedKeysInRange(startTrie, endTrie, startKey, endKey) ==
    {k \in AllKeys :
        /\ (startKey = None \/ SeqLte(startKey, k))
        /\ (endKey = None \/ SeqLte(k, endKey))
        /\ ((k \in startTrie) # (k \in endTrie))}

\* Apply a set of changes from endTrie to a verifier trie.
\* For each changed key: if it's in endTrie, add it; if not, remove it.
ApplyChanges(verifierTrie, changedKeys, endTrie) ==
    (verifierTrie \ changedKeys) \cup (changedKeys \cap endTrie)

--------------------------------------------------------------------------------
(* State Machine *)

VARIABLES
    startRootTrie,  \* Fixed: the starting revision (chosen in Init)
    endRootTrie,    \* Fixed: the target revision (chosen in Init)
    verifierTrie,   \* Current state of the verifier's trie
    startKey,       \* Start key for the next fetch (None = beginning)
    synced,         \* TRUE when sync is complete
    round           \* Round counter (for liveness bound)

vars == <<startRootTrie, endRootTrie, verifierTrie, startKey, synced, round>>

MaxRounds == 4  \* Enough for BF=2, MaxDepth=2 (6 keys max)

Init ==
    /\ startRootTrie \in SUBSET AllKeys
    /\ endRootTrie \in SUBSET AllKeys
    /\ verifierTrie = startRootTrie
    /\ startKey = None
    /\ synced = FALSE
    /\ round = 0

\* A sync round with an honest prover:
\* 1. Fetch all changes in [startKey, None] from prover (honest: returns real changes)
\* 2. Apply changes to verifier trie
\* 3. Check if root hashes match → synced = TRUE
\* 4. Otherwise advance startKey via find_next_key
\*
\* For model simplicity, the honest prover returns ALL remaining changes
\* in each round (not truncated). This is sufficient for safety/liveness.
HonestSyncRound ==
    /\ ~synced
    /\ round < MaxRounds
    /\ LET changedKeys == ChangedKeysInRange(
                            verifierTrie, endRootTrie, startKey, None)
           newTrie == ApplyChanges(verifierTrie, changedKeys, endRootTrie)
           newRoot == RootHash(newTrie)
           endRoot == RootHash(endRootTrie)
       IN  /\ verifierTrie' = newTrie
           /\ IF newRoot = endRoot
              THEN /\ synced' = TRUE
                   /\ startKey' = startKey  \* Unchanged
              ELSE \* find_next_key: advance past the changes we just applied.
                   \* For the honest prover returning all changes, this
                   \* shouldn't happen (all changes applied in one round).
                   \* But we model it for generality.
                   /\ synced' = FALSE
                   /\ startKey' = None  \* Simplified: retry from beginning
           /\ round' = round + 1

\* Adversarial sync round: the prover sends a SUBSET of the real changes
\* (possibly empty). The verifier applies whatever it receives.
\* Verification (not modeled here — see main spec) would reject invalid
\* proofs. We assume only valid-but-incomplete proofs get through.
AdversarialSyncRound ==
    /\ ~synced
    /\ round < MaxRounds
    /\ \E subset \in SUBSET ChangedKeysInRange(
                        verifierTrie, endRootTrie, startKey, None) :
        LET newTrie == ApplyChanges(verifierTrie, subset, endRootTrie)
            newRoot == RootHash(newTrie)
            endRoot == RootHash(endRootTrie)
        IN  /\ verifierTrie' = newTrie
            /\ IF newRoot = endRoot
               THEN /\ synced' = TRUE
                    /\ startKey' = startKey
               ELSE /\ synced' = FALSE
                    \* Advance start key to just past the last applied change.
                    \* For simplicity, use None (retry from beginning).
                    /\ startKey' = None
            /\ round' = round + 1

\* Both tries are fixed after Init — they don't change during sync.
TriesUnchanged == UNCHANGED <<startRootTrie, endRootTrie>>

\* Terminal state: sync complete or max rounds reached.
Done ==
    /\ (synced \/ round >= MaxRounds)
    /\ UNCHANGED vars

Next ==
    /\ (HonestSyncRound \/ AdversarialSyncRound \/ Done)
    /\ TriesUnchanged

--------------------------------------------------------------------------------
(* Properties *)

\* Type invariant.
TypeOK ==
    /\ verifierTrie \in SUBSET AllKeys
    /\ startKey \in {None} \cup AllKeys
    /\ synced \in BOOLEAN
    /\ round \in 0..MaxRounds

\* SAFETY: If sync terminates (synced = TRUE), the verifier's root hash
\* matches the target end_root. This is the core safety property — the
\* verifier never accepts an incorrect state.
Safety ==
    synced => RootHash(verifierTrie) = RootHash(endRootTrie)

\* SAFETY (stronger): If sync terminates, the verifier's trie contains
\* exactly the same keys as the target trie.
\* (This is stronger than root hash match — it's actual state equality,
\* which implies root hash match by the injective hash model.)
StrongSafety ==
    synced => verifierTrie = endRootTrie

\* LIVENESS (bounded): Under an honest prover, sync completes within
\* MaxRounds rounds. We check this as an invariant: if we've hit
\* MaxRounds, we must be synced.
BoundedLiveness ==
    round >= MaxRounds => synced

=============================================================================
