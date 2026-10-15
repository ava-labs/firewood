# Plan: Multi-File Segmented Storage

## Motivation

Today Firewood stores all nodes and free-list state in a single
backing file. Several use cases motivate splitting this across
multiple files:

* **Bootstrapping.** For faster bootstrapping, a database starting at a specific known height can be used, using a simple copy from a trusted host instead of using state sync.
* **Archival.** In long-running databases the older portion of state
  stops changing. Those older files could live on a shared network
  drive while writes continue on a fresh local file.
* **Tiered storage.** Older data could live on slower, cheaper media
  than the active write set.
* **Per-file size limits.** Some filesystems cap file size (2 TiB on
  ext4 with 4 KiB blocks, for example).

This plan adds a manual operation to freeze the current file and
start writing to a new one, and provides some followup steps for handling the per-file size limit case better.

## Address Model

A `LinearAddress` is the `u64` Firewood uses to refer to a node on
disk. Today it is simply a byte offset into the single backing file,
and code throughout the storage layer treats it as such.

To support multiple files without changing what callers pass around,
`LinearAddress` continues to identify a single global byte position.
We introduce **segments** — one per backing file — that each cover a
contiguous range of that global address space. The storage layer
translates a `LinearAddress` to (segment, file offset) at read/write
time. Callers above the storage layer never see segment boundaries.

The most recent segment, called the **tail**, is the only segment
the allocator hands out space from, and consequently the only one
that ever receives writes. All earlier segments are **sealed**:
immutable and opened read-only.

Each segment carries two range fields in its on-disk header:

* `segment_start: Option<LinearAddress>` — `None` for file 0;
  `Some(prev_limit + NodeStoreHeader::SIZE)` for every other segment
  (see "dead-space gap" below).
* `segment_limit: Option<LinearAddress>` — `None` means the segment
  is the (writable) tail; `Some(_)` means it is sealed at that
  address.

For an address `addr` in a segment whose `segment_start` is `S`
(treating `None` as 0 for arithmetic), the file offset is:

```text
file_offset = addr - S + NodeStoreHeader::SIZE
```

This formula is unchanged across all features in this plan,
including commit 7's auto-detection of concatenated files (where
multiple on-disk segments are merged into a single in-memory
segment whose `segment_start`/`segment_limit` covers the combined
range).

A read or write must satisfy:

* `addr >= S`
* `addr + len <= limit` if the segment is sealed, or
  `addr + len <= current_size` for the unsealed tail.

Crossing a segment boundary in a single read/write is a hard error.

### Dead-space gap between segments

Between any two adjacent segments the global address space contains a
deliberate `NodeStoreHeader::SIZE`-byte gap: addresses in
`[prev_segment.limit, next_segment.start)` are in *no* segment and are
never allocated. The next segment's `segment_start` is set to
`prev.limit + NodeStoreHeader::SIZE`.

The gap exists so that `fwdctl volumes join` (which appends one file
to another) is a flat byte-append with no header rewriting:

* The joined-in file's on-disk header bytes land at file offsets
  corresponding to the gap addresses. Since no node is ever allocated
  in the gap, those header bytes become harmless dead space inside
  the combined file.
* Body addresses formerly in the joined-in segment continue to point
  at the same bytes after the join — the existing
  `file_offset = addr − S + NodeStoreHeader::SIZE` formula resolves
  them correctly using the surviving segment's `S`.

The cost is `NodeStoreHeader::SIZE` (currently 2048 bytes) of
permanently unreclaimable address space per freeze. The benefit is a
much simpler, harder-to-corrupt join operation.

### Concatenated files (auto-detected at open)

> Implemented in commit 7; see Suggested commit breakdown. Commits
> 1–6 simply ignore any trailing bytes past a segment's
> `segment_limit`, which is safe but means an operator who manually
> concatenates volumes won't see the merged chain until commit 7
> lands.

A single file on disk can hold more than one segment's worth of
data. This happens in two scenarios:

* **Half-completed `join`.** A crash between `join`'s byte-append
  step and its header-update step leaves the parent file with extra
  bytes appended but `parent.segment_limit` / `parent.next_filename`
  unchanged. (Without commit 7 this is safely ignored — `join`
  remains crash-safe regardless.)
* **Operator-side concatenation.** An operator runs
  `cat <next> >> <prev>` (and optionally deletes `<next>`) without
  going through `fwdctl volumes join`. With commit 7 this just
  works.

The dead-space-gap design makes detection and resolution trivial:
when the file's on-disk size is larger than the parent segment's
header would imply, the trailing bytes are exactly what the next
segment's header + body would look like, sitting at the file offset
the chain math expects. So at open time we **fold the embedded
segment into the parent** by widening the in-memory segment's
range, rather than tracking it separately.

Detection. Define a segment's *expected file size* as
`NodeStoreHeader::SIZE + (segment_limit − segment_start)` — where
the file would end if it backed only this one segment. If the actual
file size is greater, the trailing bytes start at offset
`NodeStoreHeader::SIZE + (segment_limit − segment_start)` and must
begin with another `NodeStoreHeader`.

Resolution (in-memory only — open never rewrites the parent header,
so the read-only-filesystem-friendly open path is preserved):

1. Read a candidate `NodeStoreHeader` at the trailing-bytes offset.
2. Validate it as the next segment in the chain:
   `embedded.segment_start == parent.segment_limit + NodeStoreHeader::SIZE`,
   plus the standard
   `area_size_hash` / `endian_test` / `version` /
   `node_hash_algorithm` checks.
