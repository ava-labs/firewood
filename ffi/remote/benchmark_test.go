// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"context"
	"flag"
	"fmt"
	"net"
	"path/filepath"
	"testing"
	"time"

	ffi "github.com/ava-labs/firewood/ffi"
	pb "github.com/ava-labs/firewood/ffi/remote/proto"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/protobuf/proto"
)

// benchLength controls which benchmarks run:
//   - "short":  only pure-Go benchmarks (no DB setup)
//   - "medium": all benchmarks except 100k DB sizes (default)
//   - "long":   all benchmarks including 100k DB sizes
var benchLength = flag.String("length", "medium", "benchmark length: short, medium, long")

var benchLevels = map[string]int{"short": 0, "medium": 1, "long": 2}

// skipBench skips the current benchmark if -length is below minLevel.
func skipBench(b *testing.B, minLevel string) {
	b.Helper()
	if benchLevels[*benchLength] < benchLevels[minLevel] {
		b.Skipf("skipping (need -length=%s, have -length=%s)", minLevel, *benchLength)
	}
}

// newBenchDB creates a new Firewood database for benchmarking and registers cleanup.
func newBenchDB(b *testing.B) *ffi.Database {
	b.Helper()
	dbFile := filepath.Join(b.TempDir(), "bench.db")
	db, err := ffi.New(dbFile, detectHashAlgorithm())
	if err != nil {
		b.Fatalf("ffi.New: %v", err)
	}
	b.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		if err := db.Close(ctx); err != nil {
			b.Errorf("db.Close: %v", err)
		}
	})
	return db
}

// populateDB inserts n deterministic key-value pairs in batches of 500.
// Returns the final root hash and sorted key list.
func populateDB(b *testing.B, db *ffi.Database, n int) (ffi.Hash, []string) {
	b.Helper()
	const batchSize = 500
	keys := make([]string, 0, n)

	var rootHash ffi.Hash
	for start := 0; start < n; start += batchSize {
		end := start + batchSize
		if end > n {
			end = n
		}
		ops := make([]ffi.BatchOp, 0, end-start)
		for i := start; i < end; i++ {
			key := fmt.Sprintf("key-%06d", i)
			keys = append(keys, key)
			// 32-byte deterministic value.
			val := make([]byte, 32)
			copy(val, fmt.Sprintf("val-%06d", i))
			ops = append(ops, ffi.Put([]byte(key), val))
		}
		var err error
		rootHash, err = db.Update(ops)
		if err != nil {
			b.Fatalf("populateDB Update (batch starting at %d): %v", start, err)
		}
	}
	return rootHash, keys
}

// setupRemoteDB starts a gRPC server and returns a RemoteDB (ffi.DB) for benchmarks.
func setupRemoteDB(b *testing.B, db *ffi.Database, rootHash ffi.Hash, depth uint, opts ...ClientOption) ffi.DB {
	b.Helper()
	srv := NewServer(db)
	lis, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		b.Fatalf("net.Listen: %v", err)
	}
	grpcServer := grpc.NewServer()
	pb.RegisterFirewoodRemoteServer(grpcServer, srv)

	go func() { _ = grpcServer.Serve(lis) }()
	b.Cleanup(grpcServer.Stop)

	ctx := context.Background()
	rdb, err := NewRemoteDB(ctx, lis.Addr().String(), rootHash, depth, opts...)
	if err != nil {
		b.Fatalf("NewRemoteDB: %v", err)
	}
	b.Cleanup(func() {
		rdb.Close(context.Background())
	})
	return rdb
}

