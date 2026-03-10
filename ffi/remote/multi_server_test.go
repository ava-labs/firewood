// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"context"
	"fmt"
	"net"
	"path/filepath"
	"sync"
	"testing"
	"time"

	ffi "github.com/ava-labs/firewood/ffi"
	pb "github.com/ava-labs/firewood/ffi/remote/proto"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

// newTestMultiDB creates a new multi-validator Firewood database for testing.
func newTestMultiDB(t *testing.T, maxValidators uint) *ffi.MultiDatabase {
	t.Helper()
	dbFile := filepath.Join(t.TempDir(), "test.db")
	db, err := ffi.NewMulti(dbFile, detectHashAlgorithm(), maxValidators)
	if err != nil {
		t.Fatalf("ffi.NewMulti: %v", err)
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

// startMultiServerAndClient starts a multi-head gRPC server and returns a
// connected client that has already called Register.
func startMultiServerAndClient(t *testing.T, db *ffi.MultiDatabase, depth uint) *Client {
	t.Helper()

	srv := NewMultiServer(db)

	lis, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("net.Listen: %v", err)
	}
	grpcServer := grpc.NewServer()
	pb.RegisterFirewoodRemoteServer(grpcServer, srv)

	go func() { _ = grpcServer.Serve(lis) }()
	t.Cleanup(grpcServer.Stop)
	t.Cleanup(srv.Stop)

	conn, err := grpc.NewClient(lis.Addr().String(), grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		t.Fatalf("grpc.NewClient: %v", err)
	}
	client := &Client{
		conn:  conn,
		rpc:   pb.NewFirewoodRemoteClient(conn),
		depth: depth,
	}

	if err := client.Register(t.Context()); err != nil {
		t.Fatalf("Register: %v", err)
	}

	t.Cleanup(func() { client.Close() })
	return client
}

// startMultiServerAndGetAddr starts a multi-head gRPC server and returns its
// address and the server (for Stop).
func startMultiServerAndGetAddr(t *testing.T, db *ffi.MultiDatabase, opts ...MultiServerOption) (*MultiServer, string) {
	t.Helper()

	srv := NewMultiServer(db, opts...)
	lis, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("net.Listen: %v", err)
	}
	grpcServer := grpc.NewServer()
	pb.RegisterFirewoodRemoteServer(grpcServer, srv)

	go func() { _ = grpcServer.Serve(lis) }()
	t.Cleanup(grpcServer.Stop)
	t.Cleanup(srv.Stop)

	return srv, lis.Addr().String()
}

