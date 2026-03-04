// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"context"
	"net"
	"os"
	"path/filepath"
	"sync"
	"testing"
	"time"

	ffi "github.com/ava-labs/firewood/ffi"
	pb "github.com/ava-labs/firewood/ffi/remote/proto"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

var (
	detectedAlgo     ffi.NodeHashAlgorithm
	detectedAlgoOnce sync.Once
)

func detectHashAlgorithm() ffi.NodeHashAlgorithm {
	detectedAlgoOnce.Do(func() {
		dir, err := os.MkdirTemp("", "firewood-hash-detect-*")
		if err != nil {
			panic(err)
		}
		defer os.RemoveAll(dir)

		db, err := ffi.New(dir, ffi.EthereumNodeHashing, ffi.WithTruncate(true))
		if err == nil {
			detectedAlgo = ffi.EthereumNodeHashing
			db.Close(context.Background())
		} else {
			detectedAlgo = ffi.MerkleDBNodeHashing
		}
	})
	return detectedAlgo
}

// newTestDB creates a new Firewood database for testing and registers cleanup.
func newTestDB(t *testing.T) *ffi.Database {
	t.Helper()
	dbFile := filepath.Join(t.TempDir(), "test.db")
	db, err := ffi.New(dbFile, detectHashAlgorithm())
	if err != nil {
		t.Fatalf("ffi.New: %v", err)
	}
	t.Cleanup(func() {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		if err := db.Close(ctx); err != nil {
			t.Errorf("db.Close: %v", err)
		}
	})
	return db
}

// startServerAndClient starts a gRPC server and returns a connected client.
func startServerAndClient(t *testing.T, db *ffi.Database, depth uint) *Client {
	t.Helper()

	srv := NewServer(db)

	// Start gRPC server on a random port
	lis, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("net.Listen: %v", err)
	}
	grpcServer := grpc.NewServer()
	pb.RegisterFirewoodRemoteServer(grpcServer, srv)

	go func() {
		if err := grpcServer.Serve(lis); err != nil {
			// Serve returns an error after Stop is called; ignore it.
			_ = err
		}
	}()
	t.Cleanup(grpcServer.Stop)

	// Connect client
	conn, err := grpc.NewClient(lis.Addr().String(), grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		t.Fatalf("grpc.NewClient: %v", err)
	}
	client := &Client{
		conn:  conn,
		rpc:   pb.NewFirewoodRemoteClient(conn),
		depth: depth,
	}
	t.Cleanup(func() { client.Close() })
	return client
}

// insertData inserts key-value pairs into the database and returns the root hash.
func insertData(t *testing.T, db *ffi.Database, kvs map[string]string) ffi.Hash {
	t.Helper()
	ops := make([]ffi.BatchOp, 0, len(kvs))
	for k, v := range kvs {
		ops = append(ops, ffi.Put([]byte(k), []byte(v)))
	}
	proposal, err := db.Propose(ops)
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}
	if err := proposal.Commit(); err != nil {
		t.Fatalf("Commit: %v", err)
	}
	return proposal.Root()
}

func TestBootstrapAndGet(t *testing.T) {
	db := newTestDB(t)

	data := map[string]string{
		"apple":  "red",
		"banana": "yellow",
		"cherry": "dark",
	}
	rootHash := insertData(t, db, data)

	client := startServerAndClient(t, db, 4)
	ctx := t.Context()

	// Bootstrap
	if err := client.Bootstrap(ctx, rootHash); err != nil {
		t.Fatalf("Bootstrap: %v", err)
	}

	// Verify root matches
	if client.Root() != rootHash {
		t.Fatalf("root mismatch: got %x, want %x", client.Root(), rootHash)
	}

	// Get existing key
	val, err := client.Get(ctx, []byte("apple"))
	if err != nil {
		t.Fatalf("Get(apple): %v", err)
	}
	if string(val) != "red" {
		t.Fatalf("Get(apple) = %q, want %q", val, "red")
	}

	// Get non-existent key
	val, err = client.Get(ctx, []byte("nonexistent"))
	if err != nil {
		t.Fatalf("Get(nonexistent): %v", err)
	}
	if val != nil {
		t.Fatalf("Get(nonexistent) = %q, want nil", val)
	}
}

func TestUpdateAndVerify(t *testing.T) {
	db := newTestDB(t)

	data := map[string]string{
		"apple":  "red",
		"banana": "yellow",
	}
	rootHash := insertData(t, db, data)

	client := startServerAndClient(t, db, 4)
	ctx := t.Context()

	if err := client.Bootstrap(ctx, rootHash); err != nil {
		t.Fatalf("Bootstrap: %v", err)
	}
	oldRoot := client.Root()

	// Update: add a new key
	newRoot, err := client.Update(ctx, []ffi.BatchOp{
		ffi.Put([]byte("cherry"), []byte("dark")),
	})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}

	if newRoot == oldRoot {
		t.Fatal("root hash should have changed after update")
	}

	// Verify the new key can be read from the server
	val, err := client.Get(ctx, []byte("cherry"))
	if err != nil {
		t.Fatalf("Get(cherry): %v", err)
	}
	if string(val) != "dark" {
		t.Fatalf("Get(cherry) = %q, want %q", val, "dark")
	}

	// Verify old keys still exist
	val, err = client.Get(ctx, []byte("apple"))
	if err != nil {
		t.Fatalf("Get(apple): %v", err)
	}
	if string(val) != "red" {
		t.Fatalf("Get(apple) = %q, want %q", val, "red")
	}
}

