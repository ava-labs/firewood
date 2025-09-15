package ffi

import (
	"crypto/rand"
	"fmt"
	"os"
	"runtime"
	"strconv"
	"testing"
)

// Benchmark design goals:
// - Cover Owned vs Borrowed iterator modes
// - Cover single-step vs batched iteration sizes
// - Vary data sizes (keys, value sizes) and optional heavy cases
// - Record allocations, GC and cgo deltas per sub-benchmark
// - Report throughput via b.SetBytes for easy comparison

// benchHeavyEnabled enables very large test cases when FIREWOOD_BENCH_HEAVY=1.
func benchHeavyEnabled() bool { return os.Getenv("FIREWOOD_BENCH_HEAVY") == "1" }

// benchMetricsEnabled toggles extra runtime logs when FIREWOOD_BENCH_METRICS=1.
func benchMetricsEnabled() bool { return os.Getenv("FIREWOOD_BENCH_METRICS") == "1" }

// benchTrials controls repeated sub-benchmarks to enable per-trial stats collection
// for later mean/stddev analysis. Set FIREWOOD_BENCH_TRIALS to an integer > 1.
func benchTrials() int {
	if s := os.Getenv("FIREWOOD_BENCH_TRIALS"); s != "" {
		if n, err := strconv.Atoi(s); err == nil && n > 0 {
			return n
		}
	}
	return 1
}

type rtSnapshot struct {
	numGC      uint32
	pauseTotal uint64
	totalAlloc uint64
	mallocs    uint64
	frees      uint64
	heapAlloc  uint64
	cgoCalls   int64
}

func takeRtSnapshot() rtSnapshot {
	var ms runtime.MemStats
	runtime.ReadMemStats(&ms)
	return rtSnapshot{
		numGC:      ms.NumGC,
		pauseTotal: ms.PauseTotalNs,
		totalAlloc: ms.TotalAlloc,
		mallocs:    ms.Mallocs,
		frees:      ms.Frees,
		heapAlloc:  ms.HeapAlloc,
		cgoCalls:   runtime.NumCgoCall(),
	}
}

func diffRt(a, b rtSnapshot) rtSnapshot {
	return rtSnapshot{
		numGC:      b.numGC - a.numGC,
		pauseTotal: b.pauseTotal - a.pauseTotal,
		totalAlloc: b.totalAlloc - a.totalAlloc,
		mallocs:    b.mallocs - a.mallocs,
		frees:      b.frees - a.frees,
		heapAlloc:  b.heapAlloc, // instantaneous; keep post value for context
		cgoCalls:   b.cgoCalls - a.cgoCalls,
	}
}

// genKVSizes generates n random keys/values of fixed sizes and sorts them.
func genKVSizes(n, keyLen, valLen int) ([][]byte, [][]byte) {
	keys := make([][]byte, n)
	vals := make([][]byte, n)
	for i := 0; i < n; i++ {
		k := make([]byte, keyLen)
		v := make([]byte, valLen)
		_, _ = rand.Read(k)
		_, _ = rand.Read(v)
		keys[i] = k
		vals[i] = v
	}
	_ = sortKV(keys, vals)
	return keys, vals
}

// genKVWithPrefix generates n random keys of keyLen that all share prefix,
// with values of valLen. Keys are sorted to match DB expectations.
func genKVWithPrefix(n, keyLen, valLen int, prefix []byte) ([][]byte, [][]byte) {
	if len(prefix) >= keyLen {
		panic("prefix longer than keyLen")
	}
	keys := make([][]byte, n)
	vals := make([][]byte, n)
	for i := 0; i < n; i++ {
		k := make([]byte, keyLen)
		copy(k, prefix)
		// Fill the remainder randomly so that all keys share the prefix
		_, _ = rand.Read(k[len(prefix):])
		v := make([]byte, valLen)
		_, _ = rand.Read(v)
		keys[i] = k
		vals[i] = v
	}
	_ = sortKV(keys, vals)
	return keys, vals
}

// kvSum touches key/value bytes to ensure work is not optimized away.
func kvSum(k, v []byte) uint64 {
	var s uint64
	if len(k) > 0 {
		s += uint64(k[0])
	}
	if len(v) > 0 {
		s += uint64(v[0])
	}
	return s
}

// configure iterator wrapper for owned/borrowed access.
type iterAdapter func(iter kvIter) kvIter

func ownedAdapter() iterAdapter { return func(it kvIter) kvIter { return it } }
func borrowedAdapter() iterAdapter {
	return func(it kvIter) kvIter { return &borrowIter{it: it.(*Iterator)} }
}

