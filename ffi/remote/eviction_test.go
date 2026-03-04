// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"fmt"
	"testing"
)

// forEachPolicy runs fn as a sub-test for each eviction policy.
func forEachPolicy(t *testing.T, maxSize int, fn func(t *testing.T, s evictionStore)) {
	t.Helper()
	policies := []struct {
		name   string
		policy EvictionPolicy
	}{
		{"LRU", LRU},
		{"Random", RandomEviction},
		{"Clock", Clock},
		{"SampleKLRU", SampleKLRU},
	}
	for _, p := range policies {
		t.Run(p.name, func(t *testing.T) {
			s := newEvictionStore(p.policy, maxSize)
			fn(t, s)
		})
	}
}

// ---------------------------------------------------------------------------
// Parameterized tests across all 4 policies
// ---------------------------------------------------------------------------

func TestEvictionStore_GetMiss(t *testing.T) {
	forEachPolicy(t, 10, func(t *testing.T, s evictionStore) {
		if _, ok := s.get("missing"); ok {
			t.Fatal("expected miss on empty store")
		}
	})
}

func TestEvictionStore_PutAndGet(t *testing.T) {
	forEachPolicy(t, 10, func(t *testing.T, s evictionStore) {
		s.put("k", cacheEntry{value: []byte("v"), found: true})
		entry, ok := s.get("k")
		if !ok {
			t.Fatal("expected hit")
		}
		if string(entry.value) != "v" || !entry.found {
			t.Fatalf("got %+v, want value=v found=true", entry)
		}
	})
}

func TestEvictionStore_PutOverwrite(t *testing.T) {
	forEachPolicy(t, 10, func(t *testing.T, s evictionStore) {
		s.put("k", cacheEntry{value: []byte("v1"), found: true})
		s.put("k", cacheEntry{value: []byte("v2"), found: true})

		entry, ok := s.get("k")
		if !ok {
			t.Fatal("expected hit")
		}
		if string(entry.value) != "v2" {
			t.Fatalf("got %q, want %q", entry.value, "v2")
		}
		if s.len() != 1 {
			t.Fatalf("len = %d, want 1", s.len())
		}
	})
}

func TestEvictionStore_Del(t *testing.T) {
	forEachPolicy(t, 10, func(t *testing.T, s evictionStore) {
		s.put("k", cacheEntry{value: []byte("v"), found: true})
		if !s.del("k") {
			t.Fatal("del returned false for present key")
		}
		if _, ok := s.get("k"); ok {
			t.Fatal("expected miss after del")
		}
		if s.len() != 0 {
			t.Fatalf("len = %d, want 0", s.len())
		}
	})
}

func TestEvictionStore_DelMissing(t *testing.T) {
	forEachPolicy(t, 10, func(t *testing.T, s evictionStore) {
		if s.del("nope") {
			t.Fatal("del returned true for missing key")
		}
	})
}

func TestEvictionStore_Clear(t *testing.T) {
	forEachPolicy(t, 10, func(t *testing.T, s evictionStore) {
		for i := range 5 {
			s.put(fmt.Sprintf("k%d", i), cacheEntry{value: []byte("v"), found: true})
		}
		s.clear()
		if s.len() != 0 {
			t.Fatalf("len = %d after clear, want 0", s.len())
		}
		for i := range 5 {
			if _, ok := s.get(fmt.Sprintf("k%d", i)); ok {
				t.Fatalf("expected miss for k%d after clear", i)
			}
		}
	})
}

func TestEvictionStore_Eviction(t *testing.T) {
	forEachPolicy(t, 3, func(t *testing.T, s evictionStore) {
		// Insert maxSize+2 entries.
		for i := range 5 {
			s.put(fmt.Sprintf("k%d", i), cacheEntry{value: []byte("v"), found: true})
		}
		if s.len() > 3 {
			t.Fatalf("len = %d, want <= 3", s.len())
		}
	})
}

func TestEvictionStore_EvictionDoesNotLoseNewEntry(t *testing.T) {
	forEachPolicy(t, 3, func(t *testing.T, s evictionStore) {
		for i := range 3 {
			s.put(fmt.Sprintf("k%d", i), cacheEntry{value: []byte("v"), found: true})
		}
		// Insert one more; must evict an old one but keep the new one.
		s.put("new", cacheEntry{value: []byte("new"), found: true})
		entry, ok := s.get("new")
		if !ok {
			t.Fatal("newly inserted entry was lost during eviction")
		}
		if string(entry.value) != "new" {
			t.Fatalf("got %q, want %q", entry.value, "new")
		}
		if s.len() != 3 {
			t.Fatalf("len = %d, want 3", s.len())
		}
	})
}

func TestEvictionStore_Keys(t *testing.T) {
	forEachPolicy(t, 10, func(t *testing.T, s evictionStore) {
		expected := map[string]bool{"a": true, "b": true, "c": true}
		for k := range expected {
			s.put(k, cacheEntry{value: []byte("v"), found: true})
		}
		visited := make(map[string]bool)
		s.keys(func(key string) {
			if visited[key] {
				t.Fatalf("key %q visited twice", key)
			}
			visited[key] = true
		})
		if len(visited) != len(expected) {
			t.Fatalf("visited %d keys, want %d", len(visited), len(expected))
		}
		for k := range expected {
			if !visited[k] {
				t.Fatalf("key %q not visited", k)
			}
		}
	})
}

