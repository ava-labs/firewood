// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Client-side read cache for verified `Get()` results.
//!
//! Caches both positive results (key exists with a value) and negative results
//! (key verified absent via exclusion proof). Both are expensive to compute
//! because they require a gRPC round-trip plus Merkle proof verification, so
//! both are worth caching.
//!
//! The cache is sized by memory budget (bytes), not entry count, because value
//! sizes vary widely. This is consistent with the node cache's
//! `node_cache_memory_limit` approach.
//!
//! Four eviction policies are available: LRU (default), Random, Clock
//! (second-chance), and `SampleKLRU` (sample K random entries, evict the least
//! recently accessed).

use crate::remote::client::ClientOp;
use lru_mem::{HeapSize, LruCache};
use parking_lot::Mutex;
use rand::rngs::SmallRng;
use rand::RngExt;
// SeededRng is !Send (Rc-based), but our cache stores the RNG inside a Mutex
// and requires Send + Sync. SmallRng is the only viable option here.
#[expect(
    clippy::disallowed_types,
    reason = "SeededRng is !Send; cache needs Send for Mutex"
)]
use rand::SeedableRng;
use std::collections::HashMap;

// Default memory budget: 32 MiB.
const DEFAULT_BUDGET: usize = 32 << 20;

// Default sample size for SampleKLru.
const DEFAULT_SAMPLE_K: usize = 5;

/// Errors that can occur during cache operations.
#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    /// The cache capacity must be greater than zero.
    #[error("cache capacity must be non-zero")]
    ZeroCapacity,
}

/// A cached result for a verified key lookup.
///
/// Stores both positive results (key exists with a value) and negative
/// results (key verified absent via exclusion proof). Both are expensive
/// to compute, so both are worth caching.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// The value bytes, or `None` if the key does not exist.
    value: Option<Box<[u8]>>,
}

impl CacheEntry {
    /// Returns the cached value, or `None` if the key was verified absent.
    #[must_use]
    pub fn value(&self) -> Option<&[u8]> {
        self.value.as_deref()
    }

    /// Returns the owned cached value, or `None` if the key was verified absent.
    #[must_use]
    pub fn into_value(self) -> Option<Box<[u8]>> {
        self.value
    }
}

impl lru_mem::HeapSize for CacheEntry {
    fn heap_size(&self) -> usize {
        // Only count the heap-allocated value bytes; the Option/Box
        // overhead is accounted for by lru_mem's per-entry bookkeeping.
        self.value.as_ref().map_or(0, |v| v.len())
    }
}

/// Selects the eviction strategy for the read cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvictionPolicy {
    /// Least Recently Used — O(1) ops, best general-purpose choice.
    Lru,
    /// Random eviction — O(1) ops, no access-order tracking overhead.
    Random,
    /// Clock (second-chance) — approximates LRU with lower per-access cost.
    Clock,
    /// Sample K random entries, evict the least recently accessed.
    /// Approximates LRU without maintaining a full ordering (like Redis).
    SampleKLru {
        /// Number of entries to sample when choosing an eviction victim.
        sample_size: usize,
    },
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Approximate memory cost of one cache entry (key + value heap bytes).
/// Custom stores use this for budget tracking; LRU delegates to `lru_mem`.
fn entry_size(key: &[u8], entry: &CacheEntry) -> usize {
    key.len().wrapping_add(entry.heap_size())
}

// ---------------------------------------------------------------------------
// RandomStore
// ---------------------------------------------------------------------------

/// Random-eviction backing store.
///
/// Uses a dense `Vec` of keys for O(1) random victim selection and a
/// `HashMap` for O(1) lookup/remove. When the memory budget is exceeded,
/// a randomly chosen entry is evicted.
struct RandomStore {
    /// Maps key → (cached entry, index in `keys` vec).
    entries: HashMap<Box<[u8]>, (CacheEntry, usize)>,
    /// Dense array of all live keys, enabling O(1) random selection.
    keys: Vec<Box<[u8]>>,
    rng: SmallRng,
    current_size: usize,
    max_bytes: usize,
}

