// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"context"
	"fmt"
	"log"
	"sync"
	"sync/atomic"
	"time"

	ffi "github.com/ava-labs/firewood/ffi"
	pb "github.com/ava-labs/firewood/ffi/remote/proto"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/status"
)

// multiProposalEntry stores a pending proposal that can be committed.
type multiProposalEntry struct {
	proposal    *ffi.MultiProposal
	validatorID ffi.ValidatorID
	// committedRoot is the committed revision root that this proposal chain
	// is based on. For first-level proposals this equals root_hash from the
	// request; for chained proposals it is inherited from the parent.
	committedRoot ffi.Hash
	// cumulativeOps accumulates all batch operations from the chain root to
	// this proposal, so that GenerateWitness can produce a correct witness.
	cumulativeOps []ffi.BatchOp
	createdAt     time.Time
}

// MultiServerOption configures optional MultiServer behavior.
type MultiServerOption func(*MultiServer)

// WithMultiProposalTTL enables automatic garbage collection of proposals older
// than ttl. A zero TTL (the default) disables GC.
func WithMultiProposalTTL(ttl time.Duration) MultiServerOption {
	return func(s *MultiServer) { s.proposalTTL = ttl }
}

// WithMultiContext sets the context used by the GC goroutine. When the context
// is cancelled the GC goroutine exits and all remaining proposals are
// reaped — equivalent to calling [MultiServer.Stop].
func WithMultiContext(ctx context.Context) MultiServerOption {
	return func(s *MultiServer) { s.ctx = ctx }
}

// MultiServer implements the FirewoodRemote gRPC service backed by an FFI
// MultiDatabase. Each registered validator gets its own independent head.
type MultiServer struct {
	pb.UnimplementedFirewoodRemoteServer

	db          *ffi.MultiDatabase
	proposals   sync.Map       // map[uint64]*multiProposalEntry
	nextPropID  atomic.Uint64  // proposal IDs
	nextValID   atomic.Uint64  // server-assigned validator IDs
	proposalTTL time.Duration  // 0 = no GC
	ctx         context.Context
	stopGC      chan struct{}
	gcDone      chan struct{}
}

// NewMultiServer creates a new gRPC server wrapping the given multi-validator
// database. If WithMultiProposalTTL is used, the caller must call
// [MultiServer.Stop] or cancel the context passed via [WithMultiContext] to
// terminate the GC goroutine and free remaining proposals.
func NewMultiServer(db *ffi.MultiDatabase, opts ...MultiServerOption) *MultiServer {
	s := &MultiServer{
		db:     db,
		ctx:    context.Background(),
		stopGC: make(chan struct{}),
		gcDone: make(chan struct{}),
	}
	for _, opt := range opts {
		opt(s)
	}
	if s.proposalTTL > 0 {
		go s.runProposalGC()
	} else {
		close(s.gcDone) // no GC goroutine, mark done immediately
	}
	return s
}

func (s *MultiServer) runProposalGC() {
	defer close(s.gcDone)
	interval := s.proposalTTL / 2
	if interval < 100*time.Millisecond {
		interval = 100 * time.Millisecond
	}
	ticker := time.NewTicker(interval)
	defer ticker.Stop()
	for {
		select {
		case <-s.stopGC:
			s.reapExpiredProposals(0) // reap all on shutdown
			return
		case <-s.ctx.Done():
			s.reapExpiredProposals(0) // reap all on context cancellation
			return
		case <-ticker.C:
			s.reapExpiredProposals(s.proposalTTL)
		}
	}
}

func (s *MultiServer) reapExpiredProposals(maxAge time.Duration) {
	now := time.Now()
	s.proposals.Range(func(key, value any) bool {
		entry := value.(*multiProposalEntry)
		if maxAge == 0 || now.Sub(entry.createdAt) > maxAge {
			if _, loaded := s.proposals.LoadAndDelete(key); loaded {
				if err := entry.proposal.Drop(); err != nil {
					log.Printf("reapExpiredProposals: dropping proposal %v: %v", key, err)
				}
			}
		}
		return true
	})
}