// generateOps creates batch operations for benchmarking.
//
//   - "put": all Puts with fresh keys (bench-put-ITER-INDEX)
//   - "delete": deletes on existing keys (wraps around)
//   - "mixed": 50% new puts, 30% overwrites, 20% deletes
//   - "prefix-delete": single PrefixDelete("key-000")
func generateOps(keys []string, n int, mix string, iter int) []ffi.BatchOp {
	switch mix {
	case "put":
		ops := make([]ffi.BatchOp, n)
		for i := range n {
			key := fmt.Sprintf("bench-put-%06d-%06d", iter, i)
			val := make([]byte, 32)
			copy(val, key)
			ops[i] = ffi.Put([]byte(key), val)
		}
		return ops

	case "delete":
		ops := make([]ffi.BatchOp, n)
		for i := range n {
			ops[i] = ffi.Delete([]byte(keys[i%len(keys)]))
		}
		return ops

	case "mixed":
		ops := make([]ffi.BatchOp, n)
		for i := range n {
			switch {
			case i%10 < 5: // 50% new puts
				key := fmt.Sprintf("bench-mixed-%06d-%06d", iter, i)
				val := make([]byte, 32)
				copy(val, key)
				ops[i] = ffi.Put([]byte(key), val)
			case i%10 < 8: // 30% overwrites
				key := keys[i%len(keys)]
				val := make([]byte, 32)
				copy(val, fmt.Sprintf("overwrite-%06d", iter))
				ops[i] = ffi.Put([]byte(key), val)
			default: // 20% deletes
				ops[i] = ffi.Delete([]byte(keys[i%len(keys)]))
			}
		}
		return ops

	case "prefix-delete":
		return []ffi.BatchOp{ffi.PrefixDelete([]byte("key-000"))}

	default:
		panic("unknown mix: " + mix)
	}
}

// ---------------------------------------------------------------------------
// 1. BenchmarkGet — Read performance × DB size
// ---------------------------------------------------------------------------

func BenchmarkGet(b *testing.B) {
	skipBench(b, "medium")

	sizes := []struct {
		name     string
		n        int
		minLevel string
	}{
		{"1k", 1_000, "medium"},
		{"10k", 10_000, "medium"},
		{"100k", 100_000, "long"},
	}

	for _, sz := range sizes {
		sz := sz
		if benchLevels[*benchLength] < benchLevels[sz.minLevel] {
			continue
		}
		// Create one DB per size, shared by Local and Remote sub-benchmarks.
		db := newBenchDB(b)
		rootHash, keys := populateDB(b, db, sz.n)

		b.Run("Local/"+sz.name, func(b *testing.B) {
			ldb := ffi.NewLocalDB(db)
			ctx := context.Background()
			var i int
			for b.Loop() {
				if _, err := ldb.Get(ctx, []byte(keys[i%len(keys)])); err != nil {
					b.Fatalf("Get: %v", err)
				}
				i++
			}
		})

		b.Run("Remote/"+sz.name, func(b *testing.B) {
			rdb := setupRemoteDB(b, db, rootHash, 4)
			ctx := context.Background()
			var i int
			for b.Loop() {
				if _, err := rdb.Get(ctx, []byte(keys[i%len(keys)])); err != nil {
					b.Fatalf("Get: %v", err)
				}
				i++
			}
		})
	}
}

// ---------------------------------------------------------------------------
// 2. BenchmarkUpdate — Write throughput × batch size
// ---------------------------------------------------------------------------

func BenchmarkUpdate(b *testing.B) {
	skipBench(b, "medium")

	batchSizes := []int{1, 10, 100, 1000}

	for _, bs := range batchSizes {
		bs := bs
		name := fmt.Sprintf("%d_ops", bs)

		b.Run("Local/"+name, func(b *testing.B) {
			db := newBenchDB(b)
			rootHash, keys := populateDB(b, db, 1_000)
			_ = rootHash
			ldb := ffi.NewLocalDB(db)
			ctx := context.Background()
			var iter int
			for b.Loop() {
				ops := generateOps(keys, bs, "put", iter)
				if _, err := ldb.Update(ctx, ops); err != nil {
					b.Fatalf("Update: %v", err)
				}
				iter++
			}
		})

		b.Run("Remote/"+name, func(b *testing.B) {
			db := newBenchDB(b)
			rootHash, keys := populateDB(b, db, 1_000)
			rdb := setupRemoteDB(b, db, rootHash, 4)
			ctx := context.Background()
			var iter int
			for b.Loop() {
				ops := generateOps(keys, bs, "put", iter)
				if _, err := rdb.Update(ctx, ops); err != nil {
					b.Fatalf("Update: %v", err)
				}
				iter++
			}
		})
	}
}

