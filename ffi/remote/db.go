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
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/status"
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

	var cfg clientConfig
	for _, opt := range opts {
		opt(&cfg)
	}

	rc, err := ffi.NewRemoteClient(cfg.maxCacheBytes, cfg.cachePolicy, cfg.sampleK)
	if err != nil {
		conn.Close()
		return nil, fmt.Errorf("create remote client: %w", err)
	}

	c := &Client{
		conn:  conn,
		rpc:   pb.NewFirewoodRemoteClient(conn),
		depth: depth,
		rc:    rc,
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

func (r *RemoteDB) LatestRevision(_ context.Context) (ffi.DBRevision, error) {
	return r.client.LatestRevision()
}

func (r *RemoteDB) Root() ffi.Hash {
	return r.client.Root()
}

func (r *RemoteDB) Close(_ context.Context) error {
	return r.client.Close()
}

// remoteProposal implements [ffi.DBProposal] for a proposal living on the
// remote server.
//
// Proposals form a linear chain via parentRoot pointers: each child's
// parentRoot points to the parent's root field. On Commit, the child updates
// its parent's root, propagating up the chain to c.root. This design requires:
//   - Linear chains only (no forking — a proposal must have at most one child).
//   - Leaf-to-root commit order (commit the deepest child first).
//
// These constraints are naturally satisfied by the single-client design:
// the server rejects concurrent proposals from the same base, and the
// client controls commit ordering.
type remoteProposal struct {
	proposalID uint64
	root       ffi.Hash
	newTrie    *ffi.TruncatedTrie
	rpc        pb.FirewoodRemoteClient
	depth      uint
	// expectedCumulativeOps tracks all ops from the chain root to this proposal,
	// used for client-side validation of the witness's embedded batch_ops.
	expectedCumulativeOps []ffi.BatchOp
	// witness is kept alive for CommitTrie cache invalidation.
	witness *ffi.WitnessProof
	// rc is the client's RemoteClient for commit and child proposal operations.
	rc *ffi.RemoteClient

	// mu protects parentRoot swap during Commit; points to Client.mu.
	mu *sync.RWMutex
	// parentRoot points to the parent's root hash so Commit can update it
	// atomically (under mu). For a first-level proposal this is &c.root;
	// for a child it is &parent.root (via a direct field pointer, not used
	// for trie swapping since the trie is inside RemoteClient).
	parentRoot *ffi.Hash
}

func (p *remoteProposal) Root() ffi.Hash {
	return p.root
}

func (p *remoteProposal) Commit(ctx context.Context) error {
	// The RPC is sent before acquiring the lock so we don't hold mu during
	// a network round-trip. If the RPC succeeds but the process crashes
	// before the trie swap below completes, the server will have committed
	// but the client's truncated trie will be stale. Recovery: call
	// Client.Bootstrap with the new root hash to re-sync. The server's
	// committed state is the source of truth.
	_, err := p.rpc.CommitProposal(ctx, &pb.CommitProposalRequest{
		ProposalId: p.proposalID,
	})
	if err != nil {
		return fmt.Errorf("commit proposal: %w", err)
	}

	// Commit trie into handle (takes ownership of newTrie, invalidates cache).
	newRoot, commitErr := p.rc.CommitTrie(p.newTrie, p.witness)
	p.newTrie = nil // ownership transferred

	p.mu.Lock()
	defer p.mu.Unlock()

	if commitErr != nil {
		return fmt.Errorf("commit trie: %w", commitErr)
	}

	*p.parentRoot = newRoot

	if p.witness != nil {
		p.witness.Free()
		p.witness = nil
	}
	return nil
}

func (p *remoteProposal) Drop() error {
	var freeErr error
	if p.newTrie != nil {
		freeErr = p.newTrie.Free()
		p.newTrie = nil
	}
	if p.witness != nil {
		p.witness.Free()
		p.witness = nil
	}
	// Best-effort server-side cleanup; use a background context since
	// the caller may not provide one. Suppress "not found" errors — the
	// server already cleaned up (e.g., via GC or a prior Drop).
	_, rpcErr := p.rpc.DropProposal(context.Background(), &pb.DropProposalRequest{
		ProposalId: p.proposalID,
	})
	if rpcErr != nil && status.Code(rpcErr) != codes.NotFound {
		rpcErr = fmt.Errorf("drop proposal: %w", rpcErr)
	} else {
		rpcErr = nil
	}
	return errors.Join(freeErr, rpcErr)
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
		ctx:        ctx,
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

	verifiedTrie, witness, err := verifyWitnessFromResponse(p.rc, createResp, expectedCumulativeOps)
	if err != nil {
		return nil, err
	}

	success = true
	return &remoteProposal{
		proposalID:            proposalID,
		root:                  verifiedTrie.Root(),
		newTrie:               verifiedTrie,
		rpc:                   p.rpc,
		depth:                 p.depth,
		expectedCumulativeOps: expectedCumulativeOps,
		witness:               witness,
		rc:                    p.rc,

		mu:         p.mu,
		parentRoot: &p.root,
	}, nil
}

// remoteIterator implements [ffi.DBIterator] with lazy batched fetching from
// the server. Each batch is verified via a range proof before being consumed.
type remoteIterator struct {
	ctx        context.Context // caller's context for pagination RPCs
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

	resp, err := it.rpc.IterBatch(it.ctx, &pb.IterBatchRequest{
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
		ctx:        ctx,
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
// CreateProposalResponse against the RemoteClient's committed trie. The witness
// is NOT freed — it is returned for later use in CommitTrie.
func verifyWitnessFromResponse(
	rc *ffi.RemoteClient,
	resp *pb.CreateProposalResponse,
	expectedCumulativeOps []ffi.BatchOp,
) (*ffi.TruncatedTrie, *ffi.WitnessProof, error) {
	witness := &ffi.WitnessProof{}
	if err := witness.UnmarshalBinary(resp.GetWitnessProof()); err != nil {
		return nil, nil, fmt.Errorf("unmarshal witness: %w", err)
	}

	newTrie, err := rc.VerifyWitness(witness, expectedCumulativeOps)
	if err != nil {
		witness.Free()
		return nil, nil, fmt.Errorf("verify witness: %w", err)
	}

	return newTrie, witness, nil
}