func TestMultipleUpdates(t *testing.T) {
	db := newTestDB(t)

	data := map[string]string{"key0": "val0"}
	rootHash := insertData(t, db, data)

	client := startServerAndClient(t, db, 2)
	ctx := t.Context()

	if err := client.Bootstrap(ctx, rootHash); err != nil {
		t.Fatalf("Bootstrap: %v", err)
	}

	// Perform 5 sequential updates
	for i := 1; i <= 5; i++ {
		key := []byte("key" + string(rune('0'+i)))
		value := []byte("val" + string(rune('0'+i)))
		_, err := client.Update(ctx, []ffi.BatchOp{
			ffi.Put(key, value),
		})
		if err != nil {
			t.Fatalf("Update %d: %v", i, err)
		}
	}

	// Verify all keys can be read
	for i := 0; i <= 5; i++ {
		key := []byte("key" + string(rune('0'+i)))
		expected := []byte("val" + string(rune('0'+i)))
		val, err := client.Get(ctx, key)
		if err != nil {
			t.Fatalf("Get(%s): %v", key, err)
		}
		if string(val) != string(expected) {
			t.Fatalf("Get(%s) = %q, want %q", key, val, expected)
		}
	}
}

func TestBootstrapWithWrongHash(t *testing.T) {
	db := newTestDB(t)

	data := map[string]string{"key": "value"}
	insertData(t, db, data)

	client := startServerAndClient(t, db, 4)
	ctx := t.Context()

	// Bootstrap with an incorrect hash should fail
	var wrongHash ffi.Hash
	wrongHash[0] = 0xFF
	err := client.Bootstrap(ctx, wrongHash)
	if err == nil {
		t.Fatal("expected error bootstrapping with wrong hash")
	}
}

func TestCreateProposalReturnsWitness(t *testing.T) {
	db := newTestDB(t)

	data := map[string]string{"apple": "red"}
	rootHash := insertData(t, db, data)

	// Set up gRPC server without using Client.Update, to directly test the RPCs.
	srv := NewServer(db)
	lis, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("net.Listen: %v", err)
	}
	grpcServer := grpc.NewServer()
	pb.RegisterFirewoodRemoteServer(grpcServer, srv)
	go func() { _ = grpcServer.Serve(lis) }()
	t.Cleanup(grpcServer.Stop)

	conn, err := grpc.NewClient(lis.Addr().String(), grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		t.Fatalf("grpc.NewClient: %v", err)
	}
	defer conn.Close()
	rpcClient := pb.NewFirewoodRemoteClient(conn)
	ctx := t.Context()

	// CreateProposal should return a non-empty witness_proof.
	createResp, err := rpcClient.CreateProposal(ctx, &pb.CreateProposalRequest{
		RootHash: rootHash[:],
		Ops: []*pb.BatchOperation{
			{OpType: pb.BatchOperation_PUT, Key: []byte("banana"), Value: []byte("yellow")},
		},
		Depth: 4,
	})
	if err != nil {
		t.Fatalf("CreateProposal: %v", err)
	}
	if len(createResp.GetWitnessProof()) == 0 {
		t.Fatal("expected non-empty witness_proof in CreateProposalResponse")
	}
	if len(createResp.GetNewRootHash()) != 32 {
		t.Fatalf("expected 32-byte new_root_hash, got %d bytes", len(createResp.GetNewRootHash()))
	}

	// CommitProposal should succeed with only proposal_id.
	_, err = rpcClient.CommitProposal(ctx, &pb.CommitProposalRequest{
		ProposalId: createResp.GetProposalId(),
	})
	if err != nil {
		t.Fatalf("CommitProposal: %v", err)
	}
}

func TestDeleteOperation(t *testing.T) {
	db := newTestDB(t)

	data := map[string]string{
		"apple":  "red",
		"banana": "yellow",
		"cherry": "dark",
	}
	rootHash := insertData(t, db, data)

	client := startServerAndClient(t, db, 4)
	ctx := t.Context()

	if err := client.Bootstrap(ctx, rootHash); err != nil {
		t.Fatalf("Bootstrap: %v", err)
	}

	// Delete a key
	_, err := client.Update(ctx, []ffi.BatchOp{
		ffi.Delete([]byte("banana")),
	})
	if err != nil {
		t.Fatalf("Update (delete): %v", err)
	}

	// Verify the deleted key returns nil
	val, err := client.Get(ctx, []byte("banana"))
	if err != nil {
		t.Fatalf("Get(banana): %v", err)
	}
	if val != nil {
		t.Fatalf("Get(banana) = %q, want nil after delete", val)
	}

	// Other keys should still exist
	val, err = client.Get(ctx, []byte("apple"))
	if err != nil {
		t.Fatalf("Get(apple): %v", err)
	}
	if string(val) != "red" {
		t.Fatalf("Get(apple) = %q, want %q", val, "red")
	}
}
