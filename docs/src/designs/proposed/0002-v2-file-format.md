---
status: proposed
author: Brandon LeBlanc
created: 2026-06-25
tracking-issue: https://github.com/ava-labs/firewood/issues/804
---

# v2 On-Disk File Format

## Summary

A version 2 (v2) on-disk file format for Firewood, a clean break from v1 (no v1
backward compatibility) designed for zero-deserialization reads on the traversal
hot path — `pread` into an aligned buffer in v2.0 and true zero-copy through `mmap`
in v2.1 — while preserving Firewood's defining property: a compaction-less
store in which a node's address is its direct byte offset on disk. The format is
frozen for all of major version 2 and is built so that within major version 2 any
reader reads any file, in both directions, with no format change — a change that
would break this is by definition a v3 event — so the next migration on a
multi-terabyte state file is deferred as far as possible.

## Motivation

The v1 format serializes each trie
node with a variable-length encoding that must be parsed field-by-field on every
read, and it bakes the area-size table into the binary (a mismatch is rejected at
open). Two pressures motivate a new format:

- **Read cost.** Path traversal is Firewood's dominant operation. A format whose
  nodes can be reinterpreted directly from mapped bytes — with the data traversal
  needs (child addresses) separated from the data it does not (hashes, values) —
  removes per-node deserialization from the hot path.
- **Migration cost.** A production state file is already in the 2 TiB range, and a
  format migration on a file that size is extremely expensive. v2 must therefore
  be future-proof enough to evolve *within* major version 2 without forcing
  another migration, which v1's compiled-in assumptions and ad-hoc encoding cannot
  support.

The four tenets, in priority order:

1. **Compaction-less, direct addressing.** A node's address is its byte offset
   within the file. No compaction step; no hash-indexed indirection.
2. **Zero-copy-ready layout.** Nodes are reinterpreted from their on-disk bytes
   with no field-by-field deserialization.
3. **Within-major compatibility, future-proofed.** Any v2.x reader reads any v2.y
   file, in both directions, unless an explicit read/write gate is raised (which
   within major version 2 is a v3 signal).
4. **Traversal-first structure.** The layout favors path/trie traversal above all
   else.

### Scope

In scope: the header, the area/allocation model and alignment, node encodings,
partial-path encoding, the free-list on-disk layout, the read path (direct in
v2.0, `mmap` in v2.1), and the commit/recovery protocol.

