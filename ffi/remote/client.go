// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"context"
	"fmt"
	"sync"

	ffi "github.com/ava-labs/firewood/ffi"
	pb "github.com/ava-labs/firewood/ffi/remote/proto"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

// ClientOption configures a [Client].
type ClientOption func(*Client)

// WithCache enables a client-side read cache for verified Get results with
// the given eviction policy. maxEntries is the maximum number of key-value
// pairs to cache. A value of zero or negative disables the cache.
func WithCache(maxEntries int, policy EvictionPolicy) ClientOption {
	return func(c *Client) {
		if maxEntries > 0 {
			c.cache = newReadCache(maxEntries, policy)
		}
	}
}

// WithCacheSize enables a client-side read cache using the [LRU] eviction
// policy. maxEntries is the maximum number of key-value pairs to cache. A
// value of zero or negative disables the cache.
func WithCacheSize(maxEntries int) ClientOption {
	return WithCache(maxEntries, LRU)
}

// Client is a remote Firewood client that holds a truncated trie and
// communicates with a [Server] via gRPC. Every read is verified against a
// Merkle proof and every commit is verified via witness-based re-execution.
type Client struct {
	conn  *grpc.ClientConn
	rpc   pb.FirewoodRemoteClient
	depth uint

	mu    sync.RWMutex      // protects trie
	trie  *ffi.TruncatedTrie
	cache *readCache // nil when disabled

	// Multi-head fields. When multiHead is true, validatorID is included in
	// CreateProposalRequests and Register/Deregister are used for lifecycle.
	validatorID uint64
	multiHead   bool
}

// NewClient creates a new remote client that will connect to addr.
// It does not bootstrap — call [Client.Bootstrap] with a trusted root hash
// before performing reads or writes.
func NewClient(addr string, depth uint, opts ...ClientOption) (*Client, error) {
	conn, err := grpc.NewClient(addr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		return nil, fmt.Errorf("dial: %w", err)
	}
	c := &Client{
		conn:  conn,
		rpc:   pb.NewFirewoodRemoteClient(conn),
		depth: depth,
	}
	for _, opt := range opts {
		opt(c)
	}
	return c, nil
}

// Register calls the Register RPC on a multi-head server, storing the
// returned validator ID and bootstrapping the truncated trie using the
// assigned root hash. Only meaningful when connected to a MultiServer.
func (c *Client) Register(ctx context.Context) error {
	resp, err := c.rpc.Register(ctx, &pb.RegisterRequest{})
	if err != nil {
		return fmt.Errorf("register: %w", err)
	}

	c.validatorID = resp.GetValidatorId()
	c.multiHead = true

	var root ffi.Hash
	copy(root[:], resp.GetRootHash())

	// An empty root means the validator has no committed state yet.
	// Skip bootstrap — the client will be bootstrapped on first Update.
	if root == ffi.EmptyRoot {
		return nil
	}

	return c.Bootstrap(ctx, root)
}

// Deregister calls the Deregister RPC on a multi-head server. After this
// call the client should not perform further operations.
func (c *Client) Deregister(ctx context.Context) error {
	if !c.multiHead {
		return nil
	}

	_, err := c.rpc.Deregister(ctx, &pb.DeregisterRequest{
		ValidatorId: c.validatorID,
	})
	if err != nil {
		return fmt.Errorf("deregister: %w", err)
	}
	c.multiHead = false
	return nil
}

// AdvanceToHash advances the client's validator head to the given root hash
// on the server, and re-bootstraps the local truncated trie.
func (c *Client) AdvanceToHash(ctx context.Context, hash ffi.Hash) error {
	if !c.multiHead {
		return fmt.Errorf("client not in multi-head mode")
	}

	_, err := c.rpc.AdvanceToHash(ctx, &pb.AdvanceToHashRequest{
		ValidatorId: c.validatorID,
		RootHash:    hash[:],
	})
	if err != nil {
		return fmt.Errorf("advance to hash: %w", err)
	}

	return c.Bootstrap(ctx, hash)
}

// Bootstrap fetches a truncated trie from the server for the given trusted
// root hash and verifies it.
func (c *Client) Bootstrap(ctx context.Context, trustedRootHash ffi.Hash) error {
	resp, err := c.rpc.GetTruncatedTrie(ctx, &pb.GetTruncatedTrieRequest{
		RootHash: trustedRootHash[:],
		Depth:    uint32(c.depth),
	})
	if err != nil {
		return fmt.Errorf("get truncated trie: %w", err)
	}

	trie := &ffi.TruncatedTrie{}
	if err := trie.UnmarshalBinary(resp.GetTrieData()); err != nil {
		return fmt.Errorf("unmarshal truncated trie: %w", err)
	}

	if err := trie.VerifyRootHash(trustedRootHash); err != nil {
		trie.Free()
		return fmt.Errorf("verify root hash: %w", err)
	}

	c.mu.Lock()
	defer c.mu.Unlock()

	// Free old trie if any
	if c.trie != nil {
		c.trie.Free()
	}
	c.trie = trie
	if c.cache != nil {
		c.cache.clear()
	}
	return nil
}