// Stop signals the GC goroutine to exit and waits for it to drain all
// remaining proposals. Safe to call multiple times.
func (s *MultiServer) Stop() {
	select {
	case <-s.stopGC:
		// already stopped
	default:
		close(s.stopGC)
	}
	<-s.gcDone
}

// Register assigns a unique validator ID and registers it with the
// multi-head database.
func (s *MultiServer) Register(
	_ context.Context,
	_ *pb.RegisterRequest,
) (*pb.RegisterResponse, error) {
	id := ffi.ValidatorID(s.nextValID.Add(1))

	if err := s.db.RegisterValidator(id); err != nil {
		return nil, status.Errorf(codes.ResourceExhausted, "register validator: %v", err)
	}

	root, err := s.db.Root(id)
	if err != nil {
		return nil, status.Errorf(codes.Internal, "get root: %v", err)
	}

	// A newly registered validator may have no committed revisions. The
	// root hash returned is a computed default (e.g. Keccak empty trie
	// root) that does not correspond to a stored revision. Verify by
	// creating a truncated trie; if the trie's root doesn't match, the
	// validator is empty — return EmptyRoot so the client skips bootstrap.
	if root != ffi.EmptyRoot {
		trie, trieErr := s.db.CreateTruncatedTrie(root, 1)
		if trieErr != nil {
			root = ffi.EmptyRoot
		} else {
			trieRoot, hashErr := trie.RootHash()
			trie.Free()
			if hashErr != nil || trieRoot != root {
				root = ffi.EmptyRoot
			}
		}
	}

	return &pb.RegisterResponse{
		ValidatorId: uint64(id),
		RootHash:    root[:],
	}, nil
}

// Deregister removes a validator from the multi-head database.
func (s *MultiServer) Deregister(
	_ context.Context,
	req *pb.DeregisterRequest,
) (*pb.DeregisterResponse, error) {
	id := ffi.ValidatorID(req.GetValidatorId())
	if err := s.db.DeregisterValidator(id); err != nil {
		return nil, status.Errorf(codes.NotFound, "deregister validator %d: %v", id, err)
	}
	return &pb.DeregisterResponse{}, nil
}

// AdvanceToHash advances a validator's head to an existing revision.
func (s *MultiServer) AdvanceToHash(
	_ context.Context,
	req *pb.AdvanceToHashRequest,
) (*pb.AdvanceToHashResponse, error) {
	id := ffi.ValidatorID(req.GetValidatorId())
	var hash ffi.Hash
	copy(hash[:], req.GetRootHash())

	if err := s.db.AdvanceToHash(id, hash); err != nil {
		return nil, status.Errorf(codes.Internal, "advance to hash: %v", err)
	}
	return &pb.AdvanceToHashResponse{}, nil
}

// GetTruncatedTrie creates a truncated trie from the revision matching the
// requested root hash and returns the serialized trie bytes.
func (s *MultiServer) GetTruncatedTrie(
	_ context.Context,
	req *pb.GetTruncatedTrieRequest,
) (*pb.GetTruncatedTrieResponse, error) {
	var root ffi.Hash
	copy(root[:], req.GetRootHash())

	trie, err := s.db.CreateTruncatedTrie(root, uint(req.GetDepth()))
	if err != nil {
		return nil, fmt.Errorf("create truncated trie: %w", err)
	}
	defer trie.Free()

	data, err := trie.MarshalBinary()
	if err != nil {
		return nil, fmt.Errorf("marshal truncated trie: %w", err)
	}

	return &pb.GetTruncatedTrieResponse{TrieData: data}, nil
}

// GetValue retrieves a value and its single-key Merkle proof from the revision
// identified by the requested root hash.
func (s *MultiServer) GetValue(
	_ context.Context,
	req *pb.GetValueRequest,
) (*pb.GetValueResponse, error) {
	var root ffi.Hash
	copy(root[:], req.GetRootHash())

	result, err := s.db.GetWithProof(root, req.GetKey())
	if err != nil {
		return nil, fmt.Errorf("get with proof: %w", err)
	}

	resp := &pb.GetValueResponse{Proof: result.Proof}
	if result.Found {
		resp.Value = result.Value
	}
	return resp, nil
}