3. **Fold** the embedded segment into the parent's in-memory
   `Segment` by adopting the embedded's "tail-side" fields:
   * `segment_limit ← embedded.segment_limit`
   * `next_filename ← embedded.next_filename`
   * If the embedded segment was the chain's tail
     (`embedded.segment_limit.is_none()`), also adopt its
     `root_address` / `root_hash` / `free_lists` / `size` — these
     are the live tail metadata. The folded `Segment` is now the
     tail.
   * If the embedded segment was sealed, the parent's `root_address`
     / `root_hash` / `free_lists` / `size` are not meaningful (the
     parent is sealed and will continue to be); leave them alone.

   The combined `Segment` keeps the original `segment_start`. Its
   `segment_limit` (now embedded's) covers the entire address range
   spanned by both — including the dead-space gap addresses, which
   remain unallocated but are now nominally inside the merged
   segment's range. Lookup of any valid `LinearAddress` resolves
   correctly via the unchanged
   `file_offset = addr − segment_start + NodeStoreHeader::SIZE`
   formula, because under the dead-space-gap layout the embedded
   body's on-disk position relative to `segment_start` matches what
   the formula computes.

4. Repeat: the embedded segment's own trailing bytes (if any) may
   contain yet another embedded segment. Loop until the merged
   `Segment`'s expected file size matches the actual file size.
5. Then continue the normal chain walk via `next_filename` — which
   now lives on the merged `Segment` (adopted from the
   last-embedded segment).

If the trailing bytes fail validation (wrong magic, wrong
`segment_start`, etc.), open errors out — the trailing bytes are not
silently discarded, since they would otherwise mask real corruption.

This is purely an in-memory recovery. A future
`fwdctl volumes repair` could offer to formalize the on-disk state
by rewriting the parent header to match the merged in-memory shape,
but that is out of scope.

### Worked example

Let `H = NodeStoreHeader::SIZE`.

File 0 has `segment_start = None` (treated as 0) and
`segment_limit = Some(10000)`. File 1 has
`segment_start = Some(10000 + H)` and `segment_limit = None`.

Addresses `[10000, 10000 + H)` fall in the gap and are unreachable.

A request for `LinearAddress = 20000` is dispatched to file 1 at file
offset `20000 - (10000 + H) + H = 10000` — the file 1 offset where
that node was originally written.

## Header Changes — `storage/src/nodestore/header.rs`

Add to `NodeStoreHeader` (room exists in the 2048-byte block):

* `segment_start: Option<LinearAddress>` — `None` for file 0.
* `segment_limit: Option<LinearAddress>` — `None` for unsealed/tail.
* `next_filename: [u8; 256]` — zeroed = no next; otherwise the raw
  bytes of a path, NUL-padded to 256 bytes. A relative path is
  resolved against this file's directory; an absolute path is used
  as-is. (Rust's `PathBuf::join` semantics make this transparent —
  joining a base directory with an absolute path returns the
  absolute path unchanged.)

`free_lists`, `root_address`, and `root_hash` continue to live in every
header but are **only meaningful in the tail file**.

### Version bump (lazy, on first freeze)

The header's existing `version` field bumps from `1` to `2` the first
time a database becomes multi-file. The bump is *lazy*: a freshly
created database is still v1, and only the freeze operation rewrites
headers as v2. The protection model is asymmetric:

* **v1** means "single-file." All three new fields
  (`segment_start`, `segment_limit`, `next_filename`) must be
  zero-valued. Any v1 reader — old firewood or new — opens the file
  as a standalone database. New firewood enforces the all-zero
  invariant on read; if a v1 header carries non-zero new fields,
  that's corruption and open fails.
* **v2** means "this database may participate in a chain." Old
  firewood refuses v2 outright (it doesn't know the version). New
  firewood reads v2 with the segmented chain walk.

This is the right protection model because the failure mode we care
about is an old binary opening a *multi-file* database and silently
seeing only file 0 — returning "not found" for keys that live in
later segments, and worse, allocating into addresses the chain
considers part of the dead-space gap or file 1. The lazy bump means
existing single-file deployments upgrade with zero friction (their
files stay v1 and remain readable by the old binary), while any
database that has ever been frozen becomes opt-in-only for the new
binary.

The bump fires in two places during freeze:

* The new tail file is written with `version = 2` from the start
  (step 6).
* The old tail's in-place header rewrite (step 9) flips its
  `version` from `1` to `2` alongside the `segment_limit` /
  `next_filename` update. Combining the version flip with the
  step-9 fsync means crash recovery has the same atomicity story
  as the rest of step 9 — there's no separate "version bumped but
  fields not yet set" state to reason about.

Subsequent freezes operate on a v2 tail and leave the version
unchanged.

The reader accepts both versions from commit 1 onward (so v2
fixtures can be hand-rolled for early tests); only the freeze
*writer*, introduced later in the commit breakdown, ever produces
v2.

Validation when opening a non-first segment:

* `segment_start == Some(previous_segment.limit + NodeStoreHeader::SIZE)`.
* `area_size_hash`, `endian_test`, `version`, and `node_hash_algorithm`
  match the tail. (Every segment in a chain is v2; mixing versions
  inside a chain is rejected.)
* If `segment_limit.is_some()`, then `next_filename` must be set
  (sealed segments must point at a successor); if
  `segment_limit.is_none()`, `next_filename` must be zeroed.

## Linear Backend — `storage/src/linear/filebacked.rs`

Replace `FileBacked` internals (or wrap in a new `SegmentedFileBacked`)
with a slice of segments. Each segment owns its own fd; only the
tail's fd holds an advisory lock.

```rust
// Holds the fd for one segment. The tail variant wraps the existing
// `UnlockOnDrop` so the advisory lock is dropped automatically when
// the segment is dropped; sealed segments hold a plain `File` because
// they are immutable and never locked.
//
// Keeping these as variants of one enum (rather than two separate
// fields, only one of which is populated) makes the read path's
// pattern match exhaustive: each `Segment` has exactly one fd, and
// the lock-discipline is encoded in the type.
enum SegmentFd {
    Tail(UnlockOnDrop),
    Sealed(File),
}

struct Segment {
    start: Option<LinearAddress>,
    limit: Option<LinearAddress>,   // None = tail
    fd: SegmentFd,
    path: PathBuf,
}

struct FileBacked {
    // Lock-free for readers. `arc_swap::ArcSwap` lets readers load the
    // current segment list without blocking. The vector only changes when
    // `freeze_and_extend` runs (appending the new tail and demoting the
    // old one). Freeze does not need its own mutex — it is serialized
    // against commits by reusing `RevisionManager::in_memory_revisions`
    // (see Freeze API below). That lock already excludes commits, which
    // is what matters: a concurrent commit allocates from the tail, and
    // we must not be reshuffling the tail underneath it.
    // `Box<[Arc<Segment>]>` over `Vec<…>` because the slice is fixed-length
    // once published — only freeze rebuilds it. The inner `Arc<Segment>` is
    // required: freeze publishes a new slice with a fresh `Arc<Segment>`
    // for the demoted old-tail entry, while sealed segments before it are
    // shared into the new slice via cheap `Arc::clone`. Readers holding
    // the previous outer `Arc` keep the previous-tail's read-write fd
    // alive until they drop their guard.
    segments: ArcSwap<Box<[Arc<Segment>]>>,
    // io_uring lives on `FileBacked`, not per-segment. The ring
    // registers the tail's fd with the kernel (IORING_REGISTER_FILES)
    // for fast submission, so on freeze the ring's registered fd must
    // be replaced with the new tail's fd. `IoUringProxy` is already a
    // `Mutex<IoUring>` internally, so the outer field needs no extra
    // wrapping — freeze re-registers (or rebuilds the ring) through
    // the proxy's existing mutability.
    #[cfg(feature = "io-uring")]
    ring: IoUringProxy,
    cache: Mutex<MemLruCache<LinearAddress, CachedNode>>,   // single global cache
    free_list_cache: Mutex<EntryLruCache<LinearAddress, Option<LinearAddress>>>,
    cache_read_strategy: CacheReadStrategy,
    node_hash_algorithm: NodeHashAlgorithm,
}
```

### Open

At open time, file 0 is opened first and its header is read. File 0
is always `<db_dir>/firewood.db` — its name is fixed and cannot be
changed, since it is the entry point the database open path uses to
discover the rest of the chain. Every other file's location lives in
the previous file's `next_filename` field and can be moved (see
`fwdctl volumes rename`).

The header's `next_filename` field points at the next file in the
chain, whose header points at the file after that, and so on until a
header with no successor is reached (the tail). Every file in this
chain is opened up front. Sealed files are opened read-only; the tail
is opened read-write and receives the `IoUringProxy`.

1. Open file 0 **read-only** (no lock yet), read its
   `NodeStoreHeader`. Opening read-only first means a single-file
   database on a read-only filesystem gets as far as the header read
   without failing, and sealed files in the middle of the chain never
   have to be opened read-write at all.
2. While the most recently opened header has `segment_limit.is_some()`
   (i.e., it is sealed and points at a successor):
   * Resolve `next_filename` relative to the current segment's
     directory.
   * Open the next file **read-only** (no lock — sealed segments are
     never locked), read and validate its header
     (`segment_start == previous.limit + NodeStoreHeader::SIZE`,
     `area_size_hash`,
     `endian_test`, `version`, and `node_hash_algorithm` all match).
   * Append a `Segment { start, limit, fd: SegmentFd::Sealed(file), .. }` to
     the in-memory chain.
3. The loop exits when the most recently opened header has
   `segment_limit.is_none()` — that file is the tail. Drop its
   read-only fd, open the same path read+write, **then** take the
   advisory lock on the read-write fd (this is the only file in the
   chain that gets locked). Initialize the `FileBacked`-level
   `IoUringProxy` (under `#[cfg(feature = "io-uring")]`) bound to the
   tail's fd.
Because no lock was ever taken on the read-only fd of file 0, there is
no lock-upgrade window: the lock is acquired exactly once, on the
read-write fd of the tail. If two processes race to open the same
chain, the second one's `flock` on the tail's read-write fd fails and
open errors out, even if both have already read the header.
4. Build a `Vec<Arc<Segment>>`, convert via `Vec::into_boxed_slice`,
   and publish through `ArcSwap::store`.

If the chain walk encounters a header whose `segment_limit.is_some()` but
whose `next_filename` is missing or unreadable, opening the database
fails — see the recovery open issue.

A pure read-only open mode is a natural follow-up — a flag that skips
the read-write reopen on the tail and disables io_uring entirely. Out
of scope for this change; called out so the structure leaves room for
it.

### Lookup (lock-free for readers)

`segments.load()` returns a guard borrowing `Arc<Box<[Arc<Segment>]>>`
without any locking. Linear scan (N is expected to be small) to pick the segment
whose `[start, limit)` contains `addr`. Compute the file offset and
dispatch through the existing `read_at` / `write_all_at` /
`PredictiveReader` against the chosen segment's fd. Reads on
different segments — and on the same segment — proceed fully in
parallel; the only contention point is the existing per-cache
mutex, which is unchanged.

### Cache

A single global LRU keyed by `LinearAddress` — unchanged. The cache is
oblivious to segment boundaries.

### `size()`

Returns the global high-water mark, which is `NodeStoreHeader::size`
on the tail segment — unchanged semantics.

### Locks

Only the tail's read-write fd holds an advisory lock (via
`UnlockOnDrop`). Sealed segments are read-only and unlocked — multiple
processes can read them concurrently, which is harmless. Opening the
same chain twice fails when the second process tries to lock the
tail's read-write fd.