Out of scope (semantics retained from v1, only their serialization changes): the
revision-retention policy, revision-history semantics, and the public Rust/FFI
APIs. Durable revision history is deferred (see [Future possibilities](#future-possibilities)),
though the format reserves the hooks to add it without a break.

### Operating requirements

These are stated requirements of v2, not open questions:

- **Single writer.** At most one process writes a database at a time; the
  recoverability protocol assumes it. Reads may run concurrently with the writer,
  but only through handles registered with the revision manager: a registered
  reader pins its revision so the writer will not reap and reuse an area the reader
  is still traversing. v2.0 defines no cross-process or unregistered-`mmap` reader
  pin protocol — an independent reader that outlives its pinned revision's retention
  window may observe a reaped-and-reused area as garbage. Extending safe reads to
  such readers (a reader epoch/pin protocol) is deferred (see
  [Unresolved questions](#unresolved-questions)).
- **High-throughput storage.** The database targets an NVMe-class block device or
  better; the commit protocol and layout are tuned for low-latency random writes
  and `fsync`. Performance on spinning or network-backed storage is out of scope.

## Guide-level explanation

A v2 file is a small double-buffered header followed by a sequence of 8-byte
aligned **areas**. Each area holds one node, one overflow value, or a free-list
entry. A node's address is simply its byte offset in the file.

```text
FILE = [ Header slot A ][ Header slot B ][ Area ][ Area ][ Area ] ...
        \__ double-buffered, checksummed, seq-numbered __/   \__ 8-aligned, size-classed __/

AREA = [ AreaHeader 8B: size_class, kind, flags ]   (kind: node | value-overflow | free)
       [ payload ... ]

NODE payload (kind = node):
  HOT  (zero-copy; traversal touches only this):
    NodeHeader 8B  : flags, child_bitmap:u16, path_len_nibbles:u32
    child_addrs    : [u64; popcount(child_bitmap)]   (8-aligned)
    partial_path   : packed nibbles (ceil(n/2) bytes)
  COLD (faulted only for value reads / hashing / proofs):
    [ethhash] child_hash_lens : [u8; popcount]
    child_hashes              : [[u8;32]; popcount]
    value                     : inline (len + bytes) OR overflow (u64 addr + u64 len)
    [optional feature-gated cold TLV regions, skippable]

ADDRESS = global u64 file offset, 8-byte aligned (low 3 bits reserved as tag bits).
```

The defining idea is the **hot/cold split**: traversal reads addresses, not
hashes. Child addresses are hot and contiguous; child hashes and values are cold
and faulted only when hashing, generating a proof, or reading a value at a
destination. Following one child reads the 8-byte node header and a single indexed
`u64` child address — a bounded, contiguous hot region; for the common sparse node
this plus the packed partial path sit within the first cache line or two, though a
densely populated branch (up to 16 child addresses = 128 B) spans several.

The **read mechanism is versioned but the bytes are not.** A v2.0 reader resolves
an area in two positioned reads: an 8-byte `AreaHeader` prefix yields `size_class`,
the header's `area_sizes` table gives the byte extent, and a second `pread` pulls
the bounded area into the reader's buffer. That buffer is an over-aligned POD type
(`#[repr(C, align(16))] struct AlignedBuffer<const N: usize>([u8; N])`,
stack-allocated when small and boxed when large), so the bytes land 16-aligned —
hence 8-aligned — and the typed `bytemuck::from_bytes` casts of the `AreaHeader`,
`NodeHeader`, and address array are valid with no field-by-field deserialization and
no copy beyond the read. This mirrors how v1 already reads its header directly into
an aligned `Pod` struct (`storage/src/nodestore/header.rs`). A v2.1 reader maps the
file and reinterprets node bytes in place: mappings are page-aligned (hence
8-aligned), so the same `bytemuck::from_bytes` casts apply with zero copies — the
read copy is what v2.1 elides. Both read identical files, so `mmap` is a reader
capability, not a format change.

## Detailed design

### Areas, alignment, and addressing

**Alignment.** All areas begin at an 8-byte-aligned offset; 8-byte alignment is
frozen. It is chosen over 16-byte alignment because finer size-class granularity
(multiples of 8 vs 16) reduces internal fragmentation for the most numerous nodes
(tiny leaves), which dominates space at 2 TiB scale, and 8-byte alignment already
yields 3 structurally-zero low address bits reserved as tag bits.

**Area structure.**

```text
AREA (8-aligned):
  AreaHeader (8 bytes, repr(C), little-endian):
    off 0  size_class : u8     index into the file's size-class table
    off 1  kind       : u8     0 = node, 1 = free, 2 = value-overflow; 3.. registry
    off 2  flags      : u8     reserved per-area bits
    off 3  reserved   : u8
    off 4  reserved   : u32    future (write 0)
  payload @ +8:
    kind = node            -> NodeHeader @ +8, child_addrs @ +16 (8-aligned)
    kind = value-overflow  -> raw value bytes @ +8
    kind = free            -> next_free : u64 @ +8 (intrusive free-list link)
```

Separating allocator metadata (`AreaHeader`) from trie content (`NodeHeader`)
guarantees the 8-byte alignment the zero-copy casts need. The `reserved` bytes
exist only to fill the 8 bytes alignment already demands, so they cost no extra
space; a v2.x writer that assigns them meaning announces it with a feature bit, and
a v2.0 reader treats nonzero values there as an unknown feature, not corruption.
The `kind` byte is an **extensible registry, not a closed set.** Values 0 (node),
1 (free), and 2 (value-overflow) and their meanings are frozen; the remaining values
are allocated to features (a core range and a feature range, as for the cold-TLV
tags). A new kind — for example DRH's `fdl` log areas — is admitted within major
version 2 under one contract that keeps the rebuild scan total without forcing a v3
migration:

- **Size is kind-independent.** An area's byte extent comes from `size_class` and
  the `area_sizes` table alone, so a linear scan steps over an area of any kind —
  known or not — without interpreting its payload. (This is a frozen invariant, not
  an incidental property.)
- **New kinds are write-gated.** Introducing a kind MUST set a
  `required_writer_features` bit (and `min_reader_version` only if old readers must
  traverse the kind to read the *latest committed revision*). A writer that does not
  support the bit opens read-only and never runs the destructive rebuild reclaim, so
  it cannot misclassify an area whose kind it does not understand.
- **An out-of-range kind is never trusted as a future feature.** A writer that opens
  read-write has passed `required_writer_features`, so it understands every kind the
  file can legally contain; a value outside that set is not a feature it is missing.
  Keyed on reachability (see the rebuild scan), such an area is torn mid-fill debris
  when off the live root and is reclaimed like any debris, and is genuine corruption
  only when a live node references it. A new kind never reaches an ungated writer's
  scan, because the gate already forced it read-only.

Readers reach areas by following addresses from the root or the free-list heads,
never by scanning, so an old reader simply never touches an area of an unknown kind.

**Addressing.** An address is a global `u64` byte offset, 8-byte aligned, with `0`
reserved as null (the niche for `Option<Address>`):

```text
Address = u64
  bits 63..3 : the offset
  bits  2..0 : RESERVED TAG BITS. v2.0 writers MUST write 0; readers MUST mask
               (addr & !0b111) before dereferencing. Future per-edge metadata is
               assigned to these bits only under a header feature flag.
  value 0    : null
```

The address *is* the disk offset; segmentation (below) never leaks into it. The
masking rule applies to every stored `Address` — child addresses, the value
`overflow_addr`, `root_address`, and `free_list_heads` entries.

**Size classes (self-describing).** The size-class table is data in the header,
not a compiled-in constant. A frozen maximum of 64 classes keeps the header
fixed-size; the per-area `size_class` byte indexes the file's own table, so every
reader and writer agrees on what a class means for that file. This eliminates the
v1 trap where a compiled-in table mismatch rejects the file. v2.0 ships a
default table (geometric progression, 16 B to 16 MiB); a future writer may choose different
values for new files, and old readers honor whatever the file declares. Two
size-class invariants are validated at open and a file violating either is refused:
every class size is a multiple of 8, and `area_sizes[0] >= 16` so the smallest
class can still hold a free record (`AreaHeader(8) + next_free(8)`). The minimum is
bounded by the free record, not by any node shape — a node that needs more room
simply allocates a larger class.

**Read path and segmented mapping.** v2.0 reads via `pread` + `bytemuck` cast.
v2.1 maps the file in fixed-size **segments** (`segment_size`, frozen at 1 GiB but
recorded in the header; it MUST be a multiple of `page_size`). The v2.1 reader
decomposes an address as `seg = addr / segment_size`,
`ptr = segment_base[seg] + addr % segment_size`, maps segments lazily, and never
relocates a live mapping (new segments map additively), so growth is safe for
concurrent readers and works identically on Linux and macOS.

*No-straddle invariant:* an area never crosses a segment boundary, so any node
lies within one contiguous mapping. A bump allocation that would straddle the next
boundary pads the tail gap and registers it as free area(s). The gap is decomposed
deterministically and independently of the (per-file) size-class table:

```text
remaining = gap_bytes
while remaining >= area_sizes[0]:          # area_sizes[0] is the minimum class
    c = max index where area_sizes[c] <= remaining
    emit a `kind = free` area of class c on free_list_heads[c]
    remaining -= area_sizes[c]
# remaining (< area_sizes[0]) is unreclaimable slop
```

The gap at a boundary is at most one max-area (16 MiB per 1 GiB segment, ≈ 1.56%),
but almost all of it re-enters the free lists; permanently unreclaimed waste per
boundary is only the sub-minimum residual (< `area_sizes[0]` bytes).

**Endianness.** All integers are little-endian by definition. On the little-endian
targets Firewood supports this is native, giving true zero-copy. A big-endian host
is unsupported and refused at open rather than byte-swapped; the header
`endian_test` field also detects an accidentally big-endian-written file.

### Node encodings

```text
NodeHeader (8 bytes, repr(C), little-endian):
  off 0  flags            : u8
           bit 0  kind            0 = branch, 1 = leaf
           bit 1  has_value       node carries a value
           bit 2  value_overflow  0 = inline value, 1 = overflow reference
           bit 3  has_cold_tlv    0 = no TLV region, 1 = TLV region follows value
           bits 4..7  reserved    (write 0)
  off 1  reserved0        : u8     reserved per-node growth surface (write 0)
  off 2  child_bitmap     : u16    bit i set if a child exists at nibble i
  off 4  path_len_nibbles : u32
```

The trie is 16-ary (one nibble per level), so `child_bitmap` is exactly 16 bits.
`path_len_nibbles` is `u32` — a deliberate, frozen hard cap of ~4.29e9 nibbles
(~2 GiB key), about seven orders of magnitude above any realistic key, chosen to
eliminate an overflow branch on the hot path. The `child_addrs` array follows at
`+16` (8-aligned); the `partial_path` follows it as packed nibbles (two per byte,
high nibble first), `ceil(path_len_nibbles / 2)` bytes. When `path_len_nibbles` is
odd, the final byte's low nibble is padding: a writer MUST write it as zero and a
reader MUST ignore it. Freezing the pad to zero keeps the encoded bytes
reproducible, which the byte-identical-hash requirement below depends on.

**Branch node.**

```text
AREA (8-aligned)
  AreaHeader(8)                size_class, kind = node
  HOT:
    NodeHeader(8)  @ +8        flags(branch), child_bitmap, path_len_nibbles
    child_addrs    @ +16       [Address; pc]  (8-aligned; pc = popcount(child_bitmap))
    partial_path   @ +16+8*pc  packed nibbles
  COLD:
    child hashes (size fixed given pc + file hash algorithm):
      [ethhash only] child_hash_lens : [u8; pc]
      child_hashes                   : [[u8;32]; pc]
    value section (present iff flags.has_value):
      inline   : value_len : u32, bytes[value_len]
      overflow : overflow_addr : Address, value_len : u64
    cold TLV region (present iff flags.has_cold_tlv):
      tlv_total_len : u32                            bytes in the records that follow
      records       : [tag:u16][len:u32][bytes] ...  (exactly tlv_total_len bytes)
  (any area payload beyond the last cold element is zero-filled slop, ignored)
```

Child lookup at nibble `n` (the hot path) is branch-free aside from one `POPCNT`:

```text
if (child_bitmap & (1 << n)) == 0  -> no child
idx  = popcount(child_bitmap & ((1 << n) - 1))
addr = child_addrs[idx] & !0b111                 // strip tag bits
// child_hashes[idx] is consulted only when hashing or proving
```

A **leaf** is the same shape with `child_bitmap = 0` (no address array) and a
value section in its cold tail; it stores no hash of its own (its hash lives in its
parent's `child_hashes` slot, and the root's hash lives in the header).

**Cold-region seek invariant (frozen).** Every cold sub-region's byte extent MUST
be computable from the hot header (`pc`, `has_value`, `value_overflow`,
`path_len_nibbles`, `has_cold_tlv`) plus the file's `hash_algorithm` alone — never
from the cold bytes. The `child_hashes` block is always `32 * pc` bytes: the
physical stride is 32 bytes per child regardless of `child_hash_lens`, which is
interpretation-only. The value-section offset is therefore a closed form, computable
without faulting the hash block. The trailing TLV region is the only variable-length
cold element, and it is framed so a reader can both skip unknown records and find
later known ones: it is present iff `has_cold_tlv` is set, it opens with a
`tlv_total_len : u32` byte count, and the reader walks `[tag:u16][len:u32][bytes]`
records consuming exactly `tlv_total_len` bytes — no terminator and no inter-record
padding, so a zero byte is never mistaken for an empty `tag = 0x0000` record. Any
area payload beyond the final cold element (or beyond the value section when
`has_cold_tlv` is clear) is zero-filled slop and is ignored; the TLV walk never
runs into it. This framing is what makes "skip an unknown cold region, then read a
later known one" actually hold.

**Inline-value threshold is writer policy, not format.** A reader never consults a
threshold — `flags.value_overflow` is self-describing per node, so nodes written
under different thresholds coexist. The default is 256 B (so account RLP and
storage slots stay inline) and is persisted as an advisory
`default_inline_value_threshold` that readers ignore. The inline `value_len` is a
`u32` by format; if it exceeds the area's remaining payload, the reader MUST return
a corruption error, never truncate. The overflow case carries the symmetric
obligation: a referenced overflow area must satisfy
`value_len <= area_size(overflow_area.size_class) - 8` (its 8-byte `AreaHeader`),
and a reader MUST reject the reference as corruption rather than read past the
overflow area's payload.

### Header, versioning, and compatibility

The file opens with two header slots. The fixed slot content is the first 8192
bytes; the on-disk slot stride is `max(8192, page_size)` (zero padding fills the
remainder under large pages). Each commit writes the **older** slot (lower
`sequence`), checksums it, and `fsync`s it, so a torn header write never damages
the last-good slot.

```text
DbHeaderSlot (repr(C), little-endian; fixed content = 8192 B):
  // identity and version (FROZEN CORE)
  off    0  magic                          : [u8;8]
  off    8  major_version                  : u16      = 2; reader refuses if != its major
  off   10  minor_version                  : u16
  off   12  min_reader_version             : u16      read gate
  off   14  header_content_size            : u16      = 8192 (on-disk stride is max(8192, page_size))
  off   16  endian_test                    : u64      = 1
  off   24  feature_bits                   : [u64;4]  256 optional-capability bits
  off   56  required_writer_features       : [u64;4]  256 write-gate bits
  off   88  sequence                       : u64      monotonic commit counter
  // format parameters (self-describing)
  off   96  segment_size                   : u64      mmap segment size (default 1 GiB)
  off  104  hash_algorithm                 : u8       0 = SHA-256, 1 = Keccak-256; selects runtime hash mode
  off  105  size_class_count               : u8       N <= 64
  off  106  clean_shutdown                 : u8       1 if last close was clean
  off  107  page_size_log2                 : u8       log2 of the page alignment the file guarantees (>= 12)
  off  108  default_inline_value_threshold : u32      advisory
  // root and allocation state (rewritten every commit)
  off  112  root_address                   : u64      0 = empty tree
  off  120  high_water_mark                : u64      next bump-allocation offset
  off  128  root_hash                      : [u8;32]
  off  160  fdl_head                       : u64      RESERVED, feature-gated, 0 in v2.0
  off  168  fdl_tail                       : u64      RESERVED, feature-gated, 0 in v2.0
  off  176  revision_index_anchor          : u64      RESERVED, feature-gated, 0 in v2.0
  off  184  reserved_b                     : u64      RESERVED, feature-gated, 0 in v2.0
  // tables
  off  192  area_sizes                     : [u64;64] entries >= size_class_count are 0
  off  704  free_list_heads                : [u64;64] per-class free-list head Address
  // advisory
  off 1216  writer_build                   : [u8;64]  build stamp, diagnostic only
  off 1280  reserved                       : [u8;6904] frozen-core headroom
  // integrity (last field)
  off 8184  checksum                       : u64      XXH3-64 digest over bytes [0, 8184)
```

The `checksum` is a full 64-bit XXH3-64 digest over `[0, 8184)`; it occupies the
entire `u64` with no spare bits, so a future writer cannot smuggle meaning into it.
New fixed fields are carved from `reserved` (off 1280) low-end upward, each
recorded next to the `feature_bits` bit that announces it.

**Page alignment (frozen rule).** The file records `page_size_log2`
(`page_size = 1 << page_size_log2`, a power of two, ≥ 4096). The slot stride is
`max(8192, page_size)`, so the first area begins at `2 * max(8192, page_size)` —
always a multiple of `page_size`, hence page-aligned, and always at least 16384.
On open a reader refuses the file if `page_size < the system page size`: a file
aligned to a smaller page cannot be mapped on a host with larger pages, while a
file aligned to a larger page maps fine anywhere with pages ≤ it. `page_size_log2`
is set once at creation and immutable thereafter (every commit rewrites it with the
same value). Because each slot spans at least one full page, a single torn page
write damages at most one slot, preserving the double buffer's independent-failure
property. The reader never trusts the `page_size_log2` byte from an *unvalidated*
slot to locate the other slot: a torn-but-plausible value would point the search at
the wrong offset and miss a recoverable slot. Instead it probes the small, fixed set
of candidate strides (`max(8192, 1 << k)` for `k` in `12..=30`) for a checksum-valid
slot — see recovery below.

**Three compatibility mechanisms (the heart of tenet 3).** Within-major
bidirectional compatibility rests on two one-way gates plus a bidirectional
extension channel, over a frozen hot core:

| Concern | `feature_bits` | `min_reader_version` | `required_writer_features` |
| --- | --- | --- | --- |
| Meaning | file uses optional capability X | reader must understand minor >= V | writer must support X to write |
| On unknown | reader ignores the bit, skips the cold region | reader refuses the file | writer opens read-only |
| Compatibility | bidirectional (additions optional, skippable) | one-way read gate | one-way write gate |
| Intended use | the default channel for all evolution | a non-skippable read change; raising it is by definition a v3 signal | gate for any write-affecting feature |

`feature_bits` is the routine channel: every ordinary addition ships as a feature
bit plus self-delimiting cold data (a cold TLV record, an address tag-bit meaning,
or an advisory `reserved` field). `min_reader_version` is reserved for a
non-skippable read change — moving it is by definition a v3 signal.
`required_writer_features` is the write gate: a writer that does not support a set
bit opens read-only, which is the insurance that lets any write-affecting feature
be added later without older writers silently corrupting it. To classify a change,
ask in order: does a reader need new bytes to read the latest revision (raise
`min_reader_version`)? does a writer need to maintain a new invariant (set
`required_writer_features`)? otherwise set a `feature_bits` bit. Unknown
`feature_bits`, unknown cold TLV tags (assigned from a central registry —
`0x0000`–`0x00FF` core, `0x0100`–`0x01FF` ethhash), and set address tag bits are
all skipped, never fatal.

### Commit and recovery

The whole recovery argument rests on a single invariant:

> **Never mutate any area the live header transitively references, until a new
> header is durably published.**

The live header is the valid slot with the highest `sequence`; it transitively
references the committed root tree and the free-list chains. Everything else (bump
space beyond `high_water_mark`, areas in neither the tree nor a free chain) is safe
to write. The header swap is not a hardware-atomic store; its all-or-nothing effect
comes from double-buffering, per-slot checksums, and sequence selection: a crash
leaves either the previous valid slot or a checksum-valid newer slot selectable,
never a torn mix.

**Commit** is two `fsync`s:

```text
Phase 1 - write node bytes (only to NON-live-referenced areas):
   - new nodes -> bump space (beyond live high_water_mark), and/or
   - areas from the reserved pool (popped and published by a PRIOR commit)
   fsync                                   // node bytes durable
Phase 2 - publish (atomic):
   - write the scratch (older) header slot: root_address, root_hash,
     high_water_mark, free_list_heads[], sequence + 1, checksum
   fsync                                   // the commit point
```

The Phase-1 write of each new area includes its full `AreaHeader` (with
`kind = node`), so a `kind = node` byte may be durable before Phase 2 publishes —
this is safe because the area is off all live-header references. The two-`fsync`
ordering is a hard requirement of any write backend; a writer using io-uring must
chain a barrier `fsync` or fall back to a blocking one between phases. This requires
the writable-storage trait to gain a durability-barrier method
(`fsync`/`fdatasync`) that every backend implements — the current trait exposes only
writes, so this is a required API addition the implementation introduces.

**Reuse and reaping are safe** because each touches only non-live-referenced areas:
an area is *popped one commit ahead* of being filled (commit N−1 publishes a header
excluding area X; commit N then overwrites X), and a node is *reaped* only after
its retaining revision expires (so it is in no live tree and not yet on a free
list). This is why `free_list_heads` lives inside the checksummed, double-buffered
header: the head array publishes atomically, and the only mutable-in-place state —
the intrusive `next_free` links — is always written to non-live-referenced areas.

**Recovery on open:**

```text
1. Bootstrap and read both slots:
     a. Read the fixed 8192-byte content at offset 0 (always present: stride >= 8192).
        This is slot A; verify magic, major == 2, endian_test, and checksum.
     b. Locate slot B without trusting any unvalidated byte. The on-disk stride is
        max(8192, 1 << page_size_log2) and page_size_log2 is immutable since
        creation, but its byte may be torn in a damaged slot — so the reader probes
        each candidate stride max(8192, 1 << k) for k in 12..=30, reads a slot-B
        candidate at each, and keeps any that verifies magic, major == 2,
        endian_test, and checksum. (If slot A verified, its page_size_log2 selects
        the stride directly and the probe is a one-shot confirmation; the probe only
        does real work when slot A is damaged.)
     c. A valid slot's page_size MUST agree with the stride it was found at; a slot
        whose page_size_log2 disagrees with its discovery stride is treated as
        invalid.
     d. Refuse if neither slot is valid, or if the adopted slot's page_size is
        smaller than the system page size.
   Select the runtime hash mode from the adopted slot's hash_algorithm.
2. Enforce gates (a precondition of handle acquisition, atomic with it):
     refuse        if min_reader_version > mine;
     read-only     if any required_writer_features bit is unsupported.
3. Adopt the valid slot with the highest sequence; this IS the committed state.
4. Trust its root tree and free_list_heads / chains (the invariant guarantees it).
5. Treat anything beyond its high_water_mark as uncommitted debris and ignore it.
```

No redo/undo log is needed. A crash can strand reserved-pool areas (popped but
never filled) and bump debris beyond `high_water_mark`; both are harmless, bounded
space, recovered by an optional free-space rebuild scan that runs only after an
unclean open (see the `clean_shutdown` protocol below). Because that scan is the
*only* path that reclaims stranded reserved-pool areas, a clean close MUST NOT leave
them stranded — otherwise they would leak unrecoverably and accumulate across clean
restarts. A cooperative close therefore returns every popped-but-unfilled reserved
area to its `free_list_heads[c]` chain as part of its final commit.

**Clean-shutdown protocol.** `clean_shutdown` is not poked in place in the live
slot — an in-place byte write would break that slot's checksum and violate the
"never mutate live-referenced state" rule. A cooperative close performs one final,
ordinary header publish to the scratch (older) slot: it returns reserved areas to
the free lists, sets `clean_shutdown = 1`, bumps `sequence`, checksums, and
`fsync`s. Every Phase-2 commit, by contrast, publishes with `clean_shutdown = 0`. On
open the rebuild scan runs unless the adopted slot reads `clean_shutdown = 1`, so a
clean close both records the flag durably and leaves no reserved-pool debris for the
skipped scan to miss.

The rebuild scan is a **total** classification over every area, keyed on
`AreaHeader.kind`, reachability from the live root (`R_root`), and membership on a
published free chain (`R_free`). Reachability follows every live reference a node
holds — its child addresses *and*, when `flags.value_overflow` is set, its
`overflow_addr` — so a live overflow value counts as `R_root = yes`:

| `kind` | `R_root` | `R_free` | Classification |
| --- | --- | --- | --- |
| `node` | yes | — | **live node** — retain |
| `node` | no | — | **reclaim-to-free** — mid-fill debris whose header never published |
| `value-overflow` | yes | — | **live value** — retain (reached via a node's `overflow_addr`) |
| `value-overflow` | no | — | **reclaim-to-free** — orphaned overflow whose referrer never published |
| `free` | — | yes | **free** — already correct |
| `free` | — | no | **reclaim-to-free** — popped-but-unfilled reserved-pool area |
| unrecognized | no | — | **reclaim-to-free** — torn mid-fill debris bearing a garbage `kind` byte |
| unrecognized | yes | — | **corruption** — a live-referenced area with an unintelligible header |

Keying on `R_root` rather than on `kind` alone is what closes the "bounded per crash
but unbounded across crashes" hazard: an area off the live root is positively
classified as reclaimable — whether it is `kind = node` mid-fill debris or debris
bearing a torn `kind` byte — never mistaken for live data. Sub-minimum segment-tail
residual (`< area_sizes[0]`) carries no `AreaHeader`; it is identified positionally
rather than walked as an area, and is counted toward the waste bound.

### Allocation and free lists

To allocate, pick the smallest size class that fits `AreaHeader(8) + hot + cold`,
then reuse (`pop free_list_heads[c]`, O(1)) or bump (`carve at high_water_mark`,
honoring the no-straddle rule). A freed area's payload becomes its intrusive link:

```text
FREE AREA (8-aligned):
  AreaHeader(8)        size_class = c, kind = free
  next_free : u64 @ +8 Address of next free area in class c's list (0 = end)
```

The discriminator is the `AreaHeader.kind` field (not a magic byte), and the link
is a fixed 8-aligned `u64` (not a varint), so a free area is a trivially zero-copy
record and the free-list walk chases aligned `u64`s with no decode.

Deferred free stays in memory, as in v1: a superseded node is recorded in an
in-memory per-proposal list and returns to the free lists only when its owning
revision expires; on restart, in-flight proposals are discarded. The only
allocation state v2 persists is `free_list_heads` and `high_water_mark`. The
`fdl_*` / `revision_index_anchor` header fields are reserved for a future durable
variant.

### Hashing integration

The format stores hashes and stays agnostic to hashing *semantics* (which stay in
the hashing layer), but it is not agnostic to hash *geometry*: the layout freezes a
32-byte child-hash stride (see the cold-region seek invariant) and the ethhash
`child_hash_lens` convention. v2 therefore supports the declared 32-byte-output
algorithms; an algorithm needing a different hash width or extra per-child metadata
is a gated layout change or a v3 event, not a drop-in `hash_algorithm` value.

The `hash_algorithm` field (`0` = SHA-256/merkledb, `1` = Keccak-256/ethhash) is a
file-global, immutable header field. It **selects the runtime hash mode at open**:
both modes are built into one binary as two traits — there is no compile-time hash
feature — so any binary opens any file in the mode its header declares, and an
unrecognized value is refused. There is no cross-algorithm path to
guard. A node's own hash lives in its parent's cold `child_hashes` slot; the root's
hash lives in `root_hash`. Under ethhash, `child_hash_lens[i] == 32` is a full
Keccak hash and `1..=31` is an embedded RLP node in the slot's first `len` bytes (a
present child is never length 0; the writer zeroes bytes `len..32` and a reader uses
only `0..len`, keeping stored bytes reproducible). merkledb files omit
`child_hash_lens` entirely.

**Correctness constraint.** v2 changes storage, not Merkle semantics: for any
logical tree under a given algorithm, the v2 root hash MUST be byte-identical to
v1's — blockchain consensus depends on exact state roots.

### Frozen invariants

Frozen for all of major version 2 (changing any is a v3 event), grouped by area:

- *Alignment and addressing:* 8-byte alignment, the 3 reserved address tag bits, and
  the address-masking rule.
- *Area and node layout:* the `AreaHeader` and `NodeHeader` layouts, the `+16`
  child-address-array position, and packed-nibble paths.
- *Header layout and commit:* the 8192-byte header content layout (field offsets and
  the `area_sizes`/`free_list_heads` tables, with `checksum` at 8184) and
  double-buffering, with on-disk slot stride `max(8192, page_size)` and first area at
  `2 * slot_stride`; little-endian integers; the page-alignment rule; the
  recoverability invariant and the header-swap commit point.
- *Integrity:* the XXH3-64 checksum with no spare bits.
- *Compatibility:* the three compatibility mechanisms, the cold-TLV extension model,
  and the cold-region seek invariant.
- *Operational protocols:* the frozen `kind` values (0/1/2) and the
  kind-extensibility contract (kind-independent area size, write-gated new kinds,
  corruption checked only after the gate); the fill invariant and the total rebuild
  predicate; runtime hash-mode selection; the `required_writer_features`
  open-for-write precondition; and the `clean_shutdown` protocol.

Accepted, owned limits (each a v3 event to raise): `path_len_nibbles` at the `u32`
ceiling, the 64-class size-class cap, the 256-bit gate arrays, and the `u32` inline
value length. Values that are header data and so may differ per file without
breaking compatibility: the `area_sizes` table, `segment_size`, `page_size_log2`,
`hash_algorithm`, and `default_inline_value_threshold`.

## Drawbacks

- **A frozen format is a long commitment.** Every byte layout decision is hard to
  reverse at 2 TiB scale; the compatibility machinery (gates, reserved space,
  cold-TLV) is overhead that exists solely to defer the next migration.
- **The hot/cold split costs a second fault** when a caller does need a node's
  hashes or an overflowed value (hashing, proofs), in exchange for a faster
  traversal hot path.
- **Two `fsync`s per commit** is a real write-path cost (its magnitude versus v1 is
  unmeasured; see Unresolved questions).
- **Crash recovery trades a bounded space leak** (reserved-pool and bump debris)
  for never corrupting committed state, requiring an occasional rebuild scan after
  an unclean shutdown.
- **The `mmap` reader and an `mmap` writer are net-new** surfaces deferred to later
  versions; v2.0 ships only the simpler `pread` reader.

## Rationale and alternatives

- **8-byte vs 16-byte alignment.** 16-byte alignment would add one tag bit and
  16-aligned address arrays, but the SIMD benefit is weak (traversal does scalar
  reads) and coarser size classes would waste tens of GB on small-node padding at
  scale. 8-byte wins on space and already supplies enough tag bits.
- **Self-describing size-class table vs compiled-in.** v1 hashes a compiled-in
  table and rejects mismatches, which would break within-major compatibility.
  Storing the table (and `segment_size`, `page_size_log2`, `hash_algorithm`,
  threshold) in the header lets files evolve without rejection.
- **Split address/hash vs interleaved or fully separate.** Interleaving hashes with
  addresses pollutes the traversal cache line; a fully separate hash store doubles
  free-list and recovery bookkeeping. The node-local cold tail keeps recovery
  single-region while still keeping hashes off the hot path.
- **`pread` now, `mmap` later vs `mmap` now.** Deferring `mmap` keeps the largest
  novel-correctness surface (lazy mapping, growth-without-relocation,
  `pwrite`-to-`mmap` coherence) out of the v2.0 critical path while the format stays
  `mmap`-ready, so v2.1 drops in with no format change.
- **Page size in the header vs a fixed platform envelope.** Recording
  `page_size_log2` and scaling the slot stride lets one format serve 4/16/64 KiB
  page hosts and keeps each header slot on its own page (so a torn page write can
  damage only one slot), which a fixed 16384-byte prologue could not guarantee.
- **Runtime hash mode vs a compile-time feature.** A header-selected mode removes
  the cross-algorithm guard surface entirely and lets a single binary serve both
  merkledb and ethhash files.

## Prior art

The v1 on-disk format establishes
disk-offset addressing, end-of-file/free-list allocation, size-class free lists,
the future-delete log, and recoverability via not referencing new nodes before
flush — all of which v2 keeps in spirit while changing the encoding. The
double-buffered, checksummed, sequence-numbered superblock is the standard
crash-safe header pattern used across filesystems and embedded databases. The
bitmap + popcount child array mirrors HAMT/CHAMP node encodings. The
`feature_bits` / `min_reader_version` split mirrors read/write feature flags in
formats such as SQLite and ext-family filesystems.

## Unresolved questions

These do not gate the format freeze but must be answered before or during
implementation:

1. **Two-`fsync` commit throughput is unmeasured.** Benchmark against v1 during
   implementation; if it materially regresses the C-Chain commit path, advance the
   `mmap`-writer follow-up (which can batch differently). The format does not change
   either way.
2. **Writer-exclusivity mechanism.** Single-writer access is a stated requirement;
   the open question is only enforcement. v2 most likely keeps v1's advisory
   `flock`, but the guarantee boundary on shared/network storage or container
   runtimes (where advisory locks are unreliable) must be stated at implementation
   time.
3. **v2.1 `mmap` reader isolation model.** The default is snapshot-pinned-at-open: a
   reader reads the header once, pins `root_address`, and traverses that revision;
   because committed nodes are immutable, no re-validation and no extra header field
   are needed. A "follow-latest" reader (re-read the header each traversal) is the
   alternative; the double-buffered header supports both, so this is a reader-policy
   choice, not a format change. One caveat bounds this: "committed nodes are
   immutable" holds only within a revision's retention window — once the pinned
   revision expires, the writer may reap and reuse its areas. A reader registered
   with the revision manager pins the revision and is safe; an independent or
   cross-process reader that traverses past the retention window needs an epoch/pin
   handshake with the reaper, which v2.0 does not define and which (like the `mmap`
   writer) is a later addition in reserved, feature-gated space.

## Future possibilities

- **Durable revision history (DRH).** Make the last N revisions traversable after a
  restart (in v2.0 they are in-memory only). DRH adds a revision index of retained
  roots (anchored by `revision_index_anchor`) and an on-disk future-delete log
  (chained `kind = fdl` areas anchored by `fdl_head`/`fdl_tail` — a new kind admitted
  under the kind-extensibility contract above), plus commit and reclamation steps —
  all in reserved, feature-gated space, with node bytes unchanged. It is additive
  precisely because the `required_writer_features` write gate ships in v2.0: an older
  writer ignorant of DRH would misclassify and reclaim its `fdl` areas during a
  rebuild scan, and the gate forces it read-only instead. DRH is therefore a clean
  v2.x addition rather than a v3 migration.
- **`mmap`-based writer.** Mutate mapped pages directly with `msync` range ordering,
  landing with a test asserting zero format-byte change versus the `pread`/`pwrite`
  writer's output.
- **Cross-algorithm reads.** Not offered, and unnecessary under runtime hash-mode
  selection; recorded only as a non-goal.

## Verification and merge gates

The pre-freeze and implementation gates below must pass on the schedule indicated;
freeze gates block locking any byte layout, implementation gates block merging the
implementation.

### Freeze gates (must pass before any byte is frozen)

- [ ] **Round-trip property tests:** every node shape — leaf/branch, all
      `child_bitmap` populations, path lengths 0/1/2 + odd/even + 40..60 +
      `u32`-large, inline/overflow values, merkledb + ethhash cold blocks — encode
      then read back via the v2.0 `pread` path; structural equality + correct
      nibble-indexed child lookup.
- [ ] **Byte-layout lock-in:** `insta` snapshots of representative nodes, headers,
      and free areas, landing with the format definition.
- [ ] **Compatibility matrix:** hand-crafted future files (unknown `feature_bits`,
      unknown cold-TLV tags, set address tag bits, raised `min_reader_version`, set
      `required_writer_features`) assert skip/refuse/read-only behavior and
      v2.x-reads-v2.0, including an unknown cold-TLV *preceding* a known one (proves
      skip arithmetic, not just tail-skip).
- [ ] **Crash consistency / fault injection:** a backend that can truncate,
      torn-write, or fail at each `fsync` boundary; after any crash, `open()` yields
      exactly the old or new committed state, never a mix. Cover both
      reclaim-to-free rows of the rebuild predicate, and assert the leak bound holds
      across repeated crash-reopen cycles with no monotonic accumulation.
- [ ] **Allocator:** alloc/free/reuse, segment-straddle padding + reclaim, the
      pop-ahead reserved pool, rebuild-scan idempotence.
- [ ] **Differential vs v1 (consensus gate, non-negotiable):** identical workloads
      produce byte-identical root hashes under each algorithm, with exhaustive
      ethhash RLP-embedding coverage (every `child_hash_lens` value in `1..=32`,
      every account-depth case).
- [ ] **Hash-mode selection at open:** the open path dispatches to the trait for the
      file's `hash_algorithm`; a merkledb file opens in merkledb mode and an ethhash
      file in ethhash mode; an unrecognized value is refused.
- [ ] **Zero-copy mechanism under `unsafe`-deny:** `AreaHeader`, `NodeHeader`, and
      the address-array element derive `bytemuck` `Pod`/`Zeroable`; the v2.0 read
      buffer is the over-aligned `AlignedBuffer<N>` so the `bytemuck::from_bytes`
      casts meet their alignment precondition with no new `unsafe` outside `ffi`
      (`cargo build -p firewood-storage`; the only permitted `unsafe impl`s are the
      established `PodInOption`/`ZeroableInOption`).
- [ ] **Header checksum pinned + test vector:** the slot checksum is XXH3-64 over
      `[0, 8184)`; lock one concrete crate (candidates: `xxhash-rust` with `xxh3`,
      or `twox-hash`; pure-Rust preferred) before any byte-layout snapshot lands,
      record a byte-for-byte test vector, and assert a reader rejects a mismatch.
- [ ] **Inline `value_len` overflow rejects:** a value length exceeding the area's
      remaining payload is corruption, never truncated.

### Implementation gates (after the format is frozen)

- [ ] **`mmap` growth + concurrency (v2.1):** file grows across 1 GiB boundaries;
      segmented mapping returns correct bytes; a reader observes a consistent
      snapshot while a writer appends + swaps the header. Exercise publish-ordering,
      not just page-cache coherence. (Gates the v2.1 reader; v2.0 uses `pread`.)
- [ ] **Reader fuzzing (`cargo-fuzz`):** arbitrary/corrupted bytes yield clean
      errors only, never panic/UB. Deferrable past freeze only because the
      cold-region seek invariant makes parsing unambiguous from the hot header.
- [ ] **Scale + soak:** C-Chain reexecution on v2; disk footprint + traversal
      latency vs v1; the `test_slow_` suite under the `ci` profile.
- [ ] **io-uring Phase-1 barrier:** the io-uring write path provides the Phase-1
      `fsync` barrier before the Phase-2 header write (chained barrier or blocking
      fallback); no node bytes reorder after the header publish.
- [ ] **Endianness + page-size guard:** `endian_test` flags a big-endian-written
      file; a big-endian host is refused at open; a file whose recorded `page_size`
      is smaller than the system page size is refused at open.
- [ ] **`clean_shutdown` protocol wired correctly:** Phase-2 writes set it to 0;
      cooperative close sets it to 1; open runs the rebuild scan unless the adopted
      slot reads 1.

### Implementation notes

- Removing the compile-time `ethhash` Cargo feature (so hash mode is runtime) is a
  separate, in-flight, cross-cutting refactor and an accepted breaking change to the
  `firewood-storage` API. The frozen *byte layout* does not depend on it — the
  `hash_algorithm` field and the 32-byte cold-hash geometry are fixed regardless —
  but the "Hash-mode selection at open" freeze gate below does: it exercises runtime
  dispatch and so cannot pass until this refactor lands. Treat the refactor as a
  prerequisite of that one gate, not of the layout decisions. Once it is gone, the
  `--features ethhash` recipes in repository tooling must drop the feature.
