// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"errors"
	"math/rand"
	"testing"

	"github.com/stretchr/testify/require"
	"golang.org/x/sync/errgroup"
)

// kvCountDump is the number of key-value pairs generated for the dump
// and drop-then-use tests.
const kvCountDump = 4

func TestRevisionReconstructReadsAndChains(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	const (
		numKeys        = 10
		committedKeys  = 5
		firstBatchEnd  = 8
		secondBatchEnd = numKeys
	)
	keys, vals, batch := kvForTest(numKeys)
	root, err := db.Update(batch[:committedKeys])
	r.NoError(err)

	rev, err := db.Revision(root)
	r.NoError(err)
	t.Cleanup(func() {
		r.NoError(rev.Drop())
	})

	reconstructed, err := rev.Reconstruct(batch[committedKeys:firstBatchEnd])
	r.NoError(err)
	t.Cleanup(func() { r.NoError(reconstructed.Drop()) })

	r.NotEqual(EmptyRoot, reconstructed.Root())

	for i := range firstBatchEnd {
		got, err := reconstructed.Get(keys[i])
		r.NoError(err)
		r.Equal(vals[i], got)
	}

	for i := firstBatchEnd; i < len(keys); i++ {
		got, err := reconstructed.Get(keys[i])
		r.NoError(err)
		r.Nil(got)
	}

	oldRoot := reconstructed.Root()
	r.NoError(reconstructed.Reconstruct(batch[firstBatchEnd:secondBatchEnd]))
	r.NotEqual(oldRoot, reconstructed.Root())

	for i := range len(keys) {
		got, err := reconstructed.Get(keys[i])
		r.NoError(err)
		r.Equal(vals[i], got)
	}
}

func BenchmarkReconstructFromRevision(b *testing.B) {
	r := require.New(b)
	db := newTestDatabase(b)

	const numKeys = 1024
	_, _, batch := kvForTest(numKeys)
	root, err := db.Update(batch[:numKeys-1])
	r.NoError(err)

	rev, err := db.Revision(root)
	r.NoError(err)
	b.Cleanup(func() {
		r.NoError(rev.Drop())
	})

	reconstructBatch := batch[numKeys-1 : numKeys]

	b.ReportAllocs()
	b.ResetTimer()

	for i := 0; i < b.N; i++ {
		reconstructed, err := rev.Reconstruct(reconstructBatch)
		r.NoError(err)

		err = reconstructed.Drop()
		r.NoError(err)
	}
}

func BenchmarkReconstructChain(b *testing.B) {
	r := require.New(b)
	const (
		totalBatches             = 2_049 // first batch is committed, rest are reconstructed
		batchItems               = 100
		keyLen                   = 16
		valueLen                 = 32
		randSeedReconstructChain = 1234
		firstReconstructedBatch  = 2
	)

	rng := rand.New(rand.NewSource(randSeedReconstructChain))
	batches := make([][]BatchOp, 0, totalBatches)
	for range totalBatches {
		batches = append(batches, makeRandomBatch(b, rng, batchItems, keyLen, valueLen))
	}

	db := newTestDatabase(b)
	root, err := db.Update(batches[0])
	r.NoError(err)

	rev, err := db.Revision(root)
	r.NoError(err)
	b.Cleanup(func() {
		r.NoError(rev.Drop())
	})

	b.ReportAllocs()
	b.ResetTimer()

	for i := 0; i < b.N; i++ {
		current, err := rev.Reconstruct(batches[1])
		r.NoError(err)
		for _, batch := range batches[firstReconstructedBatch:] {
			r.NoError(current.Reconstruct(batch))
		}

		// Force root hash computation to include it in the benchmark.
		_ = current.Root()
		r.NoError(current.Drop())
	}
}

func TestReconstructedDump(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	_, _, batch := kvForTest(kvCountDump)
	root, err := db.Update(batch[:2])
	r.NoError(err)

	rev, err := db.Revision(root)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(rev.Drop()) })

	reconstructed, err := rev.Reconstruct(batch[2:kvCountDump])
	r.NoError(err)
	t.Cleanup(func() { r.NoError(reconstructed.Drop()) })

	dot, err := reconstructed.Dump()
	r.NoError(err)
	r.Contains(dot, "digraph")
}

func TestReconstructedDropThenUse(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	keys, _, batch := kvForTest(kvCountDump)
	root, err := db.Update(batch[:2])
	r.NoError(err)

	rev, err := db.Revision(root)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(rev.Drop()) })

	reconstructed, err := rev.Reconstruct(batch[2:kvCountDump])
	r.NoError(err)

	// First Drop succeeds.
	r.NoError(reconstructed.Drop())

	// Second Drop is a no-op.
	r.NoError(reconstructed.Drop())

	// All operations return ErrDroppedReconstructed after Drop.
	_, err = reconstructed.Get(keys[0])
	r.ErrorIs(err, ErrDroppedReconstructed)

	_, err = reconstructed.Iter(keys[0])
	r.ErrorIs(err, ErrDroppedReconstructed)

	_, err = reconstructed.Dump()
	r.ErrorIs(err, ErrDroppedReconstructed)

	err = reconstructed.Reconstruct(batch[:1])
	r.ErrorIs(err, ErrDroppedReconstructed)
}

// TestReconstructedConcurrentGetAndDrop verifies that concurrent Get and Drop
// calls do not panic or return unexpected errors. Every goroutine must see
// either a successful result or ErrDroppedReconstructed.
func TestReconstructedConcurrentGetAndDrop(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	const kvCountConcurrent = 8
	keys, _, batch := kvForTest(kvCountConcurrent)
	root, err := db.Update(batch[:4])
	r.NoError(err)

	rev, err := db.Revision(root)
	r.NoError(err)
	t.Cleanup(func() {
		r.NoError(rev.Drop())
	})

	reconstructed, err := rev.Reconstruct(batch[4:6])
	r.NoError(err)

	const getters = 16
	start := make(chan struct{})
	var g errgroup.Group

	acceptDropped := func(err error) error {
		if err == nil || errors.Is(err, ErrDroppedReconstructed) {
			return nil
		}
		return err
	}

	for range getters {
		g.Go(func() error {
			<-start
			_, err := reconstructed.Get(keys[0])
			return acceptDropped(err)
		})
	}

	g.Go(func() error {
		<-start
		return acceptDropped(reconstructed.Drop())
	})

	close(start)
	r.NoError(g.Wait())
}
