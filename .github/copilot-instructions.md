# Firewood — GitHub Copilot Instructions

Firewood is an embedded key-value store for Merkleized blockchain state, written in Rust
(edition 2024, MSRV 1.94.0) with a Go FFI layer (`ffi/`). It uses the trie structure
directly as the storage index rather than layering on top of a generic KV store.

When assisting with code in this repository, apply the guidelines below in addition to
your standard capabilities.

## Code Review

When reviewing pull requests or suggesting code changes, apply the firewood-specific
checks below on top of your built-in review. These supplement — not replace — your
standard review.

Claude users: a dedicated `firewood-review` skill is available at
`.claude/skills/firewood-review/SKILL.md`. It orchestrates parallel review agents
covering the Rust toolchain, Go FFI, CI workflows, and code quality.

### Rust

- **Overflow arithmetic**: Reject `saturating_*` or `checked_*` for operations that are
  provably impossible to overflow — they add runtime cost for no benefit. Scrutinize
  `wrapping_*` for operations where wrapping would silently produce incorrect results.
- **`unwrap()`**: Hard reject outside `#[cfg(test)]` modules. Also reject
  `#[allow(clippy::unwrap_used)]` and `#[expect(clippy::unwrap_used)]` outside test
  modules.
- **Lint suppression**: Prefer `#[expect(...)]` over `#[allow(...)]`. For lints that
  only apply under specific build features, use
  `#[cfg_attr(feature = "...", expect(...))]`. Any surviving `#[allow]` must include an
  inline comment explaining why `#[expect]` is insufficient.
- **Error propagation (`?`)**: Ensure propagated errors carry enough context to diagnose
  the failure at the call site. Flag uses where the error should be handled locally
  rather than propagated upward.

### General

- **Comments and messages**: Every comment, log message, and error string must be
  accurate, useful, and free of tautologies. Log and error messages must be unique enough
  to locate their source quickly.
- **Atomic operations**: Verify correct memory ordering (`Ordering`) for every load,
  store, and read-modify-write operation.
- **Locks**: Check for potential deadlocks, excessively long critical sections, and
  inappropriate `RwLock` vs `Mutex` choices given the actual access patterns.
- **Panic potential**: Flag unvalidated array indexing, unchecked arithmetic, and
  `unreachable!()` in paths that could actually be reached.
- **Anti-patterns**: Recommend modern idioms. Avoid `ToString::to_string` — prefer
  `to_owned()` for `&str → String` conversions. Flag unnecessary string allocations.
- **Visibility**: Reject unnecessary `pub` exports. Prefer the tightest visibility
  (`pub(crate)`, `pub(super)`) that satisfies the interface requirements.
- **Error handling**: Reject silently discarded errors, incorrect error types, and errors
  returned without sufficient context.
- **Tests**: New non-trivial code requires tests. Tests must be targeted (not covering
  too broad a scope), DRY, and cover edge cases. Scrutinize test names for clarity.

### FFI and Unsafe Code

When any unsafe Rust or Go code is present, or when a change crosses the FFI boundary:

- Analyze memory safety and soundness end-to-end across the boundary.
- Verify lifetime correctness for all values crossing the FFI boundary.
- Check correct use of `ManuallyDrop`, `Box::from_raw`, and pointer validity invariants.
- Assume all platforms are 64-bit. Defensive 32-bit guards are acceptable; proactive
  32-bit support changes are not needed.
- Auto-generated C-style comments in `ffi/firewood.h` may retain Rust doc style — do
  not flag these.

## Cargo Feature Matrix

The following feature combinations must all pass `cargo clippy` and `cargo nextest` before a PR is ready. Skip `--all-features` on macOS (it includes `io-uring` which requires Linux):

| Feature flags               | macOS | Linux |
| --------------------------- | ----- | ----- |
| (none)                      | ✓     | ✓     |
| `--no-default-features`     | ✓     | ✓     |
| `--features ethhash,logger` | ✓     | ✓     |
| `--all-features`            | ✗     | ✓     |

## Commit and PR Conventions

Commit messages and PR titles must follow [Conventional Commits](https://www.conventionalcommits.org/).
Allowed types: `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor`,
`style`, `test`.

## Additional Context

See `AGENTS.md` for full project context, architecture principles, and the complete
code review guidelines. See `CONTRIBUTING.md` for the contribution workflow and style
guide.
