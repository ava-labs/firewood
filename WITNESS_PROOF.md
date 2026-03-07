# Witness Proofs and Truncated Tries

This document explains how Firewood's witness proof system enables lightweight
clients to verify state changes without holding the full Merkle trie.

## Overview

Firewood supports a client-server mode where:

- The **server** holds the complete Firewood database with the full Merkle trie.
- The **client** holds a **truncated trie** — only the top K levels of the trie,
  with deeper children replaced by hash-only stubs called **Proxy nodes**.

Two verification mechanisms keep the client honest:

1. **Reads** are verified with single-key Merkle inclusion/exclusion proofs.
2. **Commits** are verified with **witness proofs** — the server sends enough
   trie nodes for the client to independently re-execute the batch and check
   the resulting root hash.

## The Truncated Trie

A `TruncatedTrie` is a partial in-memory copy of a Merkle trie. It stores every
node from the root down to a configurable **truncation depth** K, measured in
node hops (each branch node visited counts as one hop, regardless of partial
path length). Below depth K, children are replaced with `Child::Proxy(hash)`
stubs that carry only the child's Merkle hash.

```text
Full trie                          Truncated trie (K = 2)

       [root]                             [root]
      /      \                           /      \
    [A]      [B]      depth 1         [A]      [B]      depth 1
   / | \      |                      / | \      |
 [C] [D] [E] [F]     depth 2      #C  #D  #E  #F      depth 2  (Proxy stubs)
 |       / \                       (hashes only)
[G]    [H] [I]        depth 3
```

### Root Hash Invariant

The truncated trie's root hash is always identical to the full trie's root
hash. This works because a Merkle hash at any node depends only on:

- The node's own data (partial path, value)
- Its children's hashes

Since Proxy stubs carry the correct child hashes, `hash_node()` produces the
same result whether it reads from a full child node or a Proxy stub. The client
can therefore use the root hash as a trusted anchor for proof verification.

### Child Representations

Nodes above depth K store children as `Child::MaybePersisted(node, hash)` —
an in-memory node wrapped in an `Arc<Mutex<...>>` alongside its precomputed
hash. At depth K, children become `Child::Proxy(hash)`. This two-tier scheme
lets the truncated trie:

- Traverse in-memory nodes above K for proof verification.
- Use Proxy hashes below K for root hash recomputation.

### Construction

`TruncatedTrie::from_trie(trie, depth)` walks the full trie top-down and
builds the truncated copy bottom-up:

1. Recurse into each child, incrementing the hop counter by 1 per branch.
2. When `current_depth >= max_depth`, call `proxy_all_children()` to replace
   every child with a `Proxy(hash)` stub.
3. Otherwise, wrap the recursively truncated child as `MaybePersisted`.
4. Compute the hash at each node using `hash_node()` on the way back up.

## Witness Proofs

A `WitnessProof` contains three pieces:

| Field            | Description                                               |
|------------------|-----------------------------------------------------------|
| `batch_ops`      | The `Put` / `Delete` operations the server applied        |
| `new_root_hash`  | The root hash the server computed after applying the ops  |
| `witness_nodes`  | The minimal set of trie nodes below depth K needed for    |
|                  | independent re-execution                                  |

Each `WitnessNode` carries a nibble path (its position in the trie) and the
full `Node` data (partial path, value, children — with non-witness children
replaced by `Proxy` stubs).

### Why Witnesses Are Needed

The client's truncated trie has Proxy stubs below depth K. If a batch operation
touches a key whose path passes through a Proxy, the client cannot re-execute
it — the needed node data isn't there. The witness fills exactly those gaps:
the server includes every below-K node on each affected key's path so the
client can graft them onto its trie and replay the operations.

## Server Side: Generating Witnesses

`generate_witness()` runs on the server after committing a batch. It walks the
**old** trie (pre-commit state) along each key's path and collects nodes that
the client doesn't have.

### Algorithm

```text
for each batch operation:
    nibbles = key_to_nibbles(op.key)

    walk the trie from root following nibbles:
        at each branch node:
            if current_depth >= truncation_depth:
                collect this node (convert children to Proxy stubs)

            match branch.partial_path against remaining nibbles:
                if diverges → collect and stop
                if matches  → descend into next child

            if branch has ≤ 2 children:
                collect sibling children (flatten safety)
```

### Flatten Safety

When a `Delete` removes a key from a branch that has exactly two children,
Firewood's `flatten_branch()` merges the remaining child into the parent. This
reads the sibling via `read_for_update()`. If that sibling is a Proxy stub on
the client, re-execution crashes.