// ---------------------------------------------------------------------------
// 3. BenchmarkPropose — Propose-only (no commit) × batch size
// ---------------------------------------------------------------------------

func BenchmarkPropose(b *testing.B) {
	skipBench(b, "medium")

	batchSizes := []int{1, 10, 100, 1000}

	for _, bs := range batchSizes {
		bs := bs
		name := fmt.Sprintf("%d_ops", bs)

		b.Run("Local/"+name, func(b *testing.B) {
			db := newBenchDB(b)
			rootHash, keys := populateDB(b, db, 1_000)
			_ = rootHash
			ldb := ffi.NewLocalDB(db)
			ctx := context.Background()
			var iter int
			for b.Loop() {
				ops := generateOps(keys, bs, "put", iter)
				prop, err := ldb.Propose(ctx, ops)
				if err != nil {
					b.Fatalf("Propose: %v", err)
				}
				prop.Drop()
				iter++
			}
		})

		b.Run("Remote/"+name, func(b *testing.B) {
			db := newBenchDB(b)
			rootHash, keys := populateDB(b, db, 1_000)
			rdb := setupRemoteDB(b, db, rootHash, 4)
			ctx := context.Background()
			var iter int
			for b.Loop() {
				ops := generateOps(keys, bs, "put", iter)
				prop, err := rdb.Propose(ctx, ops)
				if err != nil {
					b.Fatalf("Propose: %v", err)
				}
				prop.Drop()
				iter++
			}
		})
	}
}

// ---------------------------------------------------------------------------
// 4. BenchmarkCommit — Commit isolated × batch size
// ---------------------------------------------------------------------------

func BenchmarkCommit(b *testing.B) {
	skipBench(b, "medium")

	batchSizes := []int{1, 10, 100}

	for _, bs := range batchSizes {
		bs := bs
		name := fmt.Sprintf("%d_ops", bs)

		b.Run("Local/"+name, func(b *testing.B) {
			db := newBenchDB(b)
			rootHash, keys := populateDB(b, db, 1_000)
			_ = rootHash
			ldb := ffi.NewLocalDB(db)
			ctx := context.Background()
			var iter int
			for b.Loop() {
				ops := generateOps(keys, bs, "put", iter)
				b.StopTimer()
				prop, err := ldb.Propose(ctx, ops)
				if err != nil {
					b.Fatalf("Propose: %v", err)
				}
				b.StartTimer()
				if err := prop.Commit(ctx); err != nil {
					b.Fatalf("Commit: %v", err)
				}
				iter++
			}
		})

		b.Run("Remote/"+name, func(b *testing.B) {
			db := newBenchDB(b)
			rootHash, keys := populateDB(b, db, 1_000)
			rdb := setupRemoteDB(b, db, rootHash, 4)
			ctx := context.Background()
			var iter int
			for b.Loop() {
				ops := generateOps(keys, bs, "put", iter)
				b.StopTimer()
				prop, err := rdb.Propose(ctx, ops)
				if err != nil {
					b.Fatalf("Propose: %v", err)
				}
				b.StartTimer()
				if err := prop.Commit(ctx); err != nil {
					b.Fatalf("Commit: %v", err)
				}
				iter++
			}
		})
	}
}

// ---------------------------------------------------------------------------
// 5. BenchmarkOperationMixture — Workload profiles
// ---------------------------------------------------------------------------

