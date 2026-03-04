// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"context"
	"fmt"

	ffi "github.com/ava-labs/firewood/ffi"
	pb "github.com/ava-labs/firewood/ffi/remote/proto"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
)

// Client is a remote Firewood client that holds a truncated trie and
// communicates with a [Server] via gRPC. Every read is verified against a
// Merkle proof and every commit is verified via witness-based re-execution.
type Client struct {
	conn  *grpc.ClientConn
	rpc   pb.FirewoodRemoteClient
	trie  *ffi.TruncatedTrie
	depth uint
}

// NewClient creates a new remote client that will connect to addr.
// It does not bootstrap — call [Client.Bootstrap] with a trusted root hash
// before performing reads or writes.
func NewClient(addr string, depth uint) (*Client, error) {
	conn, err := grpc.NewClient(addr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		return nil, fmt.Errorf("dial: %w", err)
	}
	return &Client{
		conn:  conn,
		rpc:   pb.NewFirewoodRemoteClient(conn),
		depth: depth,
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

	trie := &ffi.TruncatedTrie{}
	if err := trie.UnmarshalBinary(resp.GetTrieData()); err != nil {
		return fmt.Errorf("unmarshal truncated trie: %w", err)
	}

	if err := trie.VerifyRootHash(trustedRootHash); err != nil {
		trie.Free()
		return fmt.Errorf("verify root hash: %w", err)
	}

	// Free old trie if any
	if c.trie != nil {
		c.trie.Free()
	}
	c.trie = trie
	return nil
}

// Get fetches a value from the server and verifies the proof against the
// client's current root hash.
//
// Returns the value and nil error on success, or (nil, nil) if the key does
// not exist.
func (c *Client) Get(ctx context.Context, key []byte) ([]byte, error) {
	if c.trie == nil {
		return nil, fmt.Errorf("client not bootstrapped")
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

	return value, nil
}

// Update sends a batch of operations to the server, commits them, and
// verifies the resulting state change via witness-based re-execution.
// Returns the new root hash on success.
func (c *Client) Update(ctx context.Context, ops []ffi.BatchOp) (ffi.Hash, error) {
	if c.trie == nil {
		return ffi.Hash{}, fmt.Errorf("client not bootstrapped")
	}

	root := c.trie.Root()

	// Convert ops to proto
	pbOps := make([]*pb.BatchOperation, 0, len(ops))
	for _, op := range ops {
		pbOp := &pb.BatchOperation{Key: op.Key()}
		if op.IsPut() {
			pbOp.OpType = pb.BatchOperation_PUT
			pbOp.Value = op.Value()
		} else if op.IsDelete() {
			pbOp.OpType = pb.BatchOperation_DELETE
		} else {
			return ffi.Hash{}, fmt.Errorf("unsupported batch op type for remote update")
		}
		pbOps = append(pbOps, pbOp)
	}

	// Create proposal
	createResp, err := c.rpc.CreateProposal(ctx, &pb.CreateProposalRequest{
		RootHash: root[:],
		Ops:      pbOps,
	})
	if err != nil {
		return ffi.Hash{}, fmt.Errorf("create proposal: %w", err)
	}

	// Commit and get witness
	commitResp, err := c.rpc.CommitProposal(ctx, &pb.CommitProposalRequest{
		ProposalId: createResp.GetProposalId(),
		Depth:      uint32(c.depth),
	})
	if err != nil {
		return ffi.Hash{}, fmt.Errorf("commit proposal: %w", err)
	}

	// Deserialize witness
	witness := &ffi.WitnessProof{}
	if err := witness.UnmarshalBinary(commitResp.GetWitnessProof()); err != nil {
		return ffi.Hash{}, fmt.Errorf("unmarshal witness: %w", err)
	}
	defer witness.Free()

	// Verify witness and get updated trie
	newTrie, err := c.trie.VerifyWitness(witness)
	if err != nil {
		return ffi.Hash{}, fmt.Errorf("verify witness: %w", err)
	}

	// Replace old trie
	c.trie.Free()
	c.trie = newTrie

	return c.trie.Root(), nil
}

// Root returns the current root hash, or an empty hash if not bootstrapped.
func (c *Client) Root() ffi.Hash {
	if c.trie == nil {
		return ffi.Hash{}
	}
	return c.trie.Root()
}

// Close releases all resources held by the client.
func (c *Client) Close() error {
	if c.trie != nil {
		c.trie.Free()
		c.trie = nil
	}
	if c.conn != nil {
		return c.conn.Close()
	}
	return nil
}