This also keeps the lock semantics simple during freeze: dropping the
old tail's `Arc<Segment>` from the published slice releases its lock as
soon as the last in-flight reader's guard goes away. There is no need
to "transfer" a lock between fds.

## Freeze API

A new public method on `Db`, plumbed through `RevisionManager` →
nodestore → backend:

```rust
fn freeze_and_extend(&self, next_path: impl AsRef<Path>) -> Result<...>
```

Triggered explicitly by the user. The current high-water mark
automatically becomes the limit for the file being frozen. There is
**no** automatic rollover.

### Steps

`freeze_and_extend` reuses the commit lock — the write lock on
`RevisionManager::in_memory_revisions` that `commit()` holds for its
critical section. This is the right granularity: commits allocate from
the tail (and may write to it via the persist worker), and we cannot
have a commit running while we are sealing the tail and adding a new
one. Holding the same lock means freeze and commit are mutually
exclusive without introducing a second mutex that would have to be
acquired in a fixed order with the commit lock to avoid deadlock.

Out-of-band reads (e.g., `view`, `current_revision`) take a read lock
on the same `RwLock`; they will pause for the duration of freeze, which
is acceptable — freeze is an explicit, infrequent administrative
action.

The commit lock alone is not sufficient. `commit()` enqueues work onto
the `PersistWorker` (a single background thread) and returns; the
worker may still be running a `write_batch` against the tail's
io_uring-registered fd at the moment freeze starts. So freeze must
also quiesce the persist worker before touching anything io_uring
cares about.

1. Acquire the write lock on `in_memory_revisions` (the commit lock).
   New commits are now blocked; they cannot enqueue more work.
2. Call `PersistWorker::wait_persisted()` to drain anything the worker
   has already accepted. After this returns, the worker is idle.
3. Acquire the persist worker's header `Mutex` (`locked_header()`).
   Holding this excludes the worker from starting any further
   `persist_to_disk` / `reap` cycle — defensive belt-and-suspenders
   on top of the empty queue, since steps 6 and 9 below mutate
   `NodeStoreHeader` state.
4. Read the current tail header; let `hwm = header.size`.
5. Create the new file at `next_path` (read-write) and take its
   advisory lock — this fd will own the chain's only lock once the
   old tail is demoted.