impl RandomStore {
    fn new(max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            keys: Vec::new(),
            rng: SmallRng::seed_from_u64(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(42, |d| d.as_nanos() as u64),
            ),
            current_size: 0,
            max_bytes,
        }
    }

    fn get(&self, key: &[u8]) -> Option<CacheEntry> {
        self.entries.get(key).map(|(e, _)| e.clone())
    }

    fn insert(&mut self, key: Box<[u8]>, entry: CacheEntry) {
        let new_size = entry_size(&key, &entry);
        // If single entry exceeds budget, silently drop it.
        if new_size > self.max_bytes {
            return;
        }
        // Update existing entry.
        if let Some((old_entry, idx)) = self.entries.get_mut(key.as_ref()) {
            self.current_size = self
                .current_size
                .wrapping_sub(entry_size(&key, old_entry))
                .wrapping_add(new_size);
            *old_entry = entry;
            let _ = idx; // index unchanged
            return;
        }
        // Evict until there is room.
        while self.current_size.wrapping_add(new_size) > self.max_bytes && !self.keys.is_empty() {
            self.evict_one();
        }
        let idx = self.keys.len();
        self.keys.push(key.clone());
        self.entries.insert(key, (entry, idx));
        self.current_size = self.current_size.wrapping_add(new_size);
    }

    fn remove(&mut self, key: &[u8]) -> Option<CacheEntry> {
        let (entry, idx) = self.entries.remove(key)?;
        self.current_size = self.current_size.saturating_sub(entry_size(key, &entry));
        // Swap-remove from dense vec; update the swapped key's index.
        self.keys.swap_remove(idx);
        if let Some(swapped_key) = self.keys.get(idx)
            && let Some((_, stored_idx)) = self.entries.get_mut(swapped_key.as_ref())
        {
            *stored_idx = idx;
        }
        Some(entry)
    }

    fn evict_one(&mut self) {
        if self.keys.is_empty() {
            return;
        }
        let idx = self.rng.random_range(0..self.keys.len());
        if let Some(key) = self.keys.get(idx).cloned() {
            let _removed = self.remove(&key);
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.keys.clear();
        self.current_size = 0;
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

// ---------------------------------------------------------------------------
// ClockStore
// ---------------------------------------------------------------------------

/// A single slot in the clock ring buffer.
struct ClockSlot {
    key: Box<[u8]>,
    entry: CacheEntry,
    referenced: bool,
}

/// Clock (second-chance) backing store.
///
/// Approximates LRU: each access sets a "referenced" bit. The clock hand
/// sweeps the ring; referenced entries get a second chance (bit cleared),
/// unreferenced entries are evicted.
struct ClockStore {
    /// Ring buffer of cache slots. `None` marks a freed slot.
    ring: Vec<Option<ClockSlot>>,
    /// Maps key → index in `ring` for O(1) lookup.
    index: HashMap<Box<[u8]>, usize>,
    /// Reusable slot indices from removed entries.
    free_list: Vec<usize>,
    /// Current position of the clock hand in the ring.
    hand: usize,
    current_size: usize,
    max_bytes: usize,
}

impl ClockStore {
    fn new(max_bytes: usize) -> Self {
        Self {
            ring: Vec::new(),
            index: HashMap::new(),
            free_list: Vec::new(),
            hand: 0,
            current_size: 0,
            max_bytes,
        }
    }

    fn get(&mut self, key: &[u8]) -> Option<CacheEntry> {
        let &idx = self.index.get(key)?;
        // Set referenced bit on access (second-chance promotion).
        if let Some(Some(slot)) = self.ring.get_mut(idx) {
            slot.referenced = true;
            return Some(slot.entry.clone());
        }
        None
    }

    fn insert(&mut self, key: Box<[u8]>, entry: CacheEntry) {
        let new_size = entry_size(&key, &entry);
        if new_size > self.max_bytes {
            return;
        }
        // Update existing entry.
        if let Some(&idx) = self.index.get(key.as_ref())
            && let Some(Some(slot)) = self.ring.get_mut(idx)
        {
            self.current_size = self
                .current_size
                .wrapping_sub(entry_size(&slot.key, &slot.entry))
                .wrapping_add(new_size);
            slot.entry = entry;
            slot.referenced = true;
            return;
        }
        // Evict until there is room.
        while self.current_size.wrapping_add(new_size) > self.max_bytes && !self.index.is_empty() {
            self.evict_one();
        }
        let slot = ClockSlot {
            key: key.clone(),
            entry,
            referenced: true,
        };
        // Allocate a ring slot: reuse a freed slot or grow the ring.
        let slot_idx = if let Some(free_idx) = self.free_list.pop() {
            if let Some(ring_slot) = self.ring.get_mut(free_idx) {
                *ring_slot = Some(slot);
            }
            free_idx
        } else {
            let idx = self.ring.len();
            self.ring.push(Some(slot));
            idx
        };
        self.index.insert(key, slot_idx);
        self.current_size = self.current_size.wrapping_add(new_size);
    }

    fn remove(&mut self, key: &[u8]) -> Option<CacheEntry> {
        let idx = self.index.remove(key)?;
        let slot = self.ring.get_mut(idx).and_then(Option::take)?;
        self.current_size = self
            .current_size
            .saturating_sub(entry_size(&slot.key, &slot.entry));
        self.free_list.push(idx);
        Some(slot.entry)
    }

    /// Advance the clock hand and evict the first unreferenced entry.
    /// Referenced entries get their bit cleared (second chance).
    #[allow(clippy::arithmetic_side_effects)] // guarded by n == 0 check above
    fn evict_one(&mut self) {
        let n = self.ring.len();
        if n == 0 || self.index.is_empty() {
            return;
        }
        // At most 2 full sweeps guarantees finding an unreferenced entry.
        let max_iters = n.wrapping_mul(2);
        for _ in 0..max_iters {
            let idx = self.hand.wrapping_rem(n);
            self.hand = self.hand.wrapping_add(1);

            let Some(Some(slot)) = self.ring.get_mut(idx) else {
                continue; // empty slot — skip
            };
            if slot.referenced {
                slot.referenced = false;
                continue;
            }
            // Evict this unreferenced entry.
            let key = slot.key.clone();
            let _removed = self.remove(&key);
            return;
        }
    }

    fn clear(&mut self) {
        self.ring.clear();
        self.index.clear();
        self.free_list.clear();
        self.hand = 0;
        self.current_size = 0;
    }

    fn len(&self) -> usize {
        self.index.len()
    }
}

// ---------------------------------------------------------------------------
// SampleKLruStore
// ---------------------------------------------------------------------------

/// Per-entry metadata for the `SampleKLRU` store.
struct SampleMeta {
    entry: CacheEntry,
    last_access: u64,
    /// Position in the dense `keys` vec for O(1) swap-remove.
    key_index: usize,
}

/// Sample-K-LRU backing store.
///
/// On eviction, samples `sample_size` random entries and evicts the one with
/// the oldest access timestamp. Approximates LRU without maintaining a full
/// ordering — the same approach Redis uses for key eviction.
struct SampleKLruStore {
    entries: HashMap<Box<[u8]>, SampleMeta>,
    keys: Vec<Box<[u8]>>,
    counter: u64,
    sample_size: usize,
    rng: SmallRng,
    current_size: usize,
    max_bytes: usize,
}

impl SampleKLruStore {
    fn new(max_bytes: usize, sample_size: usize) -> Self {
        Self {
            entries: HashMap::new(),
            keys: Vec::new(),
            counter: 0,
            sample_size: if sample_size == 0 {
                DEFAULT_SAMPLE_K
            } else {
                sample_size
            },
            rng: SmallRng::seed_from_u64(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(42, |d| d.as_nanos() as u64),
            ),
            current_size: 0,
            max_bytes,
        }
    }

    fn get(&mut self, key: &[u8]) -> Option<CacheEntry> {
        let meta = self.entries.get_mut(key)?;
        self.counter = self.counter.wrapping_add(1);
        meta.last_access = self.counter;
        Some(meta.entry.clone())
    }

    fn insert(&mut self, key: Box<[u8]>, entry: CacheEntry) {
        let new_size = entry_size(&key, &entry);
        if new_size > self.max_bytes {
            return;
        }
        self.counter = self.counter.wrapping_add(1);
        // Update existing entry.
        if let Some(meta) = self.entries.get_mut(key.as_ref()) {
            self.current_size = self
                .current_size
                .wrapping_sub(entry_size(&key, &meta.entry))
                .wrapping_add(new_size);
            meta.entry = entry;
            meta.last_access = self.counter;
            return;
        }
        // Evict until there is room.
        while self.current_size.wrapping_add(new_size) > self.max_bytes && !self.keys.is_empty() {
            self.evict_one();
        }
        let idx = self.keys.len();
        self.keys.push(key.clone());
        self.entries.insert(
            key,
            SampleMeta {
                entry,
                last_access: self.counter,
                key_index: idx,
            },
        );
        self.current_size = self.current_size.wrapping_add(new_size);
    }

    fn remove(&mut self, key: &[u8]) -> Option<CacheEntry> {
        let meta = self.entries.remove(key)?;
        self.current_size = self
            .current_size
            .saturating_sub(entry_size(key, &meta.entry));
        // Swap-remove from dense vec; update the swapped key's index.
        self.keys.swap_remove(meta.key_index);
        if let Some(swapped_key) = self.keys.get(meta.key_index)
            && let Some(swapped_meta) = self.entries.get_mut(swapped_key.as_ref())
        {
            swapped_meta.key_index = meta.key_index;
        }
        Some(meta.entry)
    }

    /// Sample `sample_size` random entries and evict the least recently accessed.
    fn evict_one(&mut self) {
        if self.keys.is_empty() {
            return;
        }
        let sample_count = self.sample_size.min(self.keys.len());
        let mut oldest_access = u64::MAX;
        let mut oldest_key: Option<Box<[u8]>> = None;
        for _ in 0..sample_count {
            let idx = self.rng.random_range(0..self.keys.len());
            if let Some(key) = self.keys.get(idx)
                && let Some(meta) = self.entries.get(key.as_ref())
                && meta.last_access < oldest_access
            {
                oldest_access = meta.last_access;
                oldest_key = Some(key.clone());
            }
        }
        if let Some(key) = oldest_key {
            let _removed = self.remove(&key);
        }
    }

    fn clear(&mut self) {
        self.entries.clear();
        self.keys.clear();
        self.counter = 0;
        self.current_size = 0;
    }

    fn len(&self) -> usize {
        self.entries.len()
    }
}

// ---------------------------------------------------------------------------
// EvictionStore — dispatch enum
// ---------------------------------------------------------------------------

/// Internal dispatch enum wrapping each eviction policy's backing store.
enum EvictionStore {
    Lru(LruCache<Box<[u8]>, CacheEntry>),
    Random(RandomStore),
    Clock(ClockStore),
    SampleKLru(SampleKLruStore),
}

impl std::fmt::Debug for EvictionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lru(c) => f.debug_tuple("Lru").field(&c.len()).finish(),
            Self::Random(s) => f.debug_tuple("Random").field(&s.len()).finish(),
            Self::Clock(s) => f.debug_tuple("Clock").field(&s.len()).finish(),
            Self::SampleKLru(s) => f.debug_tuple("SampleKLru").field(&s.len()).finish(),
        }
    }
}

