// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![expect(unsafe_code)]

use crate::PathComponent;
use std::alloc::{Layout, alloc, dealloc, handle_alloc_error};
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::num::NonZeroU16;
use std::ptr::NonNull;
use std::{fmt, ptr, slice};

use super::Children;

/// Header of the heap block allocated by [`DenseChildren`].
///
/// `#[repr(C)]` guarantees that `data` immediately follows `bitmap` (with
/// alignment padding) so `&raw mut (*ptr).data` is the canonical data address.
#[repr(C)]
struct Inner<T> {
    bitmap: NonZeroU16,
    data: [T; 0],
}

/// Immutable, densely packed children array with a null-pointer niche.
///
/// `None` ↔ zero children, no heap allocation.
/// `Some(ptr)` ↔ heap block: `Inner<T>` header + `bitmap.count_ones()` elements.
///
/// `size_of::<DenseChildren<T>>() == size_of::<*const ()>()` on all platforms
/// (the null-pointer niche on `NonNull` makes `Option<NonNull<_>>` pointer-sized).
pub struct DenseChildren<T>(Option<NonNull<Inner<T>>>, PhantomData<Box<T>>);

// Compile-time guarantee: `DenseChildren` must be pointer-sized.
const _: () = assert!(size_of::<DenseChildren<[u8; 32]>>() == size_of::<*const ()>());

fn alloc_layout<T>(count: usize) -> Layout {
    let (layout, _) = Layout::new::<Inner<T>>()
        .extend(Layout::array::<T>(count).expect("layout overflow"))
        .expect("layout overflow");
    layout.pad_to_align()
}

/// Returns a pointer to the first element in an `Inner<T>` allocation.
///
/// Uses `&raw mut (*ptr).data` (RFC 2582) so the address is derived directly
/// from the struct field, consistent with `alloc_layout` for all `T` alignments.
///
/// # Safety
///
/// `inner` must be a valid, non-dangling `NonNull<Inner<T>>` pointing to an
/// allocation produced by `alloc_layout::<T>(count)` with `Inner.bitmap`
/// already initialised.
unsafe fn data_ptr<T>(inner: NonNull<Inner<T>>) -> *mut T {
    // SAFETY: `inner` is valid per the function precondition; `#[repr(C)]` on
    // `Inner<T>` guarantees that `data` is at the correct offset, so taking
    // `&raw mut` of the field and casting to `*mut T` gives the data base address.
    unsafe { (&raw mut (*inner.as_ptr()).data).cast::<T>() }
}

impl<T> DenseChildren<T> {
    /// Constructs an empty `DenseChildren` (null pointer, no heap allocation).
    #[must_use]
    pub const fn new() -> Self {
        Self(None, PhantomData)
    }

    /// Returns the raw bitmap — bit `i` is set iff slot `i` has a child.
    /// Returns `0` when there are no children.
    #[must_use]
    pub const fn bitmap(&self) -> u16 {
        let Some(ptr) = self.0 else { return 0 };
        // SAFETY: `ptr` is a valid `NonNull<Inner<T>>` per the `DenseChildren`
        // invariant (`Some` ↔ a valid allocation with an initialised header).
        unsafe { ptr.as_ref().bitmap.get() }
    }

    /// Returns the number of present children.
    #[must_use]
    pub const fn count(&self) -> usize {
        self.bitmap().count_ones() as usize
    }

    /// Returns a reference to the child at `index`, or `None` if absent.
    #[must_use]
    pub fn get(&self, index: PathComponent) -> Option<&T> {
        let ptr = self.0?;
        let bit = 1u16 << index.as_u8();
        // SAFETY: `ptr` is a valid `NonNull<Inner<T>>` per the `DenseChildren` invariant.
        let bm = unsafe { ptr.as_ref().bitmap.get() };
        if bm & bit == 0 {
            return None;
        }
        // `rank` is the number of set bits below `bit`, i.e. the dense index of
        // the element at `index`. It is strictly less than `bm.count_ones()`.
        // `bit` is always ≥ 1 (it is a power of two) so `wrapping_sub` never wraps.
        let rank = (bm & bit.wrapping_sub(1)).count_ones() as usize;
        // SAFETY: `data_ptr(ptr)` is valid per the invariant; `rank` < `count` so
        // `.add(rank)` is within the allocation and the element is initialised.
        Some(unsafe { &*data_ptr(ptr).add(rank) })
    }

