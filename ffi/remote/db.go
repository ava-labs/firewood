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

// Compile-time interface checks.
var (
	_ ffi.DB         = (*RemoteDB)(nil)
	_ ffi.DBProposal = (*remoteProposal)(nil)
	_ ffi.DBIterator = (*remoteIterator)(nil)
)

// RemoteDB implements [ffi.DB] backed by a gRPC connection to a remote
// Firewood server. All reads are verified via Merkle proofs and all writes
// are verified via witness-based re-execution.
type RemoteDB struct {
	client *Client
}

// NewRemoteDB creates a [ffi.DB] that talks to a remote Firewood server.
// It bootstraps using trustedRoot and returns an error if verification fails.
func NewRemoteDB(ctx context.Context, addr string, trustedRoot ffi.Hash, depth uint) (ffi.DB, error) {
	conn, err := grpc.NewClient(addr, grpc.WithTransportCredentials(insecure.NewCredentials()))
	if err != nil {
		return nil, fmt.Errorf("dial: %w", err)
	}
	c := &Client{
		conn:  conn,
		rpc:   pb.NewFirewoodRemoteClient(conn),
		depth: depth,
	}
	if err := c.Bootstrap(ctx, trustedRoot); err != nil {
		c.Close()
		return nil, fmt.Errorf("bootstrap: %w", err)
	}
	return &RemoteDB{client: c}, nil
}

func (r *RemoteDB) Get(ctx context.Context, key []byte) ([]byte, error) {
	return r.client.Get(ctx, key)
}

func (r *RemoteDB) Update(ctx context.Context, batch []ffi.BatchOp) (ffi.Hash, error) {
	return r.client.Update(ctx, batch)
}

func (r *RemoteDB) Propose(ctx context.Context, batch []ffi.BatchOp) (ffi.DBProposal, error) {
	root := r.client.Root()

	pbOps := batchOpsToProto(batch)

	createResp, err := r.client.rpc.CreateProposal(ctx, &pb.CreateProposalRequest{
		RootHash: root[:],
		Ops:      pbOps,
		Depth:    uint32(r.client.depth),
	})
	if err != nil {
		return nil, fmt.Errorf("create proposal: %w", err)
	}

	// The server generates witnesses from the committed root, so verify
	// against the client's committed trie.
	newTrie, err := verifyWitnessFromResponse(r.client.trie, createResp)
	if err != nil {
		return nil, err
	}

	return &remoteProposal{
		proposalID:    createResp.GetProposalId(),
		root:          newTrie.Root(),
		newTrie:       newTrie,
		rpc:           r.client.rpc,
		parentTrie:    &r.client.trie,
		committedTrie: r.client.trie,
		depth:         r.client.depth,
	}, nil
}

func (r *RemoteDB) Root() ffi.Hash {
	return r.client.Root()
}

func (r *RemoteDB) Close(_ context.Context) error {
	return r.client.Close()
}

// remoteProposal implements [ffi.DBProposal] for a proposal living on the
// remote server.
type remoteProposal struct {
	proposalID uint64
	root       ffi.Hash
	newTrie    *ffi.TruncatedTrie
	rpc        pb.FirewoodRemoteClient
	// parentTrie points to the parent's trie pointer so Commit can swap it.
	parentTrie *(*ffi.TruncatedTrie)
	// committedTrie is the client's committed trie (the base for all witness
	// verification in this chain). For first-level proposals this is the
	// RemoteDB's trie; for chained proposals it is inherited from the parent.
	committedTrie *ffi.TruncatedTrie
	depth         uint
}

func (p *remoteProposal) Root() ffi.Hash {
	return p.root
}

func (p *remoteProposal) Commit(ctx context.Context) error {
	_, err := p.rpc.CommitProposal(ctx, &pb.CommitProposalRequest{
		ProposalId: p.proposalID,
	})
	if err != nil {
		return fmt.Errorf("commit proposal: %w", err)
	}

	// Replace parent trie with the verified new trie.
	if *p.parentTrie != nil {
		(*p.parentTrie).Free()
	}
	*p.parentTrie = p.newTrie
	p.newTrie = nil
	return nil
}

func (p *remoteProposal) Drop() error {
	if p.newTrie != nil {
		p.newTrie.Free()
		p.newTrie = nil
	}
	// Best-effort server-side cleanup; use a background context since
	// the caller may not provide one.
	_, err := p.rpc.DropProposal(context.Background(), &pb.DropProposalRequest{
		ProposalId: p.proposalID,
	})
	if err != nil {
		return fmt.Errorf("drop proposal: %w", err)
	}
	return nil
}

func (p *remoteProposal) Get(ctx context.Context, key []byte) ([]byte, error) {
	resp, err := p.rpc.GetValue(ctx, &pb.GetValueRequest{
		RootHash: p.root[:],
		Key:      key,
	})
	if err != nil {
		return nil, fmt.Errorf("get value: %w", err)
	}

	var value []byte
	if resp.Value != nil {
		value = resp.GetValue()
	}
	if err := ffi.VerifySingleKeyProof(p.root, key, value, resp.GetProof()); err != nil {
		return nil, fmt.Errorf("proof verification failed: %w", err)
	}

	return value, nil
}