// BenchmarkIteratorMatrix iterates full datasets with varying sizes and batch modes.
func BenchmarkIteratorMatrix(b *testing.B) {
	b.ReportAllocs()

	// Keep matrix contained by default; enable heavy via env.
	valueSizes := []int{1024}
	if benchHeavyEnabled() {
		valueSizes = append(valueSizes, 4096)
	}
	datasetSizes := []int{100_000}
	if benchHeavyEnabled() {
		datasetSizes = append(datasetSizes, 1_000_000)
	}

	batchSizes := []int{1, 32, 128, 512, 1024, 2048, 2048 * 4, 2048 * 16, 2048 * 32, 2048 * 64}
	adapters := []struct {
		name string
		fn   iterAdapter
	}{
		{"Owned", ownedAdapter()},
		{"Borrowed", borrowedAdapter()},
	}
	
	// for _, n := range datasetSizes {
		// for _, vlen := range valueSizes {
			// Prepare DB once per submatrix.
			db := openTestDatabase(b, "big_db")
			// keys, vals := genKVSizes(n, 32, vlen)
			// root, err := db.Update(keys, vals)
			// if err != nil {
			// 	b.Fatalf("Update: %v", err)
			// }
			n := 2_000_000
			vlen := 1024
			for _, ad := range adapters {
				for _, bs := range batchSizes {
					baseName := fmt.Sprintf("N=%d/Val=%d/%s/Batch=%d", n, vlen, ad.name, bs)
					trials := benchTrials()
					for t := 1; t <= trials; t++ {
						name := baseName
						if trials > 1 {
							name = fmt.Sprintf("%s/Trial=%d", baseName, t)
						}
						b.Run(name, func(b *testing.B) {
							// Throughput in bytes per full scan.
							b.SetBytes(int64(n * (32 + vlen)))

							before := takeRtSnapshot()
							// Warmup: single full scan to prime caches.
							// {
							// 	it, werr := db.Iter(nil)
							// 	if werr != nil {
							// 		b.Fatalf("IterOnRoot warmup: %v", werr)
							// 	}
							// 	it.SetBatchSize(bs)
							// 	w := ad.fn(it)
							// 	// w = zd.fn(w)
							// 	var sink uint64
							// 	var i uint64
							// 	for w.Next() {
							// 		sink += kvSum(w.Key(), w.Value())
							// 		i += 1
							// 		if i % 100000 == 0 {
							// 			b.Logf("read items: %d \n", i)
							// 		}
							// 	}
							// 	if e := w.Err(); e != nil {
							// 		b.Fatalf("warmup iter err: %v", e)
							// 	}
							// 	_ = sink
							// 	_ = w.Drop()
							// }

							b.ResetTimer()
							for i := 0; i < b.N; i++ {
								it, err := db.Iter(nil)
								if err != nil {
									b.Fatalf("IterOnRoot: %v", err)
								}
								it.SetBatchSize(bs)
								w := ad.fn(it)
								// w = zd.fn(w)
								var sink uint64
								for w.Next() {
									sink += kvSum(w.Key(), w.Value())
								}
								if e := w.Err(); e != nil {
									b.Fatalf("iter err: %v", e)
								}
								_ = sink
								_ = w.Drop()
							}
							b.StopTimer()

							after := takeRtSnapshot()
							if benchMetricsEnabled() {
								d := diffRt(before, after)
								b.Logf("rt: gc=%d pauseTotal=%.3fms alloc=%.2fMB mallocs=%d frees=%d heapAlloc=%.2fMB cgoCalls=%d",
									d.numGC,
									float64(d.pauseTotal)/1e6,
									float64(d.totalAlloc)/(1024*1024),
									d.mallocs,
									d.frees,
									float64(after.heapAlloc)/(1024*1024),
									d.cgoCalls,
								)
							}
						})
					}
				}
			// }
		// }
	}
}

