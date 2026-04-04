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

\* Is the proof an inclusion proof for the target key?
\* Inclusion: last proof node is at the target key and has a value.
\* Exclusion: last node diverges or has no value.
ProofIsInclusion(proofNodes, targetKey) ==
    /\ proofNodes # <<>>
    /\ proofNodes[Len(proofNodes)].key = targetKey
    /\ proofNodes[Len(proofNodes)].value # NoVal

\* End proof operation consistency check.
\* Models verify_change_proof_structure's EndProofOperationMismatch:
\*   - Put at lastOpKey expects inclusion (key exists in end_root)
\*   - Delete at lastOpKey expects exclusion (key absent)
\* Also models StartProofOperationMismatch for first op at startKey.
EndProofOpConsistent(diff, endProof, effectiveEnd) ==
    IF diff = {} \/ endProof = <<>> THEN TRUE
    ELSE
        LET lastOp == CHOOSE d \in diff :
                \A d2 \in diff : SeqLte(d2.key, d.key)
            isInclusion == ProofIsInclusion(endProof, effectiveEnd)
        IN  (lastOp.op = "Put" => isInclusion)
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
\* Matches reconcile_branch_proof_node + on_conflict in merkle/mod.rs:2024.
\*
\* on_conflict logic (from verify_change_proof_root_hash:637-681):
\*   - If node is at the exact boundary key: reject mismatch (in-range)
\*   - Otherwise: adopt proof's value (out-of-range ancestor)

ReconcileProofNode(ptrie, pn, boundaryKeyNibbles) ==
    LET withBranch == InsertBranchAt(ptrie, pn.key)
        curVal     == GetValueAt(withBranch, pn.key)
    IN  IF pn.value = curVal THEN withBranch
        ELSE IF pn.key = boundaryKeyNibbles THEN
            withBranch  \* In-range: keep trie value.
        ELSE
            \* Out-of-range: adopt proof's value (may be NoVal).
            SetValueAt(withBranch, pn.key, pn.value)

\* Fold reconciliation over a sequence of proof nodes.
RECURSIVE FoldReconcile(_, _, _, _)
FoldReconcile(ptrie, proofNodes, boundaryKey, idx) ==
    IF idx > Len(proofNodes) THEN ptrie
    ELSE FoldReconcile(
            ReconcileProofNode(ptrie, proofNodes[idx], boundaryKey),
            proofNodes, boundaryKey, idx + 1)

--------------------------------------------------------------------------------
(* Section 4: Collapse — Strip intermediate branches between proof nodes *)

\* Between consecutive proof nodes, the end_root trie has a direct
\* path (no intermediate branches with extra children or values).
\* The proving trie may have extra structure from proposal keys that
\* are outside the range. Strip it to match end_root's shape.
\*
\* Corresponds to collapse_branch_to_path / collapse_strip in
\* merkle/mod.rs:1841-2001.

