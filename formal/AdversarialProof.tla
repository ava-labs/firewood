--------------------------- MODULE AdversarialProof ---------------------------
(*
 * Exhaustive adversarial testing of change proof verification.
 *
 * For each (startTrie, endTrie, startKey, endKey), enumerates ALL
 * possible diffs (not just the honest one) and checks that only
 * correct diffs pass verification. Any incorrect diff that passes
 * is a vulnerability.
 *
 * This spec EXTENDS ChangeProofVerification to reuse the full pipeline
 * and inherits its state variables (startTrie, endTrie, startKey, endKey).
 *)

EXTENDS ChangeProofVerification

\* All possible diffs for a given range. Each in-range key can be:
\*   - Absent from diff (unchanged)
\*   - Put with any value in Values
\*   - Delete
\* Returns the set of all possible diffs.
AllPossibleDiffs(sk, ek) ==
    LET inRangeKeys == {k \in AllKeys : InRange(k, sk, ek)}
        opsForKey(k) ==
            {[key |-> k, op |-> "Put", val |-> v] : v \in Values}
            \cup {[key |-> k, op |-> "Delete", val |-> NoVal]}
    IN  {d \in SUBSET UNION {opsForKey(k) : k \in inRangeKeys} :
            \A k \in inRangeKeys :
                Cardinality({op \in d : op.key = k}) <= 1}

\* The security property: if verification accepts a diff, the resulting
\* proposal must agree with endTrie on every in-range key.
OnlyCorrectDiffAccepted ==
    ValidRange(startKey, endKey) =>
        LET sk == startKey
            ek == endKey
            proposalFlat(diff) == ApplyDiff(startTrie, diff)
        IN  \A diff \in AllPossibleDiffs(sk, ek) :
                VerifyWithDiff(startTrie, endTrie, sk, ek, diff)
                => \A k \in AllKeys :
                    InRange(k, sk, ek) =>
                        proposalFlat(diff)[k] = endTrie[k]

\* Single-key tampering check: for each in-range key not in the honest
\* diff, try adding one spurious Delete or Put and verify rejection.
\* Lighter than OnlyCorrectDiffAccepted (linear vs exponential in keys),
\* enabling adversarial checks at larger MaxDepth.
AdversarialProofRejected ==
    ValidRange(startKey, endKey) =>
        \A tampered \in SpuriousOps(startTrie, endTrie, startKey, endKey) :
            ~VerifyWithDiff(startTrie, endTrie, startKey, endKey, tampered)

=============================================================================
