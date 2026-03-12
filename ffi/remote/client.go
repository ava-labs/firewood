// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"context"
	"errors"
	"fmt"
	"sync"

	ffi "github.com/ava-labs/firewood/ffi"
	pb "github.com/ava-labs/firewood/ffi/remote/proto"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

// ClientOption configures a [Client].
type ClientOption func(*clientConfig)

// clientConfig holds configuration for creating a [Client].
type clientConfig struct {
	maxCacheBytes int
	cachePolicy   ffi.EvictionPolicy
	sampleK       int
}

// WithCache enables a client-side read cache for verified Get results with
// the given eviction policy. maxBytes is the memory budget in bytes.
// A value of zero or negative disables the cache.
func WithCache(maxBytes int, policy ffi.EvictionPolicy) ClientOption {
	return func(cfg *clientConfig) {
		cfg.maxCacheBytes = maxBytes
		cfg.cachePolicy = policy
	}
}

// WithCacheSize enables a client-side read cache using the [ffi.EvictionLRU]
// eviction policy. maxBytes is the memory budget in bytes. A value of zero or
// negative disables the cache.
func WithCacheSize(maxBytes int) ClientOption {
	return WithCache(maxBytes, ffi.EvictionLRU)
}

// Client is a remote Firewood client that holds a truncated trie and
// communicates with a [Server] via gRPC. Every read is verified against a
// Merkle proof and every commit is verified via witness-based re-execution.
//
// The client assumes exclusive access to its server — one client per server.
// Multiple clients against the same server will cause proposal conflicts.
type Client struct {
	conn  *grpc.ClientConn
	rpc   pb.FirewoodRemoteClient
	depth uint

	mu   sync.RWMutex       // protects root
	rc   *ffi.RemoteClient  // owns committed trie + cache
	root ffi.Hash           // cached root hash (set by bootstrap/commit)
}

// NewClient creates a new remote client that will connect to addr.
// It does not bootstrap — call [Client.Bootstrap] with a trusted root hash
// before performing reads or writes.
func NewClient(addr string, depth uint, opts ...ClientOption) (*Client, error) {
	conn, err := grpc.NewClient(addr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		return nil, fmt.Errorf("dial: %w", err)
	}

	var cfg clientConfig
	for _, opt := range opts {
		opt(&cfg)
	}

	rc, err := ffi.NewRemoteClient(cfg.maxCacheBytes, cfg.cachePolicy, cfg.sampleK)
	if err != nil {
		conn.Close()
		return nil, fmt.Errorf("create remote client: %w", err)
	}

	return &Client{
		conn:  conn,
		rpc:   pb.NewFirewoodRemoteClient(conn),
		depth: depth,
		rc:    rc,
	}, nil
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

	c.mu.Lock()
	defer c.mu.Unlock()

	root, err := c.rc.Bootstrap(resp.GetTrieData(), trustedRootHash)
	if err != nil {
		return fmt.Errorf("bootstrap: %w", err)
	}
	c.root = root
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

	if c.root == (ffi.Hash{}) {
		return nil, fmt.Errorf("client not bootstrapped")
	}

	// Cache lookup.
	if result := c.rc.CacheLookup(key); result.Cached {
		if result.Found {
			return result.Value, nil
		}
		return nil, nil
	}

	root := c.root
	resp, err := c.rpc.GetValue(ctx, &pb.GetValueRequest{
		RootHash: root[:],
		Key:      key,
	})
	if err != nil {
		return nil, fmt.Errorf("get value: %w", err)
	}

	// Verify the proof and cache the result.
	var value []byte
	if resp.Value != nil {
		value = resp.GetValue()
	}
	if err := c.rc.VerifyGet(key, value, resp.GetProof()); err != nil {
		return nil, fmt.Errorf("proof verification failed: %w", err)
	}

	return value, nil
}

// Update sends a batch of operations to the server, commits them, and
// verifies the resulting state change via witness-based re-execution.
// Returns the new root hash on success.
func (c *Client) Update(ctx context.Context, ops []ffi.BatchOp) (ffi.Hash, error) {
	c.mu.Lock()
	defer c.mu.Unlock()

	if c.root == (ffi.Hash{}) {
		return ffi.Hash{}, fmt.Errorf("client not bootstrapped")
	}

	root := c.root
	pbOps := batchOpsToProto(ops)

	// Create proposal and get witness proof for verification before commit.
	createResp, err := c.rpc.CreateProposal(ctx, &pb.CreateProposalRequest{
		RootHash: root[:],
		Ops:      pbOps,
		Depth:    uint32(c.depth),
	})
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
	newTrie, witness, err := verifyWitnessFromResponse(c.rc, createResp, ops)
	if err != nil {
		return ffi.Hash{}, err
	}

	// Verification passed — commit the proposal.
	_, err = c.rpc.CommitProposal(ctx, &pb.CommitProposalRequest{
		ProposalId: proposalID,
	})
	if err != nil {
		newTrie.Free()
		witness.Free()
		return ffi.Hash{}, fmt.Errorf("commit proposal: %w", err)
	}
	committed = true

	// Commit trie into handle (takes ownership of newTrie, invalidates cache).
	newRoot, err := c.rc.CommitTrie(newTrie, witness)
	witness.Free()
	if err != nil {
		return ffi.Hash{}, fmt.Errorf("commit trie: %w", err)
	}
	c.root = newRoot

	return c.root, nil
}

// Propose creates a proposal on the server and returns a [remoteProposal]
// that can be committed or dropped later. The proposal's witness proof is
// verified against the client's current truncated trie before returning.
func (c *Client) Propose(ctx context.Context, ops []ffi.BatchOp) (*remoteProposal, error) {
	c.mu.RLock()
	defer c.mu.RUnlock()

	if c.root == (ffi.Hash{}) {
		return nil, fmt.Errorf("client not bootstrapped")
	}

	root := c.root
	pbOps := batchOpsToProto(ops)

	createResp, err := c.rpc.CreateProposal(ctx, &pb.CreateProposalRequest{
		RootHash: root[:],
		Ops:      pbOps,
		Depth:    uint32(c.depth),
	})
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
	verifiedTrie, witness, err := verifyWitnessFromResponse(c.rc, createResp, expectedCumulativeOps)
	if err != nil {
		return nil, err
	}

	success = true
	return &remoteProposal{
		proposalID:            proposalID,
		root:                  verifiedTrie.Root(),
		newTrie:               verifiedTrie,
		rpc:                   c.rpc,
		depth:                 c.depth,
		expectedCumulativeOps: expectedCumulativeOps,
		witness:               witness,
		rc:                    c.rc,

		mu:         &c.mu,
		parentRoot: &c.root,
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
	return c.root
}

// Close releases all resources held by the client.
func (c *Client) Close() error {
	c.mu.Lock()
	defer c.mu.Unlock()

	var rcErr error
	if c.rc != nil {
		rcErr = c.rc.Free()
		c.rc = nil
	}
	c.root = ffi.Hash{}
	if c.conn != nil {
		return errors.Join(rcErr, c.conn.Close())
	}
	return rcErr
}