To prevent this, `collect_siblings_for_flatten()` adds the siblings of any
branch with two or fewer children to the witness. This guarantees the client
has the data to perform the flatten.

### Node Conversion

`convert_node_for_witness()` prepares nodes for the wire: all children
(whether `AddressWithHash`, `MaybePersisted`, or `Node`) are replaced with
`Child::Proxy(hash)` stubs. The client has no access to the server's disk, so
storage addresses are meaningless to it. Only the Merkle hashes are preserved.

### Determinism

The server stores collected nodes in a `BTreeMap<Vec<u8>, Node>` keyed by
nibble path. This guarantees the same batch always produces the same witness,
which is important for reproducibility and testing.

## Client Side: Verifying Witnesses

`verify_witness()` is the core of client-side verification. It proves that the
server applied the claimed operations honestly by re-executing them and
checking the result.

### Step-by-Step

```text
1. Convert truncated trie to a plain Node tree
   ├── MaybePersisted children → unwrap to Child::Node (recursive)
   ├── Proxy / AddressWithHash → keep as Child::Proxy
   └── Result: tree with only Child::Node and Child::Proxy variants

2. Build witness lookup map
   └── BTreeMap<nibble_path, &Node> from witness_nodes

3. Graft witness nodes onto the tree
   ├── Walk tree recursively, tracking nibble path
   ├── At each Proxy child, check witness map
   │   └── If found: replace Proxy with full Node, recurse into it
   └── After grafting: Proxy stubs remain only for untouched subtrees

4. Create in-memory Merkle for re-execution
   ├── Build a MemStore with the grafted tree as root
   └── Select hash algorithm (Keccak-256 if ethhash, SHA-256 otherwise)

5. Re-execute batch operations
   └── for each op: merkle.insert(key, value) or merkle.remove(key)

6. Hash the result
   ├── Convert mutable proposal → immutable (computes all hashes)
   ├── Extract root hash (or TrieHash::empty() for empty tries)
   └── Compare against witness.new_root_hash
       └── Mismatch → WitnessError::RootHashMismatch

7. Extract new truncated trie
   └── TruncatedTrie::from_trie(result, truncation_depth)
```

If the root hash matches, the client has independently confirmed that applying
`batch_ops` to the old state produces `new_root_hash`. It then adopts the
re-truncated result as its new state.

### Grafting Illustrated

```text
Before grafting (client's tree):         After grafting (with witness W1, W2):

       [root]                                   [root]
      /      \                                 /      \
    [A]      [B]                             [A]      [B]
   / | \      |                             / | \      |
 #C  #D  #E  #F  ← Proxy stubs           [C] #D  #E  [F]  ← W1 and W2 grafted
                                           |              \
                                          #G              [H]  ← W2 had inline child
                                                           |
                                                          #I   ← Proxy (untouched)
```

Only Proxy nodes on affected paths are expanded. Proxies on unaffected
subtrees remain as stubs — the witness doesn't include them.

## Protocol Flow

The full lifecycle of the remote storage protocol:

```text
Server                                     Client
──────                                     ──────

   ┌─── Bootstrap ──────────────────────────────┐
   │                                             │
   │  get_truncated_trie(root_hash, K) ────────► │
   │                                             │ verify root hash matches
   │  ◄──────────────────── TruncatedTrie ────── │ store as trusted state
   │                                             │
   └─────────────────────────────────────────────┘

   ┌─── Reads ──────────────────────────────────┐
   │                                             │
   │  get_value(key) ──────────────────────────► │
   │                                             │ server generates proof
   │  ◄──────── (value, Merkle proof) ────────── │
   │                                             │ verify proof against
   │                                             │   trusted root hash
   └─────────────────────────────────────────────┘

   ┌─── Commits ─────────────────────────────────┐
   │                                              │
   │  create_proposal(batch_ops) ───────────────► │
   │  ◄──────── (proposal_id, new_root_hash) ─── │
   │                                              │
   │  commit_proposal(proposal_id) ─────────────► │
   │    server applies ops, generates witness     │
   │  ◄──────── witness_proof_bytes ───────────── │
   │                                              │
   │                              verify_witness: │
   │                              1. to_node_tree │
   │                              2. graft nodes  │
   │                              3. re-execute   │
   │                              4. check hash   │
   │                              5. re-truncate  │
   │                                              │
   │                              adopt new state │
   └──────────────────────────────────────────────┘
```