// Get fetches a value from the server and verifies the proof against the
// client's current root hash.
//
// Returns the value and nil error on success, or (nil, nil) if the key does
// not exist.
func (c *Client) Get(ctx context.Context, key []byte) ([]byte, error) {
	c.mu.RLock()
	defer c.mu.RUnlock()

	if c.trie == nil && c.multiHead {
		// Empty validator — no data yet.
		return nil, nil
	}
	if c.trie == nil {
		return nil, fmt.Errorf("client not bootstrapped")
	}

	// Cache lookup.
	if c.cache != nil {
		if entry, ok := c.cache.lookup(key); ok {
			if entry.found {
				return entry.value, nil
			}
			return nil, nil
		}
	}

	root := c.trie.Root()
	resp, err := c.rpc.GetValue(ctx, &pb.GetValueRequest{
		RootHash: root[:],
		Key:      key,
	})
	if err != nil {
		return nil, fmt.Errorf("get value: %w", err)
	}

	// Verify the proof
	var value []byte
	if resp.Value != nil {
		value = resp.GetValue()
	}
	if err := ffi.VerifySingleKeyProof(root, key, value, resp.GetProof()); err != nil {
		return nil, fmt.Errorf("proof verification failed: %w", err)
	}

	// Cache the verified result.
	if c.cache != nil {
		c.cache.store(key, cacheEntry{value: value, found: value != nil})
	}

	return value, nil
}

// Update sends a batch of operations to the server, commits them, and
// verifies the resulting state change via witness-based re-execution.
// Returns the new root hash on success.
func (c *Client) Update(ctx context.Context, ops []ffi.BatchOp) (ffi.Hash, error) {
	c.mu.Lock()
	defer c.mu.Unlock()

	if c.trie == nil && !c.multiHead {
		return ffi.Hash{}, fmt.Errorf("client not bootstrapped")
	}

	// For multi-head clients with an empty validator (no trie yet), commit
	// on the server then bootstrap with the resulting root.
	if c.trie == nil {
		return c.updateEmpty(ctx, ops)
	}

	root := c.trie.Root()
	pbOps := batchOpsToProto(ops)

	// Create proposal and get witness proof for verification before commit.
	createReq := &pb.CreateProposalRequest{
		RootHash: root[:],
		Ops:      pbOps,
		Depth:    uint32(c.depth),
	}
	if c.multiHead {
		createReq.ValidatorId = &c.validatorID
	}
	createResp, err := c.rpc.CreateProposal(ctx, createReq)
	if err != nil {
		return ffi.Hash{}, fmt.Errorf("create proposal: %w", err)
	}

	// Best-effort cleanup of server-side proposal on any subsequent error.
	proposalID := createResp.GetProposalId()
	committed := false
	defer func() {
		if !committed {
			_, _ = c.rpc.DropProposal(context.Background(), &pb.DropProposalRequest{
				ProposalId: proposalID,
			})
		}
	}()

	// Deserialize and verify witness before committing.
	witness := &ffi.WitnessProof{}
	if err := witness.UnmarshalBinary(createResp.GetWitnessProof()); err != nil {
		return ffi.Hash{}, fmt.Errorf("unmarshal witness: %w", err)
	}

	newTrie, err := c.trie.VerifyWitness(witness, ops)
	witness.Free()
	if err != nil {
		return ffi.Hash{}, fmt.Errorf("verify witness: %w", err)
	}

	// Verification passed — commit the proposal.
	_, err = c.rpc.CommitProposal(ctx, &pb.CommitProposalRequest{
		ProposalId: proposalID,
	})
	if err != nil {
		newTrie.Free()
		return ffi.Hash{}, fmt.Errorf("commit proposal: %w", err)
	}
	committed = true

	// Replace old trie.
	c.trie.Free()
	c.trie = newTrie
	if c.cache != nil {
		c.cache.invalidateBatch(ops)
	}

	return c.trie.Root(), nil
}

