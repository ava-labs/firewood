---
status: active
last-reviewed: 2026-06-17
source:
  - storage/src/
  - firewood/src/
---

# On-disk Format and Addressing

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
first 2048 bytes) that stores the current file size, the root node location, and the
heads of all free-list chains. The header also contains a `root_hash` field, but this
field is only populated in `firewood-v1` format databases; older databases leave it
uninitialized. After the header, the remainder of the file
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
  the table of 23 valid area sizes (16 bytes through 16 MB). Every stored area is
  prefixed with its `AreaIndex` so the allocator can determine the area's size when
  freeing it.
- **Free lists** (`storage/src/nodestore/alloc.rs`) — one singly-linked list per
  size class. The head of each list is stored in the `NodeStoreHeader`; each free
  area stores the address of the next free area of the same size class. Together they
  form 23 independent linked lists of reclaimed space.
- **Future-delete log (FDL)** — the `deleted` field in `ImmutableProposal` and
  `Committed` (`storage/src/nodestore/mod.rs`). Records `MaybePersistedNode` values
  that became unreachable when a proposal was created but that cannot be freed yet,
  because older in-memory or on-disk revisions may still reference them.

## Invariants and guarantees

- **No forward references before flush.** A node is never referenced by an address
  that has not yet been flushed to disk, so a crash cannot leave a live node pointing
  at unwritten data.
- **Careful free-list management across revisions.** Space is returned to the free
  lists only once no surviving revision can reference it, so reuse never corrupts a
  revision that is still readable.
- Together these make the database recoverable after an unclean shutdown without a
  separate write-ahead log replay over user data.

## On-disk and runtime behavior

- **Allocation.** A new node is serialized into a byte buffer; the allocator finds
  the smallest area size that fits (`AreaIndex::from_size`). It first tries to pop
  the head of that size class's free list; if the list is empty, it extends the file
  by advancing the stored size pointer (`allocate_from_end`).
- **Area layout.** Each stored area on disk begins with one byte holding the
  `AreaIndex`, followed by one byte that is `0xFF` for free areas or a node-type
  discriminant for live nodes, followed by the serialized content.
- **Free-list size classes.** There are 23 size classes: 16, 32, 64, 96, 128, 256,
  512, 768, and 1024 bytes, then powers of two from 2 KB through 16 MB. The set is
  defined in `storage/build.rs` and hashed into the `NodeStoreHeader` so a database
  cannot be opened with a mismatched size table.
- **Revision creation.** Committing a proposal writes new nodes, producing a new root
  address and hash. Nodes replaced by the commit are recorded in the proposal's
  delete list (the FDL) rather than freed immediately.
- **Revision expiration.** The `RevisionManager` (`firewood/src/manager.rs`) keeps a
  configurable number of committed revisions in memory (default 128). When the oldest
  revision is reaped, the FDL entries it carries are freed via `NodeAllocator::delete_node`,
  which writes a free-area record over the node and prepends it to the appropriate
  size-class free list.
- **Archival mode.** When `RootStore` is enabled, deleted-node tracking is disabled
  (`DeletedNodeTracking::Disabled`) and expired revisions are not freed, preserving
  all historical data on disk for lookup by root hash.

## Trade-offs

- Offset-based addressing keeps reads cheap (a child traversal is a single
  `stream_from` call) but ties a node's identity to its physical location, so
  relocation requires rewriting all referrers.
- Retaining superseded nodes in the FDL trades temporary space overhead for the
  ability to serve recent historical revisions and to recover cleanly after a crash.
- The fixed 23-size-class table is compact and cache-friendly but accepts internal
  fragmentation when a node's serialized size falls between two class boundaries.

## Related designs

- This is the first active design written under the process established by [0001 — mdBook documentation site](../proposed/0001-mdbook-documentation-site.md).
- See the backfill TODO in [Active Designs](README.md) for adjacent subsystems
  (revision management, free lists and the FDL, hashing) still to be written up.