6. Write a fresh `NodeStoreHeader` to the new file with:
   * `version = 2` — see the lazy version bump in Header Changes.
   * `segment_start = Some(hwm + NodeStoreHeader::SIZE)` — the
     dead-space gap (see Address Model) lives between the old tail's
     `segment_limit` (`hwm`) and the new tail's `segment_start`.
   * `segment_limit = None`
   * `next_filename = zeros`
   * **Empty** `free_lists`. Free lists are *not* copied from the old
     tail. Old free-list entries point into the now-frozen file; we
     don't want to keep referencing those addresses, and we don't want
     to write to the frozen file. Old free lists are simply abandoned.
   * `root_address` / `root_hash` copied from the current tail (the
     root may live anywhere; it doesn't move).
   * `node_hash_algorithm`, `area_size_hash`, `version` copied from
     the current tail.
   * `size = hwm + NodeStoreHeader::SIZE` — the new tail's
     high-water mark starts at its own `segment_start`, so the next
     allocation lands at the first body byte just past the on-disk
     header.
7. fsync the new file.
8. fsync the parent directory of the new file. This is required by
   POSIX for the new file's directory entry to survive a crash — a
   crash after step 9 lands durably but before the directory entry
   reaches disk would leave the old tail's header pointing at a path
   that no longer exists, which is the "next_filename set, file
   missing" recovery case.
9. Update the old tail header in place: set `version = 2` (if it
   was still v1 — i.e., this is the database's first freeze),
   `segment_limit = Some(hwm)`, and
   `next_filename = <relative path>`. fsync.
10. Build the new `Box<[Arc<Segment>]>` and publish it atomically:

    * Re-open the previously-tail file **read-only** on a fresh fd
      (no lock — sealed segments are unlocked) and construct a fresh
      `Arc<Segment>` for the demoted entry with `limit = Some(hwm)`,
      `next_filename = <relative path>`, and `fd: SegmentFd::Sealed(file)`.
    * Construct a fresh `Arc<Segment>` for the new tail with the
      read-write fd and `limit = None`.
    * Build a `Vec<Arc<Segment>>` of length `N + 1`: clone the `Arc`s
      for segments `0..N-1` from the current slice (cheap pointer
      bumps), then push the demoted old-tail `Arc` and the new tail
      `Arc`. Convert via `into_boxed_slice` and `ArcSwap::store`.
    * Under `#[cfg(feature = "io-uring")]`, update the registered fd
      inside `IoUringProxy` to the new tail's fd (either via
      `IORING_REGISTER_FILES_UPDATE`, or by replacing the inner
      `IoUring` wholesale through the proxy's existing internal
      `Mutex`). Because step 2 drained the persist worker and step 3
      is holding its header `Mutex`, no `write_batch` is in flight or
      can start until we release. The previously-registered fd
      remains valid in the kernel until the old `Arc<Segment>` is
      dropped by the last in-flight reader.

    Concurrent readers that loaded the previous slice keep using the
    previous `Arc<Segment>` for the old tail — including its locked
    read-write fd — until they drop their guard. That is harmless: no
    reader path writes through the fd, and the kernel keeps the inode
    open for as long as either fd is held. The lock on the old fd is
    released only when the last in-flight reader drops the previous
    slice; until then, an outside process trying to open the chain will
    see the lock and fail, which is the desired behavior. New readers
    loading after the `store` see the read-only, unlocked fd for the
    demoted segment and the new tail's locked read-write fd.
11. Release the header `Mutex`, then the commit lock. The persist
    worker is now free to resume — its next `persist_to_disk` cycle
    will see the updated header on the new tail and submit
    `write_batch`es against the newly registered fd.

### Persist worker quiescence

`wait_persisted` exists today but is `#[cfg(test)]` at every layer:

* `PersistChannel::wait_all_released` (`firewood/src/persist_worker.rs`)
* `PersistWorker::wait_persisted` (same file)
* `RevisionManager::wait_persisted` (inside `mod tests` in
  `firewood/src/manager.rs`)

The implementation is production-safe — it's a condvar wait on the
same `commit_not_full` condvar that the worker already pulses after
every persist completion, with no polling and no overhead when nobody
is waiting. The 1-minute timeout that emits a warning every minute the
drain is still pending is also a useful production diagnostic.

**Prerequisite for `freeze_and_extend`:** remove the `#[cfg(test)]`
gates on `PersistChannel::wait_all_released` and
`PersistWorker::wait_persisted`, and move `RevisionManager::wait_persisted`
out of `mod tests` into the main impl. Visibility stays `pub(crate)`.
This is a mechanical change with no behavior impact; it lands as its
own dedicated commit (commit 3 in the breakdown) ahead of the freeze
API itself.

**Ordering inside freeze.** Step 2's drain only terminates if the
worker is healthy. If the worker has previously errored, it stops
processing and will never release the outstanding permits, so
`wait_persisted` would hang forever. Freeze must therefore call
`persist_worker.check_error()` *before* `wait_persisted`, returning
any latched error to the caller — exactly the pattern `commit()`
already uses at the top of its critical section. This refines step 2
to:

```rust
2a. self.persist_worker.check_error()?;
2b. self.persist_worker.wait_persisted();
```

A failing `check_error()` aborts freeze cleanly with the underlying
`PersistError`. A successful `check_error()` followed by
`wait_persisted()` is guaranteed to make progress because the worker
is alive and the commit lock blocks any further enqueues.

### Path encoding

`next_filename` is computed as `next_path` made relative to the old
tail file's directory. Error out if it cannot be expressed as a
relative path whose encoded bytes fit in 256 bytes.

### Concurrency with concurrent openers

`freeze_and_extend` runs inside an already-open `Db`, so the calling
process already holds the tail's `flock` for the entire freeze. Another
process attempting `Db::open` on the same chain races only against the
old-tail header write in step 9 — it cannot reach the "lock the tail"
step without first being blocked by us. The possible outcomes for the
racing opener are:

1. **Pre-step-9 read.** Header says `segment_limit = None`. The opener
   treats the file as the tail, reopens read-write, and tries to
   `flock` — fails because we hold the lock.
2. **Post-step-9 read.** Header says `segment_limit = Some(hwm)` with
   `next_filename` pointing at the new file. The opener walks to the
   new file, sees it as the tail, and tries to `flock` — fails because
   step 5 already locked it.
3. **Torn read.** A pwrite of the 2048-byte header is not guaranteed
   atomic per POSIX, though in practice modern filesystems with their
   default settings (ext4 with `data=ordered`, xfs, btrfs, zfs, apfs)
   provide sector-level atomicity that covers a 2048-byte aligned
   write — i.e., the reader sees either the pre- or post-image, never
   a mix. The opener may still read a partially updated header on
   filesystems that don't guarantee this, so header validation must
   reject this rather than blindly walking to a garbage filename.

In all three cases the outside open fails or blocks, which is correct.

To guard against case 3, header validation on every read must be
strict — at minimum the existing `endian_test`, `version`,
`area_size_hash`, and `node_hash_algorithm` consistency checks, plus
the new `segment_start == previous.limit + NodeStoreHeader::SIZE`
cross-check between
adjacent segments. Any torn write that survives all of these is
indistinguishable from valid state, but the combination is robust in
practice. A retry-on-validation-failure policy on `Db::open` (re-read
the header once after a brief delay before giving up) is a cheap
defensive add and worth considering, though it's not required for
correctness — failing the open and letting the caller retry is a
valid outcome.

### Crash safety

The fsync ordering — write + fsync the new file (step 7), fsync the
parent directory (step 8), then update + fsync the old tail header
(step 9) — guarantees that on recovery, a durable
`segment_limit.is_some()` always implies the next file's directory
entry is also durable. Without the directory fsync at step 8, a crash
could leave the old tail pointing at a path the filesystem can no
longer locate.

The 2048-byte `NodeStoreHeader` pwrite at step 9 is not atomic per
POSIX, but in practice modern filesystems with their default settings
(ext4 `data=ordered`, xfs, btrfs, zfs, apfs) deliver sector-level
atomicity that covers a 2048-byte aligned write — readers see either
the pre- or post-image, not a mix. The strict header validation on
read (existing `endian_test`/`area_size_hash`/`version`/
`node_hash_algorithm` plus the new
`segment_start == previous.limit + NodeStoreHeader::SIZE`
cross-check) catches any torn write that does slip through on
filesystems without that guarantee.

**Orphan files.** A crash between step 7 and step 9 leaves the new
file on disk but unreferenced — the old tail's header still says
`segment_limit = None`. On the next open, the database walks as a
single-file chain and the stray file is ignored. Retrying
`freeze_and_extend` at the same path is rejected by the
"reject if path exists" guard, so the operator must either pick a
different path or delete the orphan manually before retrying.
Acceptable for v1; a future opportunistic "adopt an orphan that
matches the expected
`segment_start = current_hwm + NodeStoreHeader::SIZE` and otherwise
looks valid" mode is possible but out of scope here.

Detailed recovery handling for the harder cases (`segment_limit` set
but next file missing/short, join partway through) is deferred
to a dedicated fsck/repair pass — see open issues.

## Allocator — `storage/src/nodestore/alloc.rs`

The allocator only ever bumps `size` in the tail header. No automatic
rollover; the freeze API is the only thing that creates new segments.

### Reaping

When reaping a revision walks the deleted-node addresses, for each
address:

* If it falls in the tail segment's range, normal free-list insertion.
* Otherwise (it falls in a frozen segment), **drop silently**. Frozen
  files are read-only; their space is never reclaimed.

This is implemented as a simple range check against the last entry of
`segments`.

## Persist — `storage/src/nodestore/persist.rs`

* Non–io_uring write path: route writes through the segmented backend;
  the change is transparent to callers.
* `write_batch` (io_uring): assert all writes in a batch fall in the
  tail segment. This always holds because the allocator only writes to
  the tail. The single ring lives on `FileBacked`; the tail's fd is
  re-registered with the kernel as part of freeze (see Freeze API).

## Checker / FFI / fwdctl

* Checker walks via `ReadableStorage` only — works unchanged once the
  backend handles segmented ranges.
* FFI and `fwdctl` continue to take the database **directory** path.
  File 0 is always named `firewood.db` inside that directory; the
  rest of the chain is discovered from file 0's header.
* `Db::freeze_and_extend` is exposed via FFI in commit 6 — single
  `fwd_freeze_and_extend` entry point taking the handle and the new
  file's path.

### `fwdctl volumes` subcommands

All `fwdctl volumes` subcommands acquire the chain's single advisory
lock on the tail file, so they only run when nobody else has the
database open. They operate offline — no other process may have the
tail locked. Sealed files are unlocked and accessed read-only.

`join`, `new`, and `rename` may also need to mutate a sealed file
(rewriting its header during join, flipping
`segment_limit`/`next_filename` on a previously-tail file during
`new`, or updating the previous segment's `next_filename` during
`rename`). To do that, they re-open those files read-write for the
duration of the subcommand. They do **not** need to lock those fds —
the chain-level lock on the tail already excludes any other database
opener, and `fwdctl` is single-process for the duration of the
subcommand.

#### `fwdctl volumes [--db <DB_DIR_NAME>] ls`

Walks the chain starting at file 0 and prints a table with one row
per volume. Suggested columns:

* `#` — segment index (0 is the head)
* `path` — relative path as recorded in the previous segment's
  `next_filename`, or `firewood.db` for file 0
* `start` — `segment_start` (blank for file 0)
* `limit` — `segment_limit` (blank for the tail)
* `size` — current file length on disk
* `state` — `sealed` or `tail`

#### `fwdctl volumes [--db <DB_DIR_NAME>] join <FIRST> [<SECOND>] [--keep]`

Concatenates two adjacent volumes into one. `<FIRST>` and `<SECOND>`
identify volumes either by **integer index** (`0`, `1`, …) or by
**path** (relative to the database directory or absolute). Indices and
paths can be mixed: `join 0 firewood.v00001` is equivalent to
`join 0 1` if those refer to the same file.

If `<SECOND>` is omitted, it is deduced as the volume immediately
after `<FIRST>` in the chain. Adjacent-only: `<SECOND>` must be
`<FIRST> + 1` in chain order; reject otherwise.

By default the joined-in second volume is deleted from disk after the
join completes. Pass `--keep` to leave it in place (the chain no
longer references it; treat it as an orphan to clean up later).

The dead-space gap between segments (see Address Model) is what makes
this trivial — `<SECOND>`'s on-disk header lands in `<FIRST>`'s file
at offsets corresponding to the gap addresses, which were never
allocated to begin with. Body addresses formerly in `<SECOND>` keep
the same `LinearAddress` and resolve correctly via the joined file's
header.

Steps (call the resolved volumes `i` and `j`, with `j == i + 1`):

1. Acquire the chain's advisory lock on the tail file.
2. Resolve `<FIRST>`/`<SECOND>` to chain indices. Re-read both
   headers; reject if `j.segment_start != i.segment_limit +
   NodeStoreHeader::SIZE` or if any other header field disagrees with
   the in-memory chain walk.
3. Append volume `j`'s entire on-disk content (header + body, every
   byte from offset 0 to its end-of-file) to volume `i`'s end-of-file.
   fsync.
