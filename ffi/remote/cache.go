// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"bytes"

	ffi "github.com/ava-labs/firewood/ffi"
)

// cacheEntry holds a cached Get result.
type cacheEntry struct {
	value []byte
	found bool
}

// readCache is a concurrency-safe read cache for verified Get results.
// It delegates storage and eviction to a pluggable [evictionStore] backend.
type readCache struct {
	backend evictionStore
}

// newReadCache creates a new read cache with the given maximum number of
// entries and eviction policy.
func newReadCache(maxSize int, policy EvictionPolicy) *readCache {
	return &readCache{
		backend: newEvictionStore(policy, maxSize),
	}
}

// lookup returns the cached entry for key, if present.
func (rc *readCache) lookup(key []byte) (cacheEntry, bool) {
	return rc.backend.get(string(key))
}

// store adds or updates a cache entry. If the cache is at capacity, an
// existing entry is evicted according to the configured eviction policy.
func (rc *readCache) store(key []byte, entry cacheEntry) {
	rc.backend.put(string(key), entry)
}

// invalidateKey removes a single key from the cache.
func (rc *readCache) invalidateKey(key []byte) {
	rc.backend.del(string(key))
}

// invalidatePrefix removes all cached keys that start with prefix.
// This uses a two-pass approach: first collect matching keys, then delete
// them. Safe because this method is only called under the client's write
// lock, so no concurrent store/lookup calls can add new matching keys
// between the two passes.
func (rc *readCache) invalidatePrefix(prefix []byte) {
	var toDelete []string
	rc.backend.keys(func(key string) {
		if bytes.HasPrefix([]byte(key), prefix) {
			toDelete = append(toDelete, key)
		}
	})
	for _, key := range toDelete {
		rc.backend.del(key)
	}
}

// invalidateBatch processes a slice of batch operations and invalidates
// affected cache entries.
func (rc *readCache) invalidateBatch(ops []ffi.BatchOp) {
	for _, op := range ops {
		if op.IsPrefixDelete() {
			rc.invalidatePrefix(op.Key())
		} else {
			rc.invalidateKey(op.Key())
		}
	}
}

// clear removes all entries from the cache.
func (rc *readCache) clear() {
	rc.backend.clear()
}

// len returns the number of entries currently in the cache.
func (rc *readCache) len() int {
	return rc.backend.len()
}