func TestEvictionStore_DelThenInsert(t *testing.T) {
	forEachPolicy(t, 3, func(t *testing.T, s evictionStore) {
		// Fill to capacity.
		s.put("a", cacheEntry{value: []byte("1"), found: true})
		s.put("b", cacheEntry{value: []byte("2"), found: true})
		s.put("c", cacheEntry{value: []byte("3"), found: true})

		// Delete one, then insert a new one — should not evict.
		s.del("b")
		s.put("d", cacheEntry{value: []byte("4"), found: true})

		if s.len() != 3 {
			t.Fatalf("len = %d, want 3", s.len())
		}
		// "d" must be present.
		if _, ok := s.get("d"); !ok {
			t.Fatal("expected hit for d")
		}
		// "b" must be gone.
		if _, ok := s.get("b"); ok {
			t.Fatal("expected miss for deleted b")
		}
	})
}

// ---------------------------------------------------------------------------
// Policy-specific tests
// ---------------------------------------------------------------------------

func TestLRUStore_EvictsLeastRecent(t *testing.T) {
	s := newLRUStore(2)

	s.put("a", cacheEntry{value: []byte("1"), found: true})
	s.put("b", cacheEntry{value: []byte("2"), found: true})
	// Touch "a" so it becomes most recently used.
	s.get("a")
	// Insert "c" — should evict "b" (LRU).
	s.put("c", cacheEntry{value: []byte("3"), found: true})

	if _, ok := s.get("b"); ok {
		t.Fatal("expected b to be evicted")
	}
	if _, ok := s.get("a"); !ok {
		t.Fatal("expected a to survive (was touched)")
	}
	if _, ok := s.get("c"); !ok {
		t.Fatal("expected c to be present")
	}
}

func TestLRUStore_EvictsOldestWithoutTouch(t *testing.T) {
	s := newLRUStore(2)

	s.put("a", cacheEntry{value: []byte("1"), found: true})
	s.put("b", cacheEntry{value: []byte("2"), found: true})
	// No touch — "a" is LRU.
	s.put("c", cacheEntry{value: []byte("3"), found: true})

	if _, ok := s.get("a"); ok {
		t.Fatal("expected a to be evicted (oldest)")
	}
	if _, ok := s.get("b"); !ok {
		t.Fatal("expected b to survive")
	}
}

func TestClockStore_EvictsUnreferenced(t *testing.T) {
	// Clock needs two eviction cycles to demonstrate the referenced-bit
	// advantage. In the first eviction all bits are cleared; entries accessed
	// after the clear survive the second eviction.
	s := newClockStore(3)

	s.put("a", cacheEntry{value: []byte("1"), found: true})
	s.put("b", cacheEntry{value: []byte("2"), found: true})
	s.put("c", cacheEntry{value: []byte("3"), found: true})
	// First eviction: sweep clears all ref bits, then evicts "a" (first in order).
	s.put("d", cacheEntry{value: []byte("4"), found: true})

	// Now b and c have referenced=false (cleared by the sweep).
	// Touch "b" — sets its referenced bit.
	s.get("b")
	// Second eviction: sweep finds c unreferenced first, evicts it.
	s.put("e", cacheEntry{value: []byte("5"), found: true})

	if _, ok := s.get("c"); ok {
		t.Fatal("expected c to be evicted (unreferenced after sweep)")
	}
	if _, ok := s.get("b"); !ok {
		t.Fatal("expected b to survive (re-referenced after sweep)")
	}
	if _, ok := s.get("d"); !ok {
		t.Fatal("expected d to be present")
	}
	if _, ok := s.get("e"); !ok {
		t.Fatal("expected e to be present")
	}
}

func TestSampleKLRU_EvictsOldAccess(t *testing.T) {
	// Use K much larger than n to guarantee all entries are sampled,
	// eliminating random collision effects. With full coverage the
	// eviction always picks the entry with the oldest lastAccess.
	s := newSampleKLRUStore(3, 100)

	s.put("a", cacheEntry{value: []byte("1"), found: true})
	s.put("b", cacheEntry{value: []byte("2"), found: true})
	s.put("c", cacheEntry{value: []byte("3"), found: true})
	// Touch "a" and "c" — "b" has the oldest access.
	s.get("a")
	s.get("c")
	// Insert "d" — should evict "b".
	s.put("d", cacheEntry{value: []byte("4"), found: true})

	if _, ok := s.get("b"); ok {
		t.Fatal("expected b to be evicted (oldest access)")
	}
	if _, ok := s.get("a"); !ok {
		t.Fatal("expected a to survive")
	}
	if _, ok := s.get("c"); !ok {
		t.Fatal("expected c to survive")
	}
	if _, ok := s.get("d"); !ok {
		t.Fatal("expected d to be present")
	}
}

func TestRandomStore_EvictsOne(t *testing.T) {
	s := newRandomStore(3)

	s.put("a", cacheEntry{value: []byte("1"), found: true})
	s.put("b", cacheEntry{value: []byte("2"), found: true})
	s.put("c", cacheEntry{value: []byte("3"), found: true})
	// Insert "d" — exactly one of {a, b, c} should be evicted.
	s.put("d", cacheEntry{value: []byte("4"), found: true})

	if s.len() != 3 {
		t.Fatalf("len = %d, want 3", s.len())
	}
	// New entry must be present.
	if _, ok := s.get("d"); !ok {
		t.Fatal("expected d to be present")
	}
	// Exactly one of the originals should be gone.
	present := 0
	for _, k := range []string{"a", "b", "c"} {
		if _, ok := s.get(k); ok {
			present++
		}
	}
	if present != 2 {
		t.Fatalf("expected 2 of {a,b,c} to survive, got %d", present)
	}
}