4. Update volume `i`'s header:
   * If `j` was the tail (`j.segment_limit.is_none()`):
     * `i.segment_limit = None`
     * `i.next_filename = zeros`
     * `i.free_lists = j.free_lists` (free-list addresses already
       lived inside `j`'s body, which is now part of `i`'s file; the
       global `LinearAddress` values are unchanged).
     * `i.root_address = j.root_address`,
       `i.root_hash = j.root_hash`,
       `i.size = j.size`
   * If `j` was sealed:
     * `i.segment_limit = j.segment_limit`
     * `i.next_filename = j.next_filename`
     * `i.free_lists` unchanged (volume `i` was already sealed; its
       free lists were already abandoned and remain so).
     * `i.root_address`/`i.root_hash`/`i.size` unchanged.
5. fsync volume `i`.
6. Unless `--keep` is set, delete volume `j` from disk.

Crash safety: if the process dies after step 3 but before step 4,
volume `i`'s file is longer than `i.segment_limit` would imply but
its header still says it's sealed at the old limit. The dead-space
gap between segments means the trailing bytes are simply ignored on
the next open — header validation walks via `next_filename`, never by
file size. Recovery from this state therefore just requires retrying
the header update; detailed handling is deferred (see open issues).

#### `fwdctl volumes [--db <DB_DIR_NAME>] new <RELATIVE_PATH>`

Adds a new tail volume to a database that is offline. Equivalent to
calling `Db::freeze_and_extend(<RELATIVE_PATH>)` on a quiesced
database, but performed by the tool without going through the full
`Db` open path.

Steps:

1. Acquire the chain's advisory lock on the current tail file. The
   new tail's read-write fd (created at step 4) becomes the chain's
   single locked fd.
2. Walk the chain to find the current tail; let `hwm = tail.size`.
3. Reject if `<RELATIVE_PATH>` resolves to an existing file.
4. Create the new file (read-write) and take its advisory lock.
5. Write a fresh `NodeStoreHeader` with `version = 2`,
   `segment_start = Some(hwm + NodeStoreHeader::SIZE)` (skipping the
   dead-space gap, matching `Db::freeze_and_extend`),
   `segment_limit = None`, `next_filename = zeros`, empty
   `free_lists`, copied `root_address` / `root_hash` /
   `node_hash_algorithm` / `area_size_hash`, and
   `size = hwm + NodeStoreHeader::SIZE`. fsync. Then fsync the parent
   directory so the new file's directory entry is durable before the
   next step.
6. Update the previous tail's header in place: set `version = 2` (if
   it was still v1), `segment_limit = Some(hwm)`, and
   `next_filename = <RELATIVE_PATH>`. fsync.
7. Release the previous tail's advisory lock by closing its
   read-write fd. The previous tail is now sealed and unlocked,
   matching the in-memory invariant maintained by
   `Db::freeze_and_extend`.

This shares an implementation with `Db::freeze_and_extend` — both
call into the same nodestore-level routine.

#### `fwdctl volumes [--db <DB_DIR_NAME>] rename <i> <NEW_FILENAME> [--no-mv]`

Renames volume `i` to `<NEW_FILENAME>` and updates the chain so the
previous volume's `next_filename` points at the new path. Useful when
an operator wants to move a volume to a different directory (e.g.,
relocating older sealed volumes onto a shared network drive).

