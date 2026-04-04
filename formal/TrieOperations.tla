---------------------------- MODULE TrieOperations ----------------------------
(*
 * Verifies that incremental Insert and Delete operations on a
 * path-compressed trie produce the same result as building from
 * scratch via Compress.
 *
 * State machine: starts from empty, non-deterministically applies
 * Insert(key, value) or Delete(key). A shadow flat map tracks the
 * expected state. The invariant checks that the incrementally
 * maintained compressed trie always equals Compress(shadow).
 *
 * This verifies node splitting, partial path merging, and branch
 * flattening — the operations where bugs hide in the Rust code.
 *)

EXTENDS CompressedTrie

--------------------------------------------------------------------------------
(* Insert — add or update a key-value pair in a compressed trie *)

\* Insert remKey with value into the subtrie rooted at node.
\* remKey is the remaining key relative to this node's parent.
\*
\* Four cases based on how remKey aligns with node.pp:
\*   1. Exact match (remKey = pp): update value
\*   2. Full pp match, more key: descend into child
\*   3. Key is prefix of pp: split, value at split point
\*   4. Divergence: split into two children

RECURSIVE TrieInsertHelper(_, _, _)
TrieInsertHelper(node, remKey, value) ==
    IF node = None THEN
        \* No node here — create a leaf.
        [pp |-> remKey, value |-> value, children |-> EmptyChildren]
    ELSE
        LET pp  == node.pp
            cpl == CommonPrefixLen(remKey, pp)
        IN
            IF cpl = Len(pp) /\ cpl = Len(remKey) THEN
                \* Case 1: Exact match — update value.
                [node EXCEPT !.value = value]

            ELSE IF cpl = Len(pp) THEN
                \* Case 2: Full pp consumed, more key remains — descend.
                LET childIdx   == remKey[cpl + 1]
                    childRemKey == SubSeq(remKey, cpl + 2, Len(remKey))
                    newChild    == TrieInsertHelper(
                                    node.children[childIdx],
                                    childRemKey, value)
                IN  [node EXCEPT !.children[childIdx] = newChild]

            ELSE IF cpl = Len(remKey) THEN
                \* Case 3: Key exhausted before pp — split, value at split.
                \* The original node becomes a child at pp[cpl+1].
                LET origNibble == pp[cpl + 1]
                    origNode   == [node EXCEPT
                                    !.pp = SubSeq(pp, cpl + 2, Len(pp))]
                IN  [pp       |-> SubSeq(pp, 1, cpl),
                     value    |-> value,
                     children |-> [n \in Nibbles |->
                        IF n = origNibble THEN origNode ELSE None]]

            ELSE
                \* Case 4: Divergence — split into two children.
                LET origNibble == pp[cpl + 1]
                    origNode   == [node EXCEPT
                                    !.pp = SubSeq(pp, cpl + 2, Len(pp))]
                    newNibble  == remKey[cpl + 1]
                    newLeaf    == [pp       |-> SubSeq(remKey, cpl + 2,
                                                       Len(remKey)),
                                  value    |-> value,
                                  children |-> EmptyChildren]
                IN  [pp       |-> SubSeq(pp, 1, cpl),
                     value    |-> NoVal,
                     children |-> [n \in Nibbles |->
                        IF n = origNibble THEN origNode
                        ELSE IF n = newNibble THEN newLeaf
                        ELSE None]]

TrieInsert(ctrie, key, value) ==
    TrieInsertHelper(ctrie, key, value)

--------------------------------------------------------------------------------
(* Delete — remove a key from a compressed trie *)

\* After removing a value or child, a node may need flattening:
\*   - No value, no children → remove (return None)
\*   - No value, one child → merge with child (extend child's pp)
\*   - Otherwise → keep as is
Flatten(node) ==
    LET numChildren == Cardinality(
            {n \in Nibbles : node.children[n] # None})
    IN
        IF node.value = NoVal /\ numChildren = 0 THEN
            \* Empty node — remove.
            None
        ELSE IF node.value = NoVal /\ numChildren = 1 THEN
            \* Single child, no value — merge with child.
            LET childNibble == CHOOSE n \in Nibbles :
                                node.children[n] # None
                child == node.children[childNibble]
            IN  [child EXCEPT
                    !.pp = node.pp \o <<childNibble>> \o child.pp]
        ELSE
            \* Has value or multiple children — keep.
            node

\* Delete remKey from the subtrie rooted at node.
RECURSIVE TrieDeleteHelper(_, _)
TrieDeleteHelper(node, remKey) ==
    IF node = None THEN
        \* Key not in trie — no change.
        None
    ELSE
        LET pp  == node.pp
            cpl == CommonPrefixLen(remKey, pp)
        IN
            IF cpl < Len(pp) THEN
                \* Key diverges from pp — not in this subtrie.
                node

            ELSE IF cpl = Len(remKey) THEN
                \* Exact match — remove value, then flatten.
                Flatten([node EXCEPT !.value = NoVal])

            ELSE
                \* Full pp match, more key — descend into child.
                LET childIdx    == remKey[cpl + 1]
                    childRemKey == SubSeq(remKey, cpl + 2, Len(remKey))
                    newChild    == TrieDeleteHelper(
                                    node.children[childIdx], childRemKey)
                IN  Flatten([node EXCEPT !.children[childIdx] = newChild])

TrieDelete(ctrie, key) ==
    IF ctrie = None THEN None
    ELSE TrieDeleteHelper(ctrie, key)

--------------------------------------------------------------------------------
(* State machine *)

VARIABLES ctrie, shadow

vars == <<ctrie, shadow>>

Init ==
    /\ ctrie = None
    /\ shadow = EmptyTrie

InsertAction ==
    \E k \in AllKeys :
        \E v \in Values :
            /\ ctrie'  = TrieInsert(ctrie, k, v)
            /\ shadow' = [shadow EXCEPT ![k] = v]

DeleteAction ==
    \E k \in AllKeys :
        /\ ctrie'  = TrieDelete(ctrie, k)
        /\ shadow' = [shadow EXCEPT ![k] = NoVal]

Next == InsertAction \/ DeleteAction

--------------------------------------------------------------------------------
(* Invariants *)

\* The compressed trie built incrementally must equal the one built
\* from scratch via Compress at every reachable state.
TrieMatchesShadow == ctrie = Compress(shadow)

\* Structural invariants hold at every step.
TrieWellFormed == WellFormed(ctrie)

\* Lookup in the compressed trie agrees with the shadow map.
LookupMatchesShadow == \A k \in AllKeys : Lookup(ctrie, k) = shadow[k]

=============================================================================
