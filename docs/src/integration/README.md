# AvalancheGo & EVM Integration

Firewood exposes a Go API through its FFI layer, and downstream consumers (AvalancheGo
and downstream projects) depend on a separately published Go module.

## The Go API surface

The in-repo `ffi/` directory ships a Go wrapper. Its public types map onto Firewood's
Rust concepts — the Go names deliberately differ from the Rust ones:

| Go type | Rust concept | Notes |
| --- | --- | --- |
| `Database` (`firewood.go`) | `Db` | the database handle; there is no Go type named `Db` |
| `Proposal` (`proposal.go`) | `Proposal` | uncommitted batch atop a base root |
| `Revision` (`revision.go`) | `DbView` | read-only view backed by a pinned revision (`DbView` is a Rust trait) |
| `Reconstructed` (`reconstructed.go`) | reconstructed/archival view | state rebuilt from range proofs during state sync; freed by calling `Drop()` |
| `Iterator` (`iterator.go`) | view iterator | streams key/value pairs; `Next` copies, `NextBorrowed` lends Rust memory |
| `BatchOp` (`batch_op.go`) | `BatchOp` | a single `Put`/`Delete`/`PrefixDelete`; the unit of a write batch |
| `RangeProof`, `ChangeProof`, `NextKeyRange` (`proofs.go`) | proof types | range/change proofs and key-range cursors for state sync |

`New` constructs each type; the `With*` options tune it. The metrics and logging
entry points (`Gatherer`, `LogConfig`, `StartMetrics`, `StartLogs`) wire up
observability. The generated Go API reference is published as
[Go API documentation (godoc)](/firewood/ffi/).

## How downstream consumers depend on Firewood

AvalancheGo and downstream projects do **not** import the in-repo `ffi/` directory. They depend
on the separately published Go module `github.com/ava-labs/firewood-go-ethhash/ffi`,
pinned in their `go.mod` files. That module tracks Firewood's workspace releases: when a
Firewood version is released (a semver tag on the repository, which sets the
`firewood-ffi` crate version), CI builds the static libraries, copies the in-repo `ffi/`
directory into the `ava-labs/firewood-go-ethhash` repository, and tags it. See
the build flow in [`ffi/README.md`](https://github.com/ava-labs/firewood/blob/main/ffi/README.md)
(`cargo build` → `go tool cgo`) and the publish/version cadence in
[the release process](../meta/release.md).

> [!NOTE]
> The Go module path in `firewood-go-ethhash` is rewritten by CI from the in-repo
> path to `github.com/ava-labs/firewood-go-ethhash/ffi`. The tag format follows Go
> module conventions: a Firewood workspace release `vX.Y.Z` is tagged as
> `ffi/vX.Y.Z` in the `firewood-go-ethhash` repository.
