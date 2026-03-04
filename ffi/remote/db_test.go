// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"context"
	"fmt"
	"net"
	"strings"
	"testing"

	ffi "github.com/ava-labs/firewood/ffi"
	pb "github.com/ava-labs/firewood/ffi/remote/proto"
	"google.golang.org/grpc"
	"google.golang.org/protobuf/proto"
)

// startServerAndGetAddr starts a gRPC server and returns its address.
func startServerAndGetAddr(t *testing.T, db *ffi.Database) string {
	t.Helper()

	srv := NewServer(db)
	lis, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("net.Listen: %v", err)
	}
	grpcServer := grpc.NewServer()
	pb.RegisterFirewoodRemoteServer(grpcServer, srv)

	go func() { _ = grpcServer.Serve(lis) }()
	t.Cleanup(grpcServer.Stop)

	return lis.Addr().String()
}

func TestNewRemoteDB(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	data := map[string]string{"apple": "red", "banana": "yellow"}
	rootHash := insertData(t, db, data)

	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	// Root should match.
	if rdb.Root() != rootHash {
		t.Fatalf("root mismatch: got %x, want %x", rdb.Root(), rootHash)
	}

	// Get existing key.
	val, err := rdb.Get(ctx, []byte("apple"))
	if err != nil {
		t.Fatalf("Get(apple): %v", err)
	}
	if string(val) != "red" {
		t.Fatalf("Get(apple) = %q, want %q", val, "red")
	}

	// Update and read back.
	newRoot, err := rdb.Update(ctx, []ffi.BatchOp{ffi.Put([]byte("cherry"), []byte("dark"))})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}
	if newRoot == rootHash {
		t.Fatal("root should change after update")
	}

	val, err = rdb.Get(ctx, []byte("cherry"))
	if err != nil {
		t.Fatalf("Get(cherry): %v", err)
	}
	if string(val) != "dark" {
		t.Fatalf("Get(cherry) = %q, want %q", val, "dark")
	}
}

func TestRemoteDBPropose(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"a": "1"})
	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	// Propose.
	prop, err := rdb.Propose(ctx, []ffi.BatchOp{ffi.Put([]byte("b"), []byte("2"))})
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}

	// Proposal root differs from db root.
	if prop.Root() == rdb.Root() {
		t.Fatal("proposal root should differ from db root")
	}

	// Commit.
	if err := prop.Commit(ctx); err != nil {
		t.Fatalf("Commit: %v", err)
	}

	// DB root updated.
	if rdb.Root() != prop.Root() {
		t.Fatal("db root should match proposal root after commit")
	}
}

func TestRemoteDBProposalGet(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"a": "1"})
	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	prop, err := rdb.Propose(ctx, []ffi.BatchOp{ffi.Put([]byte("b"), []byte("2"))})
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}
	defer prop.Drop()

	// Read from proposal: new key.
	val, err := prop.Get(ctx, []byte("b"))
	if err != nil {
		t.Fatalf("Get(b): %v", err)
	}
	if string(val) != "2" {
		t.Fatalf("Get(b) = %q, want %q", val, "2")
	}

	// Read from proposal: pre-existing key.
	val, err = prop.Get(ctx, []byte("a"))
	if err != nil {
		t.Fatalf("Get(a): %v", err)
	}
	if string(val) != "1" {
		t.Fatalf("Get(a) = %q, want %q", val, "1")
	}
}

func TestRemoteDBProposalIter(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"a": "1", "b": "2"})
	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	prop, err := rdb.Propose(ctx, []ffi.BatchOp{ffi.Put([]byte("c"), []byte("3"))})
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}
	defer prop.Drop()

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

	expected := []string{"a", "b", "c"}
	if len(keys) != len(expected) {
		t.Fatalf("got %d keys, want %d: %v", len(keys), len(expected), keys)
	}
	for i, k := range keys {
		if k != expected[i] {
			t.Fatalf("key[%d] = %q, want %q", i, k, expected[i])
		}
	}
}

