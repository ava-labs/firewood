// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"context"
	"sync"
	"testing"

	ffi "github.com/ava-labs/firewood/ffi"
)

// ---------------------------------------------------------------------------
// Unit tests for readCache
// ---------------------------------------------------------------------------

func TestCacheLookupMiss(t *testing.T) {
	c := newReadCache(100, LRU)
	if _, ok := c.lookup([]byte("missing")); ok {
		t.Fatal("expected miss on empty cache")
	}
}

func TestCacheStoreAndLookup(t *testing.T) {
	c := newReadCache(100, LRU)

	c.store([]byte("key"), cacheEntry{value: []byte("val"), found: true})

	entry, ok := c.lookup([]byte("key"))
	if !ok {
		t.Fatal("expected hit")
	}
	if !entry.found || string(entry.value) != "val" {
		t.Fatalf("got found=%v value=%q, want found=true value=%q", entry.found, entry.value, "val")
	}
}

func TestCacheNilValue(t *testing.T) {
	c := newReadCache(100, LRU)

	// Cache an exclusion proof (key verified to not exist).
	c.store([]byte("absent"), cacheEntry{value: nil, found: false})

	entry, ok := c.lookup([]byte("absent"))
	if !ok {
		t.Fatal("expected hit for nil-value entry")
	}
	if entry.found {
		t.Fatal("expected found=false for exclusion proof")
	}
	if entry.value != nil {
		t.Fatalf("expected nil value, got %q", entry.value)
	}
}

func TestCacheClear(t *testing.T) {
	c := newReadCache(100, LRU)

	c.store([]byte("a"), cacheEntry{value: []byte("1"), found: true})
	c.store([]byte("b"), cacheEntry{value: []byte("2"), found: true})
	c.clear()

	if _, ok := c.lookup([]byte("a")); ok {
		t.Fatal("expected miss after clear")
	}
	if _, ok := c.lookup([]byte("b")); ok {
		t.Fatal("expected miss after clear")
	}
	if c.len() != 0 {
		t.Fatalf("len = %d after clear, want 0", c.len())
	}
}

func TestCacheEviction(t *testing.T) {
	c := newReadCache(2, LRU)

	c.store([]byte("a"), cacheEntry{value: []byte("1"), found: true})
	c.store([]byte("b"), cacheEntry{value: []byte("2"), found: true})
	// Cache is full; this should evict an existing entry.
	c.store([]byte("c"), cacheEntry{value: []byte("3"), found: true})

	// The new entry must be present.
	if _, ok := c.lookup([]byte("c")); !ok {
		t.Fatal("expected hit for newly inserted entry at capacity")
	}
	if c.len() != 2 {
		t.Fatalf("len = %d, want 2", c.len())
	}

	// Overwriting an existing key should succeed even at capacity.
	c.store([]byte("c"), cacheEntry{value: []byte("updated"), found: true})
	entry, ok := c.lookup([]byte("c"))
	if !ok {
		t.Fatal("expected hit for overwritten key")
	}
	if string(entry.value) != "updated" {
		t.Fatalf("got %q, want %q", entry.value, "updated")
	}
	if c.len() != 2 {
		t.Fatalf("len = %d after overwrite, want 2", c.len())
	}
}

func TestCacheInvalidateKey(t *testing.T) {
	c := newReadCache(100, LRU)

	c.store([]byte("a"), cacheEntry{value: []byte("1"), found: true})
	c.store([]byte("b"), cacheEntry{value: []byte("2"), found: true})

	c.invalidateKey([]byte("a"))

	if _, ok := c.lookup([]byte("a")); ok {
		t.Fatal("expected miss after invalidateKey")
	}
	if _, ok := c.lookup([]byte("b")); !ok {
		t.Fatal("expected hit for non-invalidated key")
	}
	if c.len() != 1 {
		t.Fatalf("len = %d, want 1", c.len())
	}
}

func TestCacheInvalidatePrefix(t *testing.T) {
	c := newReadCache(100, LRU)

	c.store([]byte("a/1"), cacheEntry{value: []byte("v1"), found: true})
	c.store([]byte("a/2"), cacheEntry{value: []byte("v2"), found: true})
	c.store([]byte("b/1"), cacheEntry{value: []byte("v3"), found: true})

	c.invalidatePrefix([]byte("a/"))

	if _, ok := c.lookup([]byte("a/1")); ok {
		t.Fatal("expected miss for a/1 after prefix invalidation")
	}
	if _, ok := c.lookup([]byte("a/2")); ok {
		t.Fatal("expected miss for a/2 after prefix invalidation")
	}
	if _, ok := c.lookup([]byte("b/1")); !ok {
		t.Fatal("expected hit for b/1 after prefix invalidation of a/")
	}
	if c.len() != 1 {
		t.Fatalf("len = %d, want 1", c.len())
	}
}

func TestCacheInvalidateBatch(t *testing.T) {
	c := newReadCache(100, LRU)

	c.store([]byte("x"), cacheEntry{value: []byte("1"), found: true})
	c.store([]byte("y"), cacheEntry{value: []byte("2"), found: true})
	c.store([]byte("p/a"), cacheEntry{value: []byte("3"), found: true})
	c.store([]byte("p/b"), cacheEntry{value: []byte("4"), found: true})
	c.store([]byte("z"), cacheEntry{value: []byte("5"), found: true})

	ops := []ffi.BatchOp{
		ffi.Put([]byte("x"), []byte("new")),   // invalidate x
		ffi.Delete([]byte("y")),                // invalidate y
		ffi.PrefixDelete([]byte("p/")),         // invalidate p/a and p/b
	}
	c.invalidateBatch(ops)

	for _, key := range []string{"x", "y", "p/a", "p/b"} {
		if _, ok := c.lookup([]byte(key)); ok {
			t.Fatalf("expected miss for %q after invalidateBatch", key)
		}
	}
	if _, ok := c.lookup([]byte("z")); !ok {
		t.Fatal("expected hit for z after invalidateBatch")
	}
	if c.len() != 1 {
		t.Fatalf("len = %d, want 1", c.len())
	}
}

