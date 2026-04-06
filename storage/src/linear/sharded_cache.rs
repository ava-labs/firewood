// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Sharded caches for [`FileBacked`](super::filebacked::FileBacked).
//!
//! Both the node cache and the free-list cache are split across
//! [`SHARD_COUNT`] independent [`Mutex`]-protected shards. Shards are
//! selected by masking low bits of each [`LinearAddress`], distributing
//! entries deterministically while allowing concurrent accesses to
//! different addresses to proceed without contention.
//!
//! # Size tracking
//!
//! Each [`Shard`] pairs its [`Mutex`]-protected cache with an [`AtomicUsize`]
//! that mirrors the cache's current size. The atomic is updated unconditionally
//! on every [`SlotGuard`] drop — including read-only accesses such as
//! [`ShardedNodeCache::get`] — because [`MemLruCache::get`] reorders entries
//! without changing `current_size`, making the extra store cheap and the
//! implementation simpler than tracking write-only paths explicitly. The
//! pay-off is that [`ShardedNodeCache::total_current_size`] and
//! [`ShardedFreeListCache::total_len`] are fully lock-free: they sum the
//! per-shard atomics without acquiring any mutex.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_utils::CachePadded;
use lru::LruCache as EntryLruCache;
use lru_mem::LruCache as MemLruCache;
use parking_lot::Mutex;

use crate::{CachedNode, LinearAddress, SharedNode};

/// A sharded node cache that distributes entries across [`SHARD_COUNT`]
/// independent [`Mutex<MemLruCache>`] shards.
///
/// Each shard holds an equal slice of the total memory budget (rounded up so
/// that no shard gets zero bytes). The shard for a given address is always the
/// same, so reads, inserts, and removals never need to consult more than one
/// shard. [`total_current_size`] and [`total_max_size`] are lock-free.
///
/// [`total_current_size`]: ShardedNodeCache::total_current_size
/// [`total_max_size`]: ShardedNodeCache::total_max_size
#[derive(Debug)]
pub(super) struct ShardedNodeCache {
    shards: ShardedLock<MemLruCache<LinearAddress, CachedNode>>,
    /// Per-shard memory limit. Stored separately so [`total_max_size`] never
    /// needs to acquire any lock.
    ///
    /// [`total_max_size`]: ShardedNodeCache::total_max_size
    per_shard_limit: usize,
}

impl ShardedNodeCache {
    /// Create a new cache with `total_limit` bytes divided equally (ceiling)
    /// across all shards.
    pub(super) fn new(total_limit: NonZeroUsize) -> Self {
        let per_shard = total_limit.div_ceil(SHARD_COUNT).get();
        Self {
            shards: ShardedLock::new(|| MemLruCache::new(per_shard)),
            per_shard_limit: per_shard,
        }
    }

    /// Returns the cached [`SharedNode`] for `addr`, or `None` on a miss.
    pub(super) fn get(&self, addr: LinearAddress) -> Option<SharedNode> {
        self.shards.slot(addr).get(&addr).map(|n| n.0.clone())
    }

    /// Inserts `node` into the cache for `addr`.
    ///
    /// Silently drops the node if it exceeds the shard's memory limit (same as
    /// the previous single-shard behaviour).
    pub(super) fn insert(&self, addr: LinearAddress, node: CachedNode) {
        let _ = self.shards.slot(addr).insert(addr, node);
    }

    /// Removes the entry for `addr` from its shard, if present.
    pub(super) fn remove(&self, addr: LinearAddress) {
        self.shards.slot(addr).remove(&addr);
    }

    /// Returns the sum of `current_size()` across all shards, in bytes.
    ///
    /// This is a lock-free read of per-shard [`AtomicUsize`] counters; no
    /// mutex is acquired.
    pub(super) fn total_current_size(&self) -> usize {
        self.shards.sum_size()
    }

    /// Returns the sum of `max_size()` across all shards, in bytes.
    ///
    /// Computed from the stored `per_shard_limit`; no lock or atomic is needed.
    pub(super) const fn total_max_size(&self) -> usize {
        self.per_shard_limit.wrapping_mul(SHARD_COUNT.get())
    }
}

/// A sharded free-list pointer cache that distributes entries across
/// [`SHARD_COUNT`] independent [`Mutex<EntryLruCache>`] shards.
///
/// Each shard holds an equal share (rounded up) of the total entry budget.
/// [`total_len`] is lock-free.
///
/// [`total_len`]: ShardedFreeListCache::total_len
#[derive(Debug)]
pub(super) struct ShardedFreeListCache {
    shards: ShardedLock<EntryLruCache<LinearAddress, Option<LinearAddress>>>,
}

impl ShardedFreeListCache {
    /// Create a new cache with `total_entries` capacity divided equally
    /// (ceiling) across all shards.
    pub(super) fn new(total_entries: NonZeroUsize) -> Self {
        let per_shard = total_entries.div_ceil(SHARD_COUNT);
        Self {
            shards: ShardedLock::new(|| EntryLruCache::new(per_shard)),
        }
    }