`<NEW_FILENAME>` is interpreted relative to volume `i-1`'s directory —
the same encoding rule the chain uses everywhere. By default the
subcommand does the filesystem move itself; with `--no-mv` it
assumes the operator has already placed the file at `<NEW_FILENAME>`
out of band and only updates the chain metadata.

**`--no-mv` requires the original file to still exist at its current
chain location.** The subcommand starts by walking the chain to
acquire the tail lock and validate the request, and the chain walk
opens every volume by following `next_filename`. If the operator
has already deleted the original, the open fails before `rename`
gets a chance to run.

The intended workflow for `--no-mv` is therefore *copy, then rename,
then delete* — not move:

1. Copy volume `i` to its new location, leaving the original in
   place.
2. Run `fwdctl volumes rename <i> <new_path> --no-mv`. The chain
   walk still succeeds because the original is still where the
   header says it is; once the chain metadata is updated to point at
   the copy, the original becomes unreferenced.
3. Delete the original.

The typical use case is a database whose head sits on a read-only
shared resource alongside an older volume, and the operator wants
to relocate the older volume onto writable local storage — copy it
locally, then run `--no-mv rename` to repoint the chain at the
local copy.

Volume 0 cannot be renamed: file 0 is always `<DB_DIR_NAME>/firewood.db`
and is the entry point for opening the chain. Reject with a clear
error.

Steps:

1. Acquire the chain's advisory lock on the tail file.
2. Walk the chain to find volume `i`; reject if `i == 0` or `i` is
   out of range.
3. Reject if `<NEW_FILENAME>`, resolved relative to volume `i-1`'s
   directory, refers to an existing file (and, in the default
   non-`--no-mv` mode, refers to a path different from volume `i`'s
   current location).
4. Default mode: filesystem-move volume `i` from its current
   location to `<NEW_FILENAME>`. Use `rename(2)` for same-filesystem
   moves; fall back to copy + fsync + unlink for cross-filesystem
   moves. fsync both source and destination directories afterward.
   `--no-mv` mode: skip this step; trust that the file already lives
   at `<NEW_FILENAME>`. Verify the file exists and its header
   matches volume `i`'s expected `segment_start`/`segment_limit`
   before proceeding, to fail closed if the operator moved the
   wrong file.
5. Update volume `i-1`'s header in place: set its `next_filename`
   to `<NEW_FILENAME>`. fsync. fsync volume `i-1`'s parent
   directory.

Crash safety: if the process dies after step 4 (move done) but
before step 5 (header update), volume `i-1`'s `next_filename` still
points at the old path — which no longer exists. The default-mode
fallback for cross-filesystem moves leaves the original file in
place until step 5 succeeds (copy first, unlink last), so for that
path the chain remains valid through any single crash. For
same-filesystem `rename(2)`, the move is atomic but the chain
metadata isn't yet updated — the database opens with a "file
missing" error on volume `i`, and recovery requires either moving
the file back or completing the metadata update with `--no-mv`.

This and `join` share most of their crash-recovery story —
both are deferred to the same fsck/repair pass (see open issues).

## Suggested commit breakdown

The split is "read everything, write nothing" first, then "create and
manipulate." Each commit ships with the tests for the surface it
introduces.

### Commit 1 — read-side multi-file support (Rust)

Everything needed to *open and read* a multi-file chain. Nothing in
this commit creates additional files, so single-file databases
continue to behave exactly as today and a hand-crafted multi-file
fixture is the only way to exercise the new code paths.

Includes:

* New `NodeStoreHeader` fields: `segment_start`, `segment_limit`,
  `next_filename`. Strict validation on read, including torn-write
  detection (existing `endian_test`, `area_size_hash`, `version`,
  `node_hash_algorithm` plus the new
  `segment_start == previous.limit + NodeStoreHeader::SIZE`
  cross-check).
* Version-aware reader. v1 headers are accepted only if all three
  new fields are zero (the single-file invariant); a v1 header with
  non-zero new fields is rejected as corrupt. v2 headers are
  accepted with non-zero new fields and drive the chain walk. No
  v2 *writer* in this commit — only freeze (commit 4) ever produces
  v2 on disk; commit 1's v2 acceptance is what lets later commits'
  hand-rolled fixtures and freeze output be readable.
* `Segment` type and `SegmentFd` enum (`Tail(UnlockOnDrop)` for the
  writable, locked tail vs. `Sealed(File)` for read-only frozen
  segments).
* `FileBacked` reshaped around `ArcSwap<Box<[Arc<Segment>]>>`. The
  vector is still always length 1 in practice.
* Eager chain walk at `Db::open`: read file 0 read-only, walk the
  chain through every sealed segment (read-only, unlocked), reopen
  the tail read-write and lock it, attach the existing
  `IoUringProxy` to the tail's fd.
* Lookup dispatch: address → segment via linear scan, dispatch
  read/`PredictiveReader` against the chosen segment's fd.
  Cross-boundary reads/writes return an error.

Tests:

* Single-file v1 database (zeroed `segment_*` fields) opens as a
  one-segment chain unchanged. Backwards-compat sanity, including
  tail-lock acquired and io_uring attached to the tail's fd.
* Header round-trip: serialize/deserialize the new fields; zeroed
  `next_filename` parses as "no next file."
* Header validation: rejects mismatched `area_size_hash`,
  `endian_test`, `version`, `node_hash_algorithm`. Tested at the
  unit level on synthetic header bytes.
* Version validation: a v1 header with any non-zero new field is
  rejected; a v2 header with all-zero new fields opens as a
  single-segment chain (legal post-freeze quiescent state); an
  unknown version is rejected.
* Cross-segment lookup logic at the unit level: hand-build a
  `Box<[Arc<Segment>]>` with two `Segment`s pointing at the same
  scratch file with synthetic `start`/`limit` values, exercise the
  address → segment dispatch and cross-boundary error paths
  without producing a real two-file database on disk.

The "real" multi-file open / lock / read-across-boundary tests
live in commit 4, where `freeze_and_extend` produces the fixture
naturally instead of via hand-crafted file layouts.

### Commit 2 — `fwdctl volumes ls`

Read-only inspection tool. Walks the chain (sharing the same
read-side code path commit 1 introduces) and prints the table
described in the `fwdctl volumes ls` section. Lands as a small
separate commit because it has no dependency on the write side and
is convenient for operators verifying commit 1 in production.

Tests:

* `ls` on a single-file db prints exactly one row with the
  expected `state = tail` and blank `start`/`limit`.

Multi-file `ls` output is verified in commit 4 once
`freeze_and_extend` exists to produce the chain.

### Commit 3 — persist worker visibility

A small, mechanical extraction. Removes `#[cfg(test)]` gates so
`freeze_and_extend` (commit 4) can call the drain helpers from
production code:

* Drop `#[cfg(test)]` from `PersistChannel::wait_all_released` in
  `firewood/src/persist_worker.rs`.
* Drop `#[cfg(test)]` from `PersistWorker::wait_persisted` in the
  same file.
* Move `RevisionManager::wait_persisted` from `mod tests` into the
  main impl in `firewood/src/manager.rs`. Visibility stays
  `pub(crate)`.