// connectMultiClient creates a client connected to the given address without
// registering.
func connectMultiClient(t *testing.T, addr string, depth uint) *Client {
	t.Helper()

	conn, err := grpc.NewClient(addr, grpc.WithTransportCredentials(insecure.NewCredentials()))
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

// --- Correctness Tests ---

func TestMultiServerRegister(t *testing.T) {
	db := newTestMultiDB(t, 4)
	_, addr := startMultiServerAndGetAddr(t, db)
	client := connectMultiClient(t, addr, 4)

	if err := client.Register(t.Context()); err != nil {
		t.Fatalf("Register: %v", err)
	}

	if !client.multiHead {
		t.Fatal("expected multiHead to be true after Register")
	}
	if client.validatorID == 0 {
		t.Fatal("expected non-zero validatorID after Register")
	}
}

func TestMultiServerRegisterMultiple(t *testing.T) {
	db := newTestMultiDB(t, 4)
	_, addr := startMultiServerAndGetAddr(t, db)

	ids := make(map[uint64]bool)
	for i := 0; i < 3; i++ {
		c := connectMultiClient(t, addr, 4)
		if err := c.Register(t.Context()); err != nil {
			t.Fatalf("Register[%d]: %v", i, err)
		}
		if ids[c.validatorID] {
			t.Fatalf("duplicate validator ID: %d", c.validatorID)
		}
		ids[c.validatorID] = true
	}

	if len(ids) != 3 {
		t.Fatalf("expected 3 unique IDs, got %d", len(ids))
	}
}

func TestMultiServerDeregister(t *testing.T) {
	db := newTestMultiDB(t, 4)
	_, addr := startMultiServerAndGetAddr(t, db)
	client := connectMultiClient(t, addr, 4)

	if err := client.Register(t.Context()); err != nil {
		t.Fatalf("Register: %v", err)
	}

	if err := client.Deregister(t.Context()); err != nil {
		t.Fatalf("Deregister: %v", err)
	}

	if client.multiHead {
		t.Fatal("expected multiHead to be false after Deregister")
	}
}

func TestMultiServerMaxValidators(t *testing.T) {
	db := newTestMultiDB(t, 2)
	_, addr := startMultiServerAndGetAddr(t, db)

	// Register two validators (the maximum).
	c1 := connectMultiClient(t, addr, 4)
	if err := c1.Register(t.Context()); err != nil {
		t.Fatalf("Register c1: %v", err)
	}

	c2 := connectMultiClient(t, addr, 4)
	if err := c2.Register(t.Context()); err != nil {
		t.Fatalf("Register c2: %v", err)
	}

	// Third should fail.
	c3 := connectMultiClient(t, addr, 4)
	if err := c3.Register(t.Context()); err == nil {
		t.Fatal("expected Register to fail when max validators exceeded")
	}
}

func TestMultiServerGetAfterUpdate(t *testing.T) {
	db := newTestMultiDB(t, 4)
	client := startMultiServerAndClient(t, db, 4)
	ctx := t.Context()

	_, err := client.Update(ctx, []ffi.BatchOp{
		ffi.Put([]byte("key1"), []byte("val1")),
	})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}

	val, err := client.Get(ctx, []byte("key1"))
	if err != nil {
		t.Fatalf("Get: %v", err)
	}
	if string(val) != "val1" {
		t.Fatalf("Get = %q, want %q", val, "val1")
	}
}

func TestMultiServerTwoClientIsolation(t *testing.T) {
	db := newTestMultiDB(t, 4)
	_, addr := startMultiServerAndGetAddr(t, db)
	ctx := t.Context()

	clientA := connectMultiClient(t, addr, 4)
	if err := clientA.Register(ctx); err != nil {
		t.Fatalf("Register A: %v", err)
	}

	clientB := connectMultiClient(t, addr, 4)
	if err := clientB.Register(ctx); err != nil {
		t.Fatalf("Register B: %v", err)
	}

	// Client A inserts a key.
	_, err := clientA.Update(ctx, []ffi.BatchOp{
		ffi.Put([]byte("a-key"), []byte("a-val")),
	})
	if err != nil {
		t.Fatalf("Update A: %v", err)
	}

	// Client A should see it.
	val, err := clientA.Get(ctx, []byte("a-key"))
	if err != nil {
		t.Fatalf("Get A: %v", err)
	}
	if string(val) != "a-val" {
		t.Fatalf("Get A = %q, want %q", val, "a-val")
	}

	// Client B should NOT see it (its head hasn't advanced).
	val, err = clientB.Get(ctx, []byte("a-key"))
	if err != nil {
		t.Fatalf("Get B: %v", err)
	}
	if val != nil {
		t.Fatalf("expected nil from B, got %q", val)
	}
}

func TestMultiServerTwoClientDivergence(t *testing.T) {
	db := newTestMultiDB(t, 4)
	_, addr := startMultiServerAndGetAddr(t, db)
	ctx := t.Context()

	clientA := connectMultiClient(t, addr, 4)
	if err := clientA.Register(ctx); err != nil {
		t.Fatalf("Register A: %v", err)
	}

	clientB := connectMultiClient(t, addr, 4)
	if err := clientB.Register(ctx); err != nil {
		t.Fatalf("Register B: %v", err)
	}

	// Both insert different values for the same key.
	_, err := clientA.Update(ctx, []ffi.BatchOp{ffi.Put([]byte("x"), []byte("from-A"))})
	if err != nil {
		t.Fatalf("Update A: %v", err)
	}

	_, err = clientB.Update(ctx, []ffi.BatchOp{ffi.Put([]byte("x"), []byte("from-B"))})
	if err != nil {
		t.Fatalf("Update B: %v", err)
	}

	// Each should see their own value.
	valA, err := clientA.Get(ctx, []byte("x"))
	if err != nil {
		t.Fatalf("Get A: %v", err)
	}
	if string(valA) != "from-A" {
		t.Fatalf("Get A = %q, want %q", valA, "from-A")
	}

	valB, err := clientB.Get(ctx, []byte("x"))
	if err != nil {
		t.Fatalf("Get B: %v", err)
	}
	if string(valB) != "from-B" {
		t.Fatalf("Get B = %q, want %q", valB, "from-B")
	}
}

