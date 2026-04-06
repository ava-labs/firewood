---------------------------- MODULE CompressedTrie ----------------------------
(*
 * Pure operators for path-compressed Merkle tries.
 *
 * This module contains no VARIABLE or state machine — it is a library
 * of operators that other specs EXTEND. It defines:
 *   - Compress: build a compressed trie from a flat key-value map
 *   - Decompress: extract a flat map from a compressed trie
 *   - Lookup: query a single key
 *   - CompressedHash: abstract Merkle hash
 *   - WellFormed / DepthOk: structural invariants
 *   - CommonPrefixLen: helper for insert/delete algorithms
 *)

EXTENDS Integers, Sequences, FiniteSets, TLC

CONSTANTS
    BF,         \* Branch factor (number of nibble values: 0..BF-1)
    MaxDepth,   \* Maximum key length in nibbles
    NumValues,  \* Number of distinct values (1..NumValues)
    None        \* Model value sentinel for absent nodes

Nibbles == 0..(BF - 1)
Values == 1..NumValues
NoVal == 0           \* Absent value (integer to avoid mixed-type sets)

\* All possible keys: even-length nibble sequences up to MaxDepth.
\* Only even lengths because Firewood keys are byte sequences — each
\* byte is two nibbles, so values only exist at even nibble depths.
\* This matches the Rust ValueAtOddNibbleLength check.
AllKeys == UNION {[1..d -> Nibbles] : d \in {d \in 1..MaxDepth : d % 2 = 0}}

\* All possible tries: partial functions from keys to values.
AllTries == [AllKeys -> Values \cup {NoVal}]

\* The empty trie: every key maps to NoVal.
EmptyTrie == [k \in AllKeys |-> NoVal]

\* Children array with all slots empty.
EmptyChildren == [n \in Nibbles |-> None]

--------------------------------------------------------------------------------
(* Compress — Build a compressed trie from a flat key-value map *)

RECURSIVE CompressHelper(_, _)
CompressHelper(trie, prefix) ==
    LET keysBelow == {k \in DOMAIN trie :
            /\ trie[k] # NoVal
            /\ Len(k) > Len(prefix)
            /\ SubSeq(k, 1, Len(prefix)) = prefix}
        valHere == IF prefix \in DOMAIN trie THEN trie[prefix] ELSE NoVal
        activeNibbles == {k[Len(prefix) + 1] : k \in keysBelow}
    IN
        IF keysBelow = {} /\ valHere = NoVal THEN
            None
        ELSE IF Cardinality(activeNibbles) = 1 /\ valHere = NoVal THEN
            LET n == CHOOSE n \in activeNibbles : TRUE
                inner == CompressHelper(trie, Append(prefix, n))
            IN  IF inner = None THEN None
                ELSE [pp       |-> <<n>> \o inner.pp,
                      value    |-> inner.value,
                      children |-> inner.children]
        ELSE
            [pp       |-> <<>>,
             value    |-> valHere,
             children |-> [n \in Nibbles |->
                CompressHelper(trie, Append(prefix, n))]]

Compress(trie) == CompressHelper(trie, <<>>)

--------------------------------------------------------------------------------
(* Decompress — Extract key-value pairs from a compressed trie *)

RECURSIVE DecompressHelper(_, _)
DecompressHelper(node, prefix) ==
    IF node = None THEN {}
    ELSE
        LET fullPath == prefix \o node.pp
            valuePairs ==
                IF node.value # NoVal
                THEN {<<fullPath, node.value>>}
                ELSE {}
            childPairs == UNION {
                DecompressHelper(node.children[n], Append(fullPath, n))
                : n \in Nibbles}
        IN  valuePairs \cup childPairs

Decompress(ctrie) ==
    LET pairs == DecompressHelper(ctrie, <<>>)
    IN  [k \in AllKeys |->
            IF \E p \in pairs : p[1] = k
            THEN (CHOOSE p \in pairs : p[1] = k)[2]
            ELSE NoVal]

--------------------------------------------------------------------------------
(* Lookup — Query a single key in a compressed trie *)

RECURSIVE LookupHelper(_, _)
LookupHelper(node, remKey) ==
    IF node = None THEN NoVal
    ELSE
        LET ppLen == Len(node.pp)
            rkLen == Len(remKey)
        IN
            IF rkLen < ppLen THEN NoVal
            ELSE IF SubSeq(remKey, 1, ppLen) # node.pp THEN NoVal
            ELSE IF rkLen = ppLen THEN node.value
            ELSE
                LET nextNibble == remKey[ppLen + 1]
                    rest == SubSeq(remKey, ppLen + 2, rkLen)
                IN  LookupHelper(node.children[nextNibble], rest)

