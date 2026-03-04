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

// Compile-time interface checks.
var (
	_ ffi.DB         = (*RemoteDB)(nil)
	_ ffi.DBProposal = (*remoteProposal)(nil)
	_ ffi.DBRevision = (*remoteRevision)(nil)
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
func NewRemoteDB(ctx context.Context, addr string, trustedRoot ffi.Hash, depth uint, opts ...ClientOption) (ffi.DB, error) {
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
	return r.client.Propose(ctx, batch)
}

func (r *RemoteDB) Revision(_ context.Context, root ffi.Hash) (ffi.DBRevision, error) {
	return &remoteRevision{root: root, rpc: r.client.rpc}, nil
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
	depth      uint
	// committedTrie is the client's committed trie (the base for all witness
	// verification in this chain). For first-level proposals this is the
	// RemoteDB's trie; for chained proposals it is inherited from the parent.
	committedTrie *ffi.TruncatedTrie
	// expectedCumulativeOps tracks all ops from the chain root to this proposal,
	// used for client-side validation of the witness's embedded batch_ops.
	expectedCumulativeOps []ffi.BatchOp
	// cache is the client's read cache; may be nil.
	cache *readCache

	// mu protects parentTrie swap during Commit; points to Client.mu.
	mu *sync.RWMutex
	// parentTrie points to the parent's trie pointer so Commit can swap it.
	parentTrie *(*ffi.TruncatedTrie)
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
	p.mu.Lock()
	defer p.mu.Unlock()

	if *p.parentTrie != nil {
		(*p.parentTrie).Free()
	}
	*p.parentTrie = p.newTrie
	p.newTrie = nil
	if p.cache != nil {
		p.cache.invalidateBatch(p.expectedCumulativeOps)
	}
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
	return verifiedGet(ctx, p.rpc, p.root, key)
}

func (p *remoteProposal) Iter(ctx context.Context, startKey []byte) (ffi.DBIterator, error) {
	const batchSize = 256

	resp, err := p.rpc.IterBatch(ctx, &pb.IterBatchRequest{
		ProposalId: p.proposalID,
		StartKey:   startKey,
		BatchSize:  batchSize,
		RootHash:   p.root[:],
	})
	if err != nil {
		return nil, fmt.Errorf("iter batch: %w", err)
	}

	verifiedPairs, err := verifyIterBatch(p.root, startKey, batchSize, resp)
	if err != nil {
		return nil, fmt.Errorf("iter batch verification: %w", err)
	}

	return &remoteIterator{
		proposalID: p.proposalID,
		pairs:      verifiedPairs,
		cursor:     -1,
		hasMore:    resp.GetHasMore(),
		rpc:        p.rpc,
		root:       p.root,
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

	// Best-effort cleanup of server-side proposal on any subsequent error.
	proposalID := createResp.GetProposalId()
	success := false
	defer func() {
		if !success {
			_, _ = p.rpc.DropProposal(context.Background(), &pb.DropProposalRequest{
				ProposalId: proposalID,
			})
		}
	}()

	// The server generates witnesses from the committed root with cumulative
	// ops, so verify against the committed trie, not this proposal's trie.
	expectedCumulativeOps := make([]ffi.BatchOp, 0, len(p.expectedCumulativeOps)+len(batch))
	expectedCumulativeOps = append(expectedCumulativeOps, p.expectedCumulativeOps...)
	expectedCumulativeOps = append(expectedCumulativeOps, batch...)

	newTrie, err := verifyWitnessFromResponse(p.committedTrie, createResp, expectedCumulativeOps)
	if err != nil {
		return nil, err
	}

	success = true
	return &remoteProposal{
		proposalID:            proposalID,
		root:                  newTrie.Root(),
		newTrie:               newTrie,
		rpc:                   p.rpc,
		depth:                 p.depth,
		committedTrie:         p.committedTrie,
		expectedCumulativeOps: expectedCumulativeOps,
		cache:                 p.cache,

		mu:         p.mu,
		parentTrie: &p.newTrie,
	}, nil
}

// remoteIterator implements [ffi.DBIterator] with lazy batched fetching from
// the server. Each batch is verified via a range proof before being consumed.
type remoteIterator struct {
	proposalID uint64
	pairs      []*pb.KeyValuePair
	cursor     int
	hasMore    bool
	rpc        pb.FirewoodRemoteClient
	err        error
	root       ffi.Hash // verified proposal root for range proof generation
}

func (it *remoteIterator) Next() bool {
	it.cursor++
	if it.cursor < len(it.pairs) {
		return true
	}
	if !it.hasMore {
		return false
	}

	const batchSize = 256

	// Start key = last verified key + "\x00" (derived from previous
	// batch's verified proof data).
	lastKey := it.pairs[len(it.pairs)-1].GetKey()
	startKey := append(append([]byte(nil), lastKey...), 0x00)

	resp, err := it.rpc.IterBatch(context.Background(), &pb.IterBatchRequest{
		ProposalId: it.proposalID,
		StartKey:   startKey,
		BatchSize:  batchSize,
		RootHash:   it.root[:],
	})
	if err != nil {
		it.err = fmt.Errorf("iter batch: %w", err)
		return false
	}

	verifiedPairs, err := verifyIterBatch(it.root, startKey, batchSize, resp)
	if err != nil {
		it.err = fmt.Errorf("iter batch verification: %w", err)
		return false
	}

	it.pairs = verifiedPairs
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

// remoteRevision implements [ffi.DBRevision] for a committed revision on the
// remote server. Lightweight: holds only a root hash and RPC client. All reads
// are verified via standalone Merkle proofs.
type remoteRevision struct {
	root ffi.Hash
	rpc  pb.FirewoodRemoteClient
}

func (r *remoteRevision) Root() ffi.Hash { return r.root }

func (r *remoteRevision) Get(ctx context.Context, key []byte) ([]byte, error) {
	return verifiedGet(ctx, r.rpc, r.root, key)
}

func (r *remoteRevision) Iter(ctx context.Context, startKey []byte) (ffi.DBIterator, error) {
	const batchSize = 256

	resp, err := r.rpc.IterBatch(ctx, &pb.IterBatchRequest{
		ProposalId: 0, // revision mode
		StartKey:   startKey,
		BatchSize:  batchSize,
		RootHash:   r.root[:],
	})
	if err != nil {
		return nil, fmt.Errorf("iter batch: %w", err)
	}

	verifiedPairs, err := verifyIterBatch(r.root, startKey, batchSize, resp)
	if err != nil {
		return nil, fmt.Errorf("iter batch verification: %w", err)
	}

	return &remoteIterator{
		proposalID: 0, // revision mode for subsequent batches
		pairs:      verifiedPairs,
		cursor:     -1,
		hasMore:    resp.GetHasMore(),
		rpc:        r.rpc,
		root:       r.root,
	}, nil
}

func (r *remoteRevision) Drop() error { return nil }

// verifiedGet fetches a value from the server and verifies its single-key
// Merkle proof against the given root hash.
func verifiedGet(
	ctx context.Context,
	rpc pb.FirewoodRemoteClient,
	root ffi.Hash,
	key []byte,
) ([]byte, error) {
	resp, err := rpc.GetValue(ctx, &pb.GetValueRequest{
		RootHash: root[:],
		Key:      key,
	})
	if err != nil {
		return nil, fmt.Errorf("get value: %w", err)
	}

	var value []byte
	if resp.Value != nil {
		value = resp.GetValue()
	}
	if err := ffi.VerifySingleKeyProof(root, key, value, resp.GetProof()); err != nil {
		return nil, fmt.Errorf("proof verification failed: %w", err)
	}
	return value, nil
}

// verifyIterBatch verifies the range proof in the response and returns the
// verified KV pairs. The response's pairs field is NOT trusted — only the
// proof's embedded pairs are used. startKey is the client-controlled start key
// (nil = beginning of keyspace).
func verifyIterBatch(
	root ffi.Hash,
	startKey []byte,
	batchSize uint32,
	resp *pb.IterBatchResponse,
) ([]*pb.KeyValuePair, error) {
	proofBytes := resp.GetRangeProof()
	if len(resp.GetPairs()) == 0 && len(proofBytes) == 0 {
		// Empty batch with no proof is valid (no entries in range).
		return nil, nil
	}
	if len(proofBytes) == 0 {
		return nil, fmt.Errorf("missing range proof")
	}

	// Single FFI call: deserialize, verify, extract.
	kvs, err := ffi.VerifyAndExtractRangeProof(
		proofBytes,
		root,
		startKey, // client-controlled start key
		nil,      // unbounded end key
		batchSize,
	)
	if err != nil {
		return nil, fmt.Errorf("range proof verification failed: %w", err)
	}

	// Rebuild pairs from verified data.
	verified := make([]*pb.KeyValuePair, len(kvs))
	for i, kv := range kvs {
		verified[i] = &pb.KeyValuePair{Key: kv.Key, Value: kv.Value}
	}
	return verified, nil
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
		} else if op.IsPrefixDelete() {
			pbOp.OpType = pb.BatchOperation_PREFIX_DELETE
		}
		pbOps = append(pbOps, pbOp)
	}
	return pbOps
}

// verifyWitnessFromResponse deserializes and verifies the witness proof from a
// CreateProposalResponse against the given base trie. If expectedCumulativeOps
// is non-nil, it also validates that the witness's embedded batch_ops match
// what the client sent (accounting for PrefixDelete expansion).
func verifyWitnessFromResponse(
	baseTrie *ffi.TruncatedTrie,
	resp *pb.CreateProposalResponse,
	expectedCumulativeOps []ffi.BatchOp,
) (*ffi.TruncatedTrie, error) {
	witness := &ffi.WitnessProof{}
	if err := witness.UnmarshalBinary(resp.GetWitnessProof()); err != nil {
		return nil, fmt.Errorf("unmarshal witness: %w", err)
	}

	// Validate witness ops match cumulative expected ops.
	if expectedCumulativeOps != nil {
		if err := witness.ValidateOps(expectedCumulativeOps); err != nil {
			witness.Free()
			return nil, fmt.Errorf("witness ops validation: %w", err)
		}
	}

	newTrie, err := baseTrie.VerifyWitness(witness)
	witness.Free()
	if err != nil {
		return nil, fmt.Errorf("verify witness: %w", err)
	}

	return newTrie, nil
}