No behavior change — the implementations are already production-safe
(condvar wait on the existing `commit_not_full` pulse, with a
1-minute warning timeout). Lands as its own commit so commit 4's
diff stays focused on freeze itself.

Tests: existing `wait_persisted` tests continue to pass; no new
tests required at this layer.

### Commit 4 — `freeze_and_extend` and allocator

The actual chain-extension machinery and the only writer that
produces v2 headers on disk. Depends on commits 1 and 3.

Includes:

* `Db::freeze_and_extend` plumbed through `RevisionManager` →
  nodestore → `FileBacked`. Implements the full step sequence:
  commit lock, `check_error`, `wait_persisted`, `locked_header`,
  write new file (v2, with all chain fields populated), fsync,
  fsync parent directory, update old tail header in place
  (flipping `version` from 1 to 2 on first freeze, setting
  `segment_limit` and `next_filename`), fsync, publish new
  `Box<[Arc<Segment>]>`, update io_uring registered fd, release
  locks.
* **Lazy v2 bump writer.** Both header writes use `version = 2`.
  This is the first and only commit that writes a v2 header to
  disk; commit 1's reader has accepted v2 since it landed.
* Allocator: reaping is a no-op for addresses outside the tail
  segment's range (only meaningful once freeze can produce frozen
  segments).
* io_uring fd-update path under `#[cfg(feature = "io-uring")]`.

Tests (the bulk of multi-file integration testing lives here,
since this is the first commit that can produce multi-file
databases):

* Populate, `freeze_and_extend`, write more, reopen, verify all
  keys readable across the freeze boundary.
* After freeze + reopen: tail's fd is RW and locked, sealed
  segment's fd is RO and unlocked, cross-boundary reads error,
  cross-boundary lookups dispatch to the correct segment.
* Three-file chain: freeze twice, reopen, verify chain walk and
  reads.
* Tail-lock contention: opening a frozen-and-extended chain twice
  fails on the new tail's lock.
* Sealed segments are unlocked: a second process can open any
  sealed file read-only while the chain is open by the first.
* Torn-header read on the previous tail during freeze (synthesize
  via fault injection on the step-9 header pwrite) returns a
  validation error to the racing opener — never a garbage chain
  walk.
* Lazy version bump: a freshly created database has `version = 1`
  in its single header; after the first `freeze_and_extend`, both
  the (now-sealed) old tail and the new tail have `version = 2`;
  a second `freeze_and_extend` leaves all `version` fields at 2.
* Old-firewood compatibility (simulated): a v1 reader stub rejects
  any header it inspects post-freeze; the same stub continues to
  accept the original single-file v1 database before freeze. (Pure
  unit test — we don't actually link an old binary.)
* `fwdctl volumes ls` on a freeze-produced multi-file chain
  prints rows with correct `start`/`limit`/`size`/`state`.
* Reap a revision that spans the freeze: confirm freed addresses
  in the frozen file are dropped silently, addresses in the tail
  are recycled.
* Concurrent commit + freeze: freeze blocks until in-flight
  persist drains; after freeze, the next commit's `write_batch`
  targets the new tail's registered fd.
* io_uring fd update is observable (e.g., a `write_batch` after
  freeze writes to the new file, not the old).

### Commit 5 — `fwdctl volumes new`, `join`, and `rename`

The offline administrative subcommands. Each shares nodestore-level
helpers with `freeze_and_extend` where it can (`new` is essentially
the offline-equivalent entry point) and is purely additive — no
runtime path depends on them.

Includes:

* `fwdctl volumes new <RELATIVE_PATH>` — invokes the same
  underlying nodestore routine as `Db::freeze_and_extend` against a
  quiesced database, writing v2 on both the new tail and the
  previously-tail file.
* `fwdctl volumes join <FIRST> [<SECOND>] [--keep]` — concatenates
  two adjacent volumes; relies on the dead-space gap to keep body
  addresses stable.
* `fwdctl volumes rename <i> <NEW_FILENAME> [--no-mv]` — relocates
  a volume and updates volume `i-1`'s `next_filename`.

All three acquire the chain's tail-file advisory lock at start and
release it at end, and share the chain-walk helper with `volumes
ls`. Lands as its own commit so commit 4's diff stays focused on
the runtime freeze path.

Tests:

* `fwdctl volumes new` matches `freeze_and_extend` semantics on a
  quiesced database (round-trip with subsequent `Db::open` reads).
* `fwdctl volumes join` round-trip: freeze, join, reopen, verify
  all keys still readable. Cover both `--keep` and default-delete
  modes. Verify body addresses formerly in the joined-in segment
  still resolve to the same node via the dead-space-gap math.
* `fwdctl volumes rename` round-trip in both default and `--no-mv`
  modes. Verify volume 0 cannot be renamed and that a rename
  preserves chain invariants on reopen.
* Lock contention: each subcommand fails cleanly if another
  process holds the chain's tail lock.

### Commit 6 — FFI: expose `freeze_and_extend`

Adds a single C entry point in `ffi/src/lib.rs` and the matching Go
wrapper in `ffi/firewood.go`. Open and read paths need no change —
they already go through `Db::open`, which the read-side commits made
chain-aware. The chain is discovered from file 0's header (always
`<db_dir>/firewood.db`), so callers continue to pass a single
directory path.

Includes:

* `pub extern "C" fn fwd_freeze_and_extend(db: Option<&DatabaseHandle>,
  next_path: BorrowedBytes) -> VoidResult`. Validates the handle,
  interprets `next_path` as raw OS path bytes, and calls
  `Db::freeze_and_extend`. Errors propagate through the existing
  `VoidResult` convention used by `fwd_close_db` and friends.
* Go wrapper method on `Db`, e.g. `func (db *Db) FreezeAndExtend(path
  string) error`, mirroring the FFI signature and using the existing
  `cgo`/error-conversion helpers in `firewood.go`.
* Regenerate the C header and `cgo` wrappers per the FFI build steps
  documented in `CLAUDE.md` (`cargo build` in `ffi/src/`, then
  `go tool cgo firewood.go` in `ffi/`).

Tests:

* Rust-side test invoking `fwd_freeze_and_extend` through the FFI
  surface (mirroring how other handle-taking exports are tested).
* Go-side test in `ffi/`: open a db, write some keys, call
  `FreezeAndExtend`, write more, reopen via Go, verify all keys are
  readable. Mirrors the Rust integration test from commit 4 but
  exercises the cgo path.
* Error-path test: invalid path (e.g., empty string, >256 bytes once
  made relative) returns the expected error.

### Commit 7 — auto-detect concatenated files at open

Adds the embedded-segment recovery / convenience feature described
in "Concatenated files (auto-detected at open)" in the Address Model.
With this commit, a file whose on-disk size exceeds its parent
segment's expected size has its trailing bytes interpreted as the
next segment's header + body, and the in-memory `Segment` is widened
to absorb them.

Purely additive — commits 1–6 already tolerate trailing bytes by
ignoring them, so `join` is crash-safe even without this commit.
Commit 7 turns those trailing bytes into a usable chain extension
instead of dead weight.

Includes:

