// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import "sync"

// clockNode is a node in the circular doubly-linked list used by the clock
// eviction algorithm.
type clockNode struct {
	key        string
	entry      cacheEntry
	referenced bool
	prev       *clockNode
	next       *clockNode
}

// clockStore implements evictionStore using the clock (second-chance)
// algorithm. Each entry has a "referenced" bit that is set on access. On
// eviction, the clock hand sweeps the circular list: referenced entries get
// their bit cleared; the first unreferenced entry is evicted.
type clockStore struct {
	mu      sync.Mutex
	items   map[string]*clockNode
	ring    clockNode // sentinel for the circular list
	hand    *clockNode
	maxSize int
}

func newClockStore(maxSize int) *clockStore {
	s := &clockStore{
		items:   make(map[string]*clockNode, maxSize),
		maxSize: maxSize,
	}
	s.ring.next = &s.ring
	s.ring.prev = &s.ring
	s.hand = &s.ring
	return s
}

func (s *clockStore) get(key string) (cacheEntry, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()

	node, ok := s.items[key]
	if !ok {
		return cacheEntry{}, false
	}
	node.referenced = true
	return node.entry, true
}

func (s *clockStore) put(key string, entry cacheEntry) {
	s.mu.Lock()
	defer s.mu.Unlock()

	if node, ok := s.items[key]; ok {
		node.entry = entry
		node.referenced = true
		return
	}

	// Evict if at capacity.
	if len(s.items) >= s.maxSize {
		s.evict()
	}

	node := &clockNode{key: key, entry: entry, referenced: true}
	s.items[key] = node
	// Insert before the sentinel (i.e., at the end of the ring).
	s.insertBefore(&s.ring, node)
	// If hand was pointing to sentinel (empty ring), advance to the new node.
	if s.hand == &s.ring {
		s.hand = node
	}
}

func (s *clockStore) del(key string) bool {
	s.mu.Lock()
	defer s.mu.Unlock()

	node, ok := s.items[key]
	if !ok {
		return false
	}
	if s.hand == node {
		s.advanceHand()
	}
	s.unlink(node)
	delete(s.items, key)
	return true
}

func (s *clockStore) keys(fn func(key string)) {
	s.mu.Lock()
	defer s.mu.Unlock()

	for key := range s.items {
		fn(key)
	}
}

func (s *clockStore) clear() {
	s.mu.Lock()
	defer s.mu.Unlock()

	s.items = make(map[string]*clockNode, s.maxSize)
	s.ring.next = &s.ring
	s.ring.prev = &s.ring
	s.hand = &s.ring
}

func (s *clockStore) len() int {
	s.mu.Lock()
	defer s.mu.Unlock()

	return len(s.items)
}

// evict removes one entry using the clock sweep algorithm. Caller must hold s.mu.
func (s *clockStore) evict() {
	for {
		// Skip the sentinel.
		if s.hand == &s.ring {
			s.hand = s.hand.next
			continue
		}
		if s.hand.referenced {
			s.hand.referenced = false
			s.hand = s.hand.next
			continue
		}
		// Found unreferenced entry — evict it.
		victim := s.hand
		s.advanceHand()
		s.unlink(victim)
		delete(s.items, victim.key)
		return
	}
}

// advanceHand moves the hand to the next node. Caller must hold s.mu.
func (s *clockStore) advanceHand() {
	s.hand = s.hand.next
}

// unlink removes node from the circular list. Caller must hold s.mu.
func (s *clockStore) unlink(node *clockNode) {
	node.prev.next = node.next
	node.next.prev = node.prev
}

// insertBefore inserts node immediately before before. Caller must hold s.mu.
func (s *clockStore) insertBefore(before *clockNode, node *clockNode) {
	node.next = before
	node.prev = before.prev
	before.prev.next = node
	before.prev = node
}