// BenchmarkIteratorExhaust measures the Rust-side iteration baseline by exhausting
// the iterator through a single FFI call that advances N items.
func BenchmarkIteratorExhaust(b *testing.B) {
	b.ReportAllocs()

	valueSizes := []int{1024}
	// valueSizes := []int{4096}
	if benchHeavyEnabled() {
		valueSizes = append(valueSizes, 4096)
	}
	datasetSizes := []int{100_000}
	// datasetSizes := []int{1_000_000}
	if benchHeavyEnabled() {
		datasetSizes = append(datasetSizes, 1_000_000)
	}

	// for _, n := range datasetSizes {
	// 	for _, vlen := range valueSizes {
			n := 2_000_000
			vlen := 1024
			db := openTestDatabase(b, "big_db")
			// keys, vals := genKVSizes(n, 32, vlen)
			// root, err := db.Update(keys, vals)
			// if err != nil {
			// 	b.Fatalf("Update: %v", err)
			// }
			baseName := fmt.Sprintf("N=%d/Val=%d", n, vlen)
			trials := benchTrials()
			for t := 1; t <= trials; t++ {
				name := baseName
				if trials > 1 {
					name = fmt.Sprintf("%s/Trial=%d", baseName, t)
				}
				b.Run(name, func(b *testing.B) {
					// Full scan payload to express MB/s
					b.SetBytes(int64(n * (32 + vlen)))

					before := takeRtSnapshot()
					// Warmup one exhaust
					// {
					// 	it, werr := db.Iter(nil)
					// 	if werr != nil {
					// 		b.Fatalf("IterOnRoot warmup: %v", werr)
					// 	}
					// 	if err := it.Exhaust(n); err != nil {
					// 		b.Fatalf("warmup exhaust: %v", err)
					// 	}
					// 	_ = it.Drop()
					// }

					b.ResetTimer()
					for i := 0; i < b.N; i++ {
						it, err := db.Iter(nil)
						if err != nil {
							b.Fatalf("IterOnRoot: %v", err)
						}
						if err := it.Exhaust(n); err != nil {
							b.Fatalf("exhaust: %v", err)
						}
						_ = it.Drop()
					}
					b.StopTimer()

					after := takeRtSnapshot()
					if benchMetricsEnabled() {
						d := diffRt(before, after)
						b.Logf("rt: gc=%d pauseTotal=%.3fms alloc=%.2fMB mallocs=%d frees=%d heapAlloc=%.2fMB cgoCalls=%d",
							d.numGC,
							float64(d.pauseTotal)/1e6,
							float64(d.totalAlloc)/(1024*1024),
							d.mallocs,
							d.frees,
							float64(after.heapAlloc)/(1024*1024),
							d.cgoCalls,
						)
					}
				})
		// 	}
		// }
	}
}

// TestIteratorExhaustLarge measures the Rust-side iteration baseline by exhausting
// the iterator through a single FFI call that advances N items.
// func TestIteratorExhaustLarge(t *testing.T) {
// 	valueSizes := []int{4096}
// 	datasetSizes := []int{1_000_000}

// 	for _, n := range datasetSizes {
// 		for _, vlen := range valueSizes {
// 			db := newTestDatabase(t)
// 			keys, vals := genKVSizes(n, 32, vlen)
// 			root, err := db.Update(keys, vals)
// 			if err != nil {
// 				t.Fatalf("Update: %v", err)
// 			}
// 			baseName := fmt.Sprintf("N=%d/Val=%d", n, vlen)
// 			trials := benchTrials()
// 			for tx := 1; tx <= trials; tx++ {
// 				name := baseName
// 				if trials > 1 {
// 					name = fmt.Sprintf("%s/Trial=%d", baseName, tx)
// 				}
// 				t.Run(name, func(b *testing.T) {
// 					before := takeRtSnapshot()
// 					// Warmup one exhaust
// 					{
// 						it, werr := db.IterOnRoot(root, nil)
// 						if werr != nil {
// 							b.Fatalf("IterOnRoot warmup: %v", werr)
// 						}
// 						if err := it.Exhaust(n); err != nil {
// 							b.Fatalf("warmup exhaust: %v", err)
// 						}
// 						err = it.Drop()
// 						if err != nil {
// 							b.Fatalf("drop exhaust: %v", err)
// 						}
// 					}

// 					for i := 0; i < 1; i++ {
// 						it, err := db.IterOnRoot(root, nil)
// 						if err != nil {
// 							b.Fatalf("IterOnRoot: %v", err)
// 						}
// 						if err := it.Exhaust(n); err != nil {
// 							b.Fatalf("exhaust: %v", err)
// 						}
// 						_ = it.Drop()
// 					}

// 					after := takeRtSnapshot()
// 					if benchMetricsEnabled() {
// 						d := diffRt(before, after)
// 						b.Logf("rt: gc=%d pauseTotal=%.3fms alloc=%.2fMB mallocs=%d frees=%d heapAlloc=%.2fMB cgoCalls=%d",
// 							d.numGC,
// 							float64(d.pauseTotal)/1e6,
// 							float64(d.totalAlloc)/(1024*1024),
// 							d.mallocs,
// 							d.frees,
// 							float64(after.heapAlloc)/(1024*1024),
// 							d.cgoCalls,
// 						)
// 					}
// 				})
// 			}
// 		}
// 	}
// }
