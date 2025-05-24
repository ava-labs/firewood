# Firewood: Compaction-Less Database Optimized for Efficiently Storing Recent Merkleized Blockchain State

![Github Actions](https://github.com/ava-labs/firewood/actions/workflows/ci.yaml/badge.svg?branch=main)
[![Ecosystem license](https://img.shields.io/badge/License-Ecosystem-blue.svg)](./LICENSE.md)

> :warning: Firewood is alpha-level software and is not ready for production
> use. The Firewood API and on-disk state representation may change with
> little to no warning.

Firewood is an embedded key-value store, optimized to store recent Merkleized blockchain
state with minimal overhead. Firewood is implemented from the ground up to directly
store trie nodes on-disk. Unlike most state management approaches in the field,
it is not built on top of a generic KV store such as LevelDB/RocksDB. Firewood, like a
B+-tree based database, directly uses the trie structure as the index on-disk. Thus,
there is no additional “emulation” of the logical trie to flatten out the data structure
to feed into the underlying database that is unaware of the data being stored. The convenient
byproduct of this approach is that iteration is still fast (for serving state sync queries)
but compaction is not required to maintain the index. Firewood was first conceived to provide
a very fast storage layer for the EVM but could be used on any blockchain that
requires an authenticated state.

Firewood only attempts to store recent revisions on-disk and will actively clean up
unused data when revisions expire. Firewood keeps some configurable number of previous states in memory and on disk to power state sync (which may occur at a few roots behind the current state). To do this, a new root is always created for each revision that can reference either new nodes from this revision or nodes from a prior revision. When creating a revision, a list of nodes that are no longer needed are computed and saved to disk in a future-delete log (FDL) as well as kept in memory. When a revision expires, the nodes that were deleted when it was created are returned to the free space.

Firewood guarantees recoverability by not referencing the new nodes in a new revision before they are flushed to disk, as well as carefully managing the free list during the creation and expiration of revisions.

## Architecture Diagram

![architecture diagram](./docs/assets/architecture.svg)

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
  yet committed to the trie. In Firewood's most recent API, a `Proposal` is required
  to `Commit`.
- `Commit` - The operation of applying one or more `Proposal`s to the most recent
  `Revision`.

## Roadmap

- [X] Complete the revision manager
- [X] Complete the API implementation
- [X] Implement a node cache
- [ ] Complete the proof code
- [ ] Hook up the RPC

## Build

In order to build firewood, the following dependencies must be installed:

- `protoc` See [installation instructions](https://grpc.io/docs/protoc-installation/).
- `cargo` See [installation instructions](https://doc.rust-lang.org/cargo/getting-started/installation.html).
- `make` See [download instructions](https://www.gnu.org/software/make/#download) or run `sudo apt install build-essential` on Linux.

## Run

There are several examples, in the examples directory, that simulate real world
use-cases. Try running them via the command-line, via `cargo run --release
--example insert`.

For maximum performance, use `cargo run --maxperf` instead, which enables maximum
link time compiler optimizations, but takes a lot longer to compile.

## Logging

If you want logging, enable the `logging` feature flag, and then set RUST\_LOG accordingly.
See the documentation for [env\_logger](https://docs.rs/env_logger/latest/env_logger/) for specifics.
We currently have very few logging statements, but this is useful for print-style debugging.

## Release

See the [release documentation](./RELEASE.md) for detailed information on how to release Firewood.

## CLI

Firewood comes with a CLI tool called `fwdctl` that enables one to create and interact with a local instance of a Firewood database. For more information, see the [fwdctl README](fwdctl/README.md).

## Test

```sh
cargo test --release
```

## License

Firewood is licensed by the Ecosystem License. For more information, see the
[LICENSE file](./LICENSE.md).
