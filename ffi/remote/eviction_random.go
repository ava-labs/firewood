// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"math/rand/v2"
	"sync"
)

// randomEntry pairs a cache entry with its index in the keys slice.
type randomEntry struct {
	entry cacheEntry
	index int
}

// randomStore implements evictionStore by evicting a uniformly random entry
// when the cache is at capacity.
type randomStore struct {
	mu      sync.Mutex
	items   map[string]*randomEntry
	order   []string // dense slice for O(1) random selection
	maxSize int
	rng     *rand.Rand
}

func newRandomStore(maxSize int) *randomStore {
	return &randomStore{
		items:   make(map[string]*randomEntry, maxSize),
		order:   make([]string, 0, maxSize),
		maxSize: maxSize,
		rng:     rand.New(rand.NewPCG(rand.Uint64(), rand.Uint64())),
	}
}

func (s *randomStore) get(key string) (cacheEntry, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()

	e, ok := s.items[key]
	if !ok {
		return cacheEntry{}, false
	}
	return e.entry, true
}

func (s *randomStore) put(key string, entry cacheEntry) {
	s.mu.Lock()
	defer s.mu.Unlock()

	if e, ok := s.items[key]; ok {
		e.entry = entry
		return
	}

	// Evict a random entry if at capacity.
	if len(s.items) >= s.maxSize {
		idx := s.randIntn(len(s.order))
		victim := s.order[idx]
		// Swap-with-last and truncate.
		last := len(s.order) - 1
		if idx != last {
			s.order[idx] = s.order[last]
			s.items[s.order[idx]].index = idx
		}
		s.order = s.order[:last]
		delete(s.items, victim)
	}

	idx := len(s.order)
	s.order = append(s.order, key)
	s.items[key] = &randomEntry{entry: entry, index: idx}
}

func (s *randomStore) del(key string) bool {
	s.mu.Lock()
	defer s.mu.Unlock()

	e, ok := s.items[key]
	if !ok {
		return false
	}

	idx := e.index
	last := len(s.order) - 1
	if idx != last {
		s.order[idx] = s.order[last]
		s.items[s.order[idx]].index = idx
	}
	s.order = s.order[:last]
	delete(s.items, key)
	return true
}

func (s *randomStore) keys(fn func(key string)) {
	s.mu.Lock()
	defer s.mu.Unlock()

	for _, key := range s.order {
		fn(key)
	}
}

func (s *randomStore) clear() {
	s.mu.Lock()
	defer s.mu.Unlock()

	s.items = make(map[string]*randomEntry, s.maxSize)
	s.order = s.order[:0]
}

func (s *randomStore) len() int {
	s.mu.Lock()
	defer s.mu.Unlock()

	return len(s.items)
}

// randIntn returns a random int in [0, n). Caller must hold s.mu.
func (s *randomStore) randIntn(n int) int {
	return s.rng.IntN(n)
}
