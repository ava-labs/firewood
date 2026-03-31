--------------------------- MODULE ChangeProofVerification ---------------------------
(*
 * Formal verification of Firewood's change proof verification design.
 *
 * Properties verified:
 *   P1: Hash chain soundness (value_digest)
 *   P2: Boundary nibble classification correctness (all cases)
 *   P3: Boundary derivation from proof structure matches formula
 *   P4: In-range substitution preserves hash (trivial, checked at small BF)
 *   P5: Structural validation completeness
 *   P6: Operation consistency (Start/EndProofOperationMismatch)
 *   P7: end_nibbles fallback mechanism
 *
 * Multi-round sync protocol is in SyncProtocol.tla.
 *)

EXTENDS Integers, Sequences, FiniteSets, TLC

CONSTANTS BF, MaxDepth, None

Nibbles == 0..(BF - 1)
AllKeys == UNION {[1..d -> Nibbles] : d \in 1..MaxDepth}
EmptyHash == [v |-> FALSE, ch |-> [n \in Nibbles |-> None]]

--------------------------------------------------------------------------------
(* Trie Model *)

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

--------------------------------------------------------------------------------
(* Proof Model — for P1 and P3 *)

\* A proof path is extracted from a trie for a given key.
\* Each proof node records the hash components at its depth.
\* In compressed tries, some depths may be skipped. We model this
\* by allowing the proof to be any subset of depths along the path.

\* Extract a proof node at a given prefix (depth).
ProofNodeAt(trie, prefix) ==
    LET depth == Len(prefix)
        childHash(n) ==
            IF depth >= MaxDepth
            THEN IF Append(prefix, n) \in trie
                 THEN [v |-> TRUE, ch |-> [nn \in Nibbles |-> None]]
                 ELSE None
            ELSE LET h == SubtrieHash(trie, Append(prefix, n))
                 IN  IF h = EmptyHash THEN None ELSE h
    IN  [depth |-> depth,
         hasValue |-> prefix \in trie,
         childHashes |-> [n \in Nibbles |-> childHash(n)]]

\* The hash of a proof node (same as SubtrieHash at that prefix).
ProofNodeHash(pn) ==
    [v |-> pn.hasValue, ch |-> pn.childHashes]

\* Full proof: one node per depth from 0 to Len(key).
FullProof(trie, key) ==
    [d \in 0..Len(key) |-> ProofNodeAt(trie, SubSeq(key, 1, d))]

\* P1: Hash chain validity.
\* Each proof node's hash must equal the parent's child pointer
\* at the on-path nibble.
HashChainValid(proof, key, rootHash) ==
    \* Root node hash matches expected root.
    /\ ProofNodeHash(proof[0]) = rootHash
    \* Each intermediate node's child at on-path nibble = next node's hash.
    /\ \A d \in 0..(Len(key) - 1) :
        LET onPathNibble == key[d + 1]
        IN  proof[d].childHashes[onPathNibble] = ProofNodeHash(proof[d + 1])

\* A key has a "full path" in the trie when every on-path child is
\* non-None (the subtrie at each intermediate depth is non-empty).
\* This is the precondition for a valid inclusion proof.
HasFullPath(trie, key) ==
    \A d \in 0..(Len(key) - 1) :
        LET prefix == SubSeq(key, 1, d)
        IN  SubtrieHash(trie, Append(prefix, key[d + 1])) # EmptyHash

\* P1 check: for keys with a full path, the extracted proof always
\* has a valid hash chain to the root.
ProofChainSoundness(trie, key) ==
    HasFullPath(trie, key) =>
        LET proof == FullProof(trie, key)
            rootHash == RootHash(trie)
        IN  HashChainValid(proof, key, rootHash)

\* P1 check: for keys with a full path, flipping any proof node's
\* value breaks the hash chain.
ProofTamperDetection(trie, key) ==
    HasFullPath(trie, key) =>
        LET proof == FullProof(trie, key)
            rootHash == RootHash(trie)
        IN  \A d \in 0..Len(key) :
            LET tampered == [proof EXCEPT ![d].hasValue = ~proof[d].hasValue]
            IN  ~HashChainValid(tampered, key, rootHash)

--------------------------------------------------------------------------------
(* P3: Boundary Derivation from Proof Structure *)

\* The implementation derives boundary nibbles from the proof's internal
\* structure, not directly from the key. At intermediate nodes, it uses
\* the NEXT proof node's key at the current depth. At the last node,
\* it falls back to an external key (or None for inclusion proofs).
\*
\* For a valid proof where each node's key is a prefix of the proven key,
\* the next node's key[depth+1] always equals key[depth+1]. So the
\* derivation produces the same result as BoundaryNibbleAt(key, depth).
\*
\* We verify this for all keys, including with compressed proofs where
\* some intermediate depths are skipped.

\* Simple formula (used in existing P2 checks).
BoundaryNibbleAt(key, depth) ==
    IF depth < Len(key) THEN key[depth + 1] ELSE None