    /// Iterates over present children as `(PathComponent, &T)`, ascending slot order.
    #[must_use]
    pub fn iter_present(&self) -> DenseChildrenIter<'_, T> {
        let Some(ptr) = self.0 else {
            return DenseChildrenIter {
                remaining: 0,
                data: ptr::null(),
                _marker: PhantomData,
            };
        };
        DenseChildrenIter {
            // SAFETY: `ptr` is valid per the `DenseChildren` invariant.
            remaining: unsafe { ptr.as_ref().bitmap.get() },
            // SAFETY: `ptr` is valid; `data_ptr` returns the base of the
            // element array within the allocation.
            data: unsafe { data_ptr(ptr).cast_const() },
            _marker: PhantomData,
        }
    }

    /// Iterates over all 16 slots as `(PathComponent, Option<&T>)`, slot 0..16.
    #[must_use]
    pub fn iter(&self) -> DenseChildrenAllIter<'_, T> {
        self.into_iter()
    }
}

/// Iterator over present children yielded as `(PathComponent, &T)`.
///
/// `remaining`: bitmap of slots not yet yielded (cleared LSB-first).
/// `data`: pointer to the next element to yield; advances on each `next()`.
pub struct DenseChildrenIter<'a, T> {
    remaining: u16,
    data: *const T,
    _marker: PhantomData<&'a T>,
}

impl<T: fmt::Debug> fmt::Debug for DenseChildrenIter<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DenseChildrenIter")
            .field("remaining_bitmap", &self.remaining)
            .finish_non_exhaustive()
    }
}

impl<'a, T> Iterator for DenseChildrenIter<'a, T> {
    type Item = (PathComponent, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        #[expect(clippy::indexing_slicing)]
        let pc = PathComponent::ALL[self.remaining.trailing_zeros() as usize];
        // `remaining` is non-zero here (checked above), so `wrapping_sub` never wraps.
        self.remaining &= self.remaining.wrapping_sub(1);
        // SAFETY: `data` points to the next unread element within the `DenseChildren`
        // allocation. `remaining` was initialised from the bitmap and cleared one bit
        // per call, so `data` is always in bounds and the element is initialised.
        // `data.add(1)` is safe because there are at least as many elements as bits
        // that were set in the original bitmap.
        let (item, next) = unsafe { (&*self.data, self.data.add(1)) };
        self.data = next;
        Some((pc, item))
    }
}

impl<T> std::iter::FusedIterator for DenseChildrenIter<'_, T> {}

/// Iterator over all 16 slots yielded as `(PathComponent, Option<&T>)`.
///
/// `remaining`: bitmap shifted right one bit per slot.
/// `data`: pointer to the next present element; advances only when `Some`.
/// `slot`: current slot index, 0..16.
pub struct DenseChildrenAllIter<'a, T> {
    remaining: u16,
    data: *const T,
    slot: u8,
    _marker: PhantomData<&'a T>,
}

impl<T: fmt::Debug> fmt::Debug for DenseChildrenAllIter<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DenseChildrenAllIter")
            .field("remaining_bitmap", &self.remaining)
            .field("slot", &self.slot)
            .finish_non_exhaustive()
    }
}