func (p *remoteProposal) Iter(ctx context.Context, startKey []byte) (ffi.DBIterator, error) {
	resp, err := p.rpc.IterBatch(ctx, &pb.IterBatchRequest{
		ProposalId: p.proposalID,
		StartKey:   startKey,
		BatchSize:  256,
	})
	if err != nil {
		return nil, fmt.Errorf("iter batch: %w", err)
	}

	return &remoteIterator{
		proposalID: p.proposalID,
		pairs:      resp.GetPairs(),
		cursor:     -1,
		hasMore:    resp.GetHasMore(),
		rpc:        p.rpc,
	}, nil
}

func (p *remoteProposal) Propose(ctx context.Context, batch []ffi.BatchOp) (ffi.DBProposal, error) {
	pbOps := batchOpsToProto(batch)

	parentID := p.proposalID
	createResp, err := p.rpc.CreateProposal(ctx, &pb.CreateProposalRequest{
		RootHash:         p.root[:],
		Ops:              pbOps,
		Depth:            uint32(p.depth),
		ParentProposalId: &parentID,
	})
	if err != nil {
		return nil, fmt.Errorf("create proposal: %w", err)
	}

	// The server generates witnesses from the committed root with cumulative
	// ops, so verify against the committed trie, not this proposal's trie.
	newTrie, err := verifyWitnessFromResponse(p.committedTrie, createResp)
	if err != nil {
		return nil, err
	}

	return &remoteProposal{
		proposalID:    createResp.GetProposalId(),
		root:          newTrie.Root(),
		newTrie:       newTrie,
		rpc:           p.rpc,
		parentTrie:    &p.newTrie,
		committedTrie: p.committedTrie,
		depth:         p.depth,
	}, nil
}

// remoteIterator implements [ffi.DBIterator] with lazy batched fetching from
// the server.
type remoteIterator struct {
	proposalID uint64
	pairs      []*pb.KeyValuePair
	cursor     int
	hasMore    bool
	rpc        pb.FirewoodRemoteClient
	err        error
}

func (it *remoteIterator) Next() bool {
	it.cursor++
	if it.cursor < len(it.pairs) {
		return true
	}
	if !it.hasMore {
		return false
	}

	// Fetch next batch. Start key = last key + "\x00".
	lastKey := it.pairs[len(it.pairs)-1].GetKey()
	startKey := append(append([]byte(nil), lastKey...), 0x00)

	resp, err := it.rpc.IterBatch(context.Background(), &pb.IterBatchRequest{
		ProposalId: it.proposalID,
		StartKey:   startKey,
		BatchSize:  256,
	})
	if err != nil {
		it.err = fmt.Errorf("iter batch: %w", err)
		return false
	}

	it.pairs = resp.GetPairs()
	it.hasMore = resp.GetHasMore()
	it.cursor = 0
	return len(it.pairs) > 0
}

func (it *remoteIterator) Key() []byte {
	if it.cursor < 0 || it.cursor >= len(it.pairs) {
		return nil
	}
	return it.pairs[it.cursor].GetKey()
}

func (it *remoteIterator) Value() []byte {
	if it.cursor < 0 || it.cursor >= len(it.pairs) {
		return nil
	}
	return it.pairs[it.cursor].GetValue()
}

func (it *remoteIterator) Err() error {
	return it.err
}

func (it *remoteIterator) Drop() error {
	// No server-side iterator state to clean up; batches are fetched per-call.
	return nil
}

// batchOpsToProto converts FFI batch ops to proto batch operations.
func batchOpsToProto(ops []ffi.BatchOp) []*pb.BatchOperation {
	pbOps := make([]*pb.BatchOperation, 0, len(ops))
	for _, op := range ops {
		pbOp := &pb.BatchOperation{Key: op.Key()}
		if op.IsPut() {
			pbOp.OpType = pb.BatchOperation_PUT
			pbOp.Value = op.Value()
		} else if op.IsDelete() {
			pbOp.OpType = pb.BatchOperation_DELETE
		}
		pbOps = append(pbOps, pbOp)
	}
	return pbOps
}

// verifyWitnessFromResponse deserializes and verifies the witness proof from a
// CreateProposalResponse against the given base trie.
func verifyWitnessFromResponse(baseTrie *ffi.TruncatedTrie, resp *pb.CreateProposalResponse) (*ffi.TruncatedTrie, error) {
	witness := &ffi.WitnessProof{}
	if err := witness.UnmarshalBinary(resp.GetWitnessProof()); err != nil {
		return nil, fmt.Errorf("unmarshal witness: %w", err)
	}

	newTrie, err := baseTrie.VerifyWitness(witness)
	witness.Free()
	if err != nil {
		return nil, fmt.Errorf("verify witness: %w", err)
	}

	return newTrie, nil
}
