// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"net"
	"testing"

	ffi "github.com/ava-labs/firewood/ffi"
	pb "github.com/ava-labs/firewood/ffi/remote/proto"
	"google.golang.org/grpc"
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