impl<'a, T> Iterator for DenseChildrenAllIter<'a, T> {
    type Item = (PathComponent, Option<&'a T>);

    fn next(&mut self) -> Option<Self::Item> {
        if self.slot >= 16 {
            return None;
        }
        // `remaining` (shifted) and `slot` (incremented) advance for every slot —
        // present or absent — before the `data` pointer is conditionally advanced.
        // This keeps `data` aligned with the next present element at all times.
        let has_child = self.remaining & 1 != 0;
        self.remaining >>= 1;
        #[expect(clippy::indexing_slicing)]
        let pc = PathComponent::ALL[self.slot as usize];
        // `slot` is < 16 here (checked above), so `wrapping_add` never wraps.
        self.slot = self.slot.wrapping_add(1);
        if has_child {
            // SAFETY: `data` points to the next present element in the allocation.
            // `has_child` is true iff the bitmap bit for this slot was set, so the
            // element exists and is initialised. `data.add(1)` is within bounds
            // because the bitmap count bounds the number of `Some` yields.
            let (item, next) = unsafe { (&*self.data, self.data.add(1)) };
            self.data = next;
            Some((pc, Some(item)))
        } else {
            Some((pc, None))
        }
    }
}

impl<T> std::iter::FusedIterator for DenseChildrenAllIter<'_, T> {}

impl<'a, T> IntoIterator for &'a DenseChildren<T> {
    type Item = (PathComponent, Option<&'a T>);
    type IntoIter = DenseChildrenAllIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        let Some(ptr) = self.0 else {
            return DenseChildrenAllIter {
                remaining: 0,
                data: ptr::null(),
                slot: 0,
                _marker: PhantomData,
            };
        };
        DenseChildrenAllIter {
            // SAFETY: `ptr` is a valid `NonNull<Inner<T>>` per the invariant.
            remaining: unsafe { ptr.as_ref().bitmap.get() },
            // SAFETY: `ptr` is valid; `data_ptr` yields the base of the element array.
            data: unsafe { data_ptr(ptr).cast_const() },
            slot: 0,
            _marker: PhantomData,
        }
    }
}

impl<T> Drop for DenseChildren<T> {
    fn drop(&mut self) {
        let Some(ptr) = self.0 else { return };
        // Read count before any destructor runs; bitmap is not accessed after.
        // SAFETY: `ptr` is a valid `NonNull<Inner<T>>` per the `DenseChildren`
        // invariant; the bitmap field is initialised and remains valid for this read.
        let count = unsafe { ptr.as_ref().bitmap.get().count_ones() as usize };
        // SAFETY: `ptr` is valid; `data_ptr` returns the base of the element array.
        let dp = unsafe { data_ptr(ptr) };
        // SAFETY: `dp` points to `count` consecutive, initialised `T` values within
        // the allocation. `slice_from_raw_parts_mut(dp, count)` constructs a fat
        // pointer to that slice; `drop_in_place` drops each element exactly once.
        unsafe { ptr::drop_in_place(ptr::slice_from_raw_parts_mut(dp, count)) };
        let layout = alloc_layout::<T>(count);
        // SAFETY: `ptr.as_ptr()` was obtained from `alloc(alloc_layout::<T>(count))`;
        // elements have been dropped above; this is the sole deallocation.
        unsafe { dealloc(ptr.as_ptr().cast(), layout) }
    }
}

impl<T: Clone> Clone for DenseChildren<T> {
    fn clone(&self) -> Self {
        let Some(ptr) = self.0 else {
            return Self::new();
        };
        // SAFETY: `ptr` is valid per the invariant; we read the bitmap and count.
        let bm = unsafe { ptr.as_ref().bitmap };
        let count = bm.get().count_ones() as usize;
        let layout = alloc_layout::<T>(count);
        // SAFETY: `alloc` returns a valid pointer or null; null is handled below.
        let Some(raw) = NonNull::new(unsafe { alloc(layout) }) else {
            handle_alloc_error(layout)
        };
        let new_inner = raw.cast::<Inner<T>>();
        // SAFETY: `new_inner` points to freshly allocated memory sized for
        // `Inner<T>`; writing the bitmap initialises the header before elements.
        unsafe { (*new_inner.as_ptr()).bitmap = bm };
        // SAFETY: `ptr` is valid per the invariant; `data_ptr` returns the base of
        // `count` initialised elements in the allocation.
        let src = unsafe { data_ptr(ptr).cast_const() };
        // SAFETY: `new_inner` is valid per the invariant; `data_ptr` returns the base
        // of the element array in the new allocation (sized for `count` elements).
        let dst = unsafe { data_ptr(new_inner) };

        // If `T::clone()` panics mid-loop, `CloneGuard::drop` drops the
        // already-initialized elements and frees the allocation. Forgotten
        // (not run) once all `count` slots have been successfully written.
        struct CloneGuard<T> {
            ptr: NonNull<Inner<T>>,
            initialized: usize,
            capacity: usize,
        }
        impl<T> Drop for CloneGuard<T> {
            fn drop(&mut self) {
                // SAFETY: `self.ptr` is a valid allocation from `alloc_layout::<T>(self.capacity)`;
                // `data_ptr(self.ptr)` yields the base of the element array; `self.initialized`
                // elements starting there are fully initialised. `alloc_layout` is called with the
                // same capacity used during allocation.
                unsafe {
                    let dp = data_ptr(self.ptr);
                    ptr::drop_in_place(ptr::slice_from_raw_parts_mut(dp, self.initialized));
                    dealloc(self.ptr.as_ptr().cast(), alloc_layout::<T>(self.capacity));
                }
            }
        }

        let mut guard = CloneGuard {
            ptr: new_inner,
            initialized: 0,
            capacity: count,
        };
        for i in 0..count {
            // SAFETY: `src.add(i)` and `dst.add(i)` are both within their respective
            // allocations (0 ≤ i < count); `src` elements are initialised; each `write`
            // to `dst` initialises a previously uninitialised slot.
            unsafe { dst.add(i).write((*src.add(i)).clone()) };
            // `i < count <= 16`; `wrapping_add` never wraps.
            guard.initialized = i.wrapping_add(1);
        }
        std::mem::forget(guard);
        Self(Some(new_inner), PhantomData)
    }
}

impl<T: PartialEq> PartialEq for DenseChildren<T> {
    fn eq(&self, other: &Self) -> bool {
        if self.bitmap() != other.bitmap() {
            return false;
        }
        let count = self.count();
        if count == 0 {
            return true;
        }
        // SAFETY: `self.0` is `Some` because `count > 0` (bitmap is non-zero);
        // `data_ptr` returns the base of `count` initialised elements.
        let a = unsafe {
            let ptr = self.0.unwrap_unchecked();
            slice::from_raw_parts(data_ptr(ptr).cast_const(), count)
        };
        // SAFETY: `other.0` is `Some` (count > 0, bitmaps equal); same reasoning.
        let b = unsafe {
            let ptr = other.0.unwrap_unchecked();
            slice::from_raw_parts(data_ptr(ptr).cast_const(), count)
        };
        a == b
    }
}

impl<T: Eq> Eq for DenseChildren<T> {}

impl<T: Hash> Hash for DenseChildren<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bitmap().hash(state);
        let count = self.count();
        if count == 0 {
            return;
        }
        // SAFETY: `self.0` is `Some` because `count > 0` (bitmap is non-zero);
        // `data_ptr` returns the base of `count` initialised elements.
        let slice = unsafe {
            let ptr = self.0.unwrap_unchecked();
            slice::from_raw_parts(data_ptr(ptr).cast_const(), count)
        };
        slice.hash(state);
    }
}