* Open-time chain walk gains an inner loop: after each header read,
  compare actual file size to expected file size; on excess,
  read+validate the candidate embedded header, fold it into the
  in-memory `Segment` (widen `segment_limit`, adopt
  `next_filename`, adopt tail-only fields when the embedded segment
  was the tail), and repeat until they match. Then continue the
  outer `next_filename` walk.
* Strict validation of embedded headers — any trailing-bytes prefix
  that doesn't parse as a valid embedded header is treated as
  corruption (open fails) rather than silently truncated.
* No changes to `Segment`, `SegmentFd`, the lookup formula, or the
  on-disk format. The merge is invisible to everything above the
  open path.

Tests:

* `cat <next> >> <prev>` followed by deletion of `<next>`: open
  succeeds, the chain reflects the merged segment, all keys in both
  former segments are readable.
* Half-completed `join` (simulate by performing step 3's
  byte-append without step 4's header rewrite, then crash): open
  detects the embedded segment and reads through.
* Embedded body lookup hits the right node — allocate a known key
  in `<next>` before concatenation, then read it back through the
  merged file.
* Multi-level embedding: concatenate three files into one, verify
  open folds all of them.
* Trailing garbage that is not a valid embedded header fails open
  with a clear error (does not silently truncate).
* Concatenating a sealed-then-tail pair: the merged `Segment`
  inherits the embedded tail's `root_address` / `root_hash` /
  `free_lists` / `size`, and subsequent allocations land in the
  combined file.

## Alternatives Considered

### No link back to the previous segment's path

An earlier draft had each segment's header carry a `prev_filename`
field mirroring `next_filename`, so the chain could be walked in
either direction. We removed it. The forward-only chain is enough
because:

* **The open path is inherently forward-only.** Every open starts at
  the fixed entry point `<db_dir>/firewood.db` and follows
  `next_filename` until it reaches the tail. Nothing in that walk ever
  needs to ask "what came before me?" — the caller already knows,
  because it just came from there.
* **Address-space ordering comes from `segment_start`, not from a back
  pointer.** A segment's place in the chain is fully determined by its
  `segment_start` / `segment_limit` pair. Given the in-memory
  `Box<[Arc<Segment>]>` (which the open path builds up front), the
  predecessor of any segment is just the previous slot in the slice —
  no on-disk field needed.
* **No subcommand needs reverse traversal.** `fwdctl volumes ls` /
  `join` / `rename` / `new` all operate on the whole chain after
  walking it from the head, so they have the predecessor in hand for
  free. `rename` updates the *previous* segment's `next_filename`, but
  it knows which segment that is from the chain walk it just did, not
  from a back-link.
* **A back pointer doubles the freeze write set.** Today freeze
  rewrites exactly one on-disk header (the old tail's, in step 9).
  With a `prev_filename`, the new file's header would also have to be
  patched after the old tail is sealed — or, worse, the old tail's
  rewrite and the new file's header initialization would have to be
  atomic with respect to each other. The forward-only design keeps
  freeze as a single-header-update commit point.
* **`rename` would have to update two headers instead of one.** Today
  renaming volume `i` only touches volume `i-1`'s `next_filename`. A
  back pointer would force `rename` to also rewrite volume `i+1`'s
  `prev_filename`, doubling the fsync count and the crash-recovery
  surface for a subcommand whose only job is metadata edits.
* **It would mildly complicate `join`'s dead-space-gap story.** A
  back pointer in the joined-in segment's embedded header would land
  inside the gap as harmless dead bytes, the same as the rest of the
  embedded header. That's fine, but it's one more field whose value
  becomes silently stale post-join, and the absence of the field
  removes the question entirely.

The cost of not having it is essentially zero: any caller that wants
the predecessor consults the in-memory chain, which is always
populated by the time anything interesting happens.

## Open Issues

* **Repairs if a crash happens while extending.** Behavior when
  `next_filename` is set but the file is missing/short, or when the
  new file exists but the old file's `segment_limit` update didn't
  land. Current plan: hard error on open. A dedicated fsck/repair
  pass is a possible follow-up PR.
* **Free-list space loss.** Reaping into a frozen segment drops
  addresses, so frozen files' fragmented space is permanently
  unreclaimable. Two reasons this is acceptable for v1:

  1. **The same loss already exists today at shutdown.** Only the
     most recent revision's free-list state is recorded in the
     header, so any space freed by older in-memory revisions that
     hasn't yet been promoted to the persisted free list is already
     lost when the database closes. A planned follow-up will flush
     these on graceful shutdown, but the wasted amount has been
     judged small enough that it has not been prioritized.
  2. **Marking the blocks as freed wouldn't actually save anything
     once a segment is frozen.** Frozen segments are read-only and
     will never have new allocations written into them. Whether the
     free-list pointer survives or not, the bytes on disk are equally
     unused. Reclamation only happens when the entire segment is
     retired wholesale, which is the intended lifecycle for older
     frozen volumes.

  Worth documenting; not blocking.
* **Automatic rollover (future enhancement).** Today
  `freeze_and_extend` is purely manual. A future enhancement could
  add a `max_file_size: Option<u64>` `DbConfig` knob that
  auto-rolls when the tail exceeds the threshold, plus a
  `rollover_template: Option<String>` (using `tinytemplate`-style
  placeholders such as `{db_dir}` and `{depth | zeropad5}`) for
  generating the new tail's filename. Default template suggestion:
  `"{db_dir}/firewood.v{depth | zeropad5}"`. Out of scope for v1
  because the manual API covers the immediate use cases and the
  template/threshold story benefits from real operator feedback
  before being baked into the config schema.
* **Checker validation of volume layout (future enhancement).**
  `fwdctl check` could grow per-volume validation that verifies each
  segment's actual file size matches its expected size
  (`NodeStoreHeader::SIZE + (segment_limit − segment_start)`),
  flags trailing bytes that don't parse as a valid embedded segment,
  and confirms the chain invariant
  `next.segment_start == prev.segment_limit + NodeStoreHeader::SIZE`
  across every adjacent pair. Useful as a pre-flight check before
  destructive operations and as a diagnostic when a chain fails to
  open.
* **`fwdctl volumes ls` shows concatenated-file detail (future
  enhancement).** Once commit 7 lands, a single file may back
  multiple logical segments. `ls` could surface this — e.g., an
  extra column or indented sub-rows showing each embedded segment's
  `start`/`limit`/`size` and which file backs it — so an operator
  can see at a glance whether a volume is plain or contains an
  embedded chain. Useful follow-up to commit 7; not required for it
  to land.
* **Validator-node pruning (future enhancement).** Validator nodes
  could lazily prune older sealed segments by copying still-reachable
  reads from the archive into the tail and tracking files with zero
  in-degrees so they can be removed. This would also need to handle
  the case where the tail tracks pruning state for older files: when
  the tracker itself is pruned, its tracking list copies to the new
  tail. Crucially, once non-tail files are subject to pruning, the
  `root_address` field in a non-tail header is not authoritative —
  it must be corroborated against an external Root Store. Out of
  scope for v1, which only covers static archival (sealed segments
  are never modified or removed except via `join` or `rename`); v1
  treats non-tail `root_address` as "not meaningful" without
  needing a Root Store.
