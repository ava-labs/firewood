# Placeholder-db: Compaction-Less Write-ahead free Database Optimized for Storing Recent Merkleized Blockchain State

![Github Actions](https://github.com/ava-labs/placeholder-db/actions/workflows/ci.yaml/badge.svg?branch=main)
[![Ecosystem license](https://img.shields.io/badge/License-Ecosystem-blue.svg)](./LICENSE.md)

> :warning: Placeholder-db is alpha-level software and is not ready for production
> use. The Placeholder-db API and on-disk state representation may change with
> little to no warning.

Placeholder-db is an embedded key-value store, optimized to store recent Merkleized blockchain
state with minimal overhead. Placeholder-db is implemented from the ground up to directly
store trie nodes on-disk. Unlike most state management approaches in the field,
it is not built on top of a generic KV store such as LevelDB/RocksDB. Placeholder-db, like a
B+-tree based database, directly uses the trie structure as the index on-disk. Thus,
there is no additional “emulation” of the logical trie to flatten out the data structure
to feed into the underlying database that is unaware of the data being stored. The convenient
byproduct of this approach is that iteration is still fast (for serving state sync queries)
but compaction is not required to maintain the index. Placeholder-db was first conceived to provide
a very fast storage layer for the EVM but could be used on any blockchain that
requires an authenticated state.

Placeholder-db only attempts to store the active state on-disk and will actively free up
unused data when state revisions are stale.
Placeholder-db uses copy-on-write (COW) for database trie nodes, keeping the representation of the
representable revisions on-disk.

Placeholder-db keeps some configurable number of previous revisions available to power
state sync (which may occur at a few roots behind the current state).

Placeholder-db guarantees that writes that occur do not overwrite existing merkleized nodes.
This guarantees atomicity and durability in the database, and offers the ability to read old
versions from disk simply by knowing the old root address.

## Architecture Diagram

TBD

## Terminology

- `Revision` - A historical point-in-time state/version of the trie. This
  represents the entire trie, including all `Key`/`Value`s at that point
  in time, and all `Node`s.
- `View` - This is the interface to read from a `Revision` or a `Proposal`.
- `Node` - A node is a portion of a trie. A trie consists of nodes that are linked
  together. Nodes can point to other nodes and/or contain `Key`/`Value` pairs.
- `Hash` - In this context, this refers to the merkle hash for a specific node.
- `Root Hash` - The hash of the root node for a specific revision.
- `Key` - Represents an individual byte array used to index into a trie. A `Key`
  usually has a specific `Value`.
- `Value` - Represents a byte array for the value of a specific `Key`. Values can
  contain 0-N bytes. In particular, a zero-length `Value` is valid.
- `Key Proof` - A proof that a `Key` exists within a specific revision of a trie.
  This includes the hash for the node containing the `Key` as well as all parents.
- `Range Proof` - A proof that consists of two `Key Proof`s, one for the start of
  the range, and one for the end of the range, as well as a list of all `Key`/`Value`
  pairs in between the two. A `Range Proof` can be validated independently of an
  actual database by constructing a trie from the `Key`/`Value`s provided.
- `Change Proof` - A proof that consists of a set of all changes between two
  revisions.
- `Put` - An operation for a `Key`/`Value` pair. A put means "create if it doesn't
  exist, or update it if it does. A put operation is how you add a `Value` for a
  specific `Key`.
- `Delete` - An operation indicating that a `Key` should be removed from the trie.
- `Batch Operation` - An operation of either `Put` or `Delete`.
- `Batch` - An ordered set of `Batch Operation`s.
- `Proposal` - A proposal consists of a base `Root Hash` and a `Batch`, but is not
  yet committed to the trie. In Placeholder-db's most recent API, a `Proposal` is required
  to `Commit`.
- `Commit` - The operation of applying one or more `Proposal`s to the most recent
  `Revision`.

## Logging

If you want logging, enable the `logging` feature flag, and then set RUST\_LOG accordingly.
See the documentation for [env\_logger](https://docs.rs/env_logger/latest/env_logger/) for specifics.
We currently have very few logging statements, but this is useful for print-style debugging.

## Test

```
cargo test --release
```

## License

TBD