func TestRemoteDBProposalChain(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"a": "1"})
	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	// First proposal.
	p1, err := rdb.Propose(ctx, []ffi.BatchOp{ffi.Put([]byte("b"), []byte("2"))})
	if err != nil {
		t.Fatalf("Propose p1: %v", err)
	}

	// Chained second proposal.
	p2, err := p1.Propose(ctx, []ffi.BatchOp{ffi.Put([]byte("c"), []byte("3"))})
	if err != nil {
		t.Fatalf("Propose p2: %v", err)
	}

	// p2 should have a root different from p1.
	if p2.Root() == p1.Root() {
		t.Fatal("p2 root should differ from p1")
	}

	// Commit chain.
	if err := p1.Commit(ctx); err != nil {
		t.Fatalf("p1.Commit: %v", err)
	}
	if err := p2.Commit(ctx); err != nil {
		t.Fatalf("p2.Commit: %v", err)
	}
}

func TestRemoteDBProposeDrop(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"a": "1"})
	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	prop, err := rdb.Propose(ctx, []ffi.BatchOp{ffi.Put([]byte("b"), []byte("2"))})
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}

	if err := prop.Drop(); err != nil {
		t.Fatalf("Drop: %v", err)
	}

	// DB root should be unchanged.
	if rdb.Root() != rootHash {
		t.Fatal("db root should be unchanged after drop")
	}
}

func TestRemoteDBPrefixDelete(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{
		"a/1": "v1",
		"a/2": "v2",
		"b/1": "v3",
	})

	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	// PrefixDelete "a/" should remove a/1 and a/2 but keep b/1.
	newRoot, err := rdb.Update(ctx, []ffi.BatchOp{ffi.PrefixDelete([]byte("a/"))})
	if err != nil {
		t.Fatalf("Update with PrefixDelete: %v", err)
	}
	if newRoot == rootHash {
		t.Fatal("root should change after PrefixDelete")
	}

	// a/1 and a/2 should be gone.
	for _, key := range []string{"a/1", "a/2"} {
		val, err := rdb.Get(ctx, []byte(key))
		if err != nil {
			t.Fatalf("Get(%s): %v", key, err)
		}
		if val != nil {
			t.Fatalf("Get(%s) = %q, want nil", key, val)
		}
	}

	// b/1 should still exist.
	val, err := rdb.Get(ctx, []byte("b/1"))
	if err != nil {
		t.Fatalf("Get(b/1): %v", err)
	}
	if string(val) != "v3" {
		t.Fatalf("Get(b/1) = %q, want %q", val, "v3")
	}
}

func TestRemoteDBPrefixDeletePropose(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{
		"a/1": "v1",
		"a/2": "v2",
		"b/1": "v3",
	})

	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	prop, err := rdb.Propose(ctx, []ffi.BatchOp{ffi.PrefixDelete([]byte("a/"))})
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}

	// Proposal should see a/1 and a/2 deleted.
	for _, key := range []string{"a/1", "a/2"} {
		val, err := prop.Get(ctx, []byte(key))
		if err != nil {
			t.Fatalf("Get(%s): %v", key, err)
		}
		if val != nil {
			t.Fatalf("Get(%s) = %q, want nil", key, val)
		}
	}

	// b/1 should still be visible.
	val, err := prop.Get(ctx, []byte("b/1"))
	if err != nil {
		t.Fatalf("Get(b/1): %v", err)
	}
	if string(val) != "v3" {
		t.Fatalf("Get(b/1) = %q, want %q", val, "v3")
	}

	if err := prop.Commit(ctx); err != nil {
		t.Fatalf("Commit: %v", err)
	}
}

func TestRemoteDBPrefixDeleteChained(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"x": "0"})

	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	// p1 adds keys under "a/".
	p1, err := rdb.Propose(ctx, []ffi.BatchOp{
		ffi.Put([]byte("a/1"), []byte("v1")),
		ffi.Put([]byte("a/2"), []byte("v2")),
	})
	if err != nil {
		t.Fatalf("Propose p1: %v", err)
	}

	// p2 PrefixDeletes "a/".
	p2, err := p1.Propose(ctx, []ffi.BatchOp{ffi.PrefixDelete([]byte("a/"))})
	if err != nil {
		t.Fatalf("Propose p2: %v", err)
	}

	// p2 should not see a/1 or a/2.
	for _, key := range []string{"a/1", "a/2"} {
		val, err := p2.Get(ctx, []byte(key))
		if err != nil {
			t.Fatalf("p2.Get(%s): %v", key, err)
		}
		if val != nil {
			t.Fatalf("p2.Get(%s) = %q, want nil", key, val)
		}
	}

	// Commit the chain.
	if err := p1.Commit(ctx); err != nil {
		t.Fatalf("p1.Commit: %v", err)
	}
	if err := p2.Commit(ctx); err != nil {
		t.Fatalf("p2.Commit: %v", err)
	}
}

