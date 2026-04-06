---------------------- MODULE ChangeProofVerification ----------------------
(*
 * Formal verification of Firewood's change proof verification pipeline.
 *
 * Models the full algorithm from firewood/src/merkle/mod.rs:
 *   1. Proof extraction from a compressed trie
 *   2. Diff computation (batch_ops generation)
 *   3. Reconcile: insert branch structure + resolve value conflicts
 *   4. Collapse: strip intermediate branches between proof nodes
 *   5. Compute outside children (ChildMask boundary classification)
 *   6. Hybrid hash (in-range from trie, outside from proof)
 *   7. Compare hybrid hash against end_root
 *
 * Two run modes:
 *   Exhaustive (BF=2): enumerate all (startTrie, endTrie) pairs
 *   Probabilistic (BF=3): TLC -simulate samples random pairs
 *)

EXTENDS CompressedTrie

--------------------------------------------------------------------------------
(* Section 1: Proof Extraction *)

\* Extract a Merkle proof from a compressed trie for a given key.
\* Returns a sequence of proof nodes from root toward the key.
\* Each proof node: [key, value, ch] where ch maps nibbles to child hashes.
\*
\* Corresponds to Merkle::prove in merkle/mod.rs.

RECURSIVE ExtractProofHelper(_, _, _)
ExtractProofHelper(node, remKey, prefix) ==
    IF node = None THEN <<>>
    ELSE
        LET fullPath == prefix \o node.pp
            ppLen    == Len(node.pp)
            rkLen    == Len(remKey)
            cpl      == CommonPrefixLen(remKey, node.pp)

            \* Compute child hashes for this proof node.
            childHashes == [n \in Nibbles |->
                CompressedHash(node.children[n], Append(fullPath, n))]

            thisPN == [key |-> fullPath, value |-> node.value,
                       ch  |-> childHashes]
        IN
            IF cpl < ppLen THEN
                \* Divergence within partial path — exclusion proof.
                <<thisPN>>
            ELSE IF rkLen = ppLen THEN
                \* Key fully consumed — inclusion or leaf exclusion.
                <<thisPN>>
            ELSE
                \* Full pp match, more key remains — descend.
                LET ni   == remKey[ppLen + 1]
                    rest == SubSeq(remKey, ppLen + 2, rkLen)
                IN  <<thisPN>> \o
                    ExtractProofHelper(node.children[ni], rest,
                                       Append(fullPath, ni))

ExtractProof(ctrie, key) ==
    IF ctrie = None THEN <<>>
    ELSE ExtractProofHelper(ctrie, key, <<>>)

\* Verify that a proof follows the correct branch toward the target key.
\* For every non-terminal proof node, the nibble linking to the next node
\* must equal the nibble toward the target key at that depth.
\*
\* This is the fix from commit ac341f67e: value_digest now rejects proofs
\* that traverse a different subtree than the one containing the target key.
ProofFollowsKey(proofNodes, targetKey) ==
    IF proofNodes = <<>> THEN TRUE
    ELSE
        \A i \in 1..(Len(proofNodes) - 1) :
            LET thisKey == proofNodes[i].key
                nextKey == proofNodes[i + 1].key
            IN  \* Proof node must be a strict prefix of both target
                \* and next node. If not, the proof doesn't follow
                \* the target path — reject.
                /\ Len(thisKey) < Len(targetKey)
                /\ Len(thisKey) < Len(nextKey)
                /\ nextKey[Len(thisKey) + 1] = targetKey[Len(thisKey) + 1]

\* Hash of a proof node (same as CompressedHash record structure).
ProofNodeHash(pn) == [fp |-> pn.key, v |-> pn.value, ch |-> pn.ch]