// CreateProposal creates a proposal from the given batch operations. The
// request must include a validator_id for the multi-head server.
func (s *MultiServer) CreateProposal(
	_ context.Context,
	req *pb.CreateProposalRequest,
) (*pb.CreateProposalResponse, error) {
	// Convert proto batch ops to FFI batch ops
	ops := make([]ffi.BatchOp, 0, len(req.GetOps()))
	for _, pbOp := range req.GetOps() {
		switch pbOp.GetOpType() {
		case pb.BatchOperation_PUT:
			ops = append(ops, ffi.Put(pbOp.GetKey(), pbOp.GetValue()))
		case pb.BatchOperation_DELETE:
			ops = append(ops, ffi.Delete(pbOp.GetKey()))
		case pb.BatchOperation_PREFIX_DELETE:
			ops = append(ops, ffi.PrefixDelete(pbOp.GetKey()))
		default:
			return nil, status.Errorf(codes.InvalidArgument, "unknown batch operation type: %v", pbOp.GetOpType())
		}
	}

	var proposal *ffi.MultiProposal
	var committedRoot ffi.Hash
	var cumulativeOps []ffi.BatchOp
	var validatorID ffi.ValidatorID

	if req.ParentProposalId != nil {
		parentID := req.GetParentProposalId()
		val, ok := s.proposals.Load(parentID)
		if !ok {
			return nil, status.Errorf(codes.NotFound, "parent proposal %d not found", parentID)
		}
		parent := val.(*multiProposalEntry)
		validatorID = parent.validatorID

		var err error
		proposal, err = parent.proposal.Propose(ops)
		if err != nil {
			return nil, fmt.Errorf("propose on parent: %w", err)
		}

		// Inherit the committed root and accumulate ops.
		committedRoot = parent.committedRoot
		cumulativeOps = make([]ffi.BatchOp, 0, len(parent.cumulativeOps)+len(ops))
		cumulativeOps = append(cumulativeOps, parent.cumulativeOps...)
		cumulativeOps = append(cumulativeOps, ops...)
	} else {
		if req.ValidatorId == nil {
			return nil, status.Errorf(codes.InvalidArgument, "validator_id is required for multi-head server")
		}
		validatorID = ffi.ValidatorID(req.GetValidatorId())
		copy(committedRoot[:], req.GetRootHash())

		var err error
		proposal, err = s.db.Propose(validatorID, ops)
		if err != nil {
			return nil, fmt.Errorf("propose: %w", err)
		}

		cumulativeOps = ops
	}

	// Drop the proposal if it is not stored in the map before returning.
	stored := false
	defer func() {
		if !stored {
			proposal.Drop()
		}
	}()

	newRoot := proposal.Root()

	// Generate witness proof from the committed root with cumulative ops so
	// the client can verify. When the committed root is EmptyRoot (first
	// proposal on an empty validator), there is no revision to generate a
	// witness from — skip witness generation.
	var witnessData []byte
	if committedRoot != ffi.EmptyRoot {
		witness, err := s.db.GenerateWitness(committedRoot, cumulativeOps, newRoot, uint(req.GetDepth()))
		if err != nil {
			return nil, fmt.Errorf("generate witness: %w", err)
		}
		defer witness.Free()

		witnessData, err = witness.MarshalBinary()
		if err != nil {
			return nil, fmt.Errorf("marshal witness: %w", err)
		}
	}

	id := s.nextPropID.Add(1)
	s.proposals.Store(id, &multiProposalEntry{
		proposal:      proposal,
		validatorID:   validatorID,
		committedRoot: committedRoot,
		cumulativeOps: cumulativeOps,
		createdAt:     time.Now(),
	})
	stored = true

	return &pb.CreateProposalResponse{
		ProposalId:   id,
		NewRootHash:  newRoot[:],
		WitnessProof: witnessData,
	}, nil
}