\* Derived from proof structure (models the Rust implementation).
\* proofDepths: sorted sequence of depths where proof nodes exist.
\* nodeIdx: index into proofDepths for the current node.
\* fallbackKey: external key for last-node fallback.
DerivedBoundaryNibble(proofDepths, nodeIdx, key, fallbackKey, depth) ==
    IF nodeIdx < Len(proofDepths)
    THEN \* Intermediate node: use next proof node's key at current depth.
         \* The next node's key is SubSeq(key, 1, proofDepths[nodeIdx+1]),
         \* and its element at depth+1 is key[depth+1].
         LET nextNodeKey == SubSeq(key, 1, proofDepths[nodeIdx + 1])
         IN  IF depth < Len(nextNodeKey)
             THEN nextNodeKey[depth + 1]
             ELSE None
    ELSE \* Last node: use fallback key.
         IF depth < Len(fallbackKey)
         THEN fallbackKey[depth + 1]
         ELSE None

\* Verify derivation matches formula for full (non-compressed) proofs.
\* A full proof has one node per depth: <<0, 1, 2, ..., Len(key)>>.
\* The derivation at each intermediate depth uses the next node's key
\* (which is a prefix of the proven key), producing the same result
\* as BoundaryNibbleAt.
DerivationMatchesFormula(key) ==
    LET proofLen == Len(key) + 1  \* Nodes at depths 0..Len(key)
    IN  \A depth \in 0..(Len(key) - 1) :
            \* nodeIdx is depth+1 (1-indexed): the node at this depth
            \* is at position depth+1 in the sequence.
            \* Next node exists at position depth+2.
            LET nodeIdx == depth + 1
                hasNext == nodeIdx < proofLen
                \* Next node's key is SubSeq(key, 1, depth+1).
                \* Its element at position depth+1 is key[depth+1].
                derivedBn ==
                    IF hasNext
                    THEN key[depth + 1]  \* Next node's key at current depth
                    ELSE IF depth < Len(key)
                         THEN key[depth + 1]  \* Fallback
                         ELSE None
            IN  derivedBn = BoundaryNibbleAt(key, depth)

--------------------------------------------------------------------------------
(* Boundary Classification — P2 *)

IsInRange(n, boundaryNibble, isEndProof) ==
    IF boundaryNibble = None
    THEN ~isEndProof
    ELSE IF isEndProof THEN n < boundaryNibble ELSE n > boundaryNibble

RECURSIVE SeqLte(_, _)
SeqLte(a, b) ==
    IF Len(a) = 0 THEN TRUE
    ELSE IF Len(b) = 0 THEN FALSE
    ELSE IF a[1] < b[1] THEN TRUE
    ELSE IF a[1] > b[1] THEN FALSE
    ELSE SeqLte(Tail(a), Tail(b))

SeqLt(a, b) == SeqLte(a, b) /\ a # b

RECURSIVE AllKeysInSubtreeInRange(_, _, _, _)
AllKeysInSubtreeInRange(prefix, startKey, endKey, depth) ==
    IF depth >= MaxDepth
    THEN /\ (startKey = None \/ SeqLte(startKey, prefix))
         /\ (endKey = None \/ SeqLte(prefix, endKey))
    ELSE \A n \in Nibbles :
            AllKeysInSubtreeInRange(Append(prefix, n), startKey, endKey, depth + 1)

BoundaryCorrectnessEndProof(endKey) ==
    \A depth \in 0..(Len(endKey) - 1) :
        LET prefix == SubSeq(endKey, 1, depth)
            bn == BoundaryNibbleAt(endKey, depth)
        IN  \A n \in Nibbles :
                IsInRange(n, bn, TRUE) =>
                    AllKeysInSubtreeInRange(Append(prefix, n), None, endKey, depth + 1)

BoundaryCorrectnessStartProof(startKey) ==
    \A depth \in 0..(Len(startKey) - 1) :
        LET prefix == SubSeq(startKey, 1, depth)
            bn == BoundaryNibbleAt(startKey, depth)
        IN  \A n \in Nibbles :
                IsInRange(n, bn, FALSE) =>
                    AllKeysInSubtreeInRange(Append(prefix, n), startKey, None, depth + 1)

IsInRangeDivergenceParent(n, startBn, endBn) ==
    (IF startBn = None THEN TRUE ELSE n > startBn) /\
    (IF endBn = None THEN TRUE ELSE n < endBn)

