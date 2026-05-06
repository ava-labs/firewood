# Configurable Read-Time Hash Verification

## Status

Partially implemented (2026-05-06).

**Landed in core (Rust):**

- `HashVerification` and `HashFailureMode` types in `firewood-storage`
- `DbConfig.hash_verification` plumbed through `ConfigManager` →
  `RevisionManager` → `FileBacked`
- `ReadableStorage::hash_verification()` and `NodeReader::read_node_verified()`
- `with_root()` gated on `root_from_rootstore` (default on)
- `open()` verifies when `root_recent` is set
- Branch/leaf hooks at all four read sites (`merkle::get_helper`,
  three iter sites)
- `HASH_VERIFICATIONS` and `HASH_VERIFICATION_FAILURES` counters with
  `type` label
- Path threading optimized: skipped entirely when `branches` and `leaves`
  are both off; uses `&mut Path` with extend/truncate when on, so deep
  recursion doesn't blow the stack
- Tests: no-false-positives (clean DB + all flags), corruption-detected
  (flipped byte → `FileIoError`), log-and-continue (corrupt data
  returned)

**Still pending:**

- Criterion benchmark per flag combination (separate commit)
- FFI: `DatabaseHandleArgs` packed-byte field + Go `WithHashVerification`
  / `WithHashFailureMode` options (separate commit)
- fwdctl: `--verify-*` and `--verify-on-failure` CLI flags (separate
  commit)

## Motivation

Firewood currently performs hash verification of nodes read from disk in
exactly one place: `NodeStore::with_root()`
(`storage/src/nodestore/mod.rs:188`), which re-hashes the root node fetched
via the rootstore and compares against the expected root hash. All other
reads — including the root loaded by `NodeStore::open()` from the header,
and every interior/leaf node read during trie traversal — trust the bytes
on disk.

We want to make read-time verification configurable so operators can trade
performance for integrity assurance. The default behavior must match
today's behavior so existing deployments see no change unless they opt in.

## Goals

- Allow opt-in verification of additional read paths beyond the current
  root-from-rootstore check.
- Independent toggles for the four read categories the user identified.
- Zero added cost when verification is disabled (no hashing, no extra
  branches in the hot path beyond a flag check).
- Default configuration matches today's behavior exactly.

## Non-goals

- Verifying writes (already covered by hash computation during commit).
- Verifying cached nodes on cache hits — a node in cache was verified (or
  trusted) the first time it was read; re-verifying every cache hit is
  pure waste.
- Background scrubbing / whole-database verification — that's `fwdctl
  check`.

## The four verification points

| Flag                  | Read path                                                          | Today         |
| --------------------- | ------------------------------------------------------------------ | ------------- |
| `root_from_rootstore` | `with_root()` — root reached via the rootstore                     | always on     |
| `root_recent`         | `open()` — root of a recent (in-memory) revision loaded via header | off (trusted) |
| `branches`            | Branch nodes resolved from a parent's `Child::AddressWithHash`     | off           |
| `leaves`              | Leaf nodes resolved from a parent's `Child::AddressWithHash`       | off           |

The expected hash for non-root nodes lives in the parent's
`Child::AddressWithHash` entry, not on disk with the node. Verification
therefore cannot be implemented inside `read_node_with_num_bytes_from_disk`
(which has only an address); it must happen at the call site that
dereferences a `Child::AddressWithHash` into a `SharedNode`.

### Category overlap

The four categories are not disjoint. Any given node falls into one of
these buckets:

- **Root + branch**: the trie has more than one node and we just loaded
  the root. This is the overwhelmingly common case — any non-trivial
  trie has a branch at the root.
- **Root + leaf**: the trie has exactly one node (root is the only key).
  A degenerate case that doesn't matter for performance discussions.
- **Non-root branch**: an interior node reached from a parent.
- **Non-root leaf**: a terminal value node reached from a parent.

A node must be hashed at most once per read regardless of how many flags
match. Category resolution rule:

- If the node is being read *as the root* (via `with_root()` or
  `open()`), the relevant flag is the corresponding **root** flag
  (`root_from_rootstore` or `root_recent`). The `branches` / `leaves`
  flags do not apply, even though the node is structurally a branch or
  leaf.
- If the node is being read *as a child* (via `Child::AddressWithHash`
  resolution), the relevant flag is `branches` or `leaves` based on
  `node.is_leaf()`.

In other words, the read path determines the category, not the node's
shape. This keeps the rule simple ("verify if the flag for *this read
site* is set") and naturally avoids double-hashing.

## Proposed configuration

Add to `firewood/src/db.rs`:

```rust
/// Controls which node reads are hash-verified.
///
/// Defaults match historical behavior: only the root node fetched via the
/// rootstore is verified.
#[derive(Copy, Clone, Debug)]
pub struct HashVerification {
    /// Verify the root node when it is reached via the rootstore.
    /// Default: true (matches existing behavior).
    pub root_from_rootstore: bool,
    /// Verify the root node of a recent (in-memory) revision when it is
    /// loaded from the header at open time.
    /// Default: false.
    pub root_recent: bool,
    /// Verify branch nodes when they are resolved from a parent.
    /// Default: false.
    pub branches: bool,
    /// Verify leaf nodes when they are resolved from a parent.
    /// Default: false.
    pub leaves: bool,
}