func TestRemoteDBPrefixDeleteMixed(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{
		"a/1": "old1",
		"b/1": "old2",
	})

	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	// Mixed batch: Put a new key under "a/", then PrefixDelete "a/".
	// The PrefixDelete should remove both the pre-existing a/1 and the
	// just-added a/3.
	newRoot, err := rdb.Update(ctx, []ffi.BatchOp{
		ffi.Put([]byte("a/3"), []byte("new")),
		ffi.PrefixDelete([]byte("a/")),
	})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}
	if newRoot == rootHash {
		t.Fatal("root should change")
	}

	// All "a/" keys should be gone.
	for _, key := range []string{"a/1", "a/3"} {
		val, err := rdb.Get(ctx, []byte(key))
		if err != nil {
			t.Fatalf("Get(%s): %v", key, err)
		}
		if val != nil {
			t.Fatalf("Get(%s) = %q, want nil", key, val)
		}
	}

	// b/1 should still exist.
	val, err := rdb.Get(ctx, []byte("b/1"))
	if err != nil {
		t.Fatalf("Get(b/1): %v", err)
	}
	if string(val) != "old2" {
		t.Fatalf("Get(b/1) = %q, want %q", val, "old2")
	}
}

func TestNewRemoteDBBadRoot(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	insertData(t, db, map[string]string{"key": "value"})
	addr := startServerAndGetAddr(t, db)

	var wrongHash ffi.Hash
	wrongHash[0] = 0xFF

	_, err := NewRemoteDB(ctx, addr, wrongHash, 4)
	if err == nil {
		t.Fatal("expected error with wrong trusted root")
	}
}

// startServerWithInterceptor starts a gRPC server with a unary response
// interceptor and returns an addr suitable for NewRemoteDB.
func startServerWithInterceptor(
	t *testing.T,
	db *ffi.Database,
	interceptor grpc.UnaryServerInterceptor,
) string {
	t.Helper()

	srv := NewServer(db)
	lis, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("net.Listen: %v", err)
	}
	grpcServer := grpc.NewServer(grpc.UnaryInterceptor(interceptor))
	pb.RegisterFirewoodRemoteServer(grpcServer, srv)

	go func() { _ = grpcServer.Serve(lis) }()
	t.Cleanup(grpcServer.Stop)

	return lis.Addr().String()
}

// TestRemoteDBProposalIterTamperedValue verifies that tampering with a value
// in the IterBatch response is detected. Since the client uses pairs extracted
// from the verified proof (not the response's pairs), the tampered data is
// never consumed and the iterator returns the correct (untampered) values.
func TestRemoteDBProposalIterTamperedValue(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"a": "1", "b": "2"})

	// Interceptor that tampers with the first pair's value in IterBatch responses.
	tamper := func(ctx context.Context, req any, info *grpc.UnaryServerInfo, handler grpc.UnaryHandler) (any, error) {
		resp, err := handler(ctx, req)
		if err != nil {
			return resp, err
		}
		if strings.HasSuffix(info.FullMethod, "/IterBatch") {
			iterResp := resp.(*pb.IterBatchResponse)
			if len(iterResp.GetPairs()) > 0 {
				// Deep-copy and tamper the pairs (not the proof).
				cloned := proto.Clone(iterResp).(*pb.IterBatchResponse)
				cloned.Pairs[0].Value = []byte("TAMPERED")
				return cloned, nil
			}
		}
		return resp, err
	}

	addr := startServerWithInterceptor(t, db, tamper)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	prop, err := rdb.Propose(ctx, []ffi.BatchOp{ffi.Put([]byte("c"), []byte("3"))})
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}
	defer prop.Drop()

	it, err := prop.Iter(ctx, nil)
	if err != nil {
		t.Fatalf("Iter: %v", err)
	}
	defer it.Drop()

	// The iterator should return the correct (untampered) values from the proof.
	expected := map[string]string{"a": "1", "b": "2", "c": "3"}
	for it.Next() {
		key := string(it.Key())
		want, ok := expected[key]
		if !ok {
			t.Fatalf("unexpected key %q", key)
		}
		if got := string(it.Value()); got != want {
			t.Fatalf("Value(%s) = %q, want %q (tampered data was not consumed)", key, got, want)
		}
		delete(expected, key)
	}
	if err := it.Err(); err != nil {
		t.Fatalf("Iter.Err: %v", err)
	}
	if len(expected) > 0 {
		t.Fatalf("missing keys: %v", expected)
	}
}

