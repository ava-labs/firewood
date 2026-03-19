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

	keys, vals, batch := kvForTest(10)
	root, err := db.Update(batch[:5])
	r.NoError(err)

	rev, err := db.Revision(root)
	r.NoError(err)
	t.Cleanup(func() {
		r.NoError(rev.Drop())
	})

	reconstructed, err := rev.Reconstruct(batch[5:8])
	r.NoError(err)

	for i := range 8 {
		got, err := reconstructed.Get(keys[i])
		r.NoError(err)
		r.Equal(vals[i], got)
	}

	for i := 8; i < len(keys); i++ {
		got, err := reconstructed.Get(keys[i])
		r.NoError(err)
		r.Nil(got)
	}

	oldRoot := reconstructed.Root()
	r.NoError(reconstructed.Reconstruct(batch[8:]))
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

	_, _, batch := kvForTest(1024)
	root, err := db.Update(batch[:1023])
	r.NoError(err)

	rev, err := db.Revision(root)
	r.NoError(err)
	b.Cleanup(func() {
		r.NoError(rev.Drop())
	})

	reconstructBatch := batch[1023:1024]

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

	start := make(chan struct{})
	var wg sync.WaitGroup
	errCh := make(chan error, 32)

	for range 16 {
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