impl<T: fmt::Debug> fmt::Debug for DenseChildren<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.iter_present().map(|(pc, v)| (pc.as_u8(), v)))
            .finish()
    }
}

impl<T> Default for DenseChildren<T> {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: `DenseChildren<T>` has exclusive ownership of its heap allocation.
// It is safe to send across threads when `T: Send`.
unsafe impl<T: Send> Send for DenseChildren<T> {}

// SAFETY: `&DenseChildren<T>` only yields `&T` references; no interior mutability.
// It is safe to share across threads when `T: Sync`.
unsafe impl<T: Sync> Sync for DenseChildren<T> {}

// `NonNull` is `!UnwindSafe`/`!RefUnwindSafe` so these don't auto-derive.
// `DenseChildren<T>` has exclusive ownership with no shared or cyclic state,
// so both impls are sound whenever the contained `T` satisfies the respective bound.
impl<T: std::panic::UnwindSafe> std::panic::UnwindSafe for DenseChildren<T> {}
impl<T: std::panic::RefUnwindSafe> std::panic::RefUnwindSafe for DenseChildren<T> {}

/// Converts an owned `Children<Option<T>>` into a `DenseChildren<T>`, moving each
/// `Some` value into a densely packed heap allocation. `None` slots are discarded.
impl<T> From<Children<Option<T>>> for DenseChildren<T> {
    fn from(src: Children<Option<T>>) -> Self {
        let mut bitmap = 0u16;
        let mut scratch: Vec<T> = Vec::with_capacity(16);
        for (pc, opt) in src {
            if let Some(v) = opt {
                bitmap |= 1u16 << pc.as_u8();
                scratch.push(v);
            }
        }
        let Some(bm) = NonZeroU16::new(bitmap) else {
            return Self::new();
        };
        let count = scratch.len();
        let layout = alloc_layout::<T>(count);
        // SAFETY: `alloc` returns a valid pointer or null; null is handled below.
        let Some(raw) = NonNull::new(unsafe { alloc(layout) }) else {
            handle_alloc_error(layout);
        };
        let inner = raw.cast::<Inner<T>>();
        // SAFETY: `inner` points to freshly allocated memory; writing bitmap
        // initialises the header before we write any element.
        unsafe {
            (*inner.as_ptr()).bitmap = bm;
        }
        // SAFETY: `inner` is valid; `data_ptr` returns the base of the element array
        // within the allocation, which holds space for exactly `count` elements.
        let dp = unsafe { data_ptr(inner) };
        for (i, v) in scratch.into_iter().enumerate() {
            // SAFETY: `dp.add(i)` is within the allocation (0 ≤ i < count);
            // each `write` initialises a previously uninitialised slot.
            unsafe { dp.add(i).write(v) }
        }
        Self(Some(inner), PhantomData)
    }
}

/// Converts an owned `DenseChildren<T>` into a `Children<Option<T>>`, moving each
/// element out of the heap block without requiring `T: Clone`.
impl<T> From<DenseChildren<T>> for Children<Option<T>> {
    fn from(src: DenseChildren<T>) -> Self {
        let mut out = Children::new();
        // `ManuallyDrop` prevents `DenseChildren::drop` from running so we can
        // move elements out and then manually free the allocation.
        let src = ManuallyDrop::new(src);
        // `Option<NonNull<_>>` is `Copy`; field access through `Deref` copies it.
        if let Some(ptr) = src.0 {
            // SAFETY: `ptr` is valid per the `DenseChildren` invariant.
            let bm = unsafe { ptr.as_ref().bitmap.get() };
            let count = bm.count_ones() as usize;
            // SAFETY: `ptr` is valid; `data_ptr` returns the base of `count` elements.
            let dp = unsafe { data_ptr(ptr) };
            let mut remaining = bm;
            let mut rank = 0usize;
            while remaining != 0 {
                let idx = remaining.trailing_zeros() as usize;
                // `remaining` is non-zero here, so `wrapping_sub` never wraps.
                remaining &= remaining.wrapping_sub(1);
                #[expect(clippy::indexing_slicing)]
                let pc = PathComponent::ALL[idx];
                // SAFETY: `rank` < `count` (each iteration clears one bit of
                // `remaining` and increments `rank`). `ptr::read` moves the value
                // without dropping it; `ManuallyDrop` ensures `DenseChildren::drop`
                // will not run, so no double-drop occurs.
                let val = unsafe { dp.add(rank).read() };
                out[pc] = Some(val);
                // `rank` < `count` (bounded by bits in `bm`), so `wrapping_add` never wraps.
                rank = rank.wrapping_add(1);
            }
            let layout = alloc_layout::<T>(count);
            // SAFETY: `ptr.as_ptr()` was allocated with `alloc_layout::<T>(count)`;
            // all elements have been moved out via `ptr::read`; this is the sole dealloc.
            unsafe { dealloc(ptr.as_ptr().cast(), layout) }
        }
        out
    }
}

/// Converts a `&Children<Option<T>>` into a `DenseChildren<T>`, cloning each
/// present element into a densely packed heap allocation.
impl<T: Clone> From<&Children<Option<T>>> for DenseChildren<T> {
    fn from(src: &Children<Option<T>>) -> Self {
        let mut bitmap = 0u16;
        let mut scratch: Vec<T> = Vec::with_capacity(16);
        for (pc, opt) in src {
            if let Some(v) = opt {
                bitmap |= 1u16 << pc.as_u8();
                scratch.push(v.clone());
            }
        }
        let Some(bm) = NonZeroU16::new(bitmap) else {
            return Self::new();
        };
        let count = scratch.len();
        let layout = alloc_layout::<T>(count);
        // SAFETY: `alloc` returns a valid pointer or null; null is handled below.
        let Some(raw) = NonNull::new(unsafe { alloc(layout) }) else {
            handle_alloc_error(layout);
        };
        let inner = raw.cast::<Inner<T>>();
        // SAFETY: `inner` points to freshly allocated memory; writing bitmap
        // initialises the header before any element is written.
        unsafe { (*inner.as_ptr()).bitmap = bm };
        // SAFETY: `inner` is valid; `data_ptr` returns the base of the element array
        // within the allocation, which holds space for exactly `count` elements.
        let dp = unsafe { data_ptr(inner) };
        for (i, v) in scratch.into_iter().enumerate() {
            // SAFETY: `dp.add(i)` is within the allocation (0 ≤ i < count);
            // each `write` initialises a previously uninitialised slot.
            unsafe { dp.add(i).write(v) }
        }
        Self(Some(inner), PhantomData)
    }
}

/// Converts a `&DenseChildren<T>` into a `Children<Option<T>>`, cloning each
/// present element into the corresponding slot.
impl<T: Clone> From<&DenseChildren<T>> for Children<Option<T>> {
    fn from(src: &DenseChildren<T>) -> Self {
        let mut out = Children::new();
        for (pc, v) in src.iter_present() {
            out[pc] = Some(v.clone());
        }
        out
    }
}

#[cfg(test)]
mod dense_tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    type DC = DenseChildren<u8>;