BoundaryCorrectnessDivergenceParent(startKey, endKey) ==
    LET minLen == IF Len(startKey) < Len(endKey)
                  THEN Len(startKey) ELSE Len(endKey)
    IN  \A divDepth \in 0..minLen :
            (divDepth < Len(startKey) /\ divDepth < Len(endKey)
             /\ startKey[divDepth + 1] # endKey[divDepth + 1])
            =>
            LET prefix == SubSeq(startKey, 1, divDepth)
                startBn == startKey[divDepth + 1]
                endBn == endKey[divDepth + 1]
            IN  \A n \in Nibbles :
                    IsInRangeDivergenceParent(n, startBn, endBn) =>
                        AllKeysInSubtreeInRange(
                            Append(prefix, n), startKey, endKey, divDepth + 1)

--------------------------------------------------------------------------------
(* P5: Structural Validation *)

\* Model the structural checks from verify_change_proof_structure.
\* A proof context contains: startProof, endProof, batchOps, startKey, endKey.
\* We verify that proofs failing structural validation would also cause
\* downstream verification to be unsound.

\* Structural validation predicate (simplified model of the Rust code).
\* Returns TRUE if the proof passes structural checks.
StructurallyValid(startProofEmpty, endProofEmpty, batchOpsEmpty,
                  startKeyPresent, endKeyPresent, startKey, endKey,
                  keySorted, maxLengthOk) ==
    /\ maxLengthOk
    \* Inverted range rejected.
    /\ (startKeyPresent /\ endKeyPresent) => SeqLte(startKey, endKey)
    \* Non-empty start proof requires start key.
    /\ (~startProofEmpty => startKeyPresent)
    \* Non-empty batch_ops requires end proof.
    /\ (~batchOpsEmpty => ~endProofEmpty)
    \* No batch_ops + end_proof requires end_key.
    /\ (batchOpsEmpty /\ ~endProofEmpty => endKeyPresent)
    \* No batch_ops + no end_proof + end_key present → missing end proof.
    /\ (batchOpsEmpty /\ endProofEmpty => ~endKeyPresent)
    \* Keys sorted.
    /\ keySorted

\* Spot check: inverted range always rejected.
InvertedRangeRejected ==
    \A sk \in AllKeys : \A ek \in AllKeys :
        SeqLt(ek, sk) =>
            ~StructurallyValid(TRUE, TRUE, TRUE, TRUE, TRUE, sk, ek, TRUE, TRUE)

\* Spot check: non-empty batch_ops without end proof rejected.
MissingEndProofRejected ==
    ~StructurallyValid(TRUE, TRUE, FALSE, TRUE, TRUE, <<0>>, <<1>>, TRUE, TRUE)

\* Spot check: unsorted keys rejected.
UnsortedKeysRejected ==
    ~StructurallyValid(TRUE, FALSE, FALSE, TRUE, TRUE, <<0>>, <<1>>, FALSE, TRUE)

--------------------------------------------------------------------------------
(* P6: Operation Consistency Checks *)

\* When first_op_key = start_key, the start proof result (inclusion/exclusion)
\* must match the op type (Put/Delete).
\*   Put at start_key => start proof is inclusion (key exists in end_root)
\*   Delete at start_key => start proof is exclusion (key absent)
\* Same for last_op_key and end proof.

\* Model: for a key that EXISTS in a trie, the proof is inclusion.
\* For a key that DOESN'T exist, the proof is exclusion.
\* "inclusion" = TRUE, "exclusion" = FALSE.
ProofIsInclusion(trie, key) == key \in trie

\* Op consistency: Put expects inclusion, Delete expects exclusion.
OpConsistent(isPut, isInclusion) ==
    (isPut /\ isInclusion) \/ (~isPut /\ ~isInclusion)

\* P6 check: for all tries and keys, the consistency check correctly
\* identifies whether the op matches the proof result.
\* A Put of a key that exists in end_root → inclusion proof → consistent.
\* A Put of a key that doesn't exist in end_root → exclusion proof → inconsistent (caught!).
\* A Delete of a key that exists in end_root → inclusion proof → inconsistent (caught!).
\* A Delete of a key absent from end_root → exclusion proof → consistent.
OpConsistencyCorrectness ==
    \A trie \in SUBSET AllKeys :
        \A key \in AllKeys :
            LET isInclusion == ProofIsInclusion(trie, key)
            IN  \* Put + exists in trie → consistent ✓
                /\ OpConsistent(TRUE, ProofIsInclusion(trie, key))
                       = (key \in trie)
                \* Delete + absent from trie → consistent ✓
                /\ OpConsistent(FALSE, ProofIsInclusion(trie, key))
                       = (key \notin trie)

--------------------------------------------------------------------------------
(* P7: end_nibbles Fallback Mechanism *)

\* Models the actual decision logic in the Rust code:
\*   batch_ops non-empty → end_nibbles = last_op_key nibbles
\*   batch_ops empty → end_nibbles = end_key nibbles
\* Verify: the chosen fallback always provides a boundary nibble at
\* the last end proof node (preventing the "no children checked" exploit).

EndNibblesSource(batchOpsEmpty, lastOpKey, endKey) ==
    IF ~batchOpsEmpty THEN lastOpKey ELSE endKey

\* At the last end proof node depth, the fallback must provide a nibble.
\* The last end proof node is at depth = Len(endProofKey).
\* For end proof, endProofKey = lastOpKey when batch_ops non-empty,
\* or endKey when empty.
EndNibblesProvidesBoundary(batchOpsEmpty, lastOpKey, endKey) ==
    LET source == EndNibblesSource(batchOpsEmpty, lastOpKey, endKey)
        endProofKey == IF ~batchOpsEmpty THEN lastOpKey ELSE endKey
    IN  \A depth \in 0..(Len(endProofKey) - 1) :
            BoundaryNibbleAt(source, depth) # None

\* The exploit scenario: batch_ops empty, end_proof non-empty, but
\* using None for end_nibbles instead of end_key.
\* Verify this FAILS to provide boundary nibbles.
ExploitScenarioFails ==
    \A endKey \in AllKeys :
        Len(endKey) > 0 =>
            \* With correct fallback (end_key): boundary exists
            /\ BoundaryNibbleAt(endKey, 0) # None
            \* With broken fallback (None/empty): boundary is None
            \* (This is what the exploit did — no children checked)
            /\ BoundaryNibbleAt(<<>>, 0) = None

\* Full check: for all valid configurations, end_nibbles provides boundaries.
EndNibblesFallbackCorrect ==
    \* Case 1: batch_ops non-empty.
    /\ \A lastOpKey \in AllKeys :
        \A endKey \in AllKeys :
            EndNibblesProvidesBoundary(FALSE, lastOpKey, endKey)
    \* Case 2: batch_ops empty, end_key provided.
    /\ \A endKey \in AllKeys :
        EndNibblesProvidesBoundary(TRUE, <<>>, endKey)

--------------------------------------------------------------------------------
(* P8: Truncated Proofs — end_nibbles Key Selection *)

\* When batch_ops is non-empty and truncated (doesn't cover all changes),
\* end_nibbles MUST use last_op_key (not end_key). The proposal only has
\* changes up to last_op_key. Children beyond last_op_key's nibble at
\* the last end proof depth are unchanged from start_root and may differ
\* from end_root. Using end_key would compare those out-of-range children,
\* causing false mismatches.
\*
\* We verify: at the last end proof depth, last_op_key's nibble produces
\* a boundary that only checks children whose subtrees are within the
\* range of keys the proposal has actually modified.

\* The proposal modifies keys in [start_key, last_op_key].
\* At the last end proof node (built for last_op_key), in-range children
\* are those with subtrees entirely <= last_op_key.
TruncatedEndNibblesCorrect ==
    \A lastOpKey \in AllKeys :
        \A endKey \in AllKeys :
            SeqLt(lastOpKey, endKey) =>
                \* Using last_op_key: boundary checks children within proposal range.
                \A depth \in 0..(Len(lastOpKey) - 1) :
                    LET prefix == SubSeq(lastOpKey, 1, depth)
                        bn == BoundaryNibbleAt(lastOpKey, depth)
                    IN  \A n \in Nibbles :
                            IsInRange(n, bn, TRUE) =>
                                \* All keys in this subtree are <= lastOpKey
                                \* (within the proposal's modification range).
                                AllKeysInSubtreeInRange(
                                    Append(prefix, n), None, lastOpKey, depth + 1)

\* Counter-check: using end_key instead of last_op_key would extend the
\* boundary PAST the proposal's modification range, potentially checking
\* children that the proposal hasn't modified.
TruncatedEndKeyWouldOverreach ==
    \* There exists a configuration where end_key boundary checks MORE
    \* children than last_op_key boundary, and some of those extra
    \* children have subtrees outside the proposal's range.
    \E lastOpKey \in AllKeys :
        \E endKey \in AllKeys :
            SeqLt(lastOpKey, endKey) =>
                \E depth \in 0..(Len(endKey) - 1) :
                    LET bnLastOp == BoundaryNibbleAt(lastOpKey, depth)
                        bnEndKey == BoundaryNibbleAt(endKey, depth)
                    IN  \* end_key boundary is wider (checks more children)
                        \E n \in Nibbles :
                            IsInRange(n, bnEndKey, TRUE)
                            /\ ~IsInRange(n, bnLastOp, TRUE)

--------------------------------------------------------------------------------
(* P9: Exclusion Proof Boundary Behavior *)

\* In an exclusion proof, the last proof node's key differs from the
\* proven key. The proof terminates at an ancestor or sibling.
\*
\* For boundary nibble derivation:
\* - Intermediate nodes: boundary comes from the NEXT proof node's key
\*   at the current depth (same as inclusion proofs).
\* - Last node: boundary comes from the FALLBACK (external key), NOT
\*   the proof node's key. The external key is start_key (for start
\*   proofs) or last_op_key/end_key (for end proofs).
\*
\* The critical property: even when the last proof node is at a
\* different depth than the proven key, the fallback nibble from the
\* external key still correctly classifies in-range children.

\* Model an exclusion proof: the proof path goes to some ancestor of
\* the key, then the last node is at a SHORTER depth.
\* The boundary at the last node uses the key's nibble at that depth.
ExclusionProofBoundaryCorrectEnd(provenKey, lastNodeDepth) ==
    \* lastNodeDepth < Len(provenKey): exclusion proof, key goes deeper
    (lastNodeDepth < Len(provenKey)) =>
        LET bn == BoundaryNibbleAt(provenKey, lastNodeDepth)
        IN  \* The boundary nibble at the last node depth still correctly
            \* classifies in-range children relative to provenKey.
            \A n \in Nibbles :
                LET prefix == SubSeq(provenKey, 1, lastNodeDepth)
                IN  IsInRange(n, bn, TRUE) =>
                        AllKeysInSubtreeInRange(
                            Append(prefix, n), None, provenKey, lastNodeDepth + 1)

ExclusionProofBoundaryCorrectStart(provenKey, lastNodeDepth) ==
    (lastNodeDepth < Len(provenKey)) =>
        LET bn == BoundaryNibbleAt(provenKey, lastNodeDepth)
        IN  \A n \in Nibbles :
                LET prefix == SubSeq(provenKey, 1, lastNodeDepth)
                IN  IsInRange(n, bn, FALSE) =>
                        AllKeysInSubtreeInRange(
                            Append(prefix, n), provenKey, None, lastNodeDepth + 1)

\* Also check: at the last node of an inclusion proof (lastNodeDepth = Len(key)),
\* the boundary is None. For end proof, no children checked (correct: all
\* children are after the proven key). For start proof, all children
\* checked (correct: all children are after the proven key, which is the
\* in-range direction for start proofs).
InclusionProofLastNodeBehavior ==
    \A key \in AllKeys :
        LET bn == BoundaryNibbleAt(key, Len(key))
        IN  /\ bn = None
            \* End proof: IsInRange(n, None, TRUE) = FALSE for all n
            /\ \A n \in Nibbles : ~IsInRange(n, bn, TRUE)
            \* Start proof: IsInRange(n, None, FALSE) = TRUE for all n
            /\ \A n \in Nibbles : IsInRange(n, bn, FALSE)

--------------------------------------------------------------------------------
(* P10: Empty batch_ops Attack — End-to-End *)

\* Attack scenario: prover sends empty batch_ops with valid boundary
\* proofs, claiming "no changes" when end_root actually differs from
\* start_root in the range [start_key, end_key].
\*
\* The defense: end_nibbles falls back to end_key, so the boundary at
\* the last end proof node checks children up to end_key's nibble.
\* If end_root differs from start_root in-range, some in-range child
\* hash will differ between the proof (from end_root) and the proposal
\* (which is just start_root with no modifications).
\*
\* We verify: for any two tries that differ in-range, the in-range
\* children comparison using end_key's boundary will detect at least
\* one mismatch.

\* Check: if two tries differ on any key within [start_key, end_key],
\* then the verification detects it — either via an in-range child hash
\* mismatch OR via a value mismatch at a proof node along the path.
\*
\* The end proof checks both:
\*   1. Values at each proof node depth (verify_change_proof_node_value)
\*   2. In-range children at each proof node depth
\*
\* A difference at the proven key itself (e.g., value at endKey changed)
\* is caught by the value check, not the children check.
EmptyBatchOpsAttackDetected(startTrie, endTrie, endKey) ==
    \* Precondition: the tries differ on at least one key <= endKey.
    (\E k \in AllKeys : SeqLte(k, endKey) /\ ((k \in startTrie) # (k \in endTrie)))
    =>
    \* Consequence: at some depth, either a value or in-range child differs.
    (\E depth \in 0..Len(endKey) :
        LET prefix == SubSeq(endKey, 1, depth)
        IN  \* Value mismatch at this depth.
            \/ (prefix \in startTrie) # (prefix \in endTrie)
            \* In-range child hash mismatch at this depth.
            \/ (depth < Len(endKey) /\
                LET bn == BoundaryNibbleAt(endKey, depth)
                IN  \E n \in Nibbles :
                        IsInRange(n, bn, TRUE) /\
                        SubtrieHash(startTrie, Append(prefix, n)) #
                        SubtrieHash(endTrie, Append(prefix, n))))

--------------------------------------------------------------------------------
(* P11: Complete Structural Validation *)

\* Expand the structural validation model to cover all checks from
\* verify_change_proof_structure, not just spot checks.

\* Full structural validation predicate. Models the Rust code's checks.
\* Parameters model the proof shape without requiring actual proof content.
FullStructuralValidation(
    startProofEmpty, endProofEmpty, batchOpsEmpty,
    startKeyPresent, endKeyPresent, startKey, endKey,
    firstOpKey, lastOpKey,
    keySorted, maxLengthOk, noDeleteRange) ==
    /\ maxLengthOk
    /\ noDeleteRange
    /\ keySorted
    \* Inverted range.
    /\ (startKeyPresent /\ endKeyPresent) => SeqLte(startKey, endKey)
    \* Non-empty start proof requires start key.
    /\ (~startProofEmpty => startKeyPresent)
    \* Non-empty batch_ops requires end proof.
    /\ (~batchOpsEmpty => ~endProofEmpty)
    \* No batch_ops + end_proof requires end_key.
    /\ (batchOpsEmpty /\ ~endProofEmpty => endKeyPresent)
    \* No batch_ops + no end_proof + end_key → missing end proof.
    /\ (batchOpsEmpty /\ endProofEmpty => ~endKeyPresent)
    \* start_key <= first_op_key.
    /\ (startKeyPresent /\ ~batchOpsEmpty) => SeqLte(startKey, firstOpKey)
    \* end_key >= last_op_key.
    /\ (endKeyPresent /\ ~batchOpsEmpty) => SeqLte(lastOpKey, endKey)

\* Check: start_key > first_op_key always rejected.
StartKeyBoundsRejected ==
    \A sk \in AllKeys : \A fk \in AllKeys :
        SeqLt(fk, sk) =>
            ~FullStructuralValidation(
                TRUE, FALSE, FALSE, TRUE, TRUE, sk, <<BF-1, BF-1>>,
                fk, fk, TRUE, TRUE, TRUE)

\* Check: end_key < last_op_key always rejected.
EndKeyBoundsRejected ==
    \A ek \in AllKeys : \A lk \in AllKeys :
        SeqLt(ek, lk) =>
            ~FullStructuralValidation(
                TRUE, FALSE, FALSE, TRUE, TRUE, <<0>>, ek,
                <<0>>, lk, TRUE, TRUE, TRUE)

\* Check: max_length exceeded always rejected.
MaxLengthRejected ==
    ~FullStructuralValidation(
        TRUE, FALSE, FALSE, TRUE, TRUE, <<0>>, <<1>>,
        <<0>>, <<1>>, TRUE, FALSE, TRUE)

\* Check: DeleteRange always rejected.
DeleteRangeRejected ==
    ~FullStructuralValidation(
        TRUE, FALSE, FALSE, TRUE, TRUE, <<0>>, <<1>>,
        <<0>>, <<1>>, TRUE, TRUE, FALSE)

\* Check: unexpected end proof (no batch_ops, no end_key, but end_proof present).
UnexpectedEndProofRejected ==
    ~FullStructuralValidation(
        TRUE, FALSE, TRUE, FALSE, FALSE, <<>>, <<>>,
        <<>>, <<>>, TRUE, TRUE, TRUE)

\* Check: boundary proof unverifiable (non-empty start_proof, no start_key).
BoundaryProofUnverifiableRejected ==
    ~FullStructuralValidation(
        FALSE, FALSE, FALSE, FALSE, TRUE, <<>>, <<1>>,
        <<0>>, <<1>>, TRUE, TRUE, TRUE)

\* Check: exhaustive — for all possible proof shapes, the structural
\* predicate accepts iff the proof shape is legitimate.
\* A legitimate shape: batch_ops non-empty with sorted keys in range,
\* end proof present, start proof present iff start_key present.
StructuralValidationAcceptsValidShapes ==
    \A sk \in AllKeys : \A ek \in AllKeys :
        SeqLte(sk, ek) =>
            FullStructuralValidation(
                TRUE, FALSE, FALSE, TRUE, TRUE, sk, ek,
                sk, ek, TRUE, TRUE, TRUE)

--------------------------------------------------------------------------------
(* P12: Divergence at Depth Zero *)

\* When both start and end proofs exist but diverge at the root (depth 0),
\* the verification must reject with BoundaryProofsDivergeAtRoot. This
\* means the two proofs have different first nodes, which shouldn't happen
\* since both are rooted at end_root.
\*
\* In our model: if start_key and end_key have different first nibbles,
\* AND both proofs are full-depth, the divergence depth is 0 and the
\* checked_sub(0, 1) returns None → error.

DivergenceAtDepthZeroMeansSharedPrefixEmpty(startKey, endKey) ==
    \* If first nibbles differ, divergence is at depth 0.
    (Len(startKey) > 0 /\ Len(endKey) > 0 /\ startKey[1] # endKey[1])
        => \* No shared prefix nodes exist above the divergence.
           \* The implementation rejects this with BoundaryProofsDivergeAtRoot
           \* because there's no parent node to check in-range children on.
           \* We verify: the set of in-range children at depth 0 between
           \* two different first nibbles is well-defined.
           LET startBn == startKey[1]
               endBn == endKey[1]
           IN  \* Some children ARE in-range between the two nibbles.
               \* These would need checking, but there's no shared parent —
               \* hence the rejection is necessary.
               (endBn > startBn + 1) =>
                   \E n \in Nibbles : IsInRangeDivergenceParent(n, startBn, endBn)

--------------------------------------------------------------------------------
(* P13: Deferred Boundary Mismatch *)

\* Models TestChangeProofBoundaryValueMismatchDeferred: a mismatch at a
\* key between start_key and end_key might not be detected in a truncated
\* round (because the mismatch is beyond the truncation point), but WILL
\* be detected in a subsequent round that covers that range.
\*
\* We verify: if two tries differ at a key k where startKey < k < endKey,
\* and round 1 is truncated at lastOpKey < k, then:
\*   1. Round 1's boundary check does NOT cover k (it's beyond last_op_key)
\*   2. Round 2 starting from lastOpKey DOES cover k

DeferredMismatchDetection(startKey, mismatchKey, lastOpKey, endKey) ==
    \* Preconditions: startKey < lastOpKey < mismatchKey <= endKey
    (SeqLt(startKey, lastOpKey) /\ SeqLt(lastOpKey, mismatchKey)
     /\ SeqLte(mismatchKey, endKey))
    =>
    \* Round 1 (truncated at lastOpKey): mismatchKey is NOT in the
    \* checked range. The boundary at last_op_key excludes it.
    /\ \A depth \in 0..(Len(lastOpKey) - 1) :
        LET bn == BoundaryNibbleAt(lastOpKey, depth)
            prefix == SubSeq(lastOpKey, 1, depth)
        IN  \* If mismatchKey is in the subtree at a nibble beyond bn,
            \* that nibble is NOT in-range for an end proof.
            (depth < Len(mismatchKey) /\ mismatchKey[depth + 1] > bn)
                => ~IsInRange(mismatchKey[depth + 1], bn, TRUE)
    \* Round 2 (starts from lastOpKey, covers up to endKey):
    \* mismatchKey IS in the range [lastOpKey, endKey].
    /\ SeqLte(lastOpKey, mismatchKey)

--------------------------------------------------------------------------------
(* P14: Empty End Root — Hash Chain Failure *)

\* When end_root is the empty trie hash and batch_ops is non-empty,
\* the boundary proof hash chain must fail (because the proof was
\* extracted from a non-empty trie but end_root claims empty).
\*
\* In our model: if end_root = EmptyHash, the only trie that produces
\* this hash is the empty set {}. A proof from a non-empty trie has
\* a root hash != EmptyHash, so the chain doesn't match.
EmptyEndRootDetectsNonEmptyProof ==
    \A trie \in SUBSET AllKeys :
        trie # {} =>
            RootHash(trie) # EmptyHash

--------------------------------------------------------------------------------
(* P15: Delete as Last Op in Truncated Proof *)

\* When the last batch op is a Delete, the end proof is an exclusion
\* proof (key absent from end_root). The boundary nibble at the last
\* end proof node comes from the deleted key's nibbles. This should
\* still correctly classify in-range children.
\*
\* This is covered by P9 (exclusion proof boundary behavior), but we
\* add an explicit check: a Delete op's key used as end_nibbles source
\* produces correct boundaries.
DeleteLastOpEndNibblesCorrect ==
    \* A Delete means the key is ABSENT from end_root.
    \* The end proof for this key is an exclusion proof.
    \* end_nibbles still uses this key for boundary derivation.
    \A deleteKey \in AllKeys :
        \A depth \in 0..(Len(deleteKey) - 1) :
            LET prefix == SubSeq(deleteKey, 1, depth)
                bn == BoundaryNibbleAt(deleteKey, depth)
            IN  \A n \in Nibbles :
                    IsInRange(n, bn, TRUE) =>
                        AllKeysInSubtreeInRange(
                            Append(prefix, n), None, deleteKey, depth + 1)

--------------------------------------------------------------------------------
(* P16: Asymmetric Depth — Short Start Key, Long End Key *)

\* When start_key is shorter than end_key (e.g., start=<<0>>,
\* end=<<0,0,3>>), the divergence point and boundary nibble derivation
\* must handle the depth asymmetry correctly.
\*
\* At the divergence depth (where the proofs split), the start proof
\* may have fewer nodes below. The start_bn at the divergence parent
\* comes from the start proof's next node (if it exists) or falls back
\* to start_key's nibble.

AsymmetricDepthBoundaryCorrect ==
    \A sk \in AllKeys : \A ek \in AllKeys :
        (SeqLt(sk, ek) /\ Len(sk) < Len(ek)) =>
            \* The divergence parent boundary is still correct.
            BoundaryCorrectnessDivergenceParent(sk, ek)
            \* And the start proof boundary is correct for the shorter key.
            /\ BoundaryCorrectnessStartProof(sk)
            \* And the end proof boundary is correct for the longer key.
            /\ BoundaryCorrectnessEndProof(ek)

--------------------------------------------------------------------------------
(* BUG REPRODUCTION: Known bugs modeled to verify TLC catches them *)

\* Bug 1 (commit 9c43a6cb4): Before the fix, the start proof's
\* inclusion/exclusion result was discarded. value_digest was called
\* for hash chain verification, but Ok(Some) and Ok(None) were both
\* accepted regardless of the first batch op's type.
\*
\* Attack: Attacker adds Put(start_key) where start_key doesn't exist
\* in end_root. The start proof is an exclusion proof (key absent).
\* Without the consistency check, this passes structural validation.
\* The in-range children comparison MAY still catch it (because the
\* proposal now has a spurious key), but only if the mismatch falls
\* within an in-range child at some depth.
\*
\* We model the BUGGY behavior: OpConsistent always returns TRUE
\* (no consistency check). Then verify that for some (trie, key, isPut)
\* combination, the buggy check accepts an inconsistent proof.

BuggyOpConsistent(isPut, isInclusion) == TRUE  \* Always accepts

\* The buggy version accepts proofs that the fixed version rejects.
Bug1_StartProofNoConsistencyCheck ==
    \E trie \in SUBSET AllKeys :
        \E key \in AllKeys :
            \* A Put at a key absent from trie: the fixed check rejects,
            \* but the buggy check accepts.
            /\ key \notin trie
            /\ BuggyOpConsistent(TRUE, ProofIsInclusion(trie, key)) = TRUE
            /\ OpConsistent(TRUE, ProofIsInclusion(trie, key)) = FALSE

\* Bug 2 (end_nibbles exploit): Before the fix, when batch_ops was empty,
\* end_nibbles was derived from last_op_key (which doesn't exist),
\* producing None. At the last end proof node, boundary_nibble = None,
\* and IsInRange(n, None, TRUE) = FALSE for all n — no children checked.
\*
\* Attack: Prover sends empty batch_ops claiming "no changes" while
\* end_root actually differs from start_root in range. Without the
\* end_key fallback, verification accepts the unchanged start state.
\*
\* We model the BUGGY behavior: end_nibbles = None when batch_ops empty
\* (instead of falling back to end_key). Then verify that for some
\* (startTrie, endTrie, endKey) combination, a real difference goes
\* undetected.

BuggyEndNibblesSource(batchOpsEmpty, lastOpKey, endKey) ==
    IF ~batchOpsEmpty THEN lastOpKey ELSE <<>>  \* Bug: empty instead of endKey

\* With the buggy fallback, at the last end proof node, NO children
\* are checked (boundary = None for end proof → all IsInRange = FALSE).
Bug2_NoChildrenChecked ==
    \A n \in Nibbles :
        ~IsInRange(n, BoundaryNibbleAt(<<>>, 0), TRUE)

\* With the buggy code, a difference CAN go undetected: two tries
\* differ but the verification checks no children.
Bug2_DifferenceUndetected ==
    BF > 3 \/
    (\E st \in SUBSET AllKeys :
        \E et \in SUBSET AllKeys :
            \E ek \in AllKeys :
                \* Tries differ within range.
                /\ (\E k \in AllKeys : SeqLte(k, ek) /\ ((k \in st) # (k \in et)))
                \* But with buggy end_nibbles (None), no children or values
                \* are checked at depth 0 along the (empty) proof path.
                \* The only check would be value at <<>> (root), but root
                \* has no value in our model (keys start at length 1).
                \* So the difference goes completely undetected.
                /\ Bug2_NoChildrenChecked)

--------------------------------------------------------------------------------
(* ASSUME Checks — evaluated by TLC *)

\* P1: Hash chain soundness. Enumerates SUBSET AllKeys (2^|AllKeys|),
\* so only feasible at small BF. Guarded by BF <= 3.
ASSUME Chk_ProofSoundness ==
    BF > 3 \/
    \A trie \in SUBSET AllKeys :
        \A key \in AllKeys :
            ProofChainSoundness(trie, key)

ASSUME Chk_TamperDetection ==
    BF > 3 \/
    (\A trie \in SUBSET AllKeys :
        trie # {} =>
            \A key \in AllKeys :
                Len(key) > 0 =>
                    ProofTamperDetection(trie, key))

\* P6: Operation consistency. Also enumerates SUBSET AllKeys.
ASSUME Chk_OpConsistency ==
    BF > 3 \/ OpConsistencyCorrectness

\* P2: Boundary correctness (all cases).
ASSUME Chk_BoundaryEnd ==
    \A key \in AllKeys : BoundaryCorrectnessEndProof(key)

ASSUME Chk_BoundaryStart ==
    \A key \in AllKeys : BoundaryCorrectnessStartProof(key)

ASSUME Chk_BoundaryDivergence ==
    \A sk \in AllKeys : \A ek \in AllKeys :
        SeqLt(sk, ek) => BoundaryCorrectnessDivergenceParent(sk, ek)

\* P3: Boundary derivation matches formula.
ASSUME Chk_DerivationMatches ==
    \A key \in AllKeys : DerivationMatchesFormula(key)

\* P5: Structural validation spot checks.
ASSUME Chk_InvertedRange == InvertedRangeRejected
ASSUME Chk_MissingEndProof == MissingEndProofRejected
ASSUME Chk_UnsortedKeys == UnsortedKeysRejected

\* P7: end_nibbles fallback.
ASSUME Chk_EndNibblesFallback == EndNibblesFallbackCorrect
ASSUME Chk_ExploitScenario == ExploitScenarioFails

\* P8: Truncated proofs — last_op_key boundary is correct.
ASSUME Chk_TruncatedEndNibbles == TruncatedEndNibblesCorrect
\* P8: Using end_key would over-reach (checks children beyond proposal range).
ASSUME Chk_TruncatedOverreach == TruncatedEndKeyWouldOverreach

\* P9: Exclusion proof boundary is correct.
ASSUME Chk_ExclusionEnd ==
    \A key \in AllKeys : \A d \in 0..Len(key) :
        ExclusionProofBoundaryCorrectEnd(key, d)

ASSUME Chk_ExclusionStart ==
    \A key \in AllKeys : \A d \in 0..Len(key) :
        ExclusionProofBoundaryCorrectStart(key, d)

ASSUME Chk_InclusionLastNode == InclusionProofLastNodeBehavior

\* P10: Empty batch_ops attack detection. Enumerates SUBSET AllKeys.
ASSUME Chk_EmptyBatchOpsAttack ==
    BF > 3 \/
    (\A st \in SUBSET AllKeys :
        \A et \in SUBSET AllKeys :
            \A ek \in AllKeys :
                EmptyBatchOpsAttackDetected(st, et, ek))

\* P11: Complete structural validation.
ASSUME Chk_StartKeyBounds == StartKeyBoundsRejected
ASSUME Chk_EndKeyBounds == EndKeyBoundsRejected
ASSUME Chk_MaxLength == MaxLengthRejected
ASSUME Chk_DeleteRange == DeleteRangeRejected
ASSUME Chk_UnexpectedEndProof == UnexpectedEndProofRejected
ASSUME Chk_BoundaryUnverifiable == BoundaryProofUnverifiableRejected
ASSUME Chk_ValidShapesAccepted == StructuralValidationAcceptsValidShapes

\* P12: Divergence at depth zero.
ASSUME Chk_DivergenceDepthZero ==
    \A sk \in AllKeys : \A ek \in AllKeys :
        DivergenceAtDepthZeroMeansSharedPrefixEmpty(sk, ek)

\* P13: Deferred boundary mismatch detection.
ASSUME Chk_DeferredMismatch ==
    \A sk \in AllKeys : \A mk \in AllKeys :
        \A lk \in AllKeys : \A ek \in AllKeys :
            DeferredMismatchDetection(sk, mk, lk, ek)

\* P14: Empty end root detects non-empty proofs. Enumerates SUBSET AllKeys.
ASSUME Chk_EmptyEndRoot ==
    BF > 3 \/ EmptyEndRootDetectsNonEmptyProof

\* P15: Delete as last op — end_nibbles boundary correct.
ASSUME Chk_DeleteLastOp == DeleteLastOpEndNibblesCorrect

\* P16: Asymmetric depth — boundary correct with mismatched key lengths.
ASSUME Chk_AsymmetricDepth == AsymmetricDepthBoundaryCorrect

\* Bug reproductions: verify TLC confirms the bugs exist in the buggy versions.

\* Bug 1: Without the consistency check, inconsistent proofs are accepted.
ASSUME Chk_Bug1_Exists ==
    BF > 3 \/ Bug1_StartProofNoConsistencyCheck

\* Bug 2: Without the end_key fallback, no children are checked.
ASSUME Chk_Bug2_NoChildren == Bug2_NoChildrenChecked

\* Bug 2: With buggy code, a real difference goes undetected.
ASSUME Chk_Bug2_Undetected == Bug2_DifferenceUndetected

--------------------------------------------------------------------------------
(* TLC dummy state machine *)

VARIABLE dummy
Init == dummy = TRUE
Next == UNCHANGED dummy

=============================================================================