func TestMultiServerAdvanceToHash(t *testing.T) {
	db := newTestMultiDB(t, 4)
	_, addr := startMultiServerAndGetAddr(t, db)
	ctx := t.Context()

	clientA := connectMultiClient(t, addr, 4)
	if err := clientA.Register(ctx); err != nil {
		t.Fatalf("Register A: %v", err)
	}

	clientB := connectMultiClient(t, addr, 4)
	if err := clientB.Register(ctx); err != nil {
		t.Fatalf("Register B: %v", err)
	}

	// Client A inserts data and gets its root.
	rootA, err := clientA.Update(ctx, []ffi.BatchOp{
		ffi.Put([]byte("key"), []byte("val")),
	})
	if err != nil {
		t.Fatalf("Update A: %v", err)
	}

	// Client B advances to A's root.
	if err := clientB.AdvanceToHash(ctx, rootA); err != nil {
		t.Fatalf("AdvanceToHash B: %v", err)
	}

	// Now B should see A's data.
	val, err := clientB.Get(ctx, []byte("key"))
	if err != nil {
		t.Fatalf("Get B: %v", err)
	}
	if string(val) != "val" {
		t.Fatalf("Get B = %q, want %q", val, "val")
	}
}

func TestMultiServerPropose(t *testing.T) {
	db := newTestMultiDB(t, 4)
	client := startMultiServerAndClient(t, db, 4)
	ctx := t.Context()

	// Bootstrap the client with initial data (required for witness verification).
	if _, err := client.Update(ctx, []ffi.BatchOp{
		ffi.Put([]byte("init"), []byte("data")),
	}); err != nil {
		t.Fatalf("initial Update: %v", err)
	}

	proposal, err := client.Propose(ctx, []ffi.BatchOp{
		ffi.Put([]byte("pk"), []byte("pv")),
	})
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}

	if err := proposal.Commit(ctx); err != nil {
		t.Fatalf("Commit: %v", err)
	}

	val, err := client.Get(ctx, []byte("pk"))
	if err != nil {
		t.Fatalf("Get: %v", err)
	}
	if string(val) != "pv" {
		t.Fatalf("Get = %q, want %q", val, "pv")
	}
}

func TestMultiServerProposalChain(t *testing.T) {
	db := newTestMultiDB(t, 4)
	client := startMultiServerAndClient(t, db, 4)
	ctx := t.Context()

	// Bootstrap the client with initial data (required for witness verification).
	if _, err := client.Update(ctx, []ffi.BatchOp{
		ffi.Put([]byte("init"), []byte("data")),
	}); err != nil {
		t.Fatalf("initial Update: %v", err)
	}

	p1, err := client.Propose(ctx, []ffi.BatchOp{
		ffi.Put([]byte("a"), []byte("1")),
	})
	if err != nil {
		t.Fatalf("Propose p1: %v", err)
	}

	p2, err := p1.Propose(ctx, []ffi.BatchOp{
		ffi.Put([]byte("b"), []byte("2")),
	})
	if err != nil {
		t.Fatalf("Propose p2: %v", err)
	}

	// p2 should have a root different from p1.
	if p2.Root() == p1.Root() {
		t.Fatal("p2 root should differ from p1")
	}

	// Commit chain.
	if err := p1.Commit(ctx); err != nil {
		t.Fatalf("Commit p1: %v", err)
	}
	if err := p2.Commit(ctx); err != nil {
		t.Fatalf("Commit p2: %v", err)
	}
}

