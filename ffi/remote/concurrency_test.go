// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"context"
	"sync"
	"testing"

	ffi "github.com/ava-labs/firewood/ffi"
)

// TestConcurrentGets verifies that 10 goroutines can perform Get()
// simultaneously on 5 keys without errors or panics.
func TestConcurrentGets(t *testing.T) {
	db := newTestDB(t)

	data := map[string]string{
		"apple":  "red",
		"banana": "yellow",
		"cherry": "dark",
		"date":   "brown",
		"elder":  "purple",
	}
	rootHash := insertData(t, db, data)

	client := startServerAndClient(t, db, 4)
	ctx := t.Context()

	if err := client.Bootstrap(ctx, rootHash); err != nil {
		t.Fatalf("Bootstrap: %v", err)
	}

	keys := []string{"apple", "banana", "cherry", "date", "elder"}

	var wg sync.WaitGroup
	for i := 0; i < 10; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for _, key := range keys {
				val, err := client.Get(ctx, []byte(key))
				if err != nil {
					t.Errorf("Get(%s): %v", key, err)
					return
				}
				expected := data[key]
				if string(val) != expected {
					t.Errorf("Get(%s) = %q, want %q", key, val, expected)
				}
			}
		}()
	}
	wg.Wait()
}

// TestConcurrentGetDuringUpdate verifies that Get() goroutines running
// while Update() swaps the trie do not panic or return corrupted data.
func TestConcurrentGetDuringUpdate(t *testing.T) {
	db := newTestDB(t)

	data := map[string]string{"apple": "red", "banana": "yellow"}
	rootHash := insertData(t, db, data)

	client := startServerAndClient(t, db, 4)
	ctx := t.Context()

	if err := client.Bootstrap(ctx, rootHash); err != nil {
		t.Fatalf("Bootstrap: %v", err)
	}

	// Start concurrent readers.
	readerCtx, cancelReaders := context.WithCancel(ctx)
	defer cancelReaders()

	var wg sync.WaitGroup
	for i := 0; i < 5; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for {
				select {
				case <-readerCtx.Done():
					return
				default:
				}
				val, err := client.Get(ctx, []byte("apple"))
				if err != nil {
					t.Errorf("Get(apple): %v", err)
					return
				}
				if string(val) != "red" {
					t.Errorf("Get(apple) = %q, want %q", val, "red")
					return
				}
			}
		}()
	}

	// Perform an update while readers are running.
	_, err := client.Update(ctx, []ffi.BatchOp{
		ffi.Put([]byte("cherry"), []byte("dark")),
	})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}

	// Stop readers and wait.
	cancelReaders()
	wg.Wait()
}

// TestConcurrentGetDuringClose verifies that Get() goroutines running
// while Close() frees the trie return a value or a "not bootstrapped"
// error, without panicking.
func TestConcurrentGetDuringClose(t *testing.T) {
	db := newTestDB(t)

	data := map[string]string{"apple": "red"}
	rootHash := insertData(t, db, data)

	client := startServerAndClient(t, db, 4)
	ctx := t.Context()

	if err := client.Bootstrap(ctx, rootHash); err != nil {
		t.Fatalf("Bootstrap: %v", err)
	}

	var wg sync.WaitGroup
	for i := 0; i < 5; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := 0; j < 20; j++ {
				val, err := client.Get(ctx, []byte("apple"))
				if err != nil {
					// After Close, we expect "client not bootstrapped" or
					// a gRPC connection-closed error. Either is acceptable.
					return
				}
				if val != nil && string(val) != "red" {
					t.Errorf("Get(apple) = %q, want %q", val, "red")
					return
				}
			}
		}()
	}

	// Close while readers are running. The test cleanup registered by
	// startServerAndClient also calls Close, but Close is idempotent.
	if err := client.Close(); err != nil {
		t.Errorf("Close: %v", err)
	}

	wg.Wait()
}

// TestConcurrentGetAndRoot verifies that Get() and Root() can proceed
// concurrently since both only require a read lock.
func TestConcurrentGetAndRoot(t *testing.T) {
	db := newTestDB(t)

	data := map[string]string{"apple": "red", "banana": "yellow"}
	rootHash := insertData(t, db, data)

	client := startServerAndClient(t, db, 4)
	ctx := t.Context()

	if err := client.Bootstrap(ctx, rootHash); err != nil {
		t.Fatalf("Bootstrap: %v", err)
	}

	var wg sync.WaitGroup
	// 5 goroutines doing Get.
	for i := 0; i < 5; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := 0; j < 20; j++ {
				val, err := client.Get(ctx, []byte("apple"))
				if err != nil {
					t.Errorf("Get(apple): %v", err)
					return
				}
				if string(val) != "red" {
					t.Errorf("Get(apple) = %q, want %q", val, "red")
					return
				}
			}
		}()
	}
	// 5 goroutines doing Root.
	for i := 0; i < 5; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for j := 0; j < 20; j++ {
				root := client.Root()
				if root != rootHash {
					t.Errorf("Root() = %x, want %x", root, rootHash)
					return
				}
			}
		}()
	}
	wg.Wait()
}