// TestRemoteDBProposalIterMissingProof verifies that the client rejects an
// IterBatch response that has pairs but no range proof.
func TestRemoteDBProposalIterMissingProof(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"a": "1"})

	// Interceptor that strips the range proof from IterBatch responses.
	stripProof := func(ctx context.Context, req any, info *grpc.UnaryServerInfo, handler grpc.UnaryHandler) (any, error) {
		resp, err := handler(ctx, req)
		if err != nil {
			return resp, err
		}
		if strings.HasSuffix(info.FullMethod, "/IterBatch") {
			iterResp := resp.(*pb.IterBatchResponse)
			iterResp.RangeProof = nil
			return iterResp, nil
		}
		return resp, err
	}

	addr := startServerWithInterceptor(t, db, stripProof)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	prop, err := rdb.Propose(ctx, []ffi.BatchOp{ffi.Put([]byte("b"), []byte("2"))})
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}
	defer prop.Drop()

	_, err = prop.Iter(ctx, nil)
	if err == nil {
		t.Fatal("expected error when range proof is missing")
	}
	if !strings.Contains(err.Error(), "missing range proof") {
		t.Fatalf("unexpected error: %v", err)
	}
}

// TestRemoteDBProposalIterTamperedProof verifies that the client rejects an
// IterBatch response with a corrupted range proof.
func TestRemoteDBProposalIterTamperedProof(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"a": "1"})

	// Interceptor that corrupts the range proof bytes.
	corruptProof := func(ctx context.Context, req any, info *grpc.UnaryServerInfo, handler grpc.UnaryHandler) (any, error) {
		resp, err := handler(ctx, req)
		if err != nil {
			return resp, err
		}
		if strings.HasSuffix(info.FullMethod, "/IterBatch") {
			iterResp := resp.(*pb.IterBatchResponse)
			if len(iterResp.RangeProof) > 0 {
				// Flip some bytes in the proof.
				corrupted := append([]byte(nil), iterResp.RangeProof...)
				for i := range min(10, len(corrupted)) {
					corrupted[i] ^= 0xFF
				}
				iterResp.RangeProof = corrupted
			}
			return iterResp, nil
		}
		return resp, err
	}

	addr := startServerWithInterceptor(t, db, corruptProof)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	prop, err := rdb.Propose(ctx, []ffi.BatchOp{ffi.Put([]byte("b"), []byte("2"))})
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}
	defer prop.Drop()

	_, err = prop.Iter(ctx, nil)
	if err == nil {
		t.Fatal("expected error when range proof is corrupted")
	}
	// The error should mention proof verification failure or deserialization error.
	if !strings.Contains(err.Error(), "range proof") {
		t.Fatalf("unexpected error: %v", err)
	}
}

// TestRemoteDBProposalIterMultiBatch verifies that proof checking works across
// multiple batch boundaries when there are more keys than fit in a single batch.
func TestRemoteDBProposalIterMultiBatch(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	// Insert enough keys to require multiple batches (batch size is 256).
	data := make(map[string]string)
	for i := 0; i < 300; i++ {
		key := fmt.Sprintf("key-%04d", i)
		data[key] = fmt.Sprintf("val-%04d", i)
	}
	rootHash := insertData(t, db, data)

	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	// Create a no-op proposal to get an iterable proposal.
	prop, err := rdb.Propose(ctx, []ffi.BatchOp{ffi.Put([]byte("zzz"), []byte("end"))})
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}
	defer prop.Drop()

	it, err := prop.Iter(ctx, nil)
	if err != nil {
		t.Fatalf("Iter: %v", err)
	}
	defer it.Drop()

	count := 0
	for it.Next() {
		count++
	}
	if err := it.Err(); err != nil {
		t.Fatalf("Iter.Err: %v", err)
	}

	// We inserted 300 keys + 1 from the proposal = 301.
	if count != 301 {
		t.Fatalf("got %d keys, want 301", count)
	}
}