\* Full proof validity check for a target key. Models value_digest:
\*   1. Hash chain: first node hash = rootHash; each non-terminal's
\*      child hash at the nibble toward targetKey = next node's hash
\*   2. Branch direction: non-terminal nodes follow targetKey's path
\*   3. Terminal: inclusion (key matches) or valid exclusion
\*
\* Returns TRUE if the proof is valid for targetKey against rootHash.
ProofValidForKey(proofNodes, targetKey, rootHash) ==
    IF proofNodes = <<>> THEN TRUE  \* Empty proof = valid exclusion
    ELSE
        \* Hash chain: first node must match rootHash.
        /\ ProofNodeHash(proofNodes[1]) = rootHash
        \* Hash chain + branch direction for non-terminal nodes.
        /\ \A i \in 1..(Len(proofNodes) - 1) :
            LET thisNode == proofNodes[i]
                nextNode == proofNodes[i + 1]
                thisKey  == thisNode.key
            IN  \* Node must be a strict prefix of targetKey.
                /\ Len(thisKey) < Len(targetKey)
                \* Nibble toward target key.
                /\ LET nibToward == targetKey[Len(thisKey) + 1]
                   IN  \* Child hash at that nibble must equal next node's hash.
                       thisNode.ch[nibToward] = ProofNodeHash(nextNode)
        \* Terminal node validity.
        /\ LET last == proofNodes[Len(proofNodes)]
               lastKey == last.key
               cpl == CommonPrefixLen(lastKey, targetKey)
           IN  \* Inclusion: key matches exactly.
               \/ lastKey = targetKey
               \* Exclusion: last node diverges from target.
               \/ cpl < Len(lastKey)
               \* Exclusion: last node is ancestor with no child
               \* at next nibble.
               \/ (cpl = Len(lastKey) /\ Len(targetKey) > cpl
                   /\ last.ch[targetKey[cpl + 1]] = None)

\* Is the proof an inclusion proof for the target key?
ProofIsInclusion(proofNodes, targetKey) ==
    /\ proofNodes # <<>>
    /\ proofNodes[Len(proofNodes)].key = targetKey
    /\ proofNodes[Len(proofNodes)].value # NoVal

\* End proof operation consistency check.
\* Models verify_change_proof_structure's EndProofOperationMismatch:
\*   - Put at lastOpKey expects inclusion (key exists in end_root)
\*   - Delete at lastOpKey expects exclusion (key absent)
\* Also models StartProofOperationMismatch for first op at startKey.
EndProofOpConsistent(diff, endProof, rightEdge) ==
    IF diff = {} THEN TRUE
    ELSE IF rightEdge = None THEN TRUE
    ELSE
        LET lastOp == CHOOSE d \in diff :
                \A d2 \in diff : SeqLte(d2.key, d.key)
            \* Empty proof = exclusion (Rust: Err(Empty) → None).
            \* Op consistency only applies when right_edge_key == last_op_key.
            isInclusion == ProofIsInclusion(endProof, rightEdge)
        IN  IF lastOp.key # rightEdge THEN TRUE
            ELSE (lastOp.op = "Put" => isInclusion)
                 /\ (lastOp.op = "Delete" => ~isInclusion)

StartProofOpConsistent(diff, startProof, startKey) ==
    IF diff = {} \/ startProof = <<>> \/ startKey = None THEN TRUE
    ELSE
        LET firstOp == CHOOSE d \in diff :
                \A d2 \in diff : SeqLte(d.key, d2.key)
        IN  IF firstOp.key # startKey THEN TRUE  \* Only check when first op is at startKey
            ELSE
                LET isInclusion == ProofIsInclusion(startProof, startKey)
                IN  (firstOp.op = "Put" => isInclusion)
                    /\ (firstOp.op = "Delete" => ~isInclusion)

--------------------------------------------------------------------------------
(* Section 2: Diff Computation *)

\* Compute the batch operations that transform startTrie into endTrie
\* within the range [startKey, endKey]. Returns a set of
\* [key, op, val] records where op is "Put" or "Delete".
\*
\* Corresponds to DiffMerkleNodeStream in merkle/changes.rs.

