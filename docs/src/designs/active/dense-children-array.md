---
status: active
source:
  - storage/src/node/children/dense.rs
  - storage/src/node/children.rs
  - firewood/src/proofs/types.rs
  - firewood/src/proofs/de.rs
  - firewood/src/proofs/ser.rs
  - firewood/src/proofs/eth.rs
---

# Dense Children Array

## Overview

`DenseChildren<T>` is an immutable, densely packed children array stored behind a thin
pointer. Defined in `storage/src/node/children/dense.rs` alongside `Children<T>` (in
`storage/src/node/children.rs`) and exported from `firewood_storage`, it is used as the
field type of `ProofNode.child_hashes`.

The motivation is issue #2099: `Children<Option<HashType>>` occupies 528 bytes per node
regardless of how many children are present (fixed 16-slot array). Before this change, a
malicious peer achieves approximately 100× memory amplification: a 5-byte minimal
`ProofNode` on the wire expands to a 528-byte in-memory allocation.
`DenseChildren<HashType>` is 8 bytes on the stack; the heap block is sized exactly to
the present children.

## Architecture

`DenseChildren<T>` stores an `Option<NonNull<Inner<T>>>` where `None` serves as the
empty sentinel (exploiting the null-pointer niche of `NonNull`, the same mechanism that
makes `Option<Box<T>>` pointer-sized). The null-pointer niche on `NonNull` makes
`Option<NonNull<Inner<T>>>` — and therefore `DenseChildren<T>` itself — pointer-sized.