func TestRemoteDBRevisionGet(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	// First commit.
	root1 := insertData(t, db, map[string]string{"a": "1", "b": "2"})

	// Second commit overwrites "a".
	root2 := insertData(t, db, map[string]string{"a": "updated", "c": "3"})

	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, root2, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	// Revision at root1 should see old data.
	rev, err := rdb.Revision(ctx, root1)
	if err != nil {
		t.Fatalf("Revision(root1): %v", err)
	}
	defer rev.Drop()

	if rev.Root() != root1 {
		t.Fatalf("root mismatch: got %x, want %x", rev.Root(), root1)
	}

	val, err := rev.Get(ctx, []byte("a"))
	if err != nil {
		t.Fatalf("Get(a): %v", err)
	}
	if string(val) != "1" {
		t.Fatalf("Get(a) = %q, want %q", val, "1")
	}

	// Key "c" should not exist in root1.
	val, err = rev.Get(ctx, []byte("c"))
	if err != nil {
		t.Fatalf("Get(c): %v", err)
	}
	if val != nil {
		t.Fatalf("Get(c) = %q, want nil", val)
	}
}

func TestRemoteDBRevisionIter(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"a": "1", "b": "2", "c": "3"})

	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	rev, err := rdb.Revision(ctx, rootHash)
	if err != nil {
		t.Fatalf("Revision: %v", err)
	}
	defer rev.Drop()

	it, err := rev.Iter(ctx, nil)
	if err != nil {
		t.Fatalf("Iter: %v", err)
	}
	defer it.Drop()

	var keys []string
	var vals []string
	for it.Next() {
		keys = append(keys, string(it.Key()))
		vals = append(vals, string(it.Value()))
	}
	if err := it.Err(); err != nil {
		t.Fatalf("Iter.Err: %v", err)
	}

	expectedKeys := []string{"a", "b", "c"}
	expectedVals := []string{"1", "2", "3"}
	if len(keys) != len(expectedKeys) {
		t.Fatalf("got %d keys, want %d: %v", len(keys), len(expectedKeys), keys)
	}
	for i := range keys {
		if keys[i] != expectedKeys[i] {
			t.Fatalf("key[%d] = %q, want %q", i, keys[i], expectedKeys[i])
		}
		if vals[i] != expectedVals[i] {
			t.Fatalf("val[%d] = %q, want %q", i, vals[i], expectedVals[i])
		}
	}
}

func TestRemoteDBRevisionIterStartKey(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"a": "1", "b": "2", "c": "3"})

	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	rev, err := rdb.Revision(ctx, rootHash)
	if err != nil {
		t.Fatalf("Revision: %v", err)
	}
	defer rev.Drop()

	// Start iterating from "b".
	it, err := rev.Iter(ctx, []byte("b"))
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

	expected := []string{"b", "c"}
	if len(keys) != len(expected) {
		t.Fatalf("got %d keys, want %d: %v", len(keys), len(expected), keys)
	}
	for i, k := range keys {
		if k != expected[i] {
			t.Fatalf("key[%d] = %q, want %q", i, k, expected[i])
		}
	}
}

func TestRemoteDBRevisionIterMultiBatch(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	// Insert enough keys to require multiple batches (batch size is 256).
	data := make(map[string]string)
	for i := 0; i < 300; i++ {
		key := fmt.Sprintf("key-%04d", i)
		data[key] = fmt.Sprintf("val-%04d", i)
	}
	rootHash := insertData(t, db, data)

	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	rev, err := rdb.Revision(ctx, rootHash)
	if err != nil {
		t.Fatalf("Revision: %v", err)
	}
	defer rev.Drop()

	it, err := rev.Iter(ctx, nil)
	if err != nil {
		t.Fatalf("Iter: %v", err)
	}
	defer it.Drop()

	count := 0
	for it.Next() {
		count++
	}
	if err := it.Err(); err != nil {
		t.Fatalf("Iter.Err: %v", err)
	}

	if count != 300 {
		t.Fatalf("got %d keys, want 300", count)
	}
}

func TestRemoteDBRevisionBadRoot(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"a": "1"})

	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	// Revision creation succeeds (no round-trip).
	var badRoot ffi.Hash
	badRoot[0] = 0xFF
	rev, err := rdb.Revision(ctx, badRoot)
	if err != nil {
		t.Fatalf("Revision: %v", err)
	}
	defer rev.Drop()

	// Get should fail.
	_, err = rev.Get(ctx, []byte("a"))
	if err == nil {
		t.Fatal("expected error for Get with bad root")
	}

	// Iter should fail.
	_, err = rev.Iter(ctx, nil)
	if err == nil {
		t.Fatal("expected error for Iter with bad root")
	}
}