impl Default for HashVerification {
    /// Matches historical behavior: only the rootstore root is verified.
    fn default() -> Self {
        Self {
            root_from_rootstore: true,
            root_recent: false,
            branches: false,
            leaves: false,
        }
    }
}

impl HashVerification {
    /// Verify every read.
    pub const fn all() -> Self {
        Self {
            root_from_rootstore: true,
            root_recent: true,
            branches: true,
            leaves: true,
        }
    }
}
```

Add to `DbConfig`:

```rust
#[builder(default)]
pub hash_verification: HashVerification,
```

Open question: bitfield (`u8`) vs struct of `bool`s. The struct is more
ergonomic; the bitfield is one byte and trivially `Copy`. Either works —
defaulting to the struct unless we need to put this in a hot per-node
struct.

## Plumbing

`HashVerification` needs to reach the code that performs reads. Path:

`DbConfig` → `ConfigManager` → `RevisionManager` → `FileBacked` (the
`ReadableStorage` impl that already carries `cache_read_strategy`).

Storing it on `FileBacked` makes it available everywhere a `NodeStore`
holds an `Arc<S: ReadableStorage>`, which is everywhere reads happen.
Alternative: store it on `NodeStore` itself. `FileBacked` is more
convenient because the read functions already access `self.storage`.

## Hook points

### 1. `open()` — `storage/src/nodestore/mod.rs:131`

Today:

```rust
let root_hash = if let Some(hash) = header.root_hash() {
    hash.into_hash_type()                      // trusted
} else {
    debug!("No root hash in header; computing from disk");
    nodestore
        .read_node_from_disk(root_address, ReadableNodeMode::Open)
        .map(|n| hash_node(&n, &Path(SmallVec::default())))?
};
```

When `root_recent` is set, also verify when the header *does* provide
a hash: read the node, recompute, compare, return error on mismatch. The
"no header hash" branch already hashes; nothing changes there.

### 2. `with_root()` — `storage/src/nodestore/mod.rs:188`

Already verifies. Gate the comparison on `root_from_rootstore`. With the
default `true`, behavior is unchanged.

Note: making this configurable means an operator *could* turn off the
existing safety check. That is the explicit ask. We document it as a
foot-gun.

### 3. Child resolution — branch and leaf reads

The right hook is the function that turns a `Child::AddressWithHash` into
a `SharedNode`. The Explore agent flagged `as_shared_node` and the merkle
traversal sites; we'll confirm exact call sites during implementation.

Sketch:

```rust
fn resolve_child(&self, addr: LinearAddress, expected: &HashType, path: &Path)
    -> Result<SharedNode, FileIoError>
{
    let node = self.read_node_from_disk(addr, ReadableNodeMode::Read)?;
    let cfg = self.storage.hash_verification();
    let want_check = if node.is_leaf() { cfg.leaves } else { cfg.branches };
    if want_check && hash_node(&node, path) != *expected {
        return Err(FileIoError::new(
            std::io::Error::other("hash verification failed"),
            None, addr.get(), None,
        ));
    }
    Ok(node)
}
```

Two subtleties:

- **Cache hits**: `read_node_from_disk` returns cached nodes without
  hitting disk. Re-verifying cached nodes is wasted work — they were
  verified (or trusted) on first read. We want verification to happen
  *only* on the disk-read branch. Cleanest fix: split the cache check
  out of `read_node_from_disk` so the verifying caller can hash only
  fresh reads.
- **`Path` argument to `hash_node`**: the trie path is required for
  `ethhash` (account nodes at specific depths). Callers in the merkle
  layer have the path; `with_root` and `open` pass an empty path for
  the root, which is correct.

## Performance considerations

Two factors drive the cost of enabling a flag:

1. **Per-node hash cost.** The hasher preimage for a leaf is the key plus
   value digest (tens of bytes). For a branch it's the same preamble plus
   up to 16 child hashes at 32 bytes each — call it 5–10× more bytes
   through SHA-256 / Keccak-256 per node. Real but not dramatic.
2. **Read frequency.** A single trie traversal touches one leaf at the
   end and many branches on the way down. So `branches = true` adds work
   on *every* hop of *every* read, while `leaves = true` adds work once
   per read. In practice the branch flag will dominate the overhead even
   though branches aren't enormously more expensive to hash than leaves.

The two root flags fire at most once per `open()` / rootstore lookup, so
their cost is negligible.

Plan: add a simple criterion bench that measures `get` / iter throughput
with each flag combination on a populated database, so we have numbers
before recommending any non-default setting.

## Failure behavior

When a hash check fails, two responses make sense in different contexts:

- **Production**: return an error and abort the read. A mismatch means
  on-disk corruption; returning the bad node risks propagating it into
  computed roots, generated proofs, or new commits. Fail loud.
- **fwdctl diagnosis**: log the failure and return the node anyway.
  When inspecting a known-broken database (`fwdctl dump`, `graph`, key
  walks), you want to enumerate *every* corrupt node, not stop at the
  first one.

Model this as a single setting on `HashVerification`, not per-flag:

```rust
#[derive(Copy, Clone, Debug, Default)]
pub enum HashFailureMode {
    /// Return `FileIoError` from the read on mismatch. Default.
    #[default]
    Error,
    /// Log at error level and return the (unverified) node.
    /// Intended for diagnostic tools, not production.
    LogAndContinue,
}

