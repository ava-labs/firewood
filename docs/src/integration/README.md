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
[Go API documentation (godoc) ↗](/firewood/ffi/) (resolves only on the deployed site).

## How downstream consumers depend on Firewood

AvalancheGo and downstream projects do **not** import the in-repo `ffi/` directory. They depend
on the separately published Go module `github.com/ava-labs/firewood-go-ethhash/ffi`,
pinned in their `go.mod` files. Firewood's crates are versioned independently, and that
module tracks the `firewood-ffi` crate: the crate version is bumped by hand before a
release (see the release process), and pushing the matching semver tag triggers CI to
build the static libraries, copy the in-repo `ffi/` directory into the
`ava-labs/firewood-go-ethhash` repository, and tag it there. See
the build flow in [`ffi/README.md`](https://github.com/ava-labs/firewood/blob/main/ffi/README.md)
(`cargo build` → `go tool cgo`) and the publish/version cadence in
[the release process](../meta/release.md).

> [!NOTE]
> The Go module path in `firewood-go-ethhash` is rewritten by CI from the in-repo
> path to `github.com/ava-labs/firewood-go-ethhash/ffi`. The tag format follows Go
> module conventions: a `firewood-ffi` release `vX.Y.Z` is tagged as
> `ffi/vX.Y.Z` in the `firewood-go-ethhash` repository.