\* Strip non-on-path children and values from intermediate branches
\* along `onPath`, then flatten single-child branches.
\* `onPath` is the nibble sequence from the current node to the
\* next proof node (excluding the current node's partial path).

RECURSIVE CollapseStripNode(_, _)
CollapseStripNode(node, onPath) ==
    IF node = None THEN None
    ELSE IF onPath = <<>> THEN node  \* Reached target — stop.
    ELSE
        LET pp    == node.pp
            ppLen == Len(pp)
            \* Consume partial path from onPath.
            afterPP == SubSeq(onPath, ppLen + 1, Len(onPath))
        IN
            IF afterPP = <<>> THEN node  \* Path consumed within pp.
            ELSE
                LET onNib == afterPP[1]
                    deeper == Tail(afterPP)
                    branch == node
                IN
                    \* Strip: remove non-on-path children and value.
                    LET stripped == [branch EXCEPT
                            !.value = NoVal,
                            !.children = [n \in Nibbles |->
                                IF n = onNib
                                THEN branch.children[n]
                                ELSE None]]
                        \* Recurse into on-path child.
                        child == stripped.children[onNib]
                        newChild == IF child = None THEN None
                                    ELSE
                                        LET childPPLen == Len(child.pp)
                                            childDeeper == SubSeq(deeper,
                                                childPPLen + 1, Len(deeper))
                                        IN  IF childDeeper = <<>> THEN child
                                            ELSE CollapseStripNode(child,
                                                     deeper)
                        updatedNode == [stripped EXCEPT
                            !.children[onNib] = newChild]
                    IN
                        \* Flatten: if no value and single child, merge pp.
                        LET numCh == Cardinality(
                                {n \in Nibbles :
                                    updatedNode.children[n] # None})
                        IN  IF updatedNode.value = NoVal /\ numCh = 1 THEN
                                LET cn == CHOOSE n \in Nibbles :
                                        updatedNode.children[n] # None
                                    ch == updatedNode.children[cn]
                                IN  [ch EXCEPT !.pp =
                                        updatedNode.pp \o <<cn>> \o ch.pp]
                            ELSE updatedNode

\* Collapse between two consecutive proof nodes.
\* Navigate from root to parentKey, then call CollapseStripNode on
\* the on-path child with the suffix leading to childKey.

RECURSIVE CollapseNavigate(_, _, _)
CollapseNavigate(node, parentKey, suffix) ==
    IF node = None THEN None
    ELSE
        LET pp  == node.pp
            cpl == CommonPrefixLen(parentKey, pp)
        IN
            IF cpl < Len(pp) THEN
                node  \* Parent not reachable here.
            ELSE IF SubSeq(parentKey, cpl + 1, Len(parentKey)) = <<>> THEN
                \* Arrived at parent — strip the on-path child.
                IF suffix = <<>> THEN node
                ELSE
                    LET firstNib == suffix[1]
                        rest     == Tail(suffix)
                        child    == node.children[firstNib]
                        newChild == IF child = None THEN None
                                    ELSE
                                        LET cpLen == Len(child.pp)
                                            afterCP == SubSeq(rest,
                                                cpLen + 1, Len(rest))
                                        IN  IF afterCP = <<>> THEN child
                                            ELSE CollapseStripNode(child,
                                                     rest)
                    IN  [node EXCEPT !.children[firstNib] = newChild]
            ELSE
                \* Descend toward parent.
                LET rem == SubSeq(parentKey, cpl + 1, Len(parentKey))
                    ci  == rem[1]
                    rest == Tail(rem)
                    newChild == CollapseNavigate(
                        node.children[ci], rest, suffix)
                IN  [node EXCEPT !.children[ci] = newChild]

CollapseBetween(ptrie, parentKey, childKey) ==
    IF ptrie = None THEN None
    ELSE
        LET suffix == SubSeq(childKey, Len(parentKey) + 1,
                             Len(childKey))
        IN  CollapseNavigate(ptrie, parentKey, suffix)

\* Fold collapse over consecutive pairs of proof nodes.
RECURSIVE FoldCollapse(_, _, _)
FoldCollapse(ptrie, proofNodes, idx) ==
    IF idx >= Len(proofNodes) THEN ptrie
    ELSE FoldCollapse(
            CollapseBetween(ptrie,
                proofNodes[idx].key, proofNodes[idx + 1].key),
            proofNodes, idx + 1)

\* Collapse the proving trie's root when end_root's root has a longer
\* partial path (due to out-of-range deletions compressing the root).
\* Strips non-on-path children and flattens single-child branches so
\* the root shape matches end_root.
\* Matches collapse_root_to_path in merkle/mod.rs.
CollapseRootToPath(ptrie, targetKey) ==
    IF ptrie = None THEN None
    ELSE
        LET ppLen == Len(ptrie.pp)
            remaining == SubSeq(targetKey, ppLen + 1, Len(targetKey))
        IN  IF remaining = <<>> THEN ptrie  \* Root already at target depth.
            ELSE IF ppLen > 0 /\ SubSeq(targetKey, 1, ppLen) # ptrie.pp
                 THEN ptrie  \* Root pp doesn't match target prefix.
                 ELSE CollapseStripNode(ptrie, remaining)

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

\* The right edge of the proven range. For non-truncated proofs
\* (which is all our model tests — no max_length parameter), the
\* right edge is end_key when present, falling back to last_op_key.
\*
\* Previously this was "effective_end" which unconditionally used
\* last_op_key, creating a gap between last_op_key and end_key
\* where borrowed subtree hashes hid tampered keys (Bug 2).
\*
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

        \* Right edge of proven range (replaces old effective_end).
        rightEdge == RightEdgeKey(diff, endKey)

        \* Step 3: Extract proofs from end trie.
        startProof == IF startKey = None THEN <<>>
                      ELSE ExtractProof(endCtrie, startKey)
        endProof   == IF rightEdge = None THEN <<>>
                      ELSE ExtractProof(endCtrie, rightEdge)

    IN
        \* Case 1: Both proofs empty — direct hash comparison.
        IF startProof = <<>> /\ endProof = <<>> THEN
            LET proposalHash == IF proposal = None THEN None
                                ELSE CompressedHash(proposal, <<>>)
            IN  proposalHash = endRoot

        ELSE
            \* Step 4: Fork proposal into proving trie and reconcile.
            LET pt1 == FoldReconcile(proposal, startProof,
                            startKey, 1)
                pt2 == FoldReconcile(pt1, endProof,
                            rightEdge, 1)

                \* Step 5: Collapse between consecutive proof nodes.
                pt3 == FoldCollapse(pt2, startProof, 1)
                pt4 == FoldCollapse(pt3, endProof, 1)

                \* Step 5b: Collapse root to match end_root's shape.
                \* When out-of-range deletions compress end_root's root,
                \* the proposal root retains the old shape. Strip
                \* non-on-path children from the root so it matches.
                firstProofKey ==
                    IF startProof # <<>> THEN startProof[1].key
                    ELSE IF endProof # <<>> THEN endProof[1].key
                    ELSE <<>>
                pt5 == CollapseRootToPath(pt4, firstProofKey)

                \* Step 6: Compute outside children.
                startOutside == ComputeOutsideChildren(
                    startProof, startKey, TRUE)
                endOutside   == ComputeOutsideChildren(
                    endProof, rightEdge, FALSE)
                allOutside   == MergeOutside(startOutside, endOutside)

                \* Step 7: Build proof node map and compute hybrid hash.
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
        honestDiff     == ComputeDiff(st, et, sk, ek)
        honestRightEdge == RightEdgeKey(honestDiff, ek)
        startProof     == IF sk = None THEN <<>>
                          ELSE ExtractProof(endCtrie, sk)
        endProof       == IF honestRightEdge = None THEN <<>>
                          ELSE ExtractProof(endCtrie, honestRightEdge)

        \* Proposal built from tampered diff.
        proposalFlat == ApplyDiff(st, tamperedDiff)
        proposal     == Compress(proposalFlat)

        \* The key that the structural check verifies the end proof
        \* against: for non-truncated proofs, end_key or last_op_key.
        tamperedRightEdge == RightEdgeKey(tamperedDiff, ek)

    IN
        \* ── Structural checks (model verify_change_proof_structure) ──

        \* 1. Proof must follow correct branch toward target key.
        /\ (endProof # <<>> /\ tamperedRightEdge # None
            => ProofFollowsKey(endProof, tamperedRightEdge))
        /\ (startProof # <<>> /\ sk # None
            => ProofFollowsKey(startProof, sk))

        \* 2. Op consistency: Put expects inclusion, Delete expects exclusion.
        /\ EndProofOpConsistent(tamperedDiff, endProof, tamperedRightEdge)
        /\ StartProofOpConsistent(tamperedDiff, startProof, sk)

        \* ── Root hash check (models verify_change_proof_root_hash) ──
        /\ IF startProof = <<>> /\ endProof = <<>> THEN
                LET proposalHash == IF proposal = None THEN None
                                    ELSE CompressedHash(proposal, <<>>)
                IN  proposalHash = endRoot
           ELSE
                LET pt1 == FoldReconcile(proposal, startProof, sk, 1)
                    pt2 == FoldReconcile(pt1, endProof,
                                honestRightEdge, 1)
                    pt3 == FoldCollapse(pt2, startProof, 1)
                    pt4 == FoldCollapse(pt3, endProof, 1)

                    firstProofKey ==
                        IF startProof # <<>> THEN startProof[1].key
                        ELSE IF endProof # <<>> THEN endProof[1].key
                        ELSE <<>>
                    pt5 == CollapseRootToPath(pt4, firstProofKey)

                    startOutside == ComputeOutsideChildren(
                        startProof, sk, TRUE)
                    endOutside   == ComputeOutsideChildren(
                        endProof, honestRightEdge, FALSE)
                    allOutside   == MergeOutside(startOutside, endOutside)

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

\* P2: Adversarial proofs (single spurious op) are always rejected.
AdversarialProofRejected ==
    \A tampered \in SpuriousOps(startTrie, endTrie, startKey, endKey) :
        ~VerifyWithDiff(startTrie, endTrie, startKey, endKey, tampered)

=============================================================================