pub struct HashVerification {
    pub root_from_rootstore: bool,
    pub root_recent: bool,
    pub branches: bool,
    pub leaves: bool,
    pub on_failure: HashFailureMode,
}
```

Reasoning for one mode rather than per-flag: the four flags answer
"should I check?"; the failure mode answers "what do I do when a check
fails?" — those concerns are independent. No realistic use case wants
"hard-fail leaf mismatches but warn-and-continue branch mismatches", so
splitting it per-flag is combinatorial overkill.

### Error path

In `Error` mode, produce a `FileIoError` whose inner `io::Error` carries
`"hash verification failed"` plus the address. Matches the existing
message in `with_root()` and lets log scrapers find all four sites under
one string.

### Log-and-continue path

Emit `log::error!` with the address, the expected hash, the computed
hash, and the read-site type (`rootstore` / `root` / `branch` / `leaf`).
Increment `HASH_VERIFICATION_FAILURES` as usual. Return the node
unchanged.

Note: the `log::error!` call is only useful when the `logging` feature
is enabled (see CLAUDE.md — `logging` gates `env_logger` and the
`RUST_LOG` plumbing). Without it the log macro compiles to a no-op, so
in `LogAndContinue` mode a build that lacks `logging` will silently
swallow mismatches — the same end-user-visible behavior as having the
verification flag off, except the `HASH_VERIFICATION_FAILURES` counter
still increments. fwdctl is built with `logging` enabled, so this is
only a footgun for embedders who turn on `LogAndContinue` without also
enabling `logging`. Document this clearly in the public API doc on
`HashFailureMode::LogAndContinue`.

### CLI exposure

Add a fwdctl flag:

```text
--verify-on-failure=<error|continue>   default: error
```

The Go wrapper exposes it via `WithHashFailureMode(HashFailureModeError | HashFailureModeContinue)`.
Default everywhere is `Error`.

## Testing

- Unit test: corrupt a node on disk (overwrite a value byte), open with
  the relevant flag, assert the right error surfaces.
- One test per flag, plus one with all flags on.
- Default-config test: corrupt a leaf, confirm reads succeed (today's
  behavior preserved).

## Open-time API changes

The `HashVerification` config has to be threadable through every layer
that opens a database. Four layers, top-down:

### 1. Rust — `DbConfig`

`firewood/src/db.rs` (around line 103). Add the field with a `#[builder(default)]`:

```rust
#[derive(Clone, TypedBuilder, Debug)]
#[non_exhaustive]
pub struct DbConfig {
    // ... existing fields ...

    /// Which node reads to hash-verify. Default matches historical
    /// behavior: only the rootstore root is checked.
    #[builder(default)]
    pub hash_verification: HashVerification,
}
```

In `Db::new` (db.rs:165), pass it through to the `ConfigManager` builder
alongside `root_store`. From there it lands in `RevisionManagerConfig`,
then into `FileBacked` at construction.

Existing callers that don't set the field get today's behavior, so this
is fully backward compatible.

### 2. fwdctl

`fwdctl` builds a `DbConfig` directly. It never enables `root_store`
(verified: no `root_store` / `RootStore` / `rootstore` references
anywhere under `fwdctl/`), so the `root_from_rootstore` flag is
unreachable from fwdctl and we don't need a CLI switch for it.

The remaining three flags are useful for operators inspecting suspect
databases:

