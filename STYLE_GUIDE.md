# Style Guide

Apply these firewood-specific checks whenever reviewing code, suggesting changes, or
providing inline feedback — in addition to general best practices.

We generally follow the same rules that `cargo fmt` and `cargo clippy` will report as warnings, with a few notable exceptions as documented in the associated Cargo.toml file.

By default, we prohibit bare `unwrap` calls and index dereferencing, as there are usually better ways to write this code. In the case where you can't, please use `expect` with a message explaining why it would be a bug, which we currently allow. For more information on our motivation, please read this great article on unwrap: [Using unwrap() in Rust is Okay](https://blog.burntsushi.net/unwrap) by [Andrew Gallant](https://blog.burntsushi.net).

## Rust

- **Overflow arithmetic**: Do not leave bare arithmetic under
  `#[expect(clippy::arithmetic_side_effects)]`. Use the contextually correct explicit
  method: `checked_*` when overflow is possible and must be handled, `saturating_*` when
  clamping at the bound is the correct behavior, and — for provably-impossible overflow in
  **production** code — `debug_assert!(<invariant>)` paired with `wrapping_*` (zero release
  cost, and the assertion restores the dev/test overflow canary). Do not use
  `checked_*`/`saturating_*` for provably-safe operations; they add runtime cost for no
  benefit. Reserve a reasoned `#[expect(clippy::arithmetic_side_effects, reason = "...")]`
  for the rare expression where every rewrite hurts clarity. Test-only arithmetic already
  runs under the dev-profile overflow canary, so a reasoned `#[expect]` there is
  acceptable. See **Lint Suppression Policy** below.
- **`unwrap()`**: Hard reject in production code, along with
  `#[allow(clippy::unwrap_used)]` and `#[expect(clippy::unwrap_used)]` there. Judge test
  context by location, not by `cfg(test)` alone: unit-test modules and integration tests
  under `tests/` are exempt, and so are Criterion benches under `benches/`, which are
  `harness = false` binaries that never compile with `cfg(test)` set.
