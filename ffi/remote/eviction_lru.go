// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import "sync"

// lruNode is a doubly-linked list node for the LRU cache.
type lruNode struct {
	key   string
	entry cacheEntry
	prev  *lruNode
	next  *lruNode
}

// lruStore implements evictionStore using a map + doubly-linked list.
// The most recently used entry is at head.next; the least recently used
// is at tail.prev. head and tail are sentinel nodes that simplify
// insertion and removal by eliminating nil checks.
type lruStore struct {
	mu      sync.Mutex
	items   map[string]*lruNode
	head    lruNode // sentinel: head.next = MRU
	tail    lruNode // sentinel: tail.prev = LRU
	maxSize int
}

func newLRUStore(maxSize int) *lruStore {
	s := &lruStore{
		items:   make(map[string]*lruNode, maxSize),
		maxSize: maxSize,
	}
	s.head.next = &s.tail
	s.tail.prev = &s.head
	return s
}

func (s *lruStore) get(key string) (cacheEntry, bool) {
	s.mu.Lock()
	defer s.mu.Unlock()

	node, ok := s.items[key]
	if !ok {
		return cacheEntry{}, false
	}
	s.moveToFront(node)
	return node.entry, true
}

func (s *lruStore) put(key string, entry cacheEntry) {
	s.mu.Lock()
	defer s.mu.Unlock()

	if node, ok := s.items[key]; ok {
		node.entry = entry
		s.moveToFront(node)
		return
	}

	// Evict LRU entry if at capacity.
	if len(s.items) >= s.maxSize {
		victim := s.tail.prev
		s.unlink(victim)
		delete(s.items, victim.key)
	}

	node := &lruNode{key: key, entry: entry}
	s.items[key] = node
	s.insertAfter(&s.head, node)
}

func (s *lruStore) del(key string) bool {
	s.mu.Lock()
	defer s.mu.Unlock()

	node, ok := s.items[key]
	if !ok {
		return false
	}
	s.unlink(node)
	delete(s.items, key)
	return true
}

func (s *lruStore) keys(fn func(key string)) {
	s.mu.Lock()
	defer s.mu.Unlock()

	for key := range s.items {
		fn(key)
	}
}

func (s *lruStore) clear() {
	s.mu.Lock()
	defer s.mu.Unlock()

	s.items = make(map[string]*lruNode, s.maxSize)
	s.head.next = &s.tail
	s.tail.prev = &s.head
}

func (s *lruStore) len() int {
	s.mu.Lock()
	defer s.mu.Unlock()

	return len(s.items)
}

// moveToFront moves node to just after the head sentinel.
// Caller must hold s.mu.
func (s *lruStore) moveToFront(node *lruNode) {
	s.unlink(node)
	s.insertAfter(&s.head, node)
}

// unlink removes node from the linked list.
// Caller must hold s.mu.
func (s *lruStore) unlink(node *lruNode) {
	node.prev.next = node.next
	node.next.prev = node.prev
}

// insertAfter inserts node immediately after after.
// Caller must hold s.mu.
func (s *lruStore) insertAfter(after *lruNode, node *lruNode) {
	node.prev = after
	node.next = after.next
	after.next.prev = node
	after.next = node
}
