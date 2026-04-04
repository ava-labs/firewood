------------------------- MODULE CompressedTrieModel -------------------------
(*
 * Validates the compressed trie model by exhaustive checking.
 *
 * Extends CompressedTrie (pure operators) and adds a state machine
 * where each state is one trie. TLC checks invariants P1-P4 on every
 * trie and reports progress. P5-P7 are ASSUME checks.
 *
 * Properties verified:
 *   P1: Round-trip (Decompress(Compress(trie)) = trie)
 *   P2: Lookup correctness
 *   P3: Structural invariants (path compression)
 *   P4: Depth bounds
 *   P5: Hash injectivity
 *   P6: Empty trie
 *   P7: Singletons
 *)

EXTENDS CompressedTrie

--------------------------------------------------------------------------------
(* State machine — each state is one trie *)

VARIABLE trie

Init == trie \in AllTries
Next == UNCHANGED trie

\* P1: Round-trip.
RoundTrip == Decompress(Compress(trie)) = trie

\* P2: Lookup correctness.
LookupCorrect == \A k \in AllKeys : Lookup(Compress(trie), k) = trie[k]

\* P3: Structural invariants.
StructurallyValid == WellFormed(Compress(trie))

\* P4: Depth bounds.
DepthBounded == DepthOk(Compress(trie), 0)

--------------------------------------------------------------------------------
(* ASSUME checks *)

\* P5: Hash injectivity. Only feasible at small configs.
ASSUME Chk_HashInjectivity ==
    (BF + MaxDepth > 4) \/
    (\A t1 \in AllTries :
        \A t2 \in AllTries :
            (t1 # t2) => (CompressedRootHash(t1) # CompressedRootHash(t2)))

\* P6: Empty trie compresses to None.
ASSUME Chk_EmptyTrie ==
    Compress(EmptyTrie) = None

\* P7: Singleton tries.
ASSUME Chk_Singletons ==
    \A k \in AllKeys :
        \A v \in Values :
            LET t == [key \in AllKeys |-> IF key = k THEN v ELSE NoVal]
                ctrie == Compress(t)
            IN  /\ ctrie # None
                /\ ctrie.pp = k
                /\ ctrie.value = v
                /\ \A n \in Nibbles : ctrie.children[n] = None

=============================================================================