// CommitProposal commits the proposal identified by proposal_id.
func (s *MultiServer) CommitProposal(
	_ context.Context,
	req *pb.CommitProposalRequest,
) (*pb.CommitProposalResponse, error) {
	id := req.GetProposalId()

	val, ok := s.proposals.LoadAndDelete(id)
	if !ok {
		return nil, status.Errorf(codes.NotFound, "proposal %d not found", id)
	}
	entry := val.(*multiProposalEntry)

	if err := entry.proposal.Commit(); err != nil {
		return nil, fmt.Errorf("commit: %w", err)
	}

	return &pb.CommitProposalResponse{}, nil
}

// DropProposal drops a pending proposal without committing it, freeing
// server-side resources.
func (s *MultiServer) DropProposal(
	_ context.Context,
	req *pb.DropProposalRequest,
) (*pb.DropProposalResponse, error) {
	id := req.GetProposalId()

	val, ok := s.proposals.LoadAndDelete(id)
	if !ok {
		return nil, status.Errorf(codes.NotFound, "proposal %d not found", id)
	}
	entry := val.(*multiProposalEntry)

	if err := entry.proposal.Drop(); err != nil {
		return nil, fmt.Errorf("drop: %w", err)
	}

	return &pb.DropProposalResponse{}, nil
}

// IterBatch returns a batch of key-value pairs from a proposal or committed
// revision, starting at the given key.
func (s *MultiServer) IterBatch(
	_ context.Context,
	req *pb.IterBatchRequest,
) (*pb.IterBatchResponse, error) {
	batchSize := int(req.GetBatchSize())
	if batchSize <= 0 {
		batchSize = 100
	}

	id := req.GetProposalId()

	var it *ffi.Iterator
	var cleanup func()

	if id == 0 {
		// Revision mode: iterate committed state.
		if len(req.GetRootHash()) != ffi.RootLength {
			return nil, status.Errorf(codes.InvalidArgument, "root_hash required when proposal_id is 0")
		}
		var root ffi.Hash
		copy(root[:], req.GetRootHash())

		rev, err := s.db.Revision(root)
		if err != nil {
			return nil, fmt.Errorf("revision: %w", err)
		}
		cleanup = func() { rev.Drop() }

		it, err = rev.Iter(req.GetStartKey())
		if err != nil {
			rev.Drop()
			return nil, fmt.Errorf("iter: %w", err)
		}
	} else {
		// Proposal mode: iterate proposal state.
		val, ok := s.proposals.Load(id)
		if !ok {
			return nil, status.Errorf(codes.NotFound, "proposal %d not found", id)
		}
		entry := val.(*multiProposalEntry)

		var err error
		it, err = entry.proposal.Iter(req.GetStartKey())
		if err != nil {
			return nil, fmt.Errorf("iter: %w", err)
		}
		cleanup = func() {} // no-op for proposals
	}
	defer it.Drop()
	defer cleanup()

	pairs := make([]*pb.KeyValuePair, 0, batchSize)
	for i := 0; i < batchSize && it.Next(); i++ {
		pairs = append(pairs, &pb.KeyValuePair{
			Key:   append([]byte(nil), it.Key()...),
			Value: append([]byte(nil), it.Value()...),
		})
	}
	if err := it.Err(); err != nil {
		return nil, fmt.Errorf("iter next: %w", err)
	}

	// Check if there are more items by attempting one more advance.
	hasMore := it.Next()

	resp := &pb.IterBatchResponse{
		Pairs:   pairs,
		HasMore: hasMore,
	}

	// Generate range proof if root_hash provided.
	if len(req.GetRootHash()) == ffi.RootLength && len(pairs) > 0 {
		var root ffi.Hash
		copy(root[:], req.GetRootHash())

		rangeProof, err := s.db.RangeProof(
			root,
			toMaybe(req.GetStartKey()),
			nil, // unbounded end
			uint32(batchSize),
		)
		if err != nil {
			return nil, fmt.Errorf("range proof: %w", err)
		}
		defer rangeProof.Free()

		proofBytes, err := rangeProof.MarshalBinary()
		if err != nil {
			return nil, fmt.Errorf("marshal range proof: %w", err)
		}
		resp.RangeProof = proofBytes
	}

	return resp, nil
}