// updateEmpty handles the first Update on a multi-head client whose validator
// has no committed state yet. It commits the proposal on the server, then
// bootstraps the local truncated trie with the resulting root.
// Caller must hold c.mu.
func (c *Client) updateEmpty(ctx context.Context, ops []ffi.BatchOp) (ffi.Hash, error) {
	var emptyRoot ffi.Hash
	pbOps := batchOpsToProto(ops)

	createReq := &pb.CreateProposalRequest{
		RootHash:    emptyRoot[:],
		Ops:         pbOps,
		Depth:       uint32(c.depth),
		ValidatorId: &c.validatorID,
	}
	createResp, err := c.rpc.CreateProposal(ctx, createReq)
	if err != nil {
		return ffi.Hash{}, fmt.Errorf("create proposal: %w", err)
	}

	proposalID := createResp.GetProposalId()
	committed := false
	defer func() {
		if !committed {
			_, _ = c.rpc.DropProposal(context.Background(), &pb.DropProposalRequest{
				ProposalId: proposalID,
			})
		}
	}()

	_, err = c.rpc.CommitProposal(ctx, &pb.CommitProposalRequest{
		ProposalId: proposalID,
	})
	if err != nil {
		return ffi.Hash{}, fmt.Errorf("commit proposal: %w", err)
	}
	committed = true

	var newRoot ffi.Hash
	copy(newRoot[:], createResp.GetNewRootHash())

	// Bootstrap the truncated trie now that there is committed state.
	// mu is already held by the caller; Bootstrap takes mu itself, so we
	// must unlock temporarily.
	c.mu.Unlock()
	err = c.Bootstrap(ctx, newRoot)
	c.mu.Lock()
	if err != nil {
		return ffi.Hash{}, fmt.Errorf("bootstrap after first update: %w", err)
	}

	return newRoot, nil
}

// Propose creates a proposal on the server and returns a [remoteProposal]
// that can be committed or dropped later. The proposal's witness proof is
// verified against the client's current truncated trie before returning.
func (c *Client) Propose(ctx context.Context, ops []ffi.BatchOp) (*remoteProposal, error) {
	c.mu.RLock()
	defer c.mu.RUnlock()

	if c.trie == nil && !c.multiHead {
		return nil, fmt.Errorf("client not bootstrapped")
	}
	if c.trie == nil {
		return nil, fmt.Errorf("call Update first on empty validator to bootstrap")
	}

	root := c.trie.Root()
	pbOps := batchOpsToProto(ops)

	createReq := &pb.CreateProposalRequest{
		RootHash: root[:],
		Ops:      pbOps,
		Depth:    uint32(c.depth),
	}
	if c.multiHead {
		createReq.ValidatorId = &c.validatorID
	}
	createResp, err := c.rpc.CreateProposal(ctx, createReq)
	if err != nil {
		return nil, fmt.Errorf("create proposal: %w", err)
	}

	// Best-effort cleanup of server-side proposal on any subsequent error.
	proposalID := createResp.GetProposalId()
	success := false
	defer func() {
		if !success {
			_, _ = c.rpc.DropProposal(context.Background(), &pb.DropProposalRequest{
				ProposalId: proposalID,
			})
		}
	}()

	// The server generates witnesses from the committed root, so verify
	// against the client's committed trie.
	expectedCumulativeOps := ops
	newTrie, err := verifyWitnessFromResponse(c.trie, createResp, expectedCumulativeOps)
	if err != nil {
		return nil, err
	}

	success = true
	return &remoteProposal{
		proposalID:            proposalID,
		root:                  newTrie.Root(),
		newTrie:               newTrie,
		rpc:                   c.rpc,
		depth:                 c.depth,
		committedTrie:         c.trie,
		expectedCumulativeOps: expectedCumulativeOps,
		cache:                 c.cache,

		mu:         &c.mu,
		parentTrie: &c.trie,
	}, nil
}

// Revision returns a lightweight [ffi.DBRevision] for the given root hash.
// No server round-trip is made; errors surface on first Get or Iter if the
// root is invalid or has been pruned.
func (c *Client) Revision(root ffi.Hash) ffi.DBRevision {
	return &remoteRevision{root: root, rpc: c.rpc}
}

// LatestRevision returns a [ffi.DBRevision] for the client's current root.
// Returns [ffi.ErrRevisionNotFound] if the client has not been bootstrapped or
// the database is empty (root is [ffi.EmptyRoot]).
func (c *Client) LatestRevision() (ffi.DBRevision, error) {
	root := c.Root()
	if root == ffi.EmptyRoot {
		return nil, ffi.ErrRevisionNotFound
	}
	return &remoteRevision{root: root, rpc: c.rpc}, nil
}

// Root returns the current root hash, or an empty hash if not bootstrapped.
func (c *Client) Root() ffi.Hash {
	c.mu.RLock()
	defer c.mu.RUnlock()

	if c.trie == nil {
		return ffi.Hash{}
	}
	return c.trie.Root()
}

// Close releases all resources held by the client. If the client was
// registered with a multi-head server, it will attempt to deregister
// (best-effort).
func (c *Client) Close() error {
	// Best-effort deregister before closing connection.
	if c.multiHead {
		_ = c.Deregister(context.Background())
	}

	c.mu.Lock()
	defer c.mu.Unlock()

	if c.trie != nil {
		c.trie.Free()
		c.trie = nil
	}
	if c.cache != nil {
		c.cache.clear()
	}
	if c.conn != nil {
		return c.conn.Close()
	}
	return nil
}
