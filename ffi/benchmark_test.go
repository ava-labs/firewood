// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"context"
	"path/filepath"
	"testing"
	"time"
)

// BenchmarkGet benchmarks single key lookups
func BenchmarkGet(b *testing.B) {
	db := newBenchDatabase(b)
	// Pre-populate with data
	keys, vals := kvForBench(1000)
	if _, err := db.Update(keys, vals); err != nil {
		b.Fatal(err)
	}

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		key := keys[i%len(keys)]
		_, err := db.Get(key)
		if err != nil {
			b.Fatal(err)
		}
	}
}

// BenchmarkGetParallel benchmarks parallel key lookups
func BenchmarkGetParallel(b *testing.B) {
	db := newBenchDatabase(b)
	keys, vals := kvForBench(1000)
	if _, err := db.Update(keys, vals); err != nil {
		b.Fatal(err)
	}

	b.ResetTimer()
	b.RunParallel(func(pb *testing.PB) {
		i := 0
		for pb.Next() {
			key := keys[i%len(keys)]
			_, err := db.Get(key)
			if err != nil {
				b.Fatal(err)
			}
			i++
		}
	})
}

// BenchmarkUpdate benchmarks batch updates with varying sizes
func BenchmarkUpdate(b *testing.B) {
	for _, size := range []int{1, 10, 100} {
		b.Run(batchSizeName(size), func(b *testing.B) {
			db := newBenchDatabase(b)
			keys, vals := kvForBench(size)

			b.ResetTimer()
			for i := 0; i < b.N; i++ {
				_, err := db.Update(keys, vals)
				if err != nil {
					b.Fatal(err)
				}
			}
		})
	}
}

// BenchmarkPropose benchmarks proposal creation
func BenchmarkPropose(b *testing.B) {
	for _, size := range []int{1, 10, 100} {
		b.Run(batchSizeName(size), func(b *testing.B) {
			db := newBenchDatabase(b)
			keys, vals := kvForBench(size)

			b.ResetTimer()
			for i := 0; i < b.N; i++ {
				p, err := db.Propose(keys, vals)
				if err != nil {
					b.Fatal(err)
				}
				if err := p.Drop(); err != nil {
					b.Fatal(err)
				}
			}
		})
	}
}

// BenchmarkRevisionGet benchmarks getting values from a historical revision
func BenchmarkRevisionGet(b *testing.B) {
	db := newBenchDatabase(b)
	keys, vals := kvForBench(1000)
	root, err := db.Update(keys, vals)
	if err != nil {
		b.Fatal(err)
	}

	rev, err := db.Revision(root)
	if err != nil {
		b.Fatal(err)
	}
	b.Cleanup(func() {
		_ = rev.Drop()
	})

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		key := keys[i%len(keys)]
		_, err := rev.Get(key)
		if err != nil {
			b.Fatal(err)
		}
	}
}

// BenchmarkIterator benchmarks iterator performance
func BenchmarkIterator(b *testing.B) {
	db := newBenchDatabase(b)
	keys, vals := kvForBench(1000)
	if _, err := db.Update(keys, vals); err != nil {
		b.Fatal(err)
	}

	rev, err := db.LatestRevision()
	if err != nil {
		b.Fatal(err)
	}
	b.Cleanup(func() {
		_ = rev.Drop()
	})

	for _, batchSize := range []int{1, 10, 100} {
		b.Run(batchSizeName(batchSize), func(b *testing.B) {
			b.ResetTimer()
			for i := 0; i < b.N; i++ {
				it, err := rev.Iter(nil)
				if err != nil {
					b.Fatal(err)
				}
				it.SetBatchSize(batchSize)

				count := 0
				for it.Next() {
					count++
				}
				if err := it.Err(); err != nil {
					b.Fatal(err)
				}
				if err := it.Drop(); err != nil {
					b.Fatal(err)
				}
			}
		})
	}
}

// BenchmarkGetFromRoot benchmarks GetFromRoot with caching
func BenchmarkGetFromRoot(b *testing.B) {
	db := newBenchDatabase(b)
	keys, vals := kvForBench(1000)
	root, err := db.Update(keys, vals)
	if err != nil {
		b.Fatal(err)
	}

	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		key := keys[i%len(keys)]
		_, err := db.GetFromRoot(root, key)
		if err != nil {
			b.Fatal(err)
		}
	}
}

// Helper functions for benchmarks

func newBenchDatabase(b *testing.B) *Database {
	b.Helper()
	dbFile := filepath.Join(b.TempDir(), "bench.db")
	conf := DefaultConfig()
	conf.Truncate = true
	db, err := New(dbFile, conf)
	if err != nil {
		b.Fatal(err)
	}
	b.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = db.Close(ctx)
	})
	return db
}

func kvForBench(num int) ([][]byte, [][]byte) {
	keys := make([][]byte, num)
	vals := make([][]byte, num)
	for i := range keys {
		keys[i] = keyForTest(i)
		vals[i] = valForTest(i)
	}
	_ = sortKV(keys, vals)
	return keys, vals
}

func batchSizeName(size int) string {
	switch size {
	case 1:
		return "batch=1"
	case 10:
		return "batch=10"
	case 100:
		return "batch=100"
	default:
		return "batch=?"
	}
}