```text
--verify-root-recent      enable verification of recent (in-memory) revision roots
--verify-branches         enable verification of branch nodes on read
--verify-leaves           enable verification of leaf nodes on read
--verify-all              shorthand for HashVerification::all()
```

### 3. FFI — `ffi/src/handle.rs`

`DatabaseHandleArgs` (handle.rs:80–110) needs four new `bool` fields. To
keep the C struct stable and avoid a flag-day for Go callers, prefer a
single packed field:

```rust
#[repr(C)]
pub struct DatabaseHandleArgs<'a> {
    // ... existing fields ...

    /// Bit-packed hash verification flags. Bits (LSB first):
    ///   0: root_from_rootstore
    ///   1: root_recent
    ///   2: branches
    ///   3: leaves
    /// A value of 0 here is rejected; callers that want defaults must
    /// pass 0b0001 explicitly. (Or we treat 0 as "use defaults" — open
    /// question.)
    pub hash_verification: u8,
}
```

In `DatabaseHandle::new` (handle.rs:165), decode the byte into a
`HashVerification` and pass it on the `DbConfig::builder()` chain.

`firewood.h` is generated; it'll pick up the new field automatically.

### 4. Go wrapper — `ffi/firewood.go`

Add to the `config` struct (line 115):

```go
type config struct {
    // ... existing fields ...
    hashVerification HashVerification // bitfield, see below
}
```

Add a `HashVerification` type and constants:

```go
type HashVerification uint8

const (
    VerifyRootFromRootstore HashVerification = 1 << iota
    VerifyRootRecent
    VerifyBranches
    VerifyLeaves

    VerifyDefault = VerifyRootFromRootstore
    VerifyAll     = VerifyRootFromRootstore | VerifyRootRecent | VerifyBranches | VerifyLeaves
)
```

`defaultConfig()` (line 141) sets `hashVerification: VerifyDefault`.

Add option helpers in the same style as the existing `With*` functions:

```go
// WithHashVerification sets which node reads are hash-verified.
// Default: VerifyDefault (only the rootstore root is verified).
func WithHashVerification(v HashVerification) Option {
    return func(c *config) { c.hashVerification = v }
}
```

In `New` (line 285), pass the byte through to the C struct:

```go
hash_verification: C.uint8_t(conf.hashVerification),
```

### Migration / compatibility

- Pure Rust callers using `DbConfig::builder()` see no breakage — the
  new field has a `#[builder(default)]`.
- FFI/Go: the new `hash_verification` field is added to
  `DatabaseHandleArgs`. Go callers using `New(dir, alg)` with no options
  get `VerifyDefault` automatically. C/C++ callers that construct the
  struct manually must zero-init or set the new field; if we treat `0`
  as "use defaults", existing zero-inited structs remain correct.

The "treat 0 as defaults" choice is worth deciding up front because it
affects the struct semantics permanently. Recommend: yes, treat 0 as
defaults — it makes ABI compatibility cleanest and matches the spirit
of "off by default" for the new flags.

## Decisions

- **Shape**: struct of `bool`s.
- **Constructors**: `HashVerification::default()` preserves existing
  behavior (only `root_from_rootstore`); `HashVerification::all()` enables
  every check.
- **Attach point**: `FileBacked`, alongside `cache_read_strategy`.
- **Metrics**: emit a `firewood_counter!` on every verification performed,
  with a `type` label distinguishing the read site.

## Metrics

Use the existing `firewood_metrics` macros (see
`firewood/src/db.rs:157`). On each verification — pass or fail — increment:

```rust
firewood_counter!(HASH_VERIFICATIONS, "type" => "rootstore").increment(1);
firewood_counter!(HASH_VERIFICATIONS, "type" => "root").increment(1);
firewood_counter!(HASH_VERIFICATIONS, "type" => "branch").increment(1);
firewood_counter!(HASH_VERIFICATIONS, "type" => "leaf").increment(1);
```

Label values map 1:1 to the four flags:

| Flag                  | `type` label |
| --------------------- | ------------ |
| `root_from_rootstore` | `rootstore`  |
| `root_recent`         | `root`       |
| `branches`            | `branch`     |
| `leaves`              | `leaf`       |

Failures additionally increment a separate counter so dashboards can
alert on mismatch rate without having to compute it from the failed
read paths:

```rust
firewood_counter!(HASH_VERIFICATION_FAILURES, "type" => "<same>").increment(1);
```

The failure counter is a strict subset of the total counter, which keeps
the math simple (`failures / total = mismatch rate`).

## Out of scope / future

- Sampling mode: verify 1 in N reads to amortize cost. Easy to add later
  on top of the bool flags (replace `bool` with an enum
  `Off | Always | Sample(N)`).
- Verifying nodes during proposal / commit reads — proposals already
  re-hash to compute the new root, so this is largely redundant.
