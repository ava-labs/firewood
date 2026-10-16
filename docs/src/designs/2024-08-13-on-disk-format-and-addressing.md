---
title: On-disk format and addressing
status: active
category: storage
authors: [demosdemon]
---

# On-disk format and addressing

**Source:** `storage/src/`, `firewood/src/`

## Overview

Firewood stores Merkle trie nodes directly on disk in a single database file and
uses the trie structure itself as the on-disk index. There is no separate key-value
store underneath and no background compaction. This document describes how nodes are
addressed, allocated, and reclaimed, and the guarantees that make the format
crash-recoverable.

## Architecture

A node's address is simply its byte offset within the database file. Branch nodes
reference their children by storing those children's disk offsets, so traversing the
trie on disk is a sequence of offset reads — no hash-to-location lookup table is
required. Each revision has a root node, and the root's address is the entry point
for reading that revision.

The database file begins with a fixed-size header (`NodeStoreHeader`, occupying the
first 2048 bytes) that stores the allocated-size high-water mark, the root node
location, and the heads of all free-list chains. The header also contains a
`root_hash` field, but it is meaningful only in databases written by the `firewood-v1`
version family (`firewood-v1` and `firewood-v1-hfix`) that also have a root address;
older databases leave it uninitialized. After the header, the remainder of the file
consists of contiguous stored areas, each prefixed by a one-byte area-index that
identifies its size class.

## Key data structures

- **`NodeStore<T, S>`** (`storage/src/nodestore/mod.rs`) — the main nodestore
  container. The type parameter `T` encodes the lifecycle state: `Committed`,
  `Mutable<Propose>`, `Arc<ImmutableProposal>`, `Mutable<Recon>`, or
  `Reconstructed`. The parameter `S` is the storage backend.
- **`LinearAddress`** (`storage/src/nodestore/primitives.rs`) — a non-zero `u64`
  byte offset into the database file. This is the node identity used by branch nodes
  to reference their children.
- **`AreaIndex`** (`storage/src/nodestore/primitives.rs`) — a one-byte index into
  the table of 23 valid area sizes (16 bytes through 16 MiB). Every stored area is
  prefixed with its `AreaIndex` so the allocator can determine the area's size when
  freeing it.
- **Free lists** (`storage/src/nodestore/alloc.rs`) — one singly-linked list per
  size class. The head of each list is stored in the `NodeStoreHeader`; each free
  area stores the address of the next free area of the same size class. Together they
  form 23 independent linked lists of reclaimed space.
- **Future-delete log (FDL)** — the `deleted` field in `Mutable<Propose>`,
  `ImmutableProposal`, and `Committed` (`storage/src/nodestore/mod.rs`). It collects
  `MaybePersistedNode` values replaced or deleted during proposal construction that
  cannot be freed yet, because older in-memory or on-disk revisions may still
  reference them.

## Invariants and guarantees

- **No forward references before flush.** A node is never referenced by an address
  that has not yet been flushed to disk, so a crash cannot leave a live node pointing
  at unwritten data.
- **Careful free-list management across revisions.** The reap path returns space to
  the free lists only once no surviving revision can reference it, so reuse never
  corrupts a revision that is still readable.

Together these invariants make the database recoverable after an unclean shutdown
without a separate write-ahead log replay over user data.

> [!NOTE]
> This durability guarantee is narrower for space reclaimed from *reaped* revisions.
> When an expired revision is reaped, its freed areas are written to disk, but the
> free-list heads in the `NodeStoreHeader` are only updated in memory. `persist` is
> what writes the header, so those heads reach disk in the next persist cycle that
> carries a committed revision — a reap-only cycle writes no header. A crash in that
> window cannot corrupt a still-readable revision (the reaped revision is already
> gone), but it can leave the reclaimed space unreachable — leaked rather than merely
> delayed. Reaping runs on the background persist thread (`PersistLoop::reap` in
> `firewood/src/persist_worker.rs`, which calls `NodeStore::reap_deleted`);
> `firewood/src/manager.rs` only decides when a revision is evicted.
>
> `Db::check` (`storage/src/checker`) detects leaked areas but does not reclaim them,
> and exposes no fix path of its own. Repair runs through `fwdctl check --fix`, which
> currently reports leaks as unfixable; reclaiming them is tracked by
> [ava-labs/firewood#1247](https://github.com/ava-labs/firewood/issues/1247).

## On-disk and runtime behavior

- **Allocation.** A new node is serialized into a byte buffer; the allocator finds
  the smallest area size that fits (`AreaIndex::from_size`). It first tries to pop
  the head of that size class's free list; if the list is empty, it extends the file
  by advancing the stored size pointer (`allocate_from_end`).
- **Area layout.** Each stored area on disk begins with one byte holding the
  `AreaIndex`, followed by one byte that is `0xFF` for free areas or a node-type
  discriminant for live nodes, followed by the serialized content.
- **Free-list size classes.** There are 23 size classes: 16, 32, 64, 96, 128, 256,
  512, 768, and 1024 bytes, then powers of two from 2 KiB through 16 MiB. The set is
  defined in `storage/build.rs` and hashed into the `NodeStoreHeader` so a database
  cannot be opened with a mismatched size table.
- **Revision creation.** Committing a proposal writes new nodes, producing a new root
  address and hash. Nodes replaced by the commit are recorded in the proposal's
  delete list (the FDL) rather than freed immediately.
- **Revision expiration.** The `RevisionManager` (`firewood/src/manager.rs`) keeps a
  configurable number of committed revisions in memory (`max_revisions`, default 128).
  When that queue is full, the oldest revision is handed to the background persist
  worker for reaping. Reaping frees the FDL entries the revision carries via
  `NodeAllocator::delete_node`, which writes a free-area record over the node and
  prepends it to the appropriate size-class free list.
- **Archival mode.** When `RootStore` is enabled, deleted-node tracking is disabled
  (`DeletedNodeTracking::Disabled`) and the root address mapping for expired
  revisions is retained, enabling reconstruction by root hash.

## Trade-offs

- Offset-based addressing keeps reads cheap (a child traversal is a single
  `stream_from` call) but ties a node's identity to its physical location, so
  relocation requires rewriting all referrers.
- Retaining superseded nodes in the FDL trades temporary space overhead for the
  ability to serve recent historical revisions and to recover cleanly after a crash.
- The fixed 23-size-class table is compact and cache-friendly but accepts internal
  fragmentation when a node's serialized size falls between two class boundaries, and
  it caps the largest storable node at 16 MiB — `AreaIndex::from_size` rejects anything
  larger.

## Related designs

- See [Design Documents](README.md) for other subsystems still to be documented.