    /// `DenseChildren<T>` must be the same size as a pointer on all platforms.
    #[test]
    fn size_is_pointer_sized() {
        assert_eq!(size_of::<DC>(), size_of::<*const ()>());
    }

    /// A freshly constructed `DenseChildren` has no children and no heap allocation.
    #[test]
    fn new_is_empty() {
        let d = DC::new();
        assert_eq!(d.count(), 0);
        assert_eq!(d.bitmap(), 0);
        assert!(d.iter_present().next().is_none());
    }

    /// A `DenseChildren` with exactly one child exercises the rank-zero boundary:
    /// `(bm & bit.wrapping_sub(1)).count_ones()` must be 0 regardless of which slot
    /// is set, the allocation holds exactly one element, and all other slots are absent.
    #[test]
    fn single_child() {
        #[expect(clippy::indexing_slicing)]
        for slot in 0..16usize {
            let mut src: Children<Option<u8>> = Children::new();
            src[PathComponent::ALL[slot]] = Some(slot as u8);
            let d = DenseChildren::from(src);

            assert_eq!(d.count(), 1, "slot {slot}");
            assert_eq!(d.bitmap(), 1u16 << slot, "slot {slot}");
            assert_eq!(
                d.get(PathComponent::ALL[slot]),
                Some(&(slot as u8)),
                "slot {slot}"
            );

            // Every other slot must be absent.
            for other in 0..16usize {
                if other != slot {
                    assert_eq!(
                        d.get(PathComponent::ALL[other]),
                        None,
                        "slot {slot}, other {other}"
                    );
                }
            }

            // iter_present yields exactly the one element.
            let items: Vec<_> = d.iter_present().collect();
            assert_eq!(
                items,
                vec![(PathComponent::ALL[slot], &(slot as u8))],
                "slot {slot}"
            );
        }
    }

