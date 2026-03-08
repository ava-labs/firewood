// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
)

func TestTruncatedTrieCreateAndFree(t *testing.T) {
	dbDir := filepath.Join(os.TempDir(), "firewood-truncated-trie-test")
	defer os.RemoveAll(dbDir)

	db, err := newDatabase(dbDir, WithTruncate(true))
	require.NoError(t, err)
	defer func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		require.NoError(t, db.Close(ctx))
	}()

	// Insert some data so the trie is non-empty
	root, err := db.Update([]BatchOp{
		Put([]byte("key1"), []byte("value1")),
		Put([]byte("key2"), []byte("value2")),
		Put([]byte("key3"), []byte("value3")),
	})
	require.NoError(t, err)
	require.NotEqual(t, EmptyRoot, root)

	// Create a truncated trie
	trie, err := db.CreateTruncatedTrie(root, 2)
	require.NoError(t, err)
	require.NotNil(t, trie)
	defer func() {
		require.NoError(t, trie.Free())
	}()

	// Root hash should match
	require.Equal(t, root, trie.Root())

	// RootHash method should also return the same hash
	hash, err := trie.RootHash()
	require.NoError(t, err)
	require.Equal(t, root, hash)

	// Verify root hash should succeed
	require.NoError(t, trie.VerifyRootHash(root))

	// Verify with wrong hash should fail
	var wrongHash Hash
	wrongHash[0] = 0xFF
	err = trie.VerifyRootHash(wrongHash)
	require.Error(t, err)
}

func TestTruncatedTrieFreeTwice(t *testing.T) {
	dbDir := filepath.Join(os.TempDir(), "firewood-truncated-trie-free-twice")
	defer os.RemoveAll(dbDir)

	db, err := newDatabase(dbDir, WithTruncate(true))
	require.NoError(t, err)
	defer func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		require.NoError(t, db.Close(ctx))
	}()

	root, err := db.Update([]BatchOp{
		Put([]byte("key1"), []byte("value1")),
	})
	require.NoError(t, err)

	trie, err := db.CreateTruncatedTrie(root, 2)
	require.NoError(t, err)

	// First free should succeed
	require.NoError(t, trie.Free())
	// Second free should be a no-op
	require.NoError(t, trie.Free())
}

func TestWitnessRoundTrip(t *testing.T) {
	dbDir := filepath.Join(os.TempDir(), "firewood-witness-roundtrip")
	defer os.RemoveAll(dbDir)

	db, err := newDatabase(dbDir, WithTruncate(true))
	require.NoError(t, err)
	defer func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		require.NoError(t, db.Close(ctx))
	}()

	// Insert initial data
	oldRoot, err := db.Update([]BatchOp{
		Put([]byte("apple"), []byte("red")),
		Put([]byte("banana"), []byte("yellow")),
	})
	require.NoError(t, err)
	require.NotEqual(t, EmptyRoot, oldRoot)

	// Create truncated trie from old state
	trie, err := db.CreateTruncatedTrie(oldRoot, 2)
	require.NoError(t, err)
	defer func() {
		require.NoError(t, trie.Free())
	}()

	// Apply a batch to get new root
	batch := []BatchOp{
		Put([]byte("cherry"), []byte("dark red")),
	}
	newRoot, err := db.Update(batch)
	require.NoError(t, err)
	require.NotEqual(t, oldRoot, newRoot)

	// Generate witness proof
	witness, err := db.GenerateWitness(oldRoot, batch, newRoot, 2)
	require.NoError(t, err)
	require.NotNil(t, witness)
	defer func() {
		require.NoError(t, witness.Free())
	}()

	// Verify witness - should produce a new truncated trie
	newTrie, err := trie.VerifyWitness(witness, batch)
	require.NoError(t, err)
	require.NotNil(t, newTrie)
	defer func() {
		require.NoError(t, newTrie.Free())
	}()

	// New trie root should match the new root
	require.Equal(t, newRoot, newTrie.Root())
}