ComputeDiff(startTrie, endTrie, startKey, endKey) ==
    {[key |-> k,
      op  |-> IF endTrie[k] # NoVal THEN "Put" ELSE "Delete",
      val |-> endTrie[k]]
     : k \in {k \in AllKeys :
        /\ InRange(k, startKey, endKey)
        /\ startTrie[k] # endTrie[k]}}

\* Apply batch ops to a flat trie, producing the proposal.
ApplyDiff(startTrie, diff) ==
    [k \in AllKeys |->
        IF \E d \in diff : d.key = k
        THEN LET d == CHOOSE d \in diff : d.key = k
             IN  IF d.op = "Put" THEN d.val ELSE NoVal
        ELSE startTrie[k]]

\* Sorted keys from a diff (for determining lastOpKey).
DiffKeys(diff) == {d.key : d \in diff}

\* Last (lexicographically greatest) key in the diff.
LastDiffKey(diff) ==
    IF diff = {} THEN None
    ELSE CHOOSE k \in DiffKeys(diff) :
            \A k2 \in DiffKeys(diff) : SeqLte(k2, k)

--------------------------------------------------------------------------------
(* Section 3: Reconcile — Insert branch structure + resolve values *)

\* Phase 1: Ensure a branch node exists at the given nibble path,
\* splitting existing nodes as needed. Does NOT modify values —
\* existing values are preserved at split points.
\* Matches insert_branch_from_nibbles in merkle/mod.rs.

RECURSIVE InsertBranchAt(_, _)
InsertBranchAt(node, remKey) ==
    IF node = None THEN
        [pp |-> remKey, value |-> NoVal, children |-> EmptyChildren]
    ELSE
        LET pp  == node.pp
            cpl == CommonPrefixLen(remKey, pp)
        IN
            IF cpl = Len(pp) /\ cpl = Len(remKey) THEN
                node  \* Already at target — no structural change.
            ELSE IF cpl = Len(pp) THEN
                LET ci   == remKey[cpl + 1]
                    rest == SubSeq(remKey, cpl + 2, Len(remKey))
                IN  [node EXCEPT !.children[ci] =
                        InsertBranchAt(node.children[ci], rest)]
            ELSE IF cpl = Len(remKey) THEN
                \* Split: key exhausted before pp.
                LET origNib  == pp[cpl + 1]
                    origNode == [node EXCEPT
                        !.pp = SubSeq(pp, cpl + 2, Len(pp))]
                IN  [pp       |-> SubSeq(pp, 1, cpl),
                     value    |-> NoVal,
                     children |-> [n \in Nibbles |->
                        IF n = origNib THEN origNode ELSE None]]
            ELSE
                \* Split: divergence.
                LET origNib  == pp[cpl + 1]
                    origNode == [node EXCEPT
                        !.pp = SubSeq(pp, cpl + 2, Len(pp))]
                    newNib   == remKey[cpl + 1]
                    newBranch == InsertBranchAt(None,
                        SubSeq(remKey, cpl + 2, Len(remKey)))
                IN  [pp       |-> SubSeq(pp, 1, cpl),
                     value    |-> NoVal,
                     children |-> [n \in Nibbles |->
                        IF n = origNib THEN origNode
                        ELSE IF n = newNib THEN newBranch
                        ELSE None]]

\* Phase 2: Resolve value conflict at the proof node's target position
\* ONLY. Does not touch intermediate nodes — those are handled by the
\* collapse step later.
\* Matches reconcile_branch_proof_node + on_conflict in merkle/mod.rs.
\*
\* on_conflict logic: a proof node is in-range if its key >= startKey
\* (for start proofs) or <= endKey (for end proofs). This covers both
\* inclusion proofs (exact match) and exclusion proofs where the
\* terminal overshoots to the nearest existing key.
\*   - In-range: reject mismatch (keep trie value)
\*   - Out-of-range: adopt proof's value

ReconcileProofNode(ptrie, pn, boundaryKeyNibbles, isStartProof) ==
    LET withBranch == InsertBranchAt(ptrie, pn.key)
        curVal     == GetValueAt(withBranch, pn.key)
        \* Start proof: in-range if pn.key >= boundaryKey
        \* End proof: in-range if pn.key <= boundaryKey
        inRange == IF isStartProof
                   THEN SeqLte(boundaryKeyNibbles, pn.key)
                   ELSE SeqLte(pn.key, boundaryKeyNibbles)
    IN  IF pn.value = curVal THEN withBranch
        ELSE IF inRange THEN
            withBranch  \* In-range: keep trie value.
        ELSE
            \* Out-of-range: adopt proof's value (may be NoVal).
            SetValueAt(withBranch, pn.key, pn.value)

\* Fold reconciliation over a sequence of proof nodes.
RECURSIVE FoldReconcile(_, _, _, _, _)
FoldReconcile(ptrie, proofNodes, boundaryKey, isStartProof, idx) ==
    IF idx > Len(proofNodes) THEN ptrie
    ELSE FoldReconcile(
            ReconcileProofNode(ptrie, proofNodes[idx], boundaryKey,
                               isStartProof),
            proofNodes, boundaryKey, isStartProof, idx + 1)

--------------------------------------------------------------------------------
(* Section 4: Collapse — Strip intermediate branches between proof nodes *)

\* Between consecutive proof nodes, the end_root trie has a direct
\* path (no intermediate branches with extra children or values).
\* The proving trie may have extra structure from proposal keys that
\* are outside the range. Strip it to match end_root's shape.
\*
\* In-range children at intermediate positions indicate tampered
\* batch_ops and cause rejection (return None to signal failure).
\* Out-of-range children are stripped normally.
\*
\* Corresponds to collapse_branch_to_path / collapse_strip in
\* merkle/mod.rs.

\* Check if a child at nibble `nib` under accumulated prefix `accPrefix`
\* falls within [startNib, endNib]. Matches child_in_range in mod.rs.
ChildInRange(accPrefix, nib, startNib, endNib) ==
    LET depth == Len(accPrefix)
        \* Compare acc_prefix against start boundary prefix.
        startSplit == IF depth < Len(startNib) THEN depth ELSE Len(startNib)
        accStart == SubSeq(accPrefix, 1, startSplit)
        startPre == SubSeq(startNib, 1, startSplit)
        startRest == SubSeq(startNib, startSplit + 1, Len(startNib))
        aboveStart ==
            IF accStart # startPre THEN
                \* Compare lexicographically.
                SeqLte(startPre, accStart) /\ accStart # startPre
            ELSE
                \* Tied — compare child nibble against boundary.
                IF startRest = <<>> THEN TRUE
                ELSE nib >= startRest[1]
        \* Compare acc_prefix against end boundary prefix.
        endSplit == IF depth < Len(endNib) THEN depth ELSE Len(endNib)
        accEnd == SubSeq(accPrefix, 1, endSplit)
        endPre == SubSeq(endNib, 1, endSplit)
        endRest == SubSeq(endNib, endSplit + 1, Len(endNib))
        belowEnd ==
            IF accEnd # endPre THEN
                SeqLte(accEnd, endPre) /\ accEnd # endPre
            ELSE
                IF endRest = <<>> THEN FALSE
                ELSE nib <= endRest[1]
    IN  aboveStart /\ belowEnd

\* Strip non-on-path children from an intermediate branch.
\* `onPath` is the nibble path from this node to the next proof node.
\* `accPrefix` is the accumulated nibble prefix to this node.
\* `range` is None (no range check) or <<startNib, endNib>>.
\*
\* Returns None if an in-range child is found (tampered batch_ops).
\* Otherwise returns the stripped and flattened node.

RECURSIVE CollapseStripNode(_, _, _, _)
CollapseStripNode(node, onPath, accPrefix, range) ==
    IF node = None THEN None
    ELSE IF onPath = <<>> THEN node
    ELSE
        LET pp    == node.pp
            ppLen == Len(pp)
            afterPP == SubSeq(onPath, ppLen + 1, Len(onPath))
        IN
            IF afterPP = <<>> THEN node
            ELSE
                LET onNib == afterPP[1]
                    deeper == Tail(afterPP)
                    \* Check each non-on-path child.
                    \* If any is in-range, return None (rejection).
                    hasInRangeChild ==
                        range # None /\
                        \E n \in Nibbles :
                            /\ n # onNib
                            /\ node.children[n] # None
                            /\ ChildInRange(accPrefix, n,
                                   range[1], range[2])
                IN
                    IF hasInRangeChild THEN None  \* Rejection.
                    ELSE
                        \* Strip out-of-range non-on-path children.
                        LET stripped == [node EXCEPT
                                !.value = NoVal,
                                !.children = [n \in Nibbles |->
                                    IF n = onNib
                                    THEN node.children[n]
                                    ELSE None]]
                            child == stripped.children[onNib]
                            childPrefix == accPrefix \o <<onNib>>
                                \o (IF child # None THEN child.pp
                                    ELSE <<>>)
                            newChild == IF child = None THEN None
                                        ELSE
                                            LET childPPLen == Len(child.pp)
                                                childDeeper == SubSeq(deeper,
                                                    childPPLen + 1,
                                                    Len(deeper))
                                            IN  IF childDeeper = <<>>
                                                THEN child
                                                ELSE CollapseStripNode(
                                                    child, deeper,
                                                    childPrefix, range)
                            updatedNode == [stripped EXCEPT
                                !.children[onNib] = newChild]
                        IN
                            \* Check for rejection from recursive call.
                            IF newChild = None /\ child # None
                            THEN None  \* Propagate rejection.
                            ELSE
                                \* Flatten: single child, no value.
                                LET numCh == Cardinality(
                                    {n \in Nibbles :
                                        updatedNode.children[n] # None})
                                IN  IF updatedNode.value = NoVal
                                       /\ numCh = 1 THEN
                                        LET cn == CHOOSE n \in Nibbles :
                                            updatedNode.children[n] # None
                                            ch == updatedNode.children[cn]
                                        IN  [ch EXCEPT !.pp =
                                            updatedNode.pp \o <<cn>>
                                            \o ch.pp]
                                    ELSE updatedNode

\* Navigate to parentKey, then collapse toward childKey.
RECURSIVE CollapseNavigate(_, _, _, _, _)
CollapseNavigate(node, parentKey, suffix, accPrefix, range) ==
    IF node = None THEN None
    ELSE
        LET pp  == node.pp
            cpl == CommonPrefixLen(parentKey, pp)
        IN
            IF cpl < Len(pp) THEN
                node
            ELSE IF SubSeq(parentKey, cpl + 1, Len(parentKey)) = <<>> THEN
                IF suffix = <<>> THEN node
                ELSE
                    LET firstNib == suffix[1]
                        rest     == Tail(suffix)
                        child    == node.children[firstNib]
                        childPrefix == accPrefix \o <<firstNib>>
                            \o (IF child # None THEN child.pp
                                ELSE <<>>)
                        newChild == IF child = None THEN None
                                    ELSE
                                        LET cpLen == Len(child.pp)
                                            afterCP == SubSeq(rest,
                                                cpLen + 1, Len(rest))
                                        IN  IF afterCP = <<>> THEN child
                                            ELSE CollapseStripNode(
                                                child, rest,
                                                childPrefix, range)
                    IN  IF newChild = None /\ child # None
                        THEN None  \* Propagate rejection.
                        ELSE [node EXCEPT !.children[firstNib] = newChild]
            ELSE
                LET rem == SubSeq(parentKey, cpl + 1, Len(parentKey))
                    ci  == rem[1]
                    rest == Tail(rem)
                    newChild == CollapseNavigate(
                        node.children[ci], rest, suffix,
                        accPrefix, range)
                IN  IF newChild = None /\ node.children[ci] # None
                    THEN None
                    ELSE [node EXCEPT !.children[ci] = newChild]

CollapseBetween(ptrie, parentKey, childKey, range) ==
    IF ptrie = None THEN None
    ELSE
        LET suffix == SubSeq(childKey, Len(parentKey) + 1,
                             Len(childKey))
            accPrefix == parentKey
        IN  CollapseNavigate(ptrie, parentKey, suffix, accPrefix, range)

\* Collapse root to match end_root's shape. Passes range to
\* CollapseStripNode so in-range children trigger rejection.
CollapseRootToPath(ptrie, targetKey, range) ==
    IF ptrie = None THEN None
    ELSE
        LET pp == ptrie.pp
            ppLen == Len(pp)
        IN
            IF Len(targetKey) <= ppLen THEN ptrie
            ELSE IF SubSeq(targetKey, 1, ppLen) # pp THEN ptrie
            ELSE
                LET remaining == SubSeq(targetKey, ppLen + 1,
                                        Len(targetKey))
                    rootPrefix == pp
                IN  CollapseStripNode(ptrie, remaining,
                                      rootPrefix, range)

\* Fold collapse over consecutive pairs of proof nodes.
\* Returns None if any collapse rejects (in-range child found).
RECURSIVE FoldCollapse(_, _, _, _)
FoldCollapse(ptrie, proofNodes, range, idx) ==
    IF ptrie = None THEN None  \* Propagate rejection.
    ELSE IF idx >= Len(proofNodes) THEN ptrie
    ELSE FoldCollapse(
            CollapseBetween(ptrie,
                proofNodes[idx].key, proofNodes[idx + 1].key, range),
            proofNodes, range, idx + 1)

--------------------------------------------------------------------------------
(* Section 5: Compute Outside Children *)

\* For each proof node, determine which children are outside the
\* proven range and should use the proof's child hashes.
\*
\* Corresponds to compute_outside_children in merkle/mod.rs:196.

\* Terminal proof node: boundary key determines outside children.
OutsideMaskTerminal(termKey, boundaryKey, isLeftEdge) ==
    IF boundaryKey = None THEN {}
    ELSE
        LET cpl == CommonPrefixLen(termKey, boundaryKey)
        IN
            IF cpl < Len(termKey) /\ cpl < Len(boundaryKey) THEN
                \* Boundary diverges within terminal's key.
                LET tk == termKey[cpl + 1]
                    bn == boundaryKey[cpl + 1]
                    allOut == IF isLeftEdge THEN bn > tk ELSE bn < tk
                IN  IF allOut THEN Nibbles ELSE {}

            ELSE IF cpl = Len(termKey) /\ cpl < Len(boundaryKey) THEN
                \* Terminal is ancestor of boundary.
                LET onNib == boundaryKey[Len(termKey) + 1]
                IN  (IF isLeftEdge
                     THEN {n \in Nibbles : n < onNib}
                     ELSE {n \in Nibbles : n > onNib})
                    \cup {onNib}  \* On-path nibble itself is outside.

            ELSE IF ~isLeftEdge THEN
                \* Boundary is prefix of / matches terminal (right edge).
                Nibbles  \* ALL children outside.
            ELSE
                {}  \* Left edge, boundary matches — no children outside.

\* Compute outside children for a proof path.
\* Returns a set of <<proofNodeKey, outsideNibbleSet>> pairs.
ComputeOutsideChildren(proofNodes, boundaryKey, isLeftEdge) ==
    LET numNodes == Len(proofNodes)
    IN  {<<proofNodes[i].key,
           IF i < numNodes THEN
               \* Non-terminal: next node determines on-path nibble.
               LET thisKey == proofNodes[i].key
                   nextKey == proofNodes[i + 1].key
                   onNib   == nextKey[Len(thisKey) + 1]
               IN  IF isLeftEdge
                   THEN {n \in Nibbles : n < onNib}
                   ELSE {n \in Nibbles : n > onNib}
           ELSE
               OutsideMaskTerminal(proofNodes[i].key,
                                   boundaryKey, isLeftEdge)
         >> : i \in 1..numNodes}

\* Merge two outside-children maps by unioning nibble sets per key.
MergeOutside(a, b) ==
    LET allKeys == {p[1] : p \in a} \cup {p[1] : p \in b}
        get(m, k) == IF \E p \in m : p[1] = k
                     THEN (CHOOSE p \in m : p[1] = k)[2]
                     ELSE {}
    IN  {<<k, get(a, k) \cup get(b, k)>> : k \in allKeys}

\* Look up the outside nibbles for a given key in the map.
GetOutsideNibbles(outsideMap, key) ==
    IF \E p \in outsideMap : p[1] = key
    THEN (CHOOSE p \in outsideMap : p[1] = key)[2]
    ELSE {}

--------------------------------------------------------------------------------
(* Section 6: Hybrid Hash *)

\* Walk the proving trie. For in-range children, hash recursively.
\* For outside children, use the proof node's child hash.
\*
\* Corresponds to compute_root_hash_with_proofs in merkle/mod.rs:280.

\* Build a lookup from proof node key -> proof node.
ProofNodeMap(startProof, endProof) ==
    {<<pn.key, pn>> : pn \in
        UNION {{startProof[i] : i \in 1..Len(startProof)},
               {endProof[i]   : i \in 1..Len(endProof)}}}

GetProofNode(pnMap, key) ==
    IF \E p \in pnMap : p[1] = key
    THEN (CHOOSE p \in pnMap : p[1] = key)[2]
    ELSE None

RECURSIVE HybridHashHelper(_, _, _, _)
HybridHashHelper(node, prefix, pnMap, outsideMap) ==
    IF node = None THEN None
    ELSE
        LET fullPath == prefix \o node.pp
            outsideNibs == GetOutsideNibbles(outsideMap, fullPath)
            pn == GetProofNode(pnMap, fullPath)
        IN
            [fp |-> fullPath,
             v  |-> node.value,
             ch |-> [n \in Nibbles |->
                IF n \in outsideNibs /\ pn # None THEN
                    \* Outside: use proof node's child hash.
                    pn.ch[n]
                ELSE
                    \* Inside: hash from the proving trie.
                    HybridHashHelper(node.children[n],
                        Append(fullPath, n), pnMap, outsideMap)]]

HybridHash(ptrie, pnMap, outsideMap) ==
    IF ptrie = None THEN None
    ELSE HybridHashHelper(ptrie, <<>>, pnMap, outsideMap)

--------------------------------------------------------------------------------
(* Section 7: Full Verification Pipeline *)

\* Convert keys to nibble sequences for proof operations.
KeyToNibbles(key) == key  \* In our model, keys are already nibbles.

\* Right edge of the proven range. For non-truncated proofs (all our
\* model tests), prefer end_key, fall back to last_op_key.
\* Matches compute_right_edge_key in proofs/change.rs.
RightEdgeKey(diff, endKey) ==
    IF endKey # None THEN endKey
    ELSE LastDiffKey(diff)

\* Run the full change proof verification pipeline.
\* Returns TRUE if the proof is accepted (hashes match).
\*
\* Corresponds to verify_change_proof_root_hash in merkle/mod.rs:583.

VerifyChangeProof(startTrie, endTrie, startKey, endKey) ==
    LET endCtrie == Compress(endTrie)
        endRoot  == IF endCtrie = None THEN None
                    ELSE CompressedHash(endCtrie, <<>>)

        \* Step 1: Compute diff (batch_ops).
        diff == ComputeDiff(startTrie, endTrie, startKey, endKey)

        \* Step 2: Build proposal (apply diff to start).
        proposalFlat == ApplyDiff(startTrie, diff)
        proposal     == Compress(proposalFlat)

        rightEdge == RightEdgeKey(diff, endKey)

        \* Step 3: Extract proofs from end trie.
        startProof == IF startKey = None THEN <<>>
                      ELSE ExtractProof(endCtrie, startKey)
        endProof   == IF rightEdge = None THEN <<>>
                      ELSE ExtractProof(endCtrie, rightEdge)

        \* Nibble-level range for collapse range checks.
        startKeyNibs == IF startKey = None THEN <<>> ELSE startKey
        endKeyNibs   == IF rightEdge = None THEN <<>> ELSE rightEdge
        range == IF startKey = None /\ rightEdge = None THEN None
                 ELSE <<startKeyNibs, endKeyNibs>>

    IN
        \* Case 1: Both proofs empty — direct hash comparison.
        IF startProof = <<>> /\ endProof = <<>> THEN
            LET proposalHash == IF proposal = None THEN None
                                ELSE CompressedHash(proposal, <<>>)
            IN  proposalHash = endRoot

        ELSE
            \* Step 4: Reconcile proof nodes (>= / <= for in-range).
            LET pt1 == FoldReconcile(proposal, startProof,
                            startKeyNibs, TRUE, 1)
                pt2 == FoldReconcile(pt1, endProof,
                            endKeyNibs, FALSE, 1)

                \* Step 5: Collapse between consecutive proof nodes.
                \* In-range children trigger rejection (None).
                pt3 == FoldCollapse(pt2, startProof, range, 1)
                pt4 == FoldCollapse(pt3, endProof, range, 1)

                \* Step 5b: Collapse root to match end_root shape.
                firstProofKey ==
                    IF startProof # <<>> THEN startProof[1].key
                    ELSE IF endProof # <<>> THEN endProof[1].key
                    ELSE <<>>
                pt5 == CollapseRootToPath(pt4, firstProofKey, range)

            IN
                \* Collapse rejection propagates as None.
                IF pt5 = None THEN FALSE
                ELSE
                    \* Step 6: Compute outside children.
                    LET startOutside == ComputeOutsideChildren(
                            startProof, startKey, TRUE)
                        endOutside   == ComputeOutsideChildren(
                            endProof, rightEdge, FALSE)
                        allOutside   == MergeOutside(
                            startOutside, endOutside)

                        \* Step 7: Build proof node map.
                        pnMap == ProofNodeMap(startProof, endProof)

                        \* Step 8: Compute hybrid hash.
                        computed == HybridHash(pt5, pnMap, allOutside)

                    \* Step 9: Compare with end_root.
                    IN  computed = endRoot

--------------------------------------------------------------------------------
(* Section 8: Adversarial Verification *)

\* Variant of VerifyChangeProof that accepts an externally-supplied diff
\* (possibly tampered by an adversary) instead of computing the honest one.
\* The proofs are still extracted honestly from endTrie — the attacker can
\* only tamper with batch_ops.

VerifyWithDiff(st, et, sk, ek, tamperedDiff) ==
    LET endCtrie == Compress(et)
        endRoot  == IF endCtrie = None THEN None
                    ELSE CompressedHash(endCtrie, <<>>)

        \* Honest proofs (extracted from end trie using right_edge_key).
        honestDiff      == ComputeDiff(st, et, sk, ek)
        honestRightEdge == RightEdgeKey(honestDiff, ek)
        startProof      == IF sk = None THEN <<>>
                           ELSE ExtractProof(endCtrie, sk)
        endProof        == IF honestRightEdge = None THEN <<>>
                           ELSE ExtractProof(endCtrie, honestRightEdge)

        \* Proposal built from tampered diff.
        proposalFlat == ApplyDiff(st, tamperedDiff)
        proposal     == Compress(proposalFlat)

        \* The key that the structural check verifies the end proof
        \* against: for non-truncated proofs, end_key or last_op_key.
        tamperedRightEdge == RightEdgeKey(tamperedDiff, ek)

        \* Range for reconcile and collapse — uses tamperedRightEdge,
        \* matching the Rust code where the verification context's
        \* right_edge_key comes from the received batch_ops.
        \* The hash chain check in ProofValidForKey ensures the proof
        \* is compatible with tamperedRightEdge before reaching here.
        skNibs == IF sk = None THEN <<>> ELSE sk
        ekNibs == IF tamperedRightEdge = None THEN <<>> ELSE tamperedRightEdge
        range  == IF sk = None /\ tamperedRightEdge = None THEN None
                  ELSE <<skNibs, ekNibs>>

    IN
        \* ── Structural checks (model verify_change_proof_structure) ──

        \* 1. Full value_digest check: hash chain + branch direction + terminal.
        /\ (tamperedRightEdge # None
            => ProofValidForKey(endProof, tamperedRightEdge, endRoot))
        /\ (sk # None
            => ProofValidForKey(startProof, sk, endRoot))

        \* 2. Op consistency: Put expects inclusion, Delete expects exclusion.
        /\ EndProofOpConsistent(tamperedDiff, endProof, tamperedRightEdge)
        /\ StartProofOpConsistent(tamperedDiff, startProof, sk)

        \* ── Root hash check (models verify_change_proof_root_hash) ──
        /\ IF startProof = <<>> /\ endProof = <<>> THEN
                LET proposalHash == IF proposal = None THEN None
                                    ELSE CompressedHash(proposal, <<>>)
                IN  proposalHash = endRoot
           ELSE
                LET pt1 == FoldReconcile(proposal, startProof,
                                skNibs, TRUE, 1)
                    pt2 == FoldReconcile(pt1, endProof,
                                ekNibs, FALSE, 1)
                    pt3 == FoldCollapse(pt2, startProof, range, 1)
                    pt4 == FoldCollapse(pt3, endProof, range, 1)

                    firstProofKey ==
                        IF startProof # <<>> THEN startProof[1].key
                        ELSE IF endProof # <<>> THEN endProof[1].key
                        ELSE <<>>
                    pt5 == CollapseRootToPath(pt4, firstProofKey, range)
                IN
                    IF pt5 = None THEN FALSE
                    ELSE
                        LET startOutside == ComputeOutsideChildren(
                                startProof, sk, TRUE)
                            endOutside   == ComputeOutsideChildren(
                                endProof, tamperedRightEdge, FALSE)
                            allOutside   == MergeOutside(
                                startOutside, endOutside)
                            pnMap    == ProofNodeMap(startProof, endProof)
                            computed == HybridHash(pt5, pnMap, allOutside)
                        IN  computed = endRoot

\* Generate all single-key tamperings of the honest diff for a given range.
\* For each in-range key NOT in the honest diff:
\*   - Try adding a spurious Delete
\*   - Try adding a spurious Put with each value
\* Returns a set of tampered diffs.
SpuriousOps(st, et, sk, ek) ==
    LET honestDiff == ComputeDiff(st, et, sk, ek)
        honestKeys == DiffKeys(honestDiff)
        inRangeKeys == {k \in AllKeys : InRange(k, sk, ek)}
        unchangedKeys == inRangeKeys \ honestKeys
        \* Keys that exist in start trie and are unchanged.
        deletable == {k \in unchangedKeys : st[k] # NoVal}
        \* (key, value) pairs for spurious puts.
        puttable == {<<k, v>> \in unchangedKeys \X Values : v # et[k]}
    IN
        \* Spurious deletes.
        {honestDiff \cup {[key |-> k, op |-> "Delete", val |-> NoVal]}
         : k \in deletable}
        \cup
        \* Spurious puts.
        {honestDiff \cup {[key |-> kv[1], op |-> "Put", val |-> kv[2]]}
         : kv \in puttable}

--------------------------------------------------------------------------------
(* Section 9: State Machine *)

VARIABLES startTrie, endTrie, startKey, endKey

vars == <<startTrie, endTrie, startKey, endKey>>

\* All valid range bounds: any key or None (unbounded).
AllBounds == AllKeys \cup {None}

\* Valid range: startKey <= endKey when both present.
ValidRange(sk, ek) ==
    \/ sk = None
    \/ ek = None
    \/ SeqLte(sk, ek)

Init ==
    /\ startTrie \in AllTries
    /\ endTrie \in AllTries
    /\ startKey \in AllBounds
    /\ endKey \in AllBounds
    /\ ValidRange(startKey, endKey)

\* Simulation mode: pick random state without enumerating the full
\* Cartesian product.
SimInit ==
    /\ startTrie = RandomElement(AllTries)
    /\ endTrie = RandomElement(AllTries)
    /\ startKey = RandomElement(AllBounds)
    /\ endKey = RandomElement(AllBounds)

\* Biased simulation mode for root structure bug: force endTrie to
\* have all keys at a single first nibble (triggering root compression).
RootBugInit ==
    /\ startTrie = RandomElement(AllTries)
    /\ endKey = RandomElement(AllBounds)
    /\ startKey = RandomElement(AllBounds)
    /\ LET nib == RandomElement(Nibbles)
       IN  endTrie = RandomElement(
            {t \in AllTries :
                \A k \in AllKeys : t[k] # NoVal => k[1] = nib})

Next == UNCHANGED vars

\* P1: Honest proofs are always accepted (for valid ranges).
HonestProofAccepted ==
    ValidRange(startKey, endKey) =>
        VerifyChangeProof(startTrie, endTrie, startKey, endKey)

=============================================================================