func TestRemoteDBRevisionAfterUpdate(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	root1 := insertData(t, db, map[string]string{"a": "1"})

	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, root1, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	// Create revision at root1.
	rev, err := rdb.Revision(ctx, root1)
	if err != nil {
		t.Fatalf("Revision: %v", err)
	}
	defer rev.Drop()

	// Update the DB to root2.
	_, err = rdb.Update(ctx, []ffi.BatchOp{ffi.Put([]byte("b"), []byte("2"))})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}

	// Revision at root1 should still work.
	val, err := rev.Get(ctx, []byte("a"))
	if err != nil {
		t.Fatalf("Get(a) after update: %v", err)
	}
	if string(val) != "1" {
		t.Fatalf("Get(a) = %q, want %q", val, "1")
	}

	// Key "b" should not exist in root1.
	val, err = rev.Get(ctx, []byte("b"))
	if err != nil {
		t.Fatalf("Get(b): %v", err)
	}
	if val != nil {
		t.Fatalf("Get(b) = %q, want nil", val)
	}
}

func TestRemoteDBRevisionDrop(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"a": "1"})

	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	// Drop is a no-op.
	rev1, err := rdb.Revision(ctx, rootHash)
	if err != nil {
		t.Fatalf("Revision 1: %v", err)
	}
	if err := rev1.Drop(); err != nil {
		t.Fatalf("Drop: %v", err)
	}

	// Can create another revision with the same root afterward.
	rev2, err := rdb.Revision(ctx, rootHash)
	if err != nil {
		t.Fatalf("Revision 2: %v", err)
	}
	defer rev2.Drop()

	val, err := rev2.Get(ctx, []byte("a"))
	if err != nil {
		t.Fatalf("Get(a): %v", err)
	}
	if string(val) != "1" {
		t.Fatalf("Get(a) = %q, want %q", val, "1")
	}
}

func TestRemoteDBLatestRevision(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"apple": "red"})
	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	// Update to get a new root.
	_, err = rdb.Update(ctx, []ffi.BatchOp{ffi.Put([]byte("banana"), []byte("yellow"))})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}

	rev, err := rdb.LatestRevision(ctx)
	if err != nil {
		t.Fatalf("LatestRevision: %v", err)
	}
	defer rev.Drop()

	// Root should match db root.
	if rev.Root() != rdb.Root() {
		t.Fatalf("root mismatch: got %x, want %x", rev.Root(), rdb.Root())
	}

	// Get should return correct data.
	val, err := rev.Get(ctx, []byte("banana"))
	if err != nil {
		t.Fatalf("Get(banana): %v", err)
	}
	if string(val) != "yellow" {
		t.Fatalf("Get(banana) = %q, want %q", val, "yellow")
	}
}

func TestRemoteDBLatestRevisionAfterUpdate(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"a": "1"})
	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	// LatestRevision before update.
	rev1, err := rdb.LatestRevision(ctx)
	if err != nil {
		t.Fatalf("LatestRevision 1: %v", err)
	}
	defer rev1.Drop()
	root1 := rev1.Root()

	// Update.
	_, err = rdb.Update(ctx, []ffi.BatchOp{ffi.Put([]byte("b"), []byte("2"))})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}

	// LatestRevision after update should have new root.
	rev2, err := rdb.LatestRevision(ctx)
	if err != nil {
		t.Fatalf("LatestRevision 2: %v", err)
	}
	defer rev2.Drop()

	if rev2.Root() == root1 {
		t.Fatal("LatestRevision root should change after update")
	}
	if rev2.Root() != rdb.Root() {
		t.Fatalf("root mismatch: got %x, want %x", rev2.Root(), rdb.Root())
	}
}

func TestRemoteDBLatestRevisionMatchesRoot(t *testing.T) {
	db := newTestDB(t)
	ctx := t.Context()

	rootHash := insertData(t, db, map[string]string{"x": "0"})
	addr := startServerAndGetAddr(t, db)

	rdb, err := NewRemoteDB(ctx, addr, rootHash, 4)
	if err != nil {
		t.Fatalf("NewRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	rev, err := rdb.LatestRevision(ctx)
	if err != nil {
		t.Fatalf("LatestRevision: %v", err)
	}
	defer rev.Drop()

	if rev.Root() != rdb.Root() {
		t.Fatalf("LatestRevision().Root() = %x, want %x", rev.Root(), rdb.Root())
	}
}

