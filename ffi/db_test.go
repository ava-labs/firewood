// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"context"
	"path/filepath"
	"testing"
	"time"
)

func newLocalDB(t *testing.T) DB {
	t.Helper()
	dbFile := filepath.Join(t.TempDir(), "test.db")
	db, err := newDatabase(dbFile)
	if err != nil {
		t.Fatalf("newDatabase: %v", err)
	}
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		if err := db.Close(ctx); err != nil {
			t.Errorf("db.Close: %v", err)
		}
	})
	return NewLocalDB(db)
}

func TestNewLocalDB(t *testing.T) {
	db := newLocalDB(t)
	ctx := t.Context()

	// Empty database should have the empty root.
	root := db.Root()

	// Insert a key.
	newRoot, err := db.Update(ctx, []BatchOp{Put([]byte("hello"), []byte("world"))})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}
	if newRoot == root {
		t.Fatal("root should change after update")
	}

	// Get the key.
	val, err := db.Get(ctx, []byte("hello"))
	if err != nil {
		t.Fatalf("Get: %v", err)
	}
	if string(val) != "world" {
		t.Fatalf("Get = %q, want %q", val, "world")
	}

	// Get non-existent key.
	val, err = db.Get(ctx, []byte("missing"))
	if err != nil {
		t.Fatalf("Get(missing): %v", err)
	}
	if val != nil {
		t.Fatalf("Get(missing) = %q, want nil", val)
	}
}

func TestLocalDBPropose(t *testing.T) {
	db := newLocalDB(t)
	ctx := t.Context()

	// Insert initial data.
	_, err := db.Update(ctx, []BatchOp{Put([]byte("a"), []byte("1"))})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}
	oldRoot := db.Root()

	// Propose a change.
	prop, err := db.Propose(ctx, []BatchOp{Put([]byte("b"), []byte("2"))})
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}

	// Proposal has a different root.
	if prop.Root() == oldRoot {
		t.Fatal("proposal root should differ from db root")
	}

	// Read from proposal.
	val, err := prop.Get(ctx, []byte("b"))
	if err != nil {
		t.Fatalf("Proposal.Get: %v", err)
	}
	if string(val) != "2" {
		t.Fatalf("Proposal.Get = %q, want %q", val, "2")
	}

	// DB should not yet see the new key.
	val, err = db.Get(ctx, []byte("b"))
	if err != nil {
		t.Fatalf("DB.Get before commit: %v", err)
	}
	if val != nil {
		t.Fatalf("DB.Get(b) before commit = %q, want nil", val)
	}

	// Commit.
	if err := prop.Commit(ctx); err != nil {
		t.Fatalf("Commit: %v", err)
	}

	// DB root should now match proposal root.
	if db.Root() != prop.Root() {
		t.Fatal("db root should match proposal root after commit")
	}
}

func TestLocalDBProposalChain(t *testing.T) {
	db := newLocalDB(t)
	ctx := t.Context()

	_, err := db.Update(ctx, []BatchOp{Put([]byte("a"), []byte("1"))})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}

	// Create first proposal.
	p1, err := db.Propose(ctx, []BatchOp{Put([]byte("b"), []byte("2"))})
	if err != nil {
		t.Fatalf("Propose p1: %v", err)
	}

	// Chain a second proposal on top of p1.
	p2, err := p1.Propose(ctx, []BatchOp{Put([]byte("c"), []byte("3"))})
	if err != nil {
		t.Fatalf("Propose p2: %v", err)
	}

	// p2 should see all three keys.
	for _, tc := range []struct {
		key, want string
	}{
		{"a", "1"},
		{"b", "2"},
		{"c", "3"},
	} {
		val, err := p2.Get(ctx, []byte(tc.key))
		if err != nil {
			t.Fatalf("p2.Get(%s): %v", tc.key, err)
		}
		if string(val) != tc.want {
			t.Fatalf("p2.Get(%s) = %q, want %q", tc.key, val, tc.want)
		}
	}

	// Commit chain: p1 first, then p2.
	if err := p1.Commit(ctx); err != nil {
		t.Fatalf("p1.Commit: %v", err)
	}
	if err := p2.Commit(ctx); err != nil {
		t.Fatalf("p2.Commit: %v", err)
	}
}

func TestLocalDBProposalIter(t *testing.T) {
	db := newLocalDB(t)
	ctx := t.Context()

	// Insert some data via proposal and commit.
	_, err := db.Update(ctx, []BatchOp{
		Put([]byte("a"), []byte("1")),
		Put([]byte("b"), []byte("2")),
		Put([]byte("c"), []byte("3")),
	})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}

	// Create proposal with additional key.
	prop, err := db.Propose(ctx, []BatchOp{Put([]byte("d"), []byte("4"))})
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}
	defer prop.Drop()

	// Iterate from beginning.
	it, err := prop.Iter(ctx, nil)
	if err != nil {
		t.Fatalf("Iter: %v", err)
	}
	defer it.Drop()

	var keys []string
	for it.Next() {
		keys = append(keys, string(it.Key()))
	}
	if err := it.Err(); err != nil {
		t.Fatalf("Iter.Err: %v", err)
	}

	expected := []string{"a", "b", "c", "d"}
	if len(keys) != len(expected) {
		t.Fatalf("got %d keys, want %d: %v", len(keys), len(expected), keys)
	}
	for i, k := range keys {
		if k != expected[i] {
			t.Fatalf("key[%d] = %q, want %q", i, k, expected[i])
		}
	}
}
