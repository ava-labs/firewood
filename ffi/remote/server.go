// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"context"
	"fmt"
	"sync"
	"sync/atomic"

	ffi "github.com/ava-labs/firewood/ffi"
	pb "github.com/ava-labs/firewood/ffi/remote/proto"
)

// proposalEntry stores a pending proposal that can be committed.
type proposalEntry struct {
	proposal *ffi.Proposal
	// committedRoot is the committed revision root that this proposal chain
	// is based on. For first-level proposals this equals root_hash from the
	// request; for chained proposals it is inherited from the parent.
	committedRoot ffi.Hash
	// cumulativeOps accumulates all batch operations from the chain root to
	// this proposal, so that GenerateWitness (which only works against
	// committed revisions) can produce a correct witness.
	cumulativeOps []ffi.BatchOp
}

// Server implements the FirewoodRemote gRPC service backed by an FFI Database.
type Server struct {
	pb.UnimplementedFirewoodRemoteServer

	db        *ffi.Database
	proposals sync.Map       // map[uint64]*proposalEntry
	nextID    atomic.Uint64
}

// NewServer creates a new gRPC server wrapping the given database.
func NewServer(db *ffi.Database) *Server {
	return &Server{db: db}
}

// GetTruncatedTrie creates a truncated trie from the revision matching the
// requested root hash and returns the serialized trie bytes.
func (s *Server) GetTruncatedTrie(
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
func (s *Server) GetValue(
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

// CreateProposal creates a proposal from the given batch operations. If
// parent_proposal_id is set, the proposal is chained on top of the parent
// proposal; otherwise it is created on top of the database's latest revision.
func (s *Server) CreateProposal(
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
		default:
			return nil, fmt.Errorf("unknown batch operation type: %v", pbOp.GetOpType())
		}
	}

	var proposal *ffi.Proposal
	var committedRoot ffi.Hash
	var cumulativeOps []ffi.BatchOp

	if req.ParentProposalId != nil {
		parentID := req.GetParentProposalId()
		val, ok := s.proposals.Load(parentID)
		if !ok {
			return nil, fmt.Errorf("parent proposal %d not found", parentID)
		}
		parent := val.(*proposalEntry)

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
		copy(committedRoot[:], req.GetRootHash())
		cumulativeOps = ops

		var err error
		proposal, err = s.db.Propose(ops)
		if err != nil {
			return nil, fmt.Errorf("propose: %w", err)
		}
	}

	newRoot := proposal.Root()

	// Generate witness proof from the committed root with cumulative ops so
	// the client can verify. GenerateWitness only works against committed
	// revisions, so for chained proposals we replay all accumulated ops
	// from the chain's committed base.
	witness, err := s.db.GenerateWitness(committedRoot, cumulativeOps, newRoot, uint(req.GetDepth()))
	if err != nil {
		return nil, fmt.Errorf("generate witness: %w", err)
	}
	defer witness.Free()

	witnessData, err := witness.MarshalBinary()
	if err != nil {
		return nil, fmt.Errorf("marshal witness: %w", err)
	}

	id := s.nextID.Add(1)
	s.proposals.Store(id, &proposalEntry{
		proposal:      proposal,
		committedRoot: committedRoot,
		cumulativeOps: cumulativeOps,
	})

	return &pb.CreateProposalResponse{
		ProposalId:   id,
		NewRootHash:  newRoot[:],
		WitnessProof: witnessData,
	}, nil
}

// CommitProposal commits the proposal identified by proposal_id.
// The client should have already verified the witness proof returned by
// [Server.CreateProposal] before calling this.
func (s *Server) CommitProposal(
	_ context.Context,
	req *pb.CommitProposalRequest,
) (*pb.CommitProposalResponse, error) {
	id := req.GetProposalId()

	val, ok := s.proposals.LoadAndDelete(id)
	if !ok {
		return nil, fmt.Errorf("proposal %d not found", id)
	}
	entry := val.(*proposalEntry)

	if err := entry.proposal.Commit(); err != nil {
		return nil, fmt.Errorf("commit: %w", err)
	}

	return &pb.CommitProposalResponse{}, nil
}

// DropProposal drops a pending proposal without committing it, freeing
// server-side resources.
func (s *Server) DropProposal(
	_ context.Context,
	req *pb.DropProposalRequest,
) (*pb.DropProposalResponse, error) {
	id := req.GetProposalId()

	val, ok := s.proposals.LoadAndDelete(id)
	if !ok {
		return nil, fmt.Errorf("proposal %d not found", id)
	}
	entry := val.(*proposalEntry)

	if err := entry.proposal.Drop(); err != nil {
		return nil, fmt.Errorf("drop: %w", err)
	}

	return &pb.DropProposalResponse{}, nil
}

// IterBatch returns a batch of key-value pairs from a proposal, starting at
// the given key. The caller can paginate by setting start_key to the last
// returned key + "\x00".
func (s *Server) IterBatch(
	_ context.Context,
	req *pb.IterBatchRequest,
) (*pb.IterBatchResponse, error) {
	id := req.GetProposalId()

	val, ok := s.proposals.Load(id)
	if !ok {
		return nil, fmt.Errorf("proposal %d not found", id)
	}
	entry := val.(*proposalEntry)

	it, err := entry.proposal.Iter(req.GetStartKey())
	if err != nil {
		return nil, fmt.Errorf("iter: %w", err)
	}
	defer it.Drop()

	batchSize := int(req.GetBatchSize())
	if batchSize <= 0 {
		batchSize = 100
	}

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

	return &pb.IterBatchResponse{
		Pairs:   pairs,
		HasMore: hasMore,
	}, nil
}