func TestMultiServerProposalDrop(t *testing.T) {
	db := newTestMultiDB(t, 4)
	client := startMultiServerAndClient(t, db, 4)
	ctx := t.Context()

	// Insert initial data.
	_, err := client.Update(ctx, []ffi.BatchOp{
		ffi.Put([]byte("keep"), []byte("me")),
	})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}
	rootBefore := client.Root()

	// Create and drop a proposal.
	p, err := client.Propose(ctx, []ffi.BatchOp{
		ffi.Put([]byte("discard"), []byte("this")),
	})
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}
	if err := p.Drop(); err != nil {
		t.Fatalf("Drop: %v", err)
	}

	// Root should be unchanged.
	if client.Root() != rootBefore {
		t.Fatal("root changed after Drop")
	}
}

func TestMultiServerRevision(t *testing.T) {
	db := newTestMultiDB(t, 4)
	client := startMultiServerAndClient(t, db, 4)
	ctx := t.Context()

	root1, err := client.Update(ctx, []ffi.BatchOp{
		ffi.Put([]byte("ver"), []byte("1")),
	})
	if err != nil {
		t.Fatalf("Update 1: %v", err)
	}

	// Read from historical revision.
	rev := client.Revision(root1)
	val, err := rev.Get(ctx, []byte("ver"))
	if err != nil {
		t.Fatalf("rev.Get: %v", err)
	}
	if string(val) != "1" {
		t.Fatalf("rev.Get = %q, want %q", val, "1")
	}
}

func TestMultiServerLatestRevision(t *testing.T) {
	db := newTestMultiDB(t, 4)
	_, addr := startMultiServerAndGetAddr(t, db)
	ctx := t.Context()

	clientA := connectMultiClient(t, addr, 4)
	if err := clientA.Register(ctx); err != nil {
		t.Fatalf("Register A: %v", err)
	}

	clientB := connectMultiClient(t, addr, 4)
	if err := clientB.Register(ctx); err != nil {
		t.Fatalf("Register B: %v", err)
	}

	// Update client A.
	rootA, err := clientA.Update(ctx, []ffi.BatchOp{
		ffi.Put([]byte("k"), []byte("v")),
	})
	if err != nil {
		t.Fatalf("Update A: %v", err)
	}

	// Client A's latest revision should have the updated root.
	revA, err := clientA.LatestRevision()
	if err != nil {
		t.Fatalf("LatestRevision A: %v", err)
	}
	if revA.Root() != rootA {
		t.Fatalf("A latest root = %x, want %x", revA.Root(), rootA)
	}

	// Client B's root should be different (empty or initial).
	rootB := clientB.Root()
	if rootB == rootA {
		t.Fatal("B root should differ from A root")
	}
}

func TestMultiServerPrefixDelete(t *testing.T) {
	db := newTestMultiDB(t, 4)
	client := startMultiServerAndClient(t, db, 4)
	ctx := t.Context()

	// Insert some data.
	_, err := client.Update(ctx, []ffi.BatchOp{
		ffi.Put([]byte("prefix/a"), []byte("1")),
		ffi.Put([]byte("prefix/b"), []byte("2")),
		ffi.Put([]byte("other"), []byte("3")),
	})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}

	// Delete by prefix.
	_, err = client.Update(ctx, []ffi.BatchOp{
		ffi.PrefixDelete([]byte("prefix/")),
	})
	if err != nil {
		t.Fatalf("PrefixDelete: %v", err)
	}

	// prefix/a and prefix/b should be gone.
	for _, key := range []string{"prefix/a", "prefix/b"} {
		val, err := client.Get(ctx, []byte(key))
		if err != nil {
			t.Fatalf("Get(%s): %v", key, err)
		}
		if val != nil {
			t.Fatalf("expected nil for %s, got %q", key, val)
		}
	}

	// "other" should still exist.
	val, err := client.Get(ctx, []byte("other"))
	if err != nil {
		t.Fatalf("Get(other): %v", err)
	}
	if string(val) != "3" {
		t.Fatalf("Get(other) = %q, want %q", val, "3")
	}
}

func TestMultiServerDeregisterOnClose(t *testing.T) {
	db := newTestMultiDB(t, 4)
	_, addr := startMultiServerAndGetAddr(t, db)

	c := connectMultiClient(t, addr, 4)
	if err := c.Register(t.Context()); err != nil {
		t.Fatalf("Register: %v", err)
	}
	wasMulti := c.multiHead

	// Close should deregister.
	if err := c.Close(); err != nil {
		t.Fatalf("Close: %v", err)
	}

	if !wasMulti {
		t.Fatal("expected multiHead before Close")
	}
}

