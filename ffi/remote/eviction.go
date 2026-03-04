// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import "fmt"

// EvictionPolicy identifies a cache eviction strategy.
type EvictionPolicy int

const (
	// LRU evicts the least-recently-used entry.
	LRU EvictionPolicy = iota
	// RandomEviction evicts a uniformly random entry.
	RandomEviction
	// Clock approximates LRU using a clock-sweep algorithm.
	Clock
	// SampleKLRU samples K random entries and evicts the least-recently-used
	// among them (similar to Redis's approximated LRU).
	SampleKLRU
)

// evictionStore is the internal interface that each eviction policy implements.
// All methods must be safe for concurrent use by multiple goroutines.
type evictionStore interface {
	// get retrieves an entry and updates access metadata (e.g., moves to
	// front in LRU, sets referenced bit in Clock).
	get(key string) (cacheEntry, bool)

	// put stores an entry, evicting one if at capacity. If the key already
	// exists, the entry is updated in place without counting as a new entry.
	put(key string, entry cacheEntry)

	// del removes a specific key. Returns true if the key was present.
	del(key string) bool

	// keys calls fn for every key currently in the store. The caller must
	// not call other methods on the store from within fn.
	keys(fn func(key string))

	// clear removes all entries.
	clear()

	// len returns the current number of entries.
	len() int
}

// newEvictionStore creates an evictionStore for the given policy and capacity.
func newEvictionStore(policy EvictionPolicy, maxSize int) evictionStore {
	switch policy {
	case LRU:
		return newLRUStore(maxSize)
	case RandomEviction:
		return newRandomStore(maxSize)
	case Clock:
		return newClockStore(maxSize)
	case SampleKLRU:
		return newSampleKLRUStore(maxSize, 5)
	default:
		panic(fmt.Sprintf("unknown eviction policy: %d", policy))
	}
}
