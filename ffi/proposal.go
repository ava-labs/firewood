// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// Package ffi provides a Go wrapper around the [Firewood] database.
//
// [Firewood]: https://github.com/ava-labs/firewood
package ffi

// #include <stdlib.h>
// #include "firewood.h"
import "C"

import (
	"errors"
	"runtime"
)

var errDroppedProposal = errors.New("proposal already dropped")

type Proposal struct {
	// The database this proposal is associated with. We hold onto this to ensure
	// the database handle outlives the proposal handle, which is required for
	// the proposal to be valid.
	db *Database

	// handle is an opaque pointer to the proposal within Firewood. It should be
	// passed to the C FFI functions that operate on proposals
	//
	// It is not safe to call these methods with a nil handle.
	//
	// Calls to `C.fwd_commit_proposal` and `C.fwd_free_proposal` will invalidate
	// this handle, so it should not be used after those calls.
	handle *C.ProposalHandle

	// The proposal root hash.
	root []byte
}

// Root retrieves the root hash of the proposal.
// If the proposal is empty (i.e. no keys in database),
// it returns nil, nil.
func (p *Proposal) Root() ([]byte, error) {
	return p.root, nil
}

// Get retrieves the value for the given key.
// If the key does not exist, it returns (nil, nil).
func (p *Proposal) Get(key []byte) ([]byte, error) {
	if p.handle == nil {
		return nil, errDroppedProposal
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	borrowed := newBorrowedBytes(key, &pinner)

	return fromValueResult(C.fwd_get_from_proposal(p.handle, borrowed))
}

// Propose creates a new proposal with the given keys and values.
// The proposal is not committed until Commit is called.
func (p *Proposal) Propose(keys, vals [][]byte) (*Proposal, error) {
	if p.handle == nil {
		return nil, errDroppedProposal
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	kvp, err := newKeyValuePairs(keys, vals, &pinner)
	if err != nil {
		return nil, err
	}

	return fromProposalResult(C.fwd_propose_on_proposal(p.handle, kvp), p.db)
}

// Commit commits the proposal and returns any errors.
//
// The proposal handle is no longer valid after this call, but the root
// hash can still be retrieved using Root().
func (p *Proposal) Commit() error {
	if p.handle == nil {
		return errDroppedProposal
	}

	_, err := fromHashResult(C.fwd_commit_proposal(p.handle))
	p.handle = nil // we no longer own the proposal handle

	if err != nil {
		return err
	}

	return nil
}

// Drop removes the proposal from memory in Firewood.
func (p *Proposal) Drop() error {
	return p.free(true)
}