func BenchmarkOperationMixture(b *testing.B) {
	skipBench(b, "medium")

	mixes := []string{"put", "delete", "mixed", "prefix-delete"}

	for _, mix := range mixes {
		mix := mix

		b.Run("Local/"+mix, func(b *testing.B) {
			db := newBenchDB(b)
			rootHash, keys := populateDB(b, db, 1_000)
			_ = rootHash
			ldb := ffi.NewLocalDB(db)
			ctx := context.Background()
			batchSize := 100
			if mix == "prefix-delete" {
				batchSize = 1
			}
			var iter int
			for b.Loop() {
				ops := generateOps(keys, batchSize, mix, iter)
				if _, err := ldb.Update(ctx, ops); err != nil {
					b.Fatalf("Update: %v", err)
				}
				iter++
			}
		})

		b.Run("Remote/"+mix, func(b *testing.B) {
			db := newBenchDB(b)
			rootHash, keys := populateDB(b, db, 1_000)
			rdb := setupRemoteDB(b, db, rootHash, 4)
			ctx := context.Background()
			batchSize := 100
			if mix == "prefix-delete" {
				batchSize = 1
			}
			var iter int
			for b.Loop() {
				ops := generateOps(keys, batchSize, mix, iter)
				if _, err := rdb.Update(ctx, ops); err != nil {
					b.Fatalf("Update: %v", err)
				}
				iter++
			}
		})
	}
}

// ---------------------------------------------------------------------------
// 6. BenchmarkProofVerification — Standalone VerifySingleKeyProof
// ---------------------------------------------------------------------------

func BenchmarkProofVerification(b *testing.B) {
	skipBench(b, "medium")

	sizes := []struct {
		name     string
		n        int
		minLevel string
	}{
		{"1k", 1_000, "medium"},
		{"10k", 10_000, "medium"},
		{"100k", 100_000, "long"},
	}

	for _, sz := range sizes {
		sz := sz
		b.Run(sz.name, func(b *testing.B) {
			skipBench(b, sz.minLevel)
			db := newBenchDB(b)
			rootHash, keys := populateDB(b, db, sz.n)

			// Get a proof once for a known key.
			result, err := db.GetWithProof(rootHash, []byte(keys[0]))
			if err != nil {
				b.Fatalf("GetWithProof: %v", err)
			}
			proof := result.Proof
			value := result.Value

			b.ReportMetric(float64(len(proof)), "proof_bytes")
			b.ResetTimer()

			for b.Loop() {
				if err := ffi.VerifySingleKeyProof(rootHash, []byte(keys[0]), value, proof); err != nil {
					b.Fatalf("VerifySingleKeyProof: %v", err)
				}
			}
		})
	}
}

// ---------------------------------------------------------------------------
// 7. BenchmarkWitnessGeneration — Standalone GenerateWitness
// ---------------------------------------------------------------------------

func BenchmarkWitnessGeneration(b *testing.B) {
	skipBench(b, "medium")

	opCounts := []int{1, 10, 100}

	for _, nOps := range opCounts {
		nOps := nOps
		b.Run(fmt.Sprintf("%d_ops", nOps), func(b *testing.B) {
			db := newBenchDB(b)
			oldRoot, _ := populateDB(b, db, 1_000)

			// Apply one Update to get oldRoot → newRoot.
			ops := make([]ffi.BatchOp, nOps)
			for i := range nOps {
				key := fmt.Sprintf("witness-gen-%06d", i)
				val := make([]byte, 32)
				copy(val, key)
				ops[i] = ffi.Put([]byte(key), val)
			}
			newRoot, err := db.Update(ops)
			if err != nil {
				b.Fatalf("Update: %v", err)
			}

			b.ResetTimer()
			for b.Loop() {
				w, err := db.GenerateWitness(oldRoot, ops, newRoot, 4)
				if err != nil {
					b.Fatalf("GenerateWitness: %v", err)
				}
				w.Free()
			}
		})
	}
}

