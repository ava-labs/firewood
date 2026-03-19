// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"errors"
	"math/rand"
	"sync"
	"testing"

	"github.com/stretchr/testify/require"
)

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
	t.Cleanup(func() { _ = reconstructed.Drop() })

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

	r.NoError(reconstructed.Drop())
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
		initialItems = 100
		batchCount   = 2_048
		batchItems   = 100
		keyLen       = 16
		valueLen     = 32
	)

	makeBatch := func(rng *rand.Rand, count int) []BatchOp {
		batch := make([]BatchOp, 0, count)
		for range count {
			key := make([]byte, keyLen)
			value := make([]byte, valueLen)
			rng.Read(key)
			rng.Read(value)
			batch = append(batch, Put(key, value))
		}
		return batch
	}

	generateBatches := func(seed int64) ([]BatchOp, [][]BatchOp) {
		rng := rand.New(rand.NewSource(seed))
		initial := makeBatch(rng, initialItems)
		batches := make([][]BatchOp, 0, batchCount)
		for range batchCount {
			batches = append(batches, makeBatch(rng, batchItems))
		}
		return initial, batches
	}

	initial, batches := generateBatches(1234)
	db := newTestDatabase(b)
	root, err := db.Update(initial)
	r.NoError(err)

	rev, err := db.Revision(root)
	r.NoError(err)
	b.Cleanup(func() {
		r.NoError(rev.Drop())
	})

	b.ReportAllocs()
	b.ResetTimer()

	for i := 0; i < b.N; i++ {
		current, err := rev.Reconstruct(batches[0])
		r.NoError(err)
		for _, batch := range batches[1:] {
			err := current.Reconstruct(batch)
			r.NoError(err)
		}

		_ = current.Root()
		r.NoError(current.Drop())
	}
}

func TestReconstructedDropThenUse(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	keys, _, batch := kvForTest(4)
	root, err := db.Update(batch[:2])
	r.NoError(err)

	rev, err := db.Revision(root)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(rev.Drop()) })

	reconstructed, err := rev.Reconstruct(batch[2:4])
	r.NoError(err)

	// Dump succeeds on a live view.
	dot, err := reconstructed.Dump()
	r.NoError(err)
	r.NotEmpty(dot)

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

func TestReconstructedConcurrentGetAndDrop(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	keys, _, batch := kvForTest(8)
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
	var wg sync.WaitGroup
	errCh := make(chan error, getters+1) // +1 for the Drop goroutine

	for range getters {
		wg.Go(func() {
			<-start
			_, err := reconstructed.Get(keys[0])
			errCh <- err
		})
	}

	wg.Go(func() {
		<-start
		errCh <- reconstructed.Drop()
	})

	close(start)
	wg.Wait()
	close(errCh)

	for err := range errCh {
		if err == nil {
			continue
		}
		if errors.Is(err, ErrDroppedReconstructed) {
			continue
		}
		r.FailNowf("unexpected error", "unexpected concurrent error: %v", err)
	}
}
