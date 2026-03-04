// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"bytes"
	"sync"
	"sync/atomic"

	ffi "github.com/ava-labs/firewood/ffi"
)

// cacheEntry holds a cached Get result.
type cacheEntry struct {
	value []byte
	found bool
}

// readCache is a concurrency-safe read cache for verified Get results.
// It uses sync.Map for lock-free reads and an atomic counter for admission
// control. When the cache reaches maxSize, new entries are silently dropped.
type readCache struct {
	entries sync.Map
	size    atomic.Int64
	maxSize int64
}

// newReadCache creates a new read cache with the given maximum number of entries.
func newReadCache(maxSize int) *readCache {
	return &readCache{
		maxSize: int64(maxSize),
	}
}

// lookup returns the cached entry for key, if present.
func (rc *readCache) lookup(key []byte) (cacheEntry, bool) {
	v, ok := rc.entries.Load(string(key))
	if !ok {
		return cacheEntry{}, false
	}
	return v.(cacheEntry), true
}

// store adds or updates a cache entry. If the cache is at capacity and the key
// is not already present, the entry is silently dropped.
func (rc *readCache) store(key []byte, entry cacheEntry) {
	k := string(key)
	if _, loaded := rc.entries.Swap(k, entry); !loaded {
		// New entry — check admission.
		if rc.size.Add(1) > rc.maxSize {
			// Over capacity — undo.
			rc.entries.Delete(k)
			rc.size.Add(-1)
		}
	}
}

// invalidateKey removes a single key from the cache.
func (rc *readCache) invalidateKey(key []byte) {
	if _, loaded := rc.entries.LoadAndDelete(string(key)); loaded {
		rc.size.Add(-1)
	}
}

// invalidatePrefix removes all cached keys that start with prefix.
func (rc *readCache) invalidatePrefix(prefix []byte) {
	rc.entries.Range(func(key, _ any) bool {
		if bytes.HasPrefix([]byte(key.(string)), prefix) {
			if _, loaded := rc.entries.LoadAndDelete(key); loaded {
				rc.size.Add(-1)
			}
		}
		return true
	})
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
	rc.entries.Clear()
	rc.size.Store(0)
}
