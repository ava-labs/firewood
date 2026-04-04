------------------------ MODULE RegressionPrefixDelete ------------------------
(*
 * Regression test for the prefix-key-deletion bug.
 *
 * Bug: Change proof verification produced EndRootMismatch when a key
 * that was a prefix of other keys was deleted between start_root and
 * end_root, and that deleted prefix key fell outside the query range.
 *
 * Example: root1 has value at key \xab (prefix of \xab\xcd, \xab\xef).
 * root2 deletes \xab. Query range starts after \xab (e.g., \xab\x00).
 * The start proof passes through the branch at [a,b] where root1 had
 * a value but root2 does not. The old code didn't handle removing an
 * out-of-range ancestor's value during reconciliation.
 *
 * Fix: reconcile_branch_proof_node with on_conflict callback adopts
 * the proof node's value for out-of-range ancestors, correctly handling
 * the case where the proof node has no value (key deleted in end_root).
 *
 * This spec verifies the fix holds for ALL compressed trie configurations
 * where a prefix key is deleted and the query range excludes it.
 *
 * Corresponds to: test_change_proof_prefix_key_deleted_in_end_root
 *)

EXTENDS ChangeProofVerification

\* Does key `a` start with prefix `p`?
IsPrefix(p, a) == Len(a) >= Len(p) /\ SubSeq(a, 1, Len(p)) = p

\* A key is a "prefix key" if some other key in the trie extends it.
IsPrefixKey(trie, k) ==
    /\ trie[k] # NoVal
    /\ \E k2 \in AllKeys : k2 # k /\ IsPrefix(k, k2) /\ trie[k2] # NoVal

\* The regression scenario: startTrie has a prefix key that is deleted
\* in endTrie, and the query range excludes that prefix key but includes
\* keys that extend it.
\*
\* For each such configuration, honest proof verification must succeed.
PrefixDeleteVerifies ==
    \A sk \in AllKeys :
        \A prefixKey \in AllKeys :
            \* prefixKey is a strict prefix of some key in startTrie
            (IsPrefixKey(startTrie, prefixKey)
            \* prefixKey is deleted in endTrie
            /\ endTrie[prefixKey] = NoVal
            \* endTrie still has MULTIPLE keys extending the prefix,
            \* so the branch node at this path persists (just without a value).
            \* This matches the Rust test where \xab\xcd and \xab\xef both
            \* survive, keeping the branch at \xab alive.
            /\ Cardinality({k2 \in AllKeys :
                    IsPrefix(prefixKey, k2) /\ endTrie[k2] # NoVal}) >= 2
            \* prefixKey is outside the query range (before startKey)
            /\ SeqLte(prefixKey, sk) /\ prefixKey # sk
            \* startKey is within or after the prefix key's extensions
            /\ IsPrefix(prefixKey, sk))
            => VerifyChangeProof(startTrie, endTrie, sk, None)

=============================================================================