    /// Removes and returns the cached next-pointer for `addr`, consuming the
    /// entry (mirrors [`lru::LruCache::pop`]).
    ///
    /// Returns `None` if `addr` is not in the cache. Returns `Some(None)` if
    /// `addr` is cached as the end of the free list.
    #[expect(
        clippy::option_option,
        reason = "Option<Option<LinearAddress>> is semantically necessary: outer None = not cached, inner None = cached as end-of-list"
    )]
    pub(super) fn pop(&self, addr: LinearAddress) -> Option<Option<LinearAddress>> {
        self.shards.slot(addr).pop(&addr)
    }

    /// Inserts or replaces the next-pointer for `addr`.
    pub(super) fn put(&self, addr: LinearAddress, next: Option<LinearAddress>) {
        self.shards.slot(addr).put(addr, next);
    }

    /// Returns the sum of `len()` across all shards.
    ///
    /// This is a lock-free read of per-shard [`AtomicUsize`] counters; no
    /// mutex is acquired.
    pub(super) fn total_len(&self) -> usize {
        self.shards.sum_size()
    }
}

/// Number of cache shards. Must be a power of two.
const SHARD_COUNT: NonZeroUsize = NonZeroUsize::new(16).unwrap();
const SHARD_MASK: u64 = SHARD_COUNT.get() as u64 - 1;

/// Computes the shard index for `addr`.
///
/// Shifts off the 4 alignment bits (all [`LinearAddress`] values are 16-byte
/// aligned, so the low 4 bits are always zero) before masking, spreading
/// entries across shards without bias from the alignment padding.
#[inline]
const fn shard_index(addr: LinearAddress) -> usize {
    ((addr.get() >> 4) & SHARD_MASK) as usize
}

/// A fixed-size array of [`CachePadded`] [`Shard`]s with address-based
/// dispatch and a lock-free aggregate size.
///
/// Each shard is cache-line padded to prevent false sharing between
/// independently accessed shards.
#[derive(Debug)]
struct ShardedLock<T>([CachePadded<Shard<T>>; SHARD_COUNT.get()]);

impl<T: CurrentSize> ShardedLock<T> {
    /// Creates a new `ShardedLock` by calling `factory` once per shard.
    #[must_use]
    fn new(mut factory: impl FnMut() -> T) -> Self {
        Self(std::array::from_fn(|_| {
            CachePadded::new(Shard {
                slot: Mutex::new(factory()),
                size: AtomicUsize::new(0),
            })
        }))
    }

    /// Locks and returns the shard for `addr` as a [`SlotGuard`].
    ///
    /// The guard's [`Drop`] impl writes the shard's post-operation size back
    /// to [`Shard::size`], keeping the atomic in sync without any extra call
    /// at each call site.
    #[must_use]
    fn slot(&self, addr: LinearAddress) -> SlotGuard<'_, T> {
        #![expect(clippy::indexing_slicing)]
        let shard = &self.0[shard_index(addr)];
        SlotGuard {
            slot: shard.slot.lock(),
            size: &shard.size,
        }
    }

    /// Returns the sum of all per-shard size atomics without acquiring any lock.
    fn sum_size(&self) -> usize {
        self.0.iter().map(|s| s.size.load(Ordering::Relaxed)).sum()
    }
}

/// A single shard: a mutex-protected cache paired with an [`AtomicUsize`] that
/// tracks its current size between lock acquisitions.
///
/// The `size` atomic is written on every [`SlotGuard`] drop, including
/// read-only accesses. See the [module-level documentation](self) for the
/// rationale.
#[derive(Debug)]
struct Shard<T> {
    slot: Mutex<T>,
    size: AtomicUsize,
}

/// Abstracts over the two LRU cache types to expose a unified `current_size`
/// method that [`SlotGuard`]'s [`Drop`] impl can call generically.
///
/// - [`MemLruCache`] already has a `current_size()` method returning bytes.
/// - [`EntryLruCache`] uses `len()` to count entries; this trait maps that to
///   the same name so [`ShardedLock`] can be generic over both.
trait CurrentSize {
    fn current_size(&self) -> usize;
}

impl CurrentSize for MemLruCache<LinearAddress, CachedNode> {
    fn current_size(&self) -> usize {
        self.current_size()
    }
}

impl CurrentSize for EntryLruCache<LinearAddress, Option<LinearAddress>> {
    fn current_size(&self) -> usize {
        self.len()
    }
}

/// A RAII guard returned by [`ShardedLock::slot`].
///
/// Derefs to the inner cache `T`, giving mutable access to the shard while
/// the mutex is held. On drop, writes [`T::current_size()`] to [`Shard::size`]
/// so that [`ShardedLock::sum_size`] always reflects the latest known state
/// without acquiring any lock.
///
/// [`T::current_size()`]: CurrentSize::current_size
struct SlotGuard<'a, T: CurrentSize> {
    slot: parking_lot::MutexGuard<'a, T>,
    size: &'a AtomicUsize,
}

impl<T: CurrentSize> std::ops::Deref for SlotGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.slot
    }
}

impl<T: CurrentSize> std::ops::DerefMut for SlotGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.slot
    }
}

impl<T: CurrentSize> Drop for SlotGuard<'_, T> {
    fn drop(&mut self) {
        self.size.store(self.slot.current_size(), Ordering::Relaxed);
    }
}
