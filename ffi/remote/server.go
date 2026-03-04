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

// proposalEntry stores the state needed to commit a proposal and generate
// a witness proof.
type proposalEntry struct {
	proposal *ffi.Proposal
	baseRoot ffi.Hash
	ops      []ffi.BatchOp
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

// CreateProposal creates a proposal from the given batch operations on top
// of the revision identified by the requested root hash. The proposal is stored
// server-side and can later be committed via [Server.CommitProposal].
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

	proposal, err := s.db.Propose(ops)
	if err != nil {
		return nil, fmt.Errorf("propose: %w", err)
	}

	var baseRoot ffi.Hash
	copy(baseRoot[:], req.GetRootHash())

	id := s.nextID.Add(1)
	s.proposals.Store(id, &proposalEntry{
		proposal: proposal,
		baseRoot: baseRoot,
		ops:      ops,
	})

	newRoot := proposal.Root()
	return &pb.CreateProposalResponse{
		ProposalId:  id,
		NewRootHash: newRoot[:],
	}, nil
}

// CommitProposal commits the proposal identified by proposal_id and returns
// a serialized witness proof that enables client-side verification.
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

	newRoot := entry.proposal.Root()

	if err := entry.proposal.Commit(); err != nil {
		return nil, fmt.Errorf("commit: %w", err)
	}

	witness, err := s.db.GenerateWitness(
		entry.baseRoot,
		entry.ops,
		newRoot,
		uint(req.GetDepth()),
	)
	if err != nil {
		return nil, fmt.Errorf("generate witness: %w", err)
	}
	defer witness.Free()

	data, err := witness.MarshalBinary()
	if err != nil {
		return nil, fmt.Errorf("marshal witness: %w", err)
	}

	return &pb.CommitProposalResponse{WitnessProof: data}, nil
}