## Wire Format

Both `WitnessProof` and `TruncatedTrie` have a binary serialization format
used for transport (e.g., over gRPC). The format uses LEB128 variable-length
integers for compact encoding.

### WitnessProof

```text
┌────────────────────────────────────────┐
│ 8 bytes   magic "fwdwitns"             │
│ 1 byte    version (0)                  │
│ 32 bytes  new_root_hash                │
├────────────────────────────────────────┤
│ varint    batch_ops count              │
│ ┌──────────────────────────────┐       │
│ │ 1 byte  op tag (PUT=1/DEL=2) │ ×N   │
│ │ varint  key length            │      │
│ │ bytes   key                   │      │
│ │ [if PUT]:                     │      │
│ │   varint  value length        │      │
│ │   bytes   value               │      │
│ └──────────────────────────────┘       │
├────────────────────────────────────────┤
│ varint    witness_nodes count          │
│ ┌──────────────────────────────┐       │
│ │ varint  path length           │ ×M   │
│ │ bytes   nibble path           │      │
│ │ [serialized Node]             │      │
│ └──────────────────────────────┘       │
└────────────────────────────────────────┘
```

### TruncatedTrie

```text
┌────────────────────────────────────────┐
│ 8 bytes   magic "fwdtrtri"             │
│ 1 byte    version (0)                  │
│ varint    truncation_depth             │
│ 1 byte    has_root (0 or 1)            │
├────────────────────────────────────────┤
│ [if has_root]:                         │
│   32 bytes  root_hash                  │
│   [serialized Node tree]               │
└────────────────────────────────────────┘
```

### Node Serialization

Nodes are serialized recursively. Branch nodes include all 16 child slots.

```text
Branch:                          Leaf:
┌─────────────────────┐          ┌─────────────────────┐
│ 1 byte  tag (0)     │          │ 1 byte  tag (1)     │
│ varint  path length  │          │ varint  path length  │
│ bytes   partial_path │          │ bytes   partial_path │
│ 1 byte  has_value    │          │ varint  value length │
│ [if has_value]:      │          │ bytes   value        │
│   varint value length│          └─────────────────────┘
│   bytes  value       │
│ [16 × child slot]:   │
│   NONE(0)            │
│   PROXY(1) + hash    │
│   NODE(2) + node     │
│   MAYBE_PERSISTED(3) │
│     + hash + node    │
└─────────────────────┘
```

## Security Properties

The witness proof system provides the following guarantees, assuming the hash
function is collision-resistant:

1. **Integrity**: If `verify_witness` succeeds, then applying `batch_ops` to
   the old trie state genuinely produces `new_root_hash`. A malicious server
   cannot forge a witness that passes verification unless it can find a hash
   collision.

2. **Completeness**: The witness always contains enough nodes for re-execution.
   The server walks every affected path and includes all nodes below the
   truncation depth, plus siblings needed for flatten safety.

3. **Consistency**: The client's truncated trie root hash matches the full
   trie's root hash at every step — after bootstrap, after every verified
   commit, and after deserialization.

4. **Read verification**: Every `get()` call verifies a Merkle proof against
   the trusted root hash. A server cannot return a fabricated value without
   forging a valid Merkle proof.

## Feature Flag: `ethhash`

The hash algorithm is selected at compile time via the `ethhash` feature flag:

| Feature    | Hash algorithm | Hash type                        |
|------------|----------------|----------------------------------|
| (default)  | SHA-256        | `TrieHash` (32 bytes)            |
| `ethhash`  | Keccak-256     | `HashType::Hash` or `HashType::Rlp` |

The witness verification selects the algorithm at runtime using `cfg!()` to
ensure the client and server use matching hash functions.

## Source Files

| File                                | Role                                 |
|-------------------------------------|--------------------------------------|
| `firewood/src/remote/mod.rs`        | Module root                          |
| `firewood/src/remote/truncated_trie.rs` | `TruncatedTrie` construction and verification |
| `firewood/src/remote/witness.rs`    | Witness generation and verification  |
| `firewood/src/remote/ser.rs`        | Binary serialization for wire format |
| `firewood/src/remote/client.rs`     | `RemoteClient` and `RemoteTransport` |
| `storage/src/node/branch.rs`        | `Child` enum, `NodeError`            |
| `storage/src/node/persist.rs`       | `MaybePersistedNode`                 |
| `storage/src/hashednode.rs`         | `hash_node()` function               |