// --- Witness Verification Tests ---

func TestMultiServerWitnessVerification(t *testing.T) {
	db := newTestMultiDB(t, 4)
	client := startMultiServerAndClient(t, db, 4)
	ctx := t.Context()

	// Insert initial data so we have a committed root.
	_, err := client.Update(ctx, []ffi.BatchOp{
		ffi.Put([]byte("init"), []byte("data")),
	})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}

	// Propose new data; this internally verifies via witness.
	p, err := client.Propose(ctx, []ffi.BatchOp{
		ffi.Put([]byte("new"), []byte("stuff")),
	})
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}
	if err := p.Commit(ctx); err != nil {
		t.Fatalf("Commit: %v", err)
	}

	val, err := client.Get(ctx, []byte("new"))
	if err != nil {
		t.Fatalf("Get: %v", err)
	}
	if string(val) != "stuff" {
		t.Fatalf("Get = %q, want %q", val, "stuff")
	}
}

func TestMultiServerIter(t *testing.T) {
	db := newTestMultiDB(t, 4)
	client := startMultiServerAndClient(t, db, 4)
	ctx := t.Context()

	// Insert multiple keys.
	ops := make([]ffi.BatchOp, 0, 10)
	for i := 0; i < 10; i++ {
		ops = append(ops, ffi.Put([]byte(fmt.Sprintf("key%02d", i)), []byte(fmt.Sprintf("val%02d", i))))
	}
	root, err := client.Update(ctx, ops)
	if err != nil {
		t.Fatalf("Update: %v", err)
	}

	// Iterate via revision.
	rev := client.Revision(root)
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
		t.Fatalf("iter error: %v", err)
	}
	if count != 10 {
		t.Fatalf("iterated %d items, want 10", count)
	}
}

func TestMultiServerIterMultiBatch(t *testing.T) {
	db := newTestMultiDB(t, 4)
	client := startMultiServerAndClient(t, db, 4)
	ctx := t.Context()

	// Insert 300+ keys to force multiple batch fetches.
	ops := make([]ffi.BatchOp, 0, 350)
	for i := 0; i < 350; i++ {
		ops = append(ops, ffi.Put([]byte(fmt.Sprintf("k%04d", i)), []byte(fmt.Sprintf("v%04d", i))))
	}
	root, err := client.Update(ctx, ops)
	if err != nil {
		t.Fatalf("Update: %v", err)
	}

	rev := client.Revision(root)
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
		t.Fatalf("iter error: %v", err)
	}
	if count != 350 {
		t.Fatalf("iterated %d items, want 350", count)
	}
}

// --- Error / Edge Case Tests ---

func TestMultiServerUnregisteredValidator(t *testing.T) {
	db := newTestMultiDB(t, 4)
	_, addr := startMultiServerAndGetAddr(t, db)

	client := connectMultiClient(t, addr, 4)
	// Set multiHead without actually registering to simulate an unregistered validator.
	client.multiHead = true
	client.validatorID = 99999

	// Bootstrap manually with an empty root so Get doesn't fail on "not bootstrapped".
	// Actually, Update will try CreateProposal which should fail on the server.
	_, err := client.Update(t.Context(), []ffi.BatchOp{
		ffi.Put([]byte("k"), []byte("v")),
	})
	if err == nil {
		t.Fatal("expected error for unregistered validator")
	}
}

func TestMultiServerProposalTTL(t *testing.T) {
	db := newTestMultiDB(t, 4)
	ttl := 100 * time.Millisecond
	srv, addr := startMultiServerAndGetAddr(t, db, WithMultiProposalTTL(ttl))
	_ = srv
	ctx := t.Context()

	client := connectMultiClient(t, addr, 4)
	if err := client.Register(ctx); err != nil {
		t.Fatalf("Register: %v", err)
	}

	// Bootstrap the client with initial data (required for Propose).
	if _, err := client.Update(ctx, []ffi.BatchOp{
		ffi.Put([]byte("init"), []byte("data")),
	}); err != nil {
		t.Fatalf("initial Update: %v", err)
	}

	// Create a proposal.
	p, err := client.Propose(ctx, []ffi.BatchOp{
		ffi.Put([]byte("ttl-key"), []byte("ttl-val")),
	})
	if err != nil {
		t.Fatalf("Propose: %v", err)
	}

	// Wait for GC to expire the proposal.
	time.Sleep(ttl * 5)

	// Commit should fail because the proposal was reaped.
	err = p.Commit(ctx)
	if err == nil {
		t.Fatal("expected error committing expired proposal")
	}
}