`Inner<T>` is `#[repr(C)]` with a `NonZeroU16` bitmap field followed by a zero-sized
`[T; 0]` marker. The data elements live immediately after `Inner<T>` in the same heap
block. `&raw mut (*ptr).data`
([RFC 2582](https://rust-lang.github.io/rfcs/2582-raw-reference-mir-operator.html)) is
the canonical base address for the element array, consistent with the allocation layout
for all `T` alignments.

```rust
#[repr(C)]
struct Inner<T> {
    bitmap: NonZeroU16,
    data: [T; 0],
}

pub struct DenseChildren<T>(Option<NonNull<Inner<T>>>, PhantomData<Box<T>>);
```

The two iterator types — `DenseChildrenIter<'_, T>` (present children only) and
`DenseChildrenAllIter<'_, T>` (all 16 slots) — are also exported from
`firewood_storage`.

## Key data structures

- **`Inner<T>`** (`storage/src/node/children/dense.rs`) — the heap block header.
  `bitmap` holds a bitmask of occupied slots; `data` marks the start of the element
  array. `#[repr(C)]` guarantees the element array begins at
  `offset_of!(Inner<T>, data)`.
- **`DenseChildren<T>`** — the public type. One `Option<NonNull<Inner<T>>>` field;
  `None` means empty. `PhantomData<Box<T>>` gives correct variance, drop-check, and
  auto-trait derivation.
- **`DenseChildrenIter<'a, T>`** — present-children iterator. Carries a bitmap of
  remaining slots and a data pointer that advances on each yield. Yields
  `(PathComponent, &T)` in ascending slot order (lowest-index slot first), consuming the
  bitmap via `trailing_zeros()` to find the next set bit and clearing the lowest set bit
  after each yield via `remaining &= remaining - 1`.
- **`DenseChildrenAllIter<'a, T>`** — all-16-slots iterator. Carries the bitmap, a data
  pointer, and a slot counter. Yields `(PathComponent, Option<&T>)` for every slot,
  advancing the data pointer only on `Some`. Traverses slots 0–15 sequentially by
  right-shifting `remaining` one bit per step. Used by the RLP encoder in `eth.rs`.

### Memory profile (64-bit)

#### Without `ethhash` (`HashType = TrieHash = [u8; 32]`)

`align_of::<TrieHash>() = 1`, so `size_of::<Inner<TrieHash>>() = 2`; per-allocation
overhead is 2 bytes. `size_of::<Option<TrieHash>>() = 33` (no niche), so
`size_of::<Children<Option<TrieHash>>>() = 528`.

| Scenario    | `Children<Option<HashType>>` | `DenseChildren<HashType>`      |
| ----------- | ---------------------------- | ------------------------------ |
| 0 children  | 528 bytes stack              | 8 bytes stack, 0 heap          |
| N children  | 528 bytes stack              | 8 bytes stack + (2 + 32N) heap |
| 16 children | 528 bytes stack              | 8 bytes stack + 514 bytes heap |

#### With `ethhash` (`HashType = HashOrRlp`)

`HashOrRlp` is an enum of `TrieHash` (`[u8; 32]`) and `SmallVec<[u8; 32]>`.
`align_of::<HashOrRlp>() = 8` (from the `usize` fields inside `SmallVec`), so
`size_of::<Inner<HashOrRlp>>() = 8` (2-byte bitmap + 6 bytes of alignment padding);
per-allocation overhead is 8 bytes. `Option<HashOrRlp>` has the same size as `HashOrRlp`
itself (48 bytes, niche present), so `size_of::<Children<Option<HashOrRlp>>>() = 768`.

| Scenario    | `Children<Option<HashType>>` | `DenseChildren<HashType>`      |
| ----------- | ---------------------------- | ------------------------------ |
| 0 children  | 768 bytes stack              | 8 bytes stack, 0 heap          |
| N children  | 768 bytes stack              | 8 bytes stack + (8 + 48N) heap |
| 16 children | 768 bytes stack              | 8 bytes stack + 776 bytes heap |

## Invariants and guarantees

- `self.0.is_none()` ↔ no children, no heap allocation.
- Non-sentinel pointer ↔ `bitmap` is non-zero (`NonZeroU16`) and the heap block holds
  exactly `bitmap.count_ones()` fully initialized `T` values starting at `data_ptr`.
- The `From` conversions and `DenseChildren::new()` are the only construction sites.
- `Drop` reads the bitmap before any element destructor runs, then calls
  `ptr::drop_in_place` on a slice pointer covering all elements in one call, then
  deallocates. The bitmap is not accessed after `drop_in_place` begins.
- `size_of::<DenseChildren<T>>() == size_of::<*const ()>()` on all platforms, verified
  by a compile-time `const` assertion and the unit test `size_is_pointer_sized`. This
  holds because `Option<NonNull<Inner<T>>>` exploits the null-pointer niche and is
  pointer-sized.

## API

### Methods

```rust
impl<T> DenseChildren<T> {
    pub const fn new() -> Self;                                   // empty, no heap
    pub const fn bitmap(&self) -> u16;                            // 0 when empty
    pub const fn count(&self) -> usize;                           // bitmap.count_ones()
    pub fn get(&self, index: PathComponent) -> Option<&T>;
    pub fn iter_present(&self) -> DenseChildrenIter<'_, T>;       // (PathComponent, &T)
    pub fn iter(&self) -> DenseChildrenAllIter<'_, T>;            // (PathComponent, Option<&T>)
}
```

### `IntoIterator`

```rust
impl<'a, T> IntoIterator for &'a DenseChildren<T> {
    type Item = (PathComponent, Option<&'a T>);
    type IntoIter = DenseChildrenAllIter<'a, T>;
}
```

### Conversions

```rust
// Owned — no T: Clone required
impl<T> From<Children<Option<T>>> for DenseChildren<T> { ... }
impl<T> From<DenseChildren<T>>   for Children<Option<T>> { ... }

// By ref — T: Clone required
impl<T: Clone> From<&Children<Option<T>>> for DenseChildren<T> { ... }
impl<T: Clone> From<&DenseChildren<T>>   for Children<Option<T>> { ... }
```

The owned `From<DenseChildren<T>> for Children<Option<T>>` uses `ManuallyDrop` +
`ptr::read` to move elements out without a `T: Clone` bound.

### Trait implementations

| Trait             | Bound                                       |
| ----------------- | ------------------------------------------- |
| `Clone`           | `T: Clone`                                  |
| `PartialEq`       | `T: PartialEq`                              |
| `Eq`              | `T: Eq`                                     |
| `Hash`            | `T: Hash`                                   |
| `Debug`           | `T: Debug`                                  |
| `Default`, `Drop` | —                                           |
| `Send`            | `T: Send`                                   |
| `Sync`            | `T: Sync`                                   |
| `Unpin`           | auto (no bound; `Box<T>` is always `Unpin`) |
| `UnwindSafe`      | `T: UnwindSafe`                             |
| `RefUnwindSafe`   | `T: RefUnwindSafe`                          |
| `FusedIterator`   | on both iterator types                      |

`UnwindSafe` and `RefUnwindSafe` are manual impls: `NonNull` is `!UnwindSafe` and
`!RefUnwindSafe`, so the auto-impl would not derive them at all. The impls are sound
because `DenseChildren<T>` has exclusive ownership of its heap allocation with no shared
or cyclic state.

## Integration with the proof layer

### `ProofNode.child_hashes` (`firewood/src/proofs/types.rs`)

```rust
pub child_hashes: DenseChildren<HashType>,  // was Children<Option<HashType>>
```

### Deserialization (`firewood/src/proofs/de.rs`)

Builds an intermediate `Children<Option<HashType>>` on the stack and converts
immediately. The intermediate (528 bytes without `ethhash`, 768 bytes with) exists for
one node at a time and is dropped before the next node is read.

```rust
let mut child_hashes = Children::new();
for idx in children_map.iter_indices() {
    child_hashes[idx] = Some(reader.read_item()?);
}
let child_hashes = DenseChildren::from(child_hashes);
```

### Serialization (`firewood/src/proofs/ser.rs`)

`ChildMask` is constructed directly from the bitmap; `iter_present` is unchanged.

```rust
ChildMask::from_le_bytes(self.child_hashes.bitmap().to_le_bytes()).write_item(out);
for (_, child) in self.child_hashes.iter_present() { ... }
```

### RLP encoding (`firewood/src/proofs/eth.rs`)

The all-slots iterator is used directly. Call sites that previously wrote
`child.as_ref()` (converting `&Option<T>` → `Option<&T>`) drop that call since
`DenseChildrenAllIter` already yields `Option<&T>`.

The `fix_account_storage_root_value` function takes `&Children<Option<HashType>>`, so
that call site converts via `Children::from(&node.child_hashes)`. This is an
account-node-only path and not performance-sensitive.

### `Hashable::children()` on `ProofNode`

```rust
fn children(&self) -> Children<Option<HashType>> {
    Children::from(&self.child_hashes)
}
```

Allocates a `Children<Option<HashType>>` stack temporary (528 or 768 bytes depending on
configuration) during proof hashing only, not during deserialization. Eliminating this
cost requires changing `Hashable::children()`'s return type to `DenseChildren<HashType>`
— a broader API change deferred to a future revision (see Trade-offs).

## Trade-offs

- **Stack intermediate in deserialization.** A `Children<Option<HashType>>` (528 or 768
  bytes depending on configuration) is built on the stack per node and immediately
  converted. The allocation that matters (the persistent `DenseChildren` heap block) is
  correctly sized. The transient stack cost is bounded to one node at a time.
- **`Hashable::children()` temporary.** The proof hashing path expands to
  `Children<Option<HashType>>` on each `children()` call. Eliminating this requires
  changing the `Hashable` trait's return type — a broader API change deferred to a
  future revision. Note that `Proof::value_digest` materializes one stack temporary per
  non-terminal node (inside the `iter.peek()` branch) and a second on the last node
  during exclusion-proof validation, for a total of one or two temporaries per
  `value_digest` call.
- **`fix_account_storage_root_value` allocation.** The account-node RLP path allocates a
  `Children<Option<HashType>>` temporary per account node. Eliminating this requires
  updating `fix_account_storage_root_value`'s signature — tracked as future work.

## Related designs

- Resolves [#2099](https://github.com/ava-labs/firewood/issues/2099).
- `Children<T>` (`storage/src/node/children.rs`) remains the mutable 16-slot array used
  by `BranchNode`. Replacing it with `DenseChildren<Child>` is tracked in
  [#2034](https://github.com/ava-labs/firewood/issues/2034).