// ---------------------------------------------------------------------------
// 8. BenchmarkWitnessVerification — Standalone VerifyWitness
// ---------------------------------------------------------------------------

func BenchmarkWitnessVerification(b *testing.B) {
	skipBench(b, "medium")

	opCounts := []int{1, 10, 100}

	for _, nOps := range opCounts {
		nOps := nOps
		b.Run(fmt.Sprintf("%d_ops", nOps), func(b *testing.B) {
			db := newBenchDB(b)
			oldRoot, _ := populateDB(b, db, 1_000)

			// Apply one Update to get oldRoot → newRoot.
			ops := make([]ffi.BatchOp, nOps)
			for i := range nOps {
				key := fmt.Sprintf("witness-ver-%06d", i)
				val := make([]byte, 32)
				copy(val, key)
				ops[i] = ffi.Put([]byte(key), val)
			}
			newRoot, err := db.Update(ops)
			if err != nil {
				b.Fatalf("Update: %v", err)
			}

			// Create the truncated trie and generate+serialize the witness once.
			trie, err := db.CreateTruncatedTrie(oldRoot, 4)
			if err != nil {
				b.Fatalf("CreateTruncatedTrie: %v", err)
			}
			b.Cleanup(func() { trie.Free() })

			witness, err := db.GenerateWitness(oldRoot, ops, newRoot, 4)
			if err != nil {
				b.Fatalf("GenerateWitness: %v", err)
			}
			witnessBytes, err := witness.MarshalBinary()
			if err != nil {
				b.Fatalf("MarshalBinary: %v", err)
			}
			witness.Free()

			b.ReportMetric(float64(len(witnessBytes)), "witness_bytes")
			b.ResetTimer()

			for b.Loop() {
				w := &ffi.WitnessProof{}
				if err := w.UnmarshalBinary(witnessBytes); err != nil {
					b.Fatalf("UnmarshalBinary: %v", err)
				}
				newTrie, err := trie.VerifyWitness(w)
				if err != nil {
					w.Free()
					b.Fatalf("VerifyWitness: %v", err)
				}
				w.Free()
				newTrie.Free()
			}
		})
	}
}

// ---------------------------------------------------------------------------
// 9. BenchmarkTrieDepth — Truncated trie depth impact (Remote only)
// ---------------------------------------------------------------------------

func BenchmarkTrieDepth(b *testing.B) {
	skipBench(b, "medium")

	depths := []uint{2, 4, 6, 8}

	for _, depth := range depths {
		depth := depth
		depthName := fmt.Sprintf("depth%d", depth)

		db := newBenchDB(b)
		rootHash, keys := populateDB(b, db, 1_000)

		b.Run(depthName+"/Get", func(b *testing.B) {
			rdb := setupRemoteDB(b, db, rootHash, depth)
			ctx := context.Background()
			var i int
			for b.Loop() {
				if _, err := rdb.Get(ctx, []byte(keys[i%len(keys)])); err != nil {
					b.Fatalf("Get: %v", err)
				}
				i++
			}
		})

		b.Run(depthName+"/Update_10ops", func(b *testing.B) {
			rdb := setupRemoteDB(b, db, rootHash, depth)
			ctx := context.Background()
			var iter int
			for b.Loop() {
				ops := generateOps(keys, 10, "put", iter)
				if _, err := rdb.Update(ctx, ops); err != nil {
					b.Fatalf("Update: %v", err)
				}
				iter++
			}
		})
	}
}

// ---------------------------------------------------------------------------
// 10. BenchmarkGRPCRoundTrip — Raw gRPC overhead
// ---------------------------------------------------------------------------