    /// `get` returns the correct value for present slots and `None` for absent ones.
    #[test]
    fn get_returns_correct_values() {
        let mut src: Children<Option<u8>> = Children::new();
        src[PathComponent::ALL[0]] = Some(10u8);
        src[PathComponent::ALL[7]] = Some(70u8);
        src[PathComponent::ALL[15]] = Some(150u8);
        let d = DenseChildren::from(src);

        assert_eq!(d.get(PathComponent::ALL[0]), Some(&10u8));
        assert_eq!(d.get(PathComponent::ALL[7]), Some(&70u8));
        assert_eq!(d.get(PathComponent::ALL[15]), Some(&150u8));
        assert_eq!(d.get(PathComponent::ALL[1]), None);
        assert_eq!(d.get(PathComponent::ALL[8]), None);
        assert_eq!(d.count(), 3);
    }

    /// `iter_present` yields present slots in ascending slot order.
    #[test]
    fn iter_present_ascending_order() {
        let mut src: Children<Option<u8>> = Children::new();
        src[PathComponent::ALL[3]] = Some(3u8);
        src[PathComponent::ALL[11]] = Some(11u8);
        let d = DenseChildren::from(src);

        let items: Vec<_> = d.iter_present().collect();
        assert_eq!(
            items,
            vec![
                (PathComponent::ALL[3], &3u8),
                (PathComponent::ALL[11], &11u8),
            ]
        );
    }