// ---------------------------------------------------------------------------
// Integration tests (with DB + gRPC)
// ---------------------------------------------------------------------------

// setupCachedRemoteDB is a test helper that creates a RemoteDB with caching enabled.
func setupCachedRemoteDB(t *testing.T, db *ffi.Database, rootHash ffi.Hash, depth uint, cacheSize int) ffi.DB {
	t.Helper()
	addr := startServerAndGetAddr(t, db)
	ctx := t.Context()
	rdb, err := NewRemoteDB(ctx, addr, rootHash, depth, WithCacheSize(cacheSize))
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	t.Cleanup(func() { rdb.Close(context.Background()) })
	return rdb
}

func TestCacheInvalidationOnUpdate(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{
		"apple":  "red",
		"banana": "yellow",
		"cherry": "dark",
	})

	rdb := setupCachedRemoteDB(t, db, rootHash, 4, 1000)

	// Read all keys to populate cache.
	for _, key := range []string{"apple", "banana", "cherry"} {
		val, err := rdb.Get(ctx, []byte(key))
		if err != nil {
			t.Fatalf("Get(%s): %v", key, err)
		}
		if val == nil {
			t.Fatalf("Get(%s) = nil, want non-nil", key)
		}
	}

	// Update only "banana".
	_, err := rdb.Update(ctx, []ffi.BatchOp{
		ffi.Put([]byte("banana"), []byte("green")),
	})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}

	// "banana" should return fresh value (cache was invalidated for it).
	val, err := rdb.Get(ctx, []byte("banana"))
	if err != nil {
		t.Fatalf("Get(banana): %v", err)
	}
	if string(val) != "green" {
		t.Fatalf("Get(banana) = %q, want %q", val, "green")
	}

	// "apple" and "cherry" should still be cached (untouched keys survive).
	for _, key := range []string{"apple", "cherry"} {
		val, err := rdb.Get(ctx, []byte(key))
		if err != nil {
			t.Fatalf("Get(%s): %v", key, err)
		}
		if val == nil {
			t.Fatalf("Get(%s) = nil after update of different key", key)
		}
	}
}

func TestCacheInvalidationOnBootstrap(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"a": "1"})
	addr := startServerAndGetAddr(t, db)

	client, err := NewClient(addr, 4, WithCacheSize(1000))
	if err != nil {
		t.Fatalf("NewClient: %v", err)
	}
	t.Cleanup(func() { client.Close() })

	if err := client.Bootstrap(ctx, rootHash); err != nil {
		t.Fatalf("Bootstrap: %v", err)
	}

	// Populate cache.
	val, err := client.Get(ctx, []byte("a"))
	if err != nil {
		t.Fatalf("Get(a): %v", err)
	}
	if string(val) != "1" {
		t.Fatalf("Get(a) = %q, want %q", val, "1")
	}

	// Re-bootstrap should clear cache.
	if err := client.Bootstrap(ctx, rootHash); err != nil {
		t.Fatalf("re-Bootstrap: %v", err)
	}

	if client.cache.len() != 0 {
		t.Fatalf("cache len = %d after re-bootstrap, want 0", client.cache.len())
	}
}

func TestCacheInvalidationOnCommit(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{
		"x": "1",
		"y": "2",
	})

	rdb := setupCachedRemoteDB(t, db, rootHash, 4, 1000)

	// Populate cache.
	for _, key := range []string{"x", "y"} {
		if _, err := rdb.Get(ctx, []byte(key)); err != nil {
			t.Fatalf("Get(%s): %v", key, err)
		}
	}

	// Propose a change to "x" and commit.
	prop, err := rdb.Propose(ctx, []ffi.BatchOp{
		ffi.Put([]byte("x"), []byte("updated")),
	})
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}
	if err := prop.Commit(ctx); err != nil {
		t.Fatalf("Commit: %v", err)
	}

	// "x" should return fresh value.
	val, err := rdb.Get(ctx, []byte("x"))
	if err != nil {
		t.Fatalf("Get(x): %v", err)
	}
	if string(val) != "updated" {
		t.Fatalf("Get(x) = %q, want %q", val, "updated")
	}

	// "y" should still be served (untouched by the proposal).
	val, err = rdb.Get(ctx, []byte("y"))
	if err != nil {
		t.Fatalf("Get(y): %v", err)
	}
	if string(val) != "2" {
		t.Fatalf("Get(y) = %q, want %q", val, "2")
	}
}

func TestCacheConcurrentGet(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	data := make(map[string]string, 20)
	for i := range 20 {
		data[string(rune('a'+i))] = string(rune('A' + i))
	}
	rootHash := insertData(t, db, data)

	rdb := setupCachedRemoteDB(t, db, rootHash, 4, 1000)

	// Concurrent reads should all succeed without races.
	var wg sync.WaitGroup
	for i := range 20 {
		wg.Add(1)
		go func(key string) {
			defer wg.Done()
			for range 10 {
				val, err := rdb.Get(ctx, []byte(key))
				if err != nil {
					t.Errorf("Get(%s): %v", key, err)
					return
				}
				if val == nil {
					t.Errorf("Get(%s) = nil, want non-nil", key)
					return
				}
			}
		}(string(rune('a' + i)))
	}
	wg.Wait()
}