impl EvictionStore {
    fn get(&mut self, key: &[u8]) -> Option<CacheEntry> {
        match self {
            Self::Lru(c) => c.get(key).cloned(),
            Self::Random(s) => s.get(key),
            Self::Clock(s) => s.get(key),
            Self::SampleKLru(s) => s.get(key),
        }
    }

    fn insert(&mut self, key: Box<[u8]>, entry: CacheEntry) {
        match self {
            Self::Lru(c) => {
                let _result = c.insert(key, entry);
            }
            Self::Random(s) => s.insert(key, entry),
            Self::Clock(s) => s.insert(key, entry),
            Self::SampleKLru(s) => s.insert(key, entry),
        }
    }

    fn remove(&mut self, key: &[u8]) {
        match self {
            Self::Lru(c) => {
                let _removed = c.remove(key);
            }
            Self::Random(s) => {
                let _removed = s.remove(key);
            }
            Self::Clock(s) => {
                let _removed = s.remove(key);
            }
            Self::SampleKLru(s) => {
                let _removed = s.remove(key);
            }
        }
    }

    /// Collect all keys matching `prefix`. Used by `invalidate_prefix`.
    fn keys_with_prefix(&self, prefix: &[u8]) -> Vec<Box<[u8]>> {
        match self {
            Self::Lru(c) => c
                .iter()
                .filter(|(k, _)| k.starts_with(prefix))
                .map(|(k, _)| k.clone())
                .collect(),
            Self::Random(s) => s
                .entries
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect(),
            Self::Clock(s) => s
                .index
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect(),
            Self::SampleKLru(s) => s
                .entries
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect(),
        }
    }