func BenchmarkGRPCRoundTrip(b *testing.B) {
	skipBench(b, "medium")

	db := newBenchDB(b)
	rootHash, keys := populateDB(b, db, 1_000)

	// Start gRPC server and create a raw proto client.
	srv := NewServer(db)
	lis, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		b.Fatalf("net.Listen: %v", err)
	}
	grpcServer := grpc.NewServer()
	pb.RegisterFirewoodRemoteServer(grpcServer, srv)
	go func() { _ = grpcServer.Serve(lis) }()
	b.Cleanup(grpcServer.Stop)

	conn, err := grpc.NewClient(lis.Addr().String(), grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		b.Fatalf("grpc.NewClient: %v", err)
	}
	b.Cleanup(func() { conn.Close() })
	rpcClient := pb.NewFirewoodRemoteClient(conn)
	ctx := context.Background()

	b.ResetTimer()
	var i int
	for b.Loop() {
		_, err := rpcClient.GetValue(ctx, &pb.GetValueRequest{
			RootHash: rootHash[:],
			Key:      []byte(keys[i%len(keys)]),
		})
		if err != nil {
			b.Fatalf("GetValue: %v", err)
		}
		i++
	}
}

// ---------------------------------------------------------------------------
// 11. BenchmarkProtoSerialization — Marshal/Unmarshal cost
// ---------------------------------------------------------------------------

func BenchmarkProtoSerialization(b *testing.B) {
	opCounts := []int{1, 10, 100, 1000}

	for _, nOps := range opCounts {
		nOps := nOps
		name := fmt.Sprintf("%d_ops", nOps)

		// Build a CreateProposalRequest with nOps operations.
		pbOps := make([]*pb.BatchOperation, nOps)
		for i := range nOps {
			key := fmt.Sprintf("proto-bench-%06d", i)
			val := make([]byte, 32)
			copy(val, key)
			pbOps[i] = &pb.BatchOperation{
				OpType: pb.BatchOperation_PUT,
				Key:    []byte(key),
				Value:  val,
			}
		}
		rootHash := make([]byte, 32)
		req := &pb.CreateProposalRequest{
			RootHash: rootHash,
			Ops:      pbOps,
			Depth:    4,
		}

		b.Run("Marshal/"+name, func(b *testing.B) {
			for b.Loop() {
				if _, err := proto.Marshal(req); err != nil {
					b.Fatalf("Marshal: %v", err)
				}
			}
		})

		data, err := proto.Marshal(req)
		if err != nil {
			b.Fatalf("Marshal: %v", err)
		}

		b.Run("Unmarshal/"+name, func(b *testing.B) {
			b.ReportMetric(float64(len(data)), "payload_bytes")
			for b.Loop() {
				var out pb.CreateProposalRequest
				if err := proto.Unmarshal(data, &out); err != nil {
					b.Fatalf("Unmarshal: %v", err)
				}
			}
		})
	}
}

// ---------------------------------------------------------------------------
// 12. BenchmarkGetCached — Cached vs uncached read performance
// ---------------------------------------------------------------------------

func BenchmarkGetCached(b *testing.B) {
	skipBench(b, "medium")

	db := newBenchDB(b)
	rootHash, keys := populateDB(b, db, 1_000)

	b.Run("NoCache", func(b *testing.B) {
		rdb := setupRemoteDB(b, db, rootHash, 4)
		ctx := context.Background()
		var i int
		for b.Loop() {
			if _, err := rdb.Get(ctx, []byte(keys[i%len(keys)])); err != nil {
				b.Fatalf("Get: %v", err)
			}
			i++
		}
	})

	b.Run("Cached", func(b *testing.B) {
		rdb := setupRemoteDB(b, db, rootHash, 4, WithCacheSize(10_000))
		ctx := context.Background()
		// Warm the cache.
		for _, key := range keys {
			if _, err := rdb.Get(ctx, []byte(key)); err != nil {
				b.Fatalf("warm Get: %v", err)
			}
		}
		b.ResetTimer()
		var i int
		for b.Loop() {
			if _, err := rdb.Get(ctx, []byte(keys[i%len(keys)])); err != nil {
				b.Fatalf("Get: %v", err)
			}
			i++
		}
	})
}