// --- Concurrency Tests ---

func TestMultiServerConcurrentUpdates(t *testing.T) {
	db := newTestMultiDB(t, 20)
	_, addr := startMultiServerAndGetAddr(t, db)
	ctx := t.Context()

	const numClients = 5
	const numUpdates = 10

	var wg sync.WaitGroup
	errs := make(chan error, numClients*numUpdates)

	for i := 0; i < numClients; i++ {
		wg.Add(1)
		go func(clientIdx int) {
			defer wg.Done()
			c := connectMultiClient(t, addr, 4)
			if err := c.Register(ctx); err != nil {
				errs <- fmt.Errorf("client %d Register: %w", clientIdx, err)
				return
			}

			for j := 0; j < numUpdates; j++ {
				key := fmt.Sprintf("client%d-key%d", clientIdx, j)
				val := fmt.Sprintf("client%d-val%d", clientIdx, j)
				_, err := c.Update(ctx, []ffi.BatchOp{
					ffi.Put([]byte(key), []byte(val)),
				})
				if err != nil {
					errs <- fmt.Errorf("client %d Update %d: %w", clientIdx, j, err)
					return
				}
			}
		}(i)
	}

	wg.Wait()
	close(errs)

	for err := range errs {
		t.Error(err)
	}
}

func TestMultiServerConcurrentRegisterDeregister(t *testing.T) {
	// MAX_VALIDATORS is 16 (hard-coded in storage layer), so limit
	// concurrency to stay within the slot limit.
	db := newTestMultiDB(t, 16)
	_, addr := startMultiServerAndGetAddr(t, db)
	ctx := t.Context()

	const concurrency = 10
	const cycles = 3
	var wg sync.WaitGroup
	errs := make(chan error, concurrency*cycles)

	for i := 0; i < concurrency; i++ {
		wg.Add(1)
		go func(idx int) {
			defer wg.Done()
			for j := 0; j < cycles; j++ {
				c := connectMultiClient(t, addr, 4)
				if err := c.Register(ctx); err != nil {
					errs <- fmt.Errorf("goroutine %d cycle %d Register: %w", idx, j, err)
					return
				}
				if err := c.Deregister(ctx); err != nil {
					errs <- fmt.Errorf("goroutine %d cycle %d Deregister: %w", idx, j, err)
					return
				}
			}
		}(i)
	}

	wg.Wait()
	close(errs)

	for err := range errs {
		t.Error(err)
	}
}

// --- Integration Test ---

func TestNewMultiRemoteDB(t *testing.T) {
	db := newTestMultiDB(t, 4)
	_, addr := startMultiServerAndGetAddr(t, db)
	ctx := t.Context()

	rdb, err := NewMultiRemoteDB(ctx, addr, 4)
	if err != nil {
		t.Fatalf("NewMultiRemoteDB: %v", err)
	}
	defer rdb.Close(ctx)

	// Write data.
	newRoot, err := rdb.Update(ctx, []ffi.BatchOp{
		ffi.Put([]byte("hello"), []byte("world")),
	})
	if err != nil {
		t.Fatalf("Update: %v", err)
	}
	if newRoot == (ffi.Hash{}) {
		t.Fatal("expected non-empty root")
	}

	// Read back.
	val, err := rdb.Get(ctx, []byte("hello"))
	if err != nil {
		t.Fatalf("Get: %v", err)
	}
	if string(val) != "world" {
		t.Fatalf("Get = %q, want %q", val, "world")
	}

	// Verify Root() matches.
	if rdb.Root() != newRoot {
		t.Fatalf("Root() = %x, want %x", rdb.Root(), newRoot)
	}
}
