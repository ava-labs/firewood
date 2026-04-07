// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Caches for [`FileBacked`](super::filebacked::FileBacked), backed by
//! [`quick_cache`].
//!
//! Both caches delegate all sharding, eviction, and synchronization to
//! `quick_cache::sync::Cache`, which uses Clock-PRO eviction (scan-resistant,
//! comparable to S3-FIFO) and allows true parallel reads without exclusive
//! locking.
//!
//! # Memory-bounded node cache
//!
//! [`ShardedNodeCache`] uses a custom [`NodeWeighter`] that reports each
//! entry's heap footprint via [`HeapSize`]. `quick_cache` accumulates
//! these weights and evicts entries once the total exceeds the configured byte
//! limit.
//!
//! # Entry-count free-list cache
//!
//! [`ShardedFreeListCache`] uses `quick_cache`'s built-in unit weighter (each
//! entry weighs 1), so capacity equals the maximum number of entries.

use std::num::NonZeroUsize;

use quick_cache::Weighter;
use quick_cache::sync::Cache;

use crate::{CachedNode, HeapSize, LinearAddress, SharedNode};

/// A concurrent, memory-bounded node cache backed by [`quick_cache`].
///
/// Eviction is weight-based: each entry's weight is its heap footprint in
/// bytes as reported by [`NodeWeighter`]. [`total_current_size`] and
/// [`total_max_size`] are lock-free reads of `quick_cache`'s internal
/// counters.
///
/// [`total_current_size`]: ShardedNodeCache::total_current_size
/// [`total_max_size`]: ShardedNodeCache::total_max_size
#[derive(Debug)]
pub(super) struct ShardedNodeCache(Cache<LinearAddress, CachedNode, NodeWeighter>);

impl ShardedNodeCache {
    /// Create a new cache with `total_limit` bytes of capacity.
    pub(super) fn new(total_limit: NonZeroUsize) -> Self {
        Self(Cache::with_weighter(
            // estimated_items: used only for initial hash-table sizing.
            // A modest estimate; the cache will resize as needed.
            1024,
            total_limit.get() as u64,
            NodeWeighter,
        ))
    }

    /// Returns the cached [`SharedNode`] for `addr`, or `None` on a miss.
    pub(super) fn get(&self, addr: LinearAddress) -> Option<SharedNode> {
        self.0.get(&addr).map(|n| n.node.clone())
    }

    /// Inserts `node` into the cache for `addr`.
    ///
    /// Silently drops the node if it exceeds the total weight capacity.
    pub(super) fn insert(&self, addr: LinearAddress, node: SharedNode) {
        self.0.insert(addr, CachedNode::from(node));
    }

    /// Removes the entry for `addr`, if present.
    pub(super) fn remove(&self, addr: LinearAddress) {
        self.0.remove(&addr);
    }

    /// Returns the current total weight of all cached entries, in bytes.
    ///
    /// Lock-free read of `quick_cache`'s internal counter.
    pub(super) fn total_current_size(&self) -> u64 {
        self.0.weight()
    }

    /// Returns the configured weight capacity, in bytes.
    pub(super) fn total_max_size(&self) -> u64 {
        self.0.capacity()
    }
}

/// A concurrent, entry-count-bounded free-list pointer cache backed by
/// [`quick_cache`].
///
/// Each entry weighs 1 unit (the default), so capacity equals the maximum
/// number of entries. [`total_len`] is a lock-free read of `quick_cache`'s
/// internal counter.
///
/// [`total_len`]: ShardedFreeListCache::total_len
#[derive(Debug)]
pub(super) struct ShardedFreeListCache(Cache<LinearAddress, Option<LinearAddress>>);

impl ShardedFreeListCache {
    /// Create a new cache that holds up to `total_entries` entries.
    pub(super) fn new(total_entries: NonZeroUsize) -> Self {
        Self(Cache::new(total_entries.get()))
    }

    /// Removes and returns the cached next-pointer for `addr`, consuming the
    /// entry.
    ///
    /// Returns `None` if `addr` is not in the cache. Returns `Some(None)` if
    /// `addr` is cached as the end of the free list.
    #[expect(
        clippy::option_option,
        reason = "Option<Option<LinearAddress>> is semantically necessary: outer None = not cached, inner None = cached as end-of-list"
    )]
    pub(super) fn pop(&self, addr: LinearAddress) -> Option<Option<LinearAddress>> {
        self.0.remove(&addr).map(|(_, v)| v)
    }

    /// Inserts or replaces the next-pointer for `addr`.
    pub(super) fn put(&self, addr: LinearAddress, next: Option<LinearAddress>) {
        self.0.insert(addr, next);
    }

    /// Returns the current number of entries in the cache.
    ///
    /// Lock-free read of `quick_cache`'s internal counter.
    pub(super) fn total_len(&self) -> usize {
        self.0.len()
    }
}

/// A [`quick_cache`] weighter that maps each [`CachedNode`] to its heap
/// footprint in bytes.
///
/// The weight is computed via [`HeapSize::heap_size`], which recursively sums
/// the on-heap allocations of the node's path, value, and (for branch nodes)
/// any inlined children. The key ([`LinearAddress`]) is a plain `NonZeroU64`
/// with no heap allocation, so its contribution is zero.
#[derive(Clone)]
struct NodeWeighter;

impl Weighter<LinearAddress, CachedNode> for NodeWeighter {
    fn weight(&self, _key: &LinearAddress, value: &CachedNode) -> u64 {
        // Floor at 1: quick_cache silently drops zero-weight entries. A leaf
        // node with an empty value and a short (inline) path has heap_size == 0
        // but should still be cacheable.
        value.heap_size().max(1) as u64
    }
}