    /// `IntoIterator` yields all 16 slots, with `None` for absent ones.
    #[test]
    fn into_iter_yields_all_16_slots() {
        let mut src: Children<Option<u8>> = Children::new();
        src[PathComponent::ALL[5]] = Some(5u8);
        let d = DenseChildren::from(src);

        let items: Vec<_> = (&d).into_iter().collect();
        assert_eq!(items.len(), 16);
        // items has exactly 16 elements (asserted above); indices 0, 5, 15 are in bounds.
        #[expect(
            clippy::indexing_slicing,
            reason = "indices are in-bounds per the len assert"
        )]
        {
            assert_eq!(items[5], (PathComponent::ALL[5], Some(&5u8)));
            assert_eq!(items[0], (PathComponent::ALL[0], None));
            assert_eq!(items[15], (PathComponent::ALL[15], None));
        }
    }

    /// Owned round-trip through `Children<Option<T>>` is lossless for an empty value.
    #[test]
    fn owned_roundtrip_empty() {
        let src: Children<Option<u8>> = Children::new();
        let recovered: Children<Option<u8>> = Children::from(DenseChildren::from(src));
        assert_eq!(recovered, src);
    }

    /// Owned round-trip through `Children<Option<T>>` is lossless for a sparse value.
    #[test]
    fn owned_roundtrip_sparse() {
        let mut src: Children<Option<u8>> = Children::new();
        src[PathComponent::ALL[0]] = Some(1u8);
        src[PathComponent::ALL[7]] = Some(7u8);
        src[PathComponent::ALL[15]] = Some(15u8);
        let dense = DenseChildren::from(src);
        let recovered: Children<Option<u8>> = Children::from(dense);
        assert_eq!(recovered, src);
    }

    /// Owned round-trip through `Children<Option<T>>` is lossless when all 16 slots are occupied.
    #[test]
    fn owned_roundtrip_full() {
        let src: Children<Option<u8>> = Children::from_fn(|pc| Some(pc.as_u8()));
        let dense = DenseChildren::from(src);
        let recovered: Children<Option<u8>> = Children::from(dense);
        assert_eq!(recovered, src);
    }

    /// By-ref round-trip through `Children<Option<T>>` is lossless.
    #[test]
    fn ref_roundtrip() {
        let mut src: Children<Option<u8>> = Children::new();
        src[PathComponent::ALL[4]] = Some(4u8);
        src[PathComponent::ALL[9]] = Some(9u8);

        let dense = DenseChildren::from(&src);
        let recovered: Children<Option<u8>> = Children::from(&dense);
        assert_eq!(recovered, src);
    }

    /// A cloned `DenseChildren` compares equal to the original.
    #[test]
    fn clone_is_equal() {
        let mut src: Children<Option<u8>> = Children::new();
        src[PathComponent::ALL[2]] = Some(2u8);
        let d = DenseChildren::from(src);
        let c = d.clone();
        assert_eq!(d, c);
    }

    /// Dropping both the original and the clone triggers exactly 2×n destructors,
    /// confirming that each copy owns its elements independently (no shared state).
    #[test]
    fn clone_is_independent() {
        let counter = Arc::new(AtomicUsize::new(0));

        struct Counted(Arc<AtomicUsize>);
        impl Clone for Counted {
            fn clone(&self) -> Self {
                Self(self.0.clone())
            }
        }
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let mut src: Children<Option<Counted>> = Children::new();
        src[PathComponent::ALL[1]] = Some(Counted(counter.clone()));
        src[PathComponent::ALL[5]] = Some(Counted(counter.clone()));
        src[PathComponent::ALL[9]] = Some(Counted(counter.clone()));

        let original = DenseChildren::from(src);
        let cloned = original.clone();

        assert_eq!(counter.load(Ordering::SeqCst), 0);
        drop(original);
        assert_eq!(counter.load(Ordering::SeqCst), 3, "original drop");
        drop(cloned);
        assert_eq!(counter.load(Ordering::SeqCst), 6, "clone drop");
    }

    /// `Default` produces an empty `DenseChildren` with no allocation.
    #[test]
    fn default_is_empty() {
        let d = DC::default();
        assert_eq!(d.count(), 0);
    }

    /// Each element's destructor runs exactly once when the `DenseChildren` is dropped.
    #[test]
    fn drop_runs_exactly_once_per_element() {
        let counter = Arc::new(AtomicUsize::new(0));

        struct Counted(Arc<AtomicUsize>);
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }

        let mut src: Children<Option<Counted>> = Children::new();
        src[PathComponent::ALL[2]] = Some(Counted(counter.clone()));
        src[PathComponent::ALL[7]] = Some(Counted(counter.clone()));

        let dense = DenseChildren::from(src);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        drop(dense);
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    /// `bitmap` has exactly the bits set for the slots that have children.
    #[test]
    fn bitmap_matches_expected_bits() {
        let mut src: Children<Option<u8>> = Children::new();
        src[PathComponent::ALL[1]] = Some(1u8);
        src[PathComponent::ALL[14]] = Some(14u8);
        let d = DenseChildren::from(src);
        assert_eq!(d.bitmap(), (1u16 << 1) | (1u16 << 14));
    }

    /// When `T::clone()` panics after some elements have been written, the drop
    /// guard frees those elements and the allocation — no leak, no double-drop.
    #[test]
    fn clone_panic_drops_partial_clones() {
        use std::panic;

        struct PanickingClone {
            drop_count: Arc<AtomicUsize>,
            clone_calls: Arc<AtomicUsize>,
        }

        impl Clone for PanickingClone {
            fn clone(&self) -> Self {
                assert!(
                    self.clone_calls.fetch_add(1, Ordering::SeqCst) < 2,
                    "deliberate clone panic"
                );
                Self {
                    drop_count: self.drop_count.clone(),
                    clone_calls: self.clone_calls.clone(),
                }
            }
        }

        impl Drop for PanickingClone {
            fn drop(&mut self) {
                self.drop_count.fetch_add(1, Ordering::SeqCst);
            }
        }

        let drop_count = Arc::new(AtomicUsize::new(0));
        let clone_calls = Arc::new(AtomicUsize::new(0));

        let mut src: Children<Option<PanickingClone>> = Children::new();
        #[expect(
            clippy::indexing_slicing,
            reason = "0..4 is within PathComponent::ALL's 16 slots"
        )]
        for i in 0..4 {
            src[PathComponent::ALL[i]] = Some(PanickingClone {
                drop_count: drop_count.clone(),
                clone_calls: clone_calls.clone(),
            });
        }

        let dense = DenseChildren::from(src);
        assert_eq!(drop_count.load(Ordering::SeqCst), 0);

        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let _ = dense.clone();
        }));
        assert!(result.is_err(), "expected clone to panic");

        // The guard dropped the 2 successfully-cloned elements on unwind;
        // the 4 originals in `dense` are still alive.
        assert_eq!(drop_count.load(Ordering::SeqCst), 2);

        drop(dense);
        assert_eq!(drop_count.load(Ordering::SeqCst), 6);
    }
}