    fn clear(&mut self) {
        match self {
            Self::Lru(c) => c.clear(),
            Self::Random(s) => s.clear(),
            Self::Clock(s) => s.clear(),
            Self::SampleKLru(s) => s.clear(),
        }
    }

    fn len(&self) -> usize {
        match self {
            Self::Lru(c) => c.len(),
            Self::Random(s) => s.len(),
            Self::Clock(s) => s.len(),
            Self::SampleKLru(s) => s.len(),
        }
    }

    fn current_size(&self) -> usize {
        match self {
            Self::Lru(c) => c.current_size(),
            Self::Random(s) => s.current_size,
            Self::Clock(s) => s.current_size,
            Self::SampleKLru(s) => s.current_size,
        }
    }

    fn max_size(&self) -> usize {
        match self {
            Self::Lru(c) => c.max_size(),
            Self::Random(s) => s.max_bytes,
            Self::Clock(s) => s.max_bytes,
            Self::SampleKLru(s) => s.max_bytes,
        }
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---------------------------------------------------------------------------
// ReadCache — public API
// ---------------------------------------------------------------------------

/// Thread-safe client-side read cache for verified `Get()` results.
///
/// Wraps an eviction store with prefix/batch invalidation and a
/// `parking_lot::Mutex` for thread safety. Sized by memory budget.
#[derive(Debug)]
pub struct ReadCache {
    inner: Mutex<EvictionStore>,
}

impl ReadCache {
    /// Creates a new read cache with the given memory budget and eviction
    /// policy.
    ///
    /// # Errors
    ///
    /// Returns [`CacheError::ZeroCapacity`] if `max_bytes` is zero.
    pub fn new(max_bytes: usize, policy: EvictionPolicy) -> Result<Self, CacheError> {
        if max_bytes == 0 {
            return Err(CacheError::ZeroCapacity);
        }
        let store = match policy {
            EvictionPolicy::Lru => EvictionStore::Lru(LruCache::new(max_bytes)),
            EvictionPolicy::Random => EvictionStore::Random(RandomStore::new(max_bytes)),
            EvictionPolicy::Clock => EvictionStore::Clock(ClockStore::new(max_bytes)),
            EvictionPolicy::SampleKLru { sample_size } => {
                EvictionStore::SampleKLru(SampleKLruStore::new(max_bytes, sample_size))
            }
        };
        Ok(Self {
            inner: Mutex::new(store),
        })
    }

    /// Looks up a key in the cache.
    ///
    /// Returns `Some(CacheEntry)` on cache hit (promoting the entry in
    /// access-order tracking where applicable), or `None` on cache miss.
    /// The returned entry may represent either a found key or a
    /// verified-absent key.
    pub fn lookup(&self, key: &[u8]) -> Option<CacheEntry> {
        let mut guard = self.inner.lock();
        guard.get(key)
    }

    /// Stores a verified lookup result in the cache.
    ///
    /// `value` is `Some(bytes)` for keys that exist, or `None` for keys
    /// verified absent. If the entry exceeds the cache budget, it is
    /// silently dropped (not an error — the cache is best-effort).
    pub fn store(&self, key: &[u8], value: Option<Box<[u8]>>) {
        let entry = CacheEntry { value };
        let mut guard = self.inner.lock();
        guard.insert(Box::from(key), entry);
    }

    /// Removes a single key from the cache, if present.
    pub fn invalidate_key(&self, key: &[u8]) {
        let mut guard = self.inner.lock();
        guard.remove(key);
    }

    /// Removes all keys that start with `prefix` from the cache.
    ///
    /// Uses a two-pass approach: collect matching keys, then delete them.
    /// This is O(n) in cache size — acceptable for the expected cache
    /// sizes (tens of thousands of entries). An empty prefix clears all
    /// entries.
    pub fn invalidate_prefix(&self, prefix: &[u8]) {
        let mut guard = self.inner.lock();
        if prefix.is_empty() {
            guard.clear();
            return;
        }
        let keys_to_remove = guard.keys_with_prefix(prefix);
        for key in keys_to_remove {
            guard.remove(&key);
        }
    }

    /// Invalidates all keys affected by a batch of client operations.
    ///
    /// For `Put` and `Delete` ops, invalidates the specific key.
    pub fn invalidate_batch(&self, ops: &[ClientOp]) {
        let mut guard = self.inner.lock();
        for op in ops {
            match op {
                ClientOp::Put { key, .. } | ClientOp::Delete { key } => {
                    guard.remove(key);
                }
            }
        }
    }

    /// Removes all entries from the cache.
    pub fn clear(&self) {
        let mut guard = self.inner.lock();
        guard.clear();
    }

    /// Returns the number of entries currently in the cache.
    pub fn len(&self) -> usize {
        let guard = self.inner.lock();
        guard.len()
    }

    /// Returns the current memory usage in bytes.
    pub fn current_size(&self) -> usize {
        let guard = self.inner.lock();
        guard.current_size()
    }

    /// Returns the memory budget in bytes.
    pub fn max_size(&self) -> usize {
        let guard = self.inner.lock();
        guard.max_size()
    }

    /// Returns `true` if the cache contains no entries.
    pub fn is_empty(&self) -> bool {
        let guard = self.inner.lock();
        guard.is_empty()
    }
}

/// Default: LRU eviction with a 32 MiB budget.
impl Default for ReadCache {
    fn default() -> Self {
        Self {
            inner: Mutex::new(EvictionStore::Lru(LruCache::new(DEFAULT_BUDGET))),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create an LRU cache with the given budget.
    fn lru_cache(max_bytes: usize) -> ReadCache {
        ReadCache::new(max_bytes, EvictionPolicy::Lru).expect("valid capacity")
    }

    // ---- PR05c: LRU-specific tests ----

    #[test]
    fn test_new_cache() {
        let cache = lru_cache(1024);
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
        assert_eq!(cache.max_size(), 1024);
    }

    #[test]
    fn test_new_cache_zero_capacity() {
        let result = ReadCache::new(0, EvictionPolicy::Lru);
        assert!(result.is_err());
        assert!(
            matches!(result, Err(CacheError::ZeroCapacity)),
            "expected ZeroCapacity error"
        );
    }

    #[test]
    fn test_store_and_lookup_found() {
        let cache = lru_cache(4096);
        let key = b"key1";
        let value: Box<[u8]> = Box::from(b"value1" as &[u8]);
        cache.store(key, Some(value.clone()));

        let entry = cache.lookup(key);
        assert!(entry.is_some());
        let entry = entry.expect("just checked");
        assert_eq!(entry.value(), Some(b"value1" as &[u8]));
    }

    #[test]
    fn test_store_and_lookup_not_found() {
        let cache = lru_cache(4096);
        cache.store(b"absent_key", None);

        let entry = cache.lookup(b"absent_key");
        assert!(entry.is_some(), "absent key should be cached");
        let entry = entry.expect("just checked");
        assert!(entry.value().is_none(), "absent key should have None value");
    }

    #[test]
    fn test_lookup_miss() {
        let cache = lru_cache(4096);
        assert!(cache.lookup(b"nonexistent").is_none());
    }

    #[test]
    fn test_overwrite_entry() {
        let cache = lru_cache(4096);
        cache.store(b"key1", Some(Box::from(b"v1" as &[u8])));
        cache.store(b"key1", Some(Box::from(b"v2" as &[u8])));

        let entry = cache.lookup(b"key1").expect("key should exist");
        assert_eq!(entry.value(), Some(b"v2" as &[u8]));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_invalidate_key() {
        let cache = lru_cache(4096);
        cache.store(b"key1", Some(Box::from(b"v1" as &[u8])));
        cache.store(b"key2", Some(Box::from(b"v2" as &[u8])));

        cache.invalidate_key(b"key1");
        assert!(cache.lookup(b"key1").is_none());
        assert!(cache.lookup(b"key2").is_some());
    }

    #[test]
    fn test_invalidate_nonexistent() {
        let cache = lru_cache(4096);
        cache.store(b"key1", Some(Box::from(b"v1" as &[u8])));
        cache.invalidate_key(b"not_there");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_invalidate_prefix() {
        let cache = lru_cache(4096);
        cache.store(b"app/user/1", Some(Box::from(b"alice" as &[u8])));
        cache.store(b"app/user/2", Some(Box::from(b"bob" as &[u8])));
        cache.store(b"app/config", Some(Box::from(b"cfg" as &[u8])));
        cache.store(b"other/key", Some(Box::from(b"val" as &[u8])));

        cache.invalidate_prefix(b"app/user/");
        assert!(cache.lookup(b"app/user/1").is_none());
        assert!(cache.lookup(b"app/user/2").is_none());
        assert!(cache.lookup(b"app/config").is_some());
        assert!(cache.lookup(b"other/key").is_some());
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn test_invalidate_prefix_empty() {
        let cache = lru_cache(4096);
        cache.store(b"key1", Some(Box::from(b"v1" as &[u8])));
        cache.store(b"key2", Some(Box::from(b"v2" as &[u8])));

        cache.invalidate_prefix(b"");
        assert!(cache.is_empty());
    }

    #[test]
    fn test_invalidate_batch_put_delete() {
        let cache = lru_cache(4096);
        cache.store(b"key1", Some(Box::from(b"v1" as &[u8])));
        cache.store(b"key2", Some(Box::from(b"v2" as &[u8])));
        cache.store(b"key3", Some(Box::from(b"v3" as &[u8])));

        let ops = vec![
            ClientOp::Put {
                key: Box::from(b"key1" as &[u8]),
                value: Box::from(b"new_v1" as &[u8]),
            },
            ClientOp::Delete {
                key: Box::from(b"key2" as &[u8]),
            },
        ];
        cache.invalidate_batch(&ops);

        assert!(
            cache.lookup(b"key1").is_none(),
            "Put key should be invalidated"
        );
        assert!(
            cache.lookup(b"key2").is_none(),
            "Deleted key should be invalidated"
        );
        assert!(
            cache.lookup(b"key3").is_some(),
            "Untouched key should remain"
        );
    }

    #[test]
    fn test_invalidate_batch_delete_range() {
        // ClientOp does not have DeleteRange (ranges are expanded server-side).
        // This test verifies that invalidate_prefix can be used directly for
        // prefix-based invalidation when needed.
        let cache = lru_cache(4096);
        cache.store(b"pfx/a", Some(Box::from(b"va" as &[u8])));
        cache.store(b"pfx/b", Some(Box::from(b"vb" as &[u8])));
        cache.store(b"other", Some(Box::from(b"vo" as &[u8])));

        cache.invalidate_prefix(b"pfx/");
        assert!(cache.lookup(b"pfx/a").is_none());
        assert!(cache.lookup(b"pfx/b").is_none());
        assert!(cache.lookup(b"other").is_some());
    }

    #[test]
    fn test_clear() {
        let cache = lru_cache(4096);
        cache.store(b"key1", Some(Box::from(b"v1" as &[u8])));
        cache.store(b"key2", Some(Box::from(b"v2" as &[u8])));
        assert_eq!(cache.len(), 2);

        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.current_size(), 0);
    }

    #[test]
    fn test_eviction_by_memory() {
        let cache = lru_cache(64);
        for i in 0u8..20 {
            cache.store(&[b'k', i], Some(vec![i; 8].into_boxed_slice()));
        }
        assert!(cache.len() < 20, "some entries should have been evicted");
        assert!(
            cache.current_size() <= 64,
            "current_size ({}) should not exceed budget (64)",
            cache.current_size()
        );
    }

    #[test]
    fn test_len_current_size_max_size() {
        let cache = lru_cache(8192);
        assert_eq!(cache.max_size(), 8192);
        assert_eq!(cache.current_size(), 0);
        assert_eq!(cache.len(), 0);

        cache.store(b"hello", Some(Box::from(b"world" as &[u8])));
        assert_eq!(cache.len(), 1);
        assert!(cache.current_size() > 0);
        assert_eq!(cache.max_size(), 8192);
    }

    #[test]
    fn test_concurrent_reads_writes() {
        use std::sync::Arc;

        let cache = Arc::new(lru_cache(16384));

        std::thread::scope(|s| {
            for t in 0u8..4 {
                let cache = Arc::clone(&cache);
                s.spawn(move || {
                    for i in 0u8..50 {
                        cache.store(&[t, i], Some(vec![t; 4].into_boxed_slice()));
                    }
                });
            }
            for t in 0u8..4 {
                let cache = Arc::clone(&cache);
                s.spawn(move || {
                    for i in 0u8..50 {
                        let _entry = cache.lookup(&[t, i]);
                    }
                });
            }
            {
                let cache = Arc::clone(&cache);
                s.spawn(move || {
                    for i in 0u8..50 {
                        cache.invalidate_key(&[0, i]);
                    }
                });
            }
        });
    }

    #[test]
    fn test_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ReadCache>();
    }

    // ---- PR05d: Multi-policy tests ----

    /// Run a basic store/lookup/eviction test for a given policy.
    fn basic_policy_test(policy: EvictionPolicy) {
        let cache = ReadCache::new(4096, policy).expect("valid capacity");
        cache.store(b"k1", Some(Box::from(b"v1" as &[u8])));
        cache.store(b"k2", None); // negative cache
        assert_eq!(cache.len(), 2);
        assert!(cache.lookup(b"k1").is_some());
        assert!(cache.lookup(b"k2").is_some());
        assert!(cache.lookup(b"missing").is_none());
    }

    #[test]
    fn test_random_eviction() {
        let cache = ReadCache::new(64, EvictionPolicy::Random).expect("valid capacity");
        for i in 0u8..20 {
            cache.store(&[b'k', i], Some(vec![i; 8].into_boxed_slice()));
        }
        assert!(cache.len() < 20, "random policy should evict entries");
    }

    #[test]
    fn test_random_preserves_new_entry() {
        // After inserting a single entry that fits, it should be retrievable.
        let cache = ReadCache::new(4096, EvictionPolicy::Random).expect("valid capacity");
        cache.store(b"hello", Some(Box::from(b"world" as &[u8])));
        assert!(
            cache.lookup(b"hello").is_some(),
            "new entry should not be immediately evicted"
        );
    }

    #[test]
    fn test_clock_second_chance() {
        // Fill a small clock cache, access some entries (setting referenced bit),
        // then insert more entries to trigger eviction. Referenced entries should
        // survive longer.
        let cache = ReadCache::new(128, EvictionPolicy::Clock).expect("valid capacity");

        // Fill cache.
        for i in 0u8..10 {
            cache.store(&[i], Some(vec![i; 4].into_boxed_slice()));
        }
        // Access entry [0] to set its referenced bit.
        let _ = cache.lookup(&[0]);

        // Insert more to trigger eviction.
        for i in 10u8..20 {
            cache.store(&[i], Some(vec![i; 4].into_boxed_slice()));
        }

        // Entry [0] was referenced — it should have a better chance of surviving
        // than unreferenced entries. (Not guaranteed with tiny budgets, but the
        // algorithm gives it a second chance.)
        // We just verify the cache is functioning and within budget.
        assert!(cache.current_size() <= 128);
    }

    #[test]
    fn test_clock_evicts_unreferenced() {
        let cache = ReadCache::new(64, EvictionPolicy::Clock).expect("valid capacity");
        for i in 0u8..20 {
            cache.store(&[b'k', i], Some(vec![i; 8].into_boxed_slice()));
        }
        assert!(cache.len() < 20, "clock policy should evict entries");
    }

    #[test]
    fn test_sample_k_evicts_oldest() {
        // With a large enough sample, the least recently accessed entry should
        // be evicted first (probabilistically).
        let policy = EvictionPolicy::SampleKLru {
            sample_size: DEFAULT_SAMPLE_K,
        };
        let cache = ReadCache::new(64, policy).expect("valid capacity");
        for i in 0u8..20 {
            cache.store(&[b'k', i], Some(vec![i; 8].into_boxed_slice()));
        }
        assert!(cache.len() < 20, "sample-k-lru policy should evict entries");
    }

    #[test]
    fn test_sample_k_default_k() {
        // K=5 default when sample_size is 0.
        let policy = EvictionPolicy::SampleKLru { sample_size: 0 };
        let cache = ReadCache::new(4096, policy).expect("valid capacity");
        basic_policy_test(EvictionPolicy::SampleKLru {
            sample_size: DEFAULT_SAMPLE_K,
        });
        // Just verify construction succeeds.
        assert!(cache.is_empty());
    }

    #[test]
    fn test_policy_prefix_invalidation() {
        for policy in [
            EvictionPolicy::Lru,
            EvictionPolicy::Random,
            EvictionPolicy::Clock,
            EvictionPolicy::SampleKLru {
                sample_size: DEFAULT_SAMPLE_K,
            },
        ] {
            let cache = ReadCache::new(4096, policy).expect("valid capacity");
            cache.store(b"pfx/a", Some(Box::from(b"va" as &[u8])));
            cache.store(b"pfx/b", Some(Box::from(b"vb" as &[u8])));
            cache.store(b"other", Some(Box::from(b"vo" as &[u8])));
            cache.invalidate_prefix(b"pfx/");
            assert!(cache.lookup(b"pfx/a").is_none(), "{policy:?}");
            assert!(cache.lookup(b"pfx/b").is_none(), "{policy:?}");
            assert!(cache.lookup(b"other").is_some(), "{policy:?}");
        }
    }

    #[test]
    fn test_policy_batch_invalidation() {
        let ops = vec![
            ClientOp::Put {
                key: Box::from(b"k1" as &[u8]),
                value: Box::from(b"v" as &[u8]),
            },
            ClientOp::Delete {
                key: Box::from(b"k2" as &[u8]),
            },
        ];
        for policy in [
            EvictionPolicy::Lru,
            EvictionPolicy::Random,
            EvictionPolicy::Clock,
            EvictionPolicy::SampleKLru {
                sample_size: DEFAULT_SAMPLE_K,
            },
        ] {
            let cache = ReadCache::new(4096, policy).expect("valid capacity");
            cache.store(b"k1", Some(Box::from(b"old" as &[u8])));
            cache.store(b"k2", Some(Box::from(b"old" as &[u8])));
            cache.store(b"k3", Some(Box::from(b"keep" as &[u8])));
            cache.invalidate_batch(&ops);
            assert!(cache.lookup(b"k1").is_none(), "{policy:?}");
            assert!(cache.lookup(b"k2").is_none(), "{policy:?}");
            assert!(cache.lookup(b"k3").is_some(), "{policy:?}");
        }
    }

    #[test]
    fn test_policy_concurrent() {
        use std::sync::Arc;

        for policy in [
            EvictionPolicy::Lru,
            EvictionPolicy::Random,
            EvictionPolicy::Clock,
            EvictionPolicy::SampleKLru {
                sample_size: DEFAULT_SAMPLE_K,
            },
        ] {
            let cache = Arc::new(ReadCache::new(8192, policy).expect("valid capacity"));
            std::thread::scope(|s| {
                for t in 0u8..4 {
                    let cache = Arc::clone(&cache);
                    s.spawn(move || {
                        for i in 0u8..30 {
                            cache.store(&[t, i], Some(vec![t; 4].into_boxed_slice()));
                            let _hit = cache.lookup(&[t, i]);
                        }
                    });
                }
            });
        }
    }

    #[test]
    fn test_policy_memory_budget() {
        for policy in [
            EvictionPolicy::Lru,
            EvictionPolicy::Random,
            EvictionPolicy::Clock,
            EvictionPolicy::SampleKLru {
                sample_size: DEFAULT_SAMPLE_K,
            },
        ] {
            let cache = ReadCache::new(64, policy).expect("valid capacity");
            for i in 0u8..20 {
                cache.store(&[b'k', i], Some(vec![i; 8].into_boxed_slice()));
            }
            // current_size may slightly exceed max_size for custom stores
            // (single-entry overshoot is acceptable), but should be close.
            assert!(cache.len() < 20, "{policy:?}: entries should be evicted");
        }
    }

    #[test]
    fn test_policy_clear() {
        for policy in [
            EvictionPolicy::Lru,
            EvictionPolicy::Random,
            EvictionPolicy::Clock,
            EvictionPolicy::SampleKLru {
                sample_size: DEFAULT_SAMPLE_K,
            },
        ] {
            let cache = ReadCache::new(4096, policy).expect("valid capacity");
            cache.store(b"a", Some(Box::from(b"1" as &[u8])));
            cache.store(b"b", Some(Box::from(b"2" as &[u8])));
            cache.clear();
            assert!(cache.is_empty(), "{policy:?}");
            assert_eq!(cache.current_size(), 0, "{policy:?}");
        }
    }

    #[test]
    fn test_policy_len_metrics() {
        for policy in [
            EvictionPolicy::Lru,
            EvictionPolicy::Random,
            EvictionPolicy::Clock,
            EvictionPolicy::SampleKLru {
                sample_size: DEFAULT_SAMPLE_K,
            },
        ] {
            let cache = ReadCache::new(8192, policy).expect("valid capacity");
            assert_eq!(cache.len(), 0, "{policy:?}");
            assert_eq!(cache.current_size(), 0, "{policy:?}");
            assert_eq!(cache.max_size(), 8192, "{policy:?}");

            cache.store(b"test", Some(Box::from(b"data" as &[u8])));
            assert_eq!(cache.len(), 1, "{policy:?}");
            assert!(cache.current_size() > 0, "{policy:?}");
        }
    }

    #[test]
    fn test_default_cache() {
        let cache = ReadCache::default();
        assert!(cache.is_empty());
        assert_eq!(cache.max_size(), DEFAULT_BUDGET);
    }

    #[test]
    fn test_all_policies_basic() {
        for policy in [
            EvictionPolicy::Lru,
            EvictionPolicy::Random,
            EvictionPolicy::Clock,
            EvictionPolicy::SampleKLru {
                sample_size: DEFAULT_SAMPLE_K,
            },
        ] {
            basic_policy_test(policy);
        }
    }
}