- **`expect()`**: The sanctioned alternative to `unwrap` (see
  [Style Guide](#style-guide)), and clippy does not
  enforce `expect_used`, so review is the only gate. The message must state the invariant
  that makes the panic unreachable — why reaching it would be a bug — rather than name the
  operation that failed. Reject `.expect("valid")` and `.expect("should not fail")`. Where
  the caller can act on the failure, returning a `Result` beats any message.
- **Lint suppression**: No local `#[allow(...)]` / `#![allow(...)]`. Use a reasoned
  `#[expect(...)]` (or `#[cfg_attr(<cfg>, expect(...))]` for feature-gated lints),
  scoped as tightly as possible. See **Lint Suppression Policy** below for the full rules
  and per-lint guidance.
- **Error propagation (`?`)**: Ensure propagated errors carry enough context to diagnose
  the failure at the call site. Reject silently discarded errors and uses where the error
  should be handled locally rather than propagated upward.
- **Trie-depth recursion**: Trie depth follows key length, and a peer controls key length
  during proof verification. A function that recurses once per trie level, directly or
  through another function, must pass every recursive call through
  `firewood_storage::ensure_stack`, so depth costs heap rather than call stack. Check
  compiler-generated recursion too: `Node`'s `Clone` and `BranchNode`'s `Drop` are
  hand-written for this reason and must not be re-derived, and a new `Child` variant that
  can own a `Node` must be added to the scan in `BranchNode::drop`. `Node`'s derived
  `PartialEq` recurses and is for tests only.
- **Nibble range bounds**: An inclusive nibble-path lower bound uses the empty slice for
  "unbounded" — the empty slice already sorts as the minimum key. An upper bound must be
  `Option<&[u8]>` with `None` for unbounded (+∞); reusing the empty slice there judges
  every key out of range. An *exclusive* lower bound is the one case that needs
  `Option` too, because `Some(&[])` excludes the empty key and `None` admits it (see
  `proofs::holes::open_interval`). When either bound is threaded through new code, check
  every comparison site. See `merkle::collapse::CollapseRange` and
  `proofs::holes::KeySpan::as_key_range`.
- **Proof node structure**: A consumer of a verified proof takes trie structure from
  `ProofNode::key`, which the node hash commits to in both modes. `partial_len` (and
  `partial_path()`) is authenticated only under the Ethereum mode; under MerkleDB a
  tampered split still verifies. See `merkle::holes::Walk::boundary`.

## Lint Suppression Policy

Every suppression must be deliberate, minimally scoped, and self-documenting.

### Mechanism

- Prefer a **fix** that avoids the lint when it is local and low-risk (e.g. `.get()`
  instead of indexing; write the `# Errors`/`# Panics` doc; make a fn `const`).
- Otherwise use `#[expect(lint, reason = "why the code is correct / why the lint can't be
  satisfied")]`. The `reason` is the `reason = "..."` attribute field, not a `//` comment.
  A reason that merely restates the lint or counts occurrences is not a reason.
- Feature-conditional lints (fire only under some feature combinations) use
  `#[cfg_attr(<cfg where the lint fires>, expect(lint, reason = "..."))]`.
- **No local `#[allow]` / `#![allow]`.** Use a reasoned `#[expect]` instead. A
  workspace-level `allow` in `Cargo.toml` `[workspace.lints]` (a deliberate, documented,
  project-wide policy such as `cast_possible_truncation`) is a different thing and is
  permitted.

### Scope

- Start at the tightest scope (expression / statement / item).
- Elevate exactly **one** level when the *same lint* appears **3+** times in the wider
  scope: 3+ suppressible statements in a block → the block; 3+ in a function → the
  function/item.
- This ladder **stops before module level.** A module-level suppression (`#![expect(...)]`
  inside a `mod`, or `#[expect]` on a `mod`) carries a higher bar and **must not** sit on a
  module that has non-test submodules — push it down. Exceptions: a module whose only
  submodule is a `#[cfg(test)] mod tests` (or an all-test subtree) may keep a module-level
  suppression; benchmark and integration-test crate roots (`benches/*.rs`, `tests/*.rs`)
  are test code and may carry a crate-root suppression, subject to the same bar on the
  `reason`; and `ffi/src/lib.rs` keeps a single crate-root
  `#![expect(unsafe_code, ...)]` as a documented FFI-boundary carve-out.

### `unsafe_code` exemption

`#[expect(unsafe_code)]` needs **no** `reason`. For a per-block `unsafe { ... }` the
`// SAFETY:` comment (required by `undocumented_unsafe_blocks`) is the justification; for
`#[unsafe(no_mangle)]`/`extern "C"` functions and `unsafe impl` bridges the item's own
doc-comment is. An existing reason is fine but not required.

### Per-lint guidance

For lints we leave suppressible:

| Lint | Fix first | Suppress when | Reason states |
| --- | --- | --- | --- |
| `arithmetic_side_effects` | `checked_`/`saturating_` per context; provably-safe → `debug_assert!` + `wrapping_` | rewrite hurts clarity | why overflow can't occur / is handled |
| `indexing_slicing` | `.get()` / iterators (allowed in tests) | non-test, index provably in bounds, `.get()` adds noise | what bounds the index |
| `unwrap_used` | non-test → return `Result` (allowed in tests) | never outside tests | n/a in tests |
| `missing_errors_doc` / `missing_panics_doc` | write the `# Errors` / `# Panics` section | only the uniform `Error`, nothing method-specific | where conditions are documented |
| `cast_precision_loss` / `cast_sign_loss` | avoid the cast if feasible | intrinsic to f64 metric/progress math | why precision/sign loss is acceptable |
| `cast_possible_truncation` | — | — | handled globally (documented workspace `allow`) |
| `disallowed_methods` / `disallowed_types` (`split_at`, …) | use the recommended replacement | the panicking precondition is provably met | the in-bounds/length argument |
| `unsafe_code` | — | when unsafe is required | no reason (see exemption above) |
| `unused_variables` (feature-gated) | use the binding, or `_`-prefix | used only under some features | `cfg_attr(<cfg where unused>, expect(...))` |
| `too_many_lines` / `too_many_arguments` / `large_types_passed_by_value` / `type_complexity` / `dead_code` / … | split / borrow / delete where reasonable | the shape is intentional | why the shape is intended |

## Comments and documentation

A comment competes with the code for the reader's attention and must win on
information. Write what the code cannot say for itself — the invariant, the
reason for the ordering, the failure the guard prevents, the layout of the bytes
on disk. This comment loses, and deleting it improves the file:

    // Increment the count.
    self.count += 1;

This one wins, because nothing in the surrounding code states it:

    // Publish the free-list head only after the node is durable. A crash
    // between the two leaks the space; the reverse order can hand out an
    // address that still holds a live node.

Be deliberate about what each comment carries:

- **Self-contained and context free.** The reader is a stranger years from now
  who did not see the review, does not know the issue number, and has no diff in
  front of them. Describe the code as it stands, not as it changed: avoid "now",
  "no longer", "previously", "this fix", and pointers to a review conversation.
  History belongs in the commit message and the PR description.
- **Said once.** State each piece of context in the place that owns it and refer
  to that place from everywhere else — an intra-doc link in Rust
  (`` [`NodeStore::flush_to`] ``), a doc link in Go (`[Revision.EthGetProof]`), or a
  plain path (`see storage/src/nodestore/persist.rs`) where neither is available. Copies
  drift, and a stale copy misleads more than a reference ever could.
  [Documenting wrappers, shims, and FFI adapters](#documenting-wrappers-shims-and-ffi-adapters)
  applies this rule to delegating functions.
- **Consistent in voice.** Prefer the present tense or the imperative mood
  ("returns the root hash", "hold the lock across the flush"). Use the
  terminology defined in [AGENTS.md](./AGENTS.md#important-terminology) —
  revision, view, proposal, commit, batch — rather than a synonym coined on the
  spot, and do not alternate between "we", "you", and "the caller" within a
  file.
- **Free of filler.** No throat-clearing ("Note that", "It is important to note
  that", "Basically"), no hype ("blazing fast", "robust", "cleanly handles"), no
  prose restatement of the signature, and no doc section that exists only to be
  present.

## User-facing strings

Error messages, log lines, and CLI help are part of the interface. Hold them to
the same bar as the API:

- Name what failed and what the operator can do about it. "failed to open
  database" is a category; "failed to open database at {path}: {source}" is a
  starting point for a fix.
- Keep capitalization, punctuation, and terminology consistent across messages,
  following the convention of the surface rather than inventing one: Rust error
  strings are lowercase and unpunctuated because they are composed into a larger
  chain, while CLI help is sentence case.
- Make each message distinct enough that searching the source for it lands on a
  single site.
- Do not apologize, and do not suggest retrying unless a retry can succeed.

Reviewers should challenge comments and strings that read as generated rather
than written — padded, hedged, repeated at every call site, or narrating the
change that introduced them. "This comment does not earn its line" and "say this
once here and link to it from the other two places" are legitimate review
feedback.

## Documenting wrappers, shims, and FFI adapters

When a function exists only to delegate to another — an FFI adapter, a thin
wrapper, or a shim that adds no behavior of its own — **do not duplicate the
callee's documentation**. Duplicated docs drift: when the underlying function
changes, every copy must be found and updated, and a stale copy misleads readers
more than a one-line reference ever could.

Instead:

- Say what the wrapper *is* and link to the function it delegates to for the
  details (parameters, return values, formats, and safety requirements). In
  Rust, use intra-doc links (`` [`fwd_eth_get_proof`] ``) so readers can click
  through to the canonical documentation.
- Document only what is **unique to the wrapper**: why it exists (if
  non-obvious), and any behavior it adds or changes — extra error conditions,
  additional safety requirements, or different argument handling.
- **Reference a target at least as visible as the item being documented.**
  Public docs that link to a private item break — Rust fails CI, and Go renders
  the link as dead plain text. A wrapper may delegate to a private helper for its
  *implementation*, but its docs should reference the **public** canonical
  function (or inline the details) — never the private helper.

In Rust, `rustdoc` renders `[Type::method]` as a clickable link, so referencing
the canonical documentation is both DRY and convenient — prefer it:

    /// Produce an `eth_getProof`-compatible proof against a reconstructed view
    /// rather than a committed revision.
    ///
    /// See [`fwd_eth_get_proof`] for the proof format, arguments, return values,
    /// and key-encoding requirements.
    ///
    /// # Safety
    ///
    /// As [`fwd_eth_get_proof`], except `reconstructed` must be a valid pointer to
    /// a [`ReconstructedHandle`].

In Go, doc links such as `[Revision.EthGetProof]` (available since Go 1.19) are
clickable on pkg.go.dev and navigable via `gopls`, just as in Rust, so the same
balance applies: cross-reference the shared contract, but keep wrapper-specific
details (such as error conditions) local rather than referring the reader away
entirely. Go's style guides reinforce this preference for clarity over strict
DRY — the [Google Go Style Guide][google-go-style] lists clarity as its foremost
principle and does not treat DRY as overriding, and the
[Uber Go Style Guide][uber-go-style] is likewise a catalog of conventions that
favor clarity and consistency:

    // EthGetProof is [Revision.EthGetProof] evaluated against this reconstructed
    // view. It returns [ErrDroppedReconstructed] if the view has been released.

[google-go-style]: https://google.github.io/styleguide/go/guide
[uber-go-style]: https://github.com/uber-go/guide/blob/master/style.md

## General

- **Comments and messages**: Every comment, log message, and error string must be
  accurate, useful, and free of tautologies. Log and error messages must be unique enough
  to locate their source quickly. Challenge text that reads as generated rather than
  written — padded, hedged, repeated at every call site, or narrating the change that
  introduced it. See
  [Comments and documentation](#comments-and-documentation) and
  [User-facing strings](#user-facing-strings) for the authoring rules.
- **Atomic operations**: Verify correct memory ordering (`Ordering`) for every load,
  store, and read-modify-write operation.
- **Locks**: Check for potential deadlocks, excessively long critical sections, and
  inappropriate `RwLock` vs `Mutex` choices given the actual access patterns.
- **Indexing and panic potential**: Prefer iterator methods over direct indexing —
  treat index access as a code smell worth scrutiny. Flag unchecked arithmetic and
  `unreachable!()` in paths that could actually be reached.
- **Anti-patterns**: Recommend modern idioms. Avoid `ToString::to_string` — prefer
  `to_owned()` for `&str → String` conversions.
- **Visibility**: Reject unnecessary `pub` exports. Prefer the tightest visibility
  (`pub(crate)`, `pub(super)`) that satisfies the interface requirements.
- **Tests**: New non-trivial code requires tests. Tests must be targeted (not covering
  too broad a scope), DRY, and cover edge cases. Scrutinize test names for clarity.
  Group similar tests together.

## FFI and Unsafe Code

When any unsafe Rust or Go code is present, or when a change crosses the FFI boundary:

- Analyze memory safety and soundness end-to-end across the boundary.
- Verify lifetime correctness for all values crossing the FFI boundary.
- Check correct use of `ManuallyDrop`, `Box::from_raw`, and pointer validity invariants.
- Assume all platforms are 64-bit. Defensive 32-bit guards are acceptable; proactive
  32-bit support changes are not needed.
- Auto-generated C-style comments in `ffi/firewood.h` may retain Rust doc style — do
  not flag these.
- Go handle types that hold a borrow on the `Database` must register a lease in the
  `keepAliveRegistry` via `lease.attach`. The registered drop function must be a method
  on the inner handle (or any type that does not back-reference the outer wrapper) so
  that `runtime.AddCleanup` can reclaim the wrapper when the user drops their last
  reference. See `ffi/keepalive.go` for the lock-ordering invariants and the
  closed-registry contract.