Lookup(ctrie, key) == LookupHelper(ctrie, key)

--------------------------------------------------------------------------------
(* CompressedHash — Abstract Merkle hash *)

RECURSIVE CompressedHash(_, _)
CompressedHash(node, parentPrefix) ==
    IF node = None THEN None
    ELSE
        LET fullPath == parentPrefix \o node.pp
        IN  [fp |-> fullPath,
             v  |-> node.value,
             ch |-> [n \in Nibbles |->
                CompressedHash(node.children[n], Append(fullPath, n))]]

CompressedRootHash(trie) ==
    LET ctrie == Compress(trie)
    IN  IF ctrie = None THEN None
        ELSE CompressedHash(ctrie, <<>>)

--------------------------------------------------------------------------------
(* Structural Invariants *)

RECURSIVE WellFormed(_)
WellFormed(node) ==
    IF node = None THEN TRUE
    ELSE
        LET numChildren == Cardinality(
                {n \in Nibbles : node.children[n] # None})
        IN  /\ (numChildren = 0 => node.value # NoVal)
            /\ (numChildren = 1 => node.value # NoVal)
            /\ \A n \in Nibbles : WellFormed(node.children[n])

RECURSIVE DepthOk(_, _)
DepthOk(node, currentDepth) ==
    IF node = None THEN TRUE
    ELSE
        LET newDepth == currentDepth + Len(node.pp)
        IN  /\ newDepth <= MaxDepth
            /\ \A n \in Nibbles :
                node.children[n] # None =>
                    DepthOk(node.children[n], newDepth + 1)

--------------------------------------------------------------------------------
(* Utilities *)

\* Length of the common prefix of two sequences.
RECURSIVE CommonPrefixLen(_, _)
CommonPrefixLen(a, b) ==
    IF a = <<>> \/ b = <<>> THEN 0
    ELSE IF Head(a) = Head(b) THEN 1 + CommonPrefixLen(Tail(a), Tail(b))
    ELSE 0

\* Lexicographic less-than-or-equal on nibble sequences.
RECURSIVE SeqLte(_, _)
SeqLte(a, b) ==
    IF Len(a) = 0 THEN TRUE
    ELSE IF Len(b) = 0 THEN FALSE
    ELSE IF a[1] < b[1] THEN TRUE
    ELSE IF a[1] > b[1] THEN FALSE
    ELSE SeqLte(Tail(a), Tail(b))

\* Is key in the range [startKey, endKey]?  None means unbounded.
InRange(key, startKey, endKey) ==
    /\ (startKey = None \/ SeqLte(startKey, key))
    /\ (endKey = None \/ SeqLte(key, endKey))

\* Navigate to a node at a specific nibble path and return its value.
\* Returns NoVal if the path doesn't lead to a node.
RECURSIVE GetValueAt(_, _)
GetValueAt(node, remKey) ==
    IF node = None THEN NoVal
    ELSE
        LET pp == node.pp
            cpl == CommonPrefixLen(remKey, pp)
        IN  IF cpl = Len(pp) /\ cpl = Len(remKey) THEN node.value
            ELSE IF cpl = Len(pp) /\ Len(remKey) > cpl THEN
                GetValueAt(node.children[remKey[cpl + 1]],
                           SubSeq(remKey, cpl + 2, Len(remKey)))
            ELSE NoVal

\* Set a value at a specific nibble path. Node must already exist there.
RECURSIVE SetValueAt(_, _, _)
SetValueAt(node, remKey, newValue) ==
    IF node = None THEN None
    ELSE
        LET pp == node.pp
            cpl == CommonPrefixLen(remKey, pp)
        IN  IF cpl = Len(pp) /\ cpl = Len(remKey) THEN
                [node EXCEPT !.value = newValue]
            ELSE IF cpl = Len(pp) THEN
                LET ci == remKey[cpl + 1]
                    rest == SubSeq(remKey, cpl + 2, Len(remKey))
                IN  [node EXCEPT !.children[ci] =
                        SetValueAt(node.children[ci], rest, newValue)]
            ELSE node

=============================================================================
