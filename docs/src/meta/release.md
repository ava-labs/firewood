# Release Process

Firewood's release process is documented in
[`RELEASE.md`](https://github.com/ava-labs/firewood/blob/main/RELEASE.md). It covers
semver tagging, publishing the Rust workspace crates to crates.io, and triggering the
CI pipeline that builds the static library, copies `ffi/` into the
`ava-labs/firewood-go-ethhash` repository, and tags the resulting Go module.

## Related repository processes

- [Contributing](https://github.com/ava-labs/firewood/blob/main/CONTRIBUTING.md) —
  testing requirements, PR workflow, and code review process
- [Style guide](https://github.com/ava-labs/firewood/blob/main/STYLE_GUIDE.md) —
  coding conventions and the Firewood-specific review checklist: Rust safety,
  lint suppression, and FFI/unsafe code
