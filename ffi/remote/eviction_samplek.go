// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"math/rand/v2"
	"sync"
)

// sampleEntry pairs a cache entry with a monotonic access timestamp and its
// index in the keys slice.
type sampleEntry struct {
	entry      cacheEntry
	lastAccess uint64
	index      int
}

// sampleKLRUStore implements evictionStore by sampling K random entries and
// evicting the one with the oldest access time. This approximates LRU
// without maintaining a full ordering, similar to Redis's eviction strategy.
type sampleKLRUStore struct {
	mu      sync.Mutex
	items   map[string]*sampleEntry
	order   []string // dense slice for random sampling
	counter uint64   // monotonic access counter
	maxSize int
	k       int // number of random samples per eviction
	rng     *rand.Rand
}

func newSampleKLRUStore(maxSize, k int) *sampleKLRUStore {
	return &sampleKLRUStore{
		items:   make(map[string]*sampleEntry, maxSize),
		order:   make([]string, 0, maxSize),
		maxSize: maxSize,
		k:       k,
		rng:     rand.New(rand.NewPCG(rand.Uint64(), rand.Uint64())),
	}
}

func (s *sampleKLRUStore) get(key string) (cacheEntry, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()

	e, ok := s.items[key]
	if !ok {
		return cacheEntry{}, false
	}
	s.counter++
	e.lastAccess = s.counter
	return e.entry, true
}

func (s *sampleKLRUStore) put(key string, entry cacheEntry) {
	s.mu.Lock()
	defer s.mu.Unlock()

	s.counter++

	if e, ok := s.items[key]; ok {
		e.entry = entry
		e.lastAccess = s.counter
		return
	}

	// Evict via sampling if at capacity.
	if len(s.items) >= s.maxSize {
		s.evict()
	}

	idx := len(s.order)
	s.order = append(s.order, key)
	s.items[key] = &sampleEntry{entry: entry, lastAccess: s.counter, index: idx}
}

func (s *sampleKLRUStore) del(key string) bool {
	s.mu.Lock()
	defer s.mu.Unlock()

	e, ok := s.items[key]
	if !ok {
		return false
	}
	s.swapRemove(e.index)
	delete(s.items, key)
	return true
}

func (s *sampleKLRUStore) keys(fn func(key string)) {
	s.mu.Lock()
	defer s.mu.Unlock()

	for _, key := range s.order {
		fn(key)
	}
}

func (s *sampleKLRUStore) clear() {
	s.mu.Lock()
	defer s.mu.Unlock()

	s.items = make(map[string]*sampleEntry, s.maxSize)
	s.order = s.order[:0]
	s.counter = 0
}

func (s *sampleKLRUStore) len() int {
	s.mu.Lock()
	defer s.mu.Unlock()

	return len(s.items)
}

// evict samples up to K random entries and removes the one with the smallest
// lastAccess value. Caller must hold s.mu.
func (s *sampleKLRUStore) evict() {
	n := len(s.order)
	if n == 0 {
		return
	}

	samples := s.k
	if samples > n {
		samples = n
	}

	// Find the entry with the oldest access among up to K samples.
	bestIdx := -1
	var bestAccess uint64

	if samples >= n {
		// Full scan — guaranteed to find the true LRU entry.
		for i, key := range s.order {
			e := s.items[key]
			if bestIdx == -1 || e.lastAccess < bestAccess {
				bestIdx = i
				bestAccess = e.lastAccess
			}
		}
	} else {
		// Sample K distinct random indices.
		seen := make(map[int]bool, samples)
		for range samples {
			idx := s.randIntn(n)
			for attempts := 0; seen[idx] && attempts < samples; attempts++ {
				idx = s.randIntn(n)
			}
			if seen[idx] {
				continue
			}
			seen[idx] = true

			key := s.order[idx]
			e := s.items[key]
			if bestIdx == -1 || e.lastAccess < bestAccess {
				bestIdx = idx
				bestAccess = e.lastAccess
			}
		}
	}

	if bestIdx >= 0 {
		victimKey := s.order[bestIdx]
		s.swapRemove(bestIdx)
		delete(s.items, victimKey)
	}
}

// swapRemove removes the entry at idx from the keys slice by swapping it
// with the last element. Caller must hold s.mu.
func (s *sampleKLRUStore) swapRemove(idx int) {
	last := len(s.order) - 1
	if idx != last {
		s.order[idx] = s.order[last]
		s.items[s.order[idx]].index = idx
	}
	s.order = s.order[:last]
}

// randIntn returns a random int in [0, n). Caller must hold s.mu.
func (s *sampleKLRUStore) randIntn(n int) int {
	return s.rng.IntN(n)
}
