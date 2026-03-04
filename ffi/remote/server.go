// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package remote

import (
	"bytes"
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
	hasPrefixDelete := false
	for _, pbOp := range req.GetOps() {
		switch pbOp.GetOpType() {
		case pb.BatchOperation_PUT:
			ops = append(ops, ffi.Put(pbOp.GetKey(), pbOp.GetValue()))
		case pb.BatchOperation_DELETE:
			ops = append(ops, ffi.Delete(pbOp.GetKey()))
		case pb.BatchOperation_PREFIX_DELETE:
			ops = append(ops, ffi.PrefixDelete(pbOp.GetKey()))
			hasPrefixDelete = true
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

		// If this batch has PrefixDelete ops, expand them for witness generation.
		expandedOps := ops
		if hasPrefixDelete {
			expandedOps, err = expandPrefixDeletes(ops, func(prefix []byte) ([][]byte, error) {
				return prefixScan(parent.proposal, prefix)
			})
			if err != nil {
				return nil, fmt.Errorf("expand prefix deletes: %w", err)
			}
		}

		cumulativeOps = make([]ffi.BatchOp, 0, len(parent.cumulativeOps)+len(expandedOps))
		cumulativeOps = append(cumulativeOps, parent.cumulativeOps...)
		cumulativeOps = append(cumulativeOps, expandedOps...)
	} else {
		copy(committedRoot[:], req.GetRootHash())

		var err error
		proposal, err = s.db.Propose(ops)
		if err != nil {
			return nil, fmt.Errorf("propose: %w", err)
		}

		// If this batch has PrefixDelete ops, expand them for witness generation.
		expandedOps := ops
		if hasPrefixDelete {
			// Create a temporary empty proposal to scan the parent (committed) state.
			scanProposal, err := s.db.Propose(nil)
			if err != nil {
				return nil, fmt.Errorf("create scan proposal: %w", err)
			}
			expandedOps, err = expandPrefixDeletes(ops, func(prefix []byte) ([][]byte, error) {
				return prefixScan(scanProposal, prefix)
			})
			scanProposal.Drop()
			if err != nil {
				return nil, fmt.Errorf("expand prefix deletes: %w", err)
			}
		}

		cumulativeOps = expandedOps
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
	stored = true

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

	resp := &pb.IterBatchResponse{
		Pairs:   pairs,
		HasMore: hasMore,
	}

	// Generate range proof if root_hash provided.
	if len(req.GetRootHash()) == ffi.RootLength && len(pairs) > 0 {
		var root ffi.Hash
		copy(root[:], req.GetRootHash())

		// Proof covers [startKey, ∞) limited to batchSize entries.
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

// prefixScan iterates a proposal starting at prefix and collects all keys that
// have the given prefix.
func prefixScan(proposal *ffi.Proposal, prefix []byte) ([][]byte, error) {
	it, err := proposal.Iter(prefix)
	if err != nil {
		return nil, fmt.Errorf("iter: %w", err)
	}
	defer it.Drop()

	var keys [][]byte
	for it.Next() {
		key := it.Key()
		if !bytes.HasPrefix(key, prefix) {
			break
		}
		keys = append(keys, append([]byte(nil), key...))
	}
	if err := it.Err(); err != nil {
		return nil, fmt.Errorf("iter next: %w", err)
	}
	return keys, nil
}

// expandPrefixDeletes walks ops sequentially and expands each PrefixDelete into
// individual Delete ops. parentIter returns the keys matching a prefix in the
// parent state. Keys added by earlier Puts in the same batch are also expanded.
func expandPrefixDeletes(
	ops []ffi.BatchOp,
	parentIter func(prefix []byte) ([][]byte, error),
) ([]ffi.BatchOp, error) {
	expanded := make([]ffi.BatchOp, 0, len(ops))
	// Track keys added by Puts in this batch (for intra-batch PrefixDelete).
	addedKeys := make(map[string]struct{})

	for _, op := range ops {
		switch {
		case op.IsPut():
			addedKeys[string(op.Key())] = struct{}{}
			expanded = append(expanded, op)

		case op.IsDelete():
			delete(addedKeys, string(op.Key()))
			expanded = append(expanded, op)

		case op.IsPrefixDelete():
			prefix := op.Key()

			// Keys matching prefix in parent state.
			parentKeys, err := parentIter(prefix)
			if err != nil {
				return nil, fmt.Errorf("prefix scan for %q: %w", prefix, err)
			}
			for _, key := range parentKeys {
				expanded = append(expanded, ffi.Delete(key))
			}

			// Keys added by earlier Puts in this batch that match.
			for key := range addedKeys {
				if bytes.HasPrefix([]byte(key), prefix) {
					expanded = append(expanded, ffi.Delete([]byte(key)))
					delete(addedKeys, key)
				}
			}
		}
	}

	return expanded, nil
}

// serverMaybe implements ffi.Maybe[[]byte] for server-side use.
type serverMaybe struct {
	value    []byte
	hasValue bool
}

func (m *serverMaybe) HasValue() bool { return m.hasValue }
func (m *serverMaybe) Value() []byte  { return m.value }

// toMaybe converts a byte slice to a Maybe[[]byte]. A nil or empty slice
// returns nil (None); a non-empty slice returns Something.
func toMaybe(b []byte) ffi.Maybe[[]byte] {
	if len(b) == 0 {
		return nil
	}
	return &serverMaybe{value: b, hasValue: true}
}
