// Package ffi provides a Go wrapper around the [Firewood] database.
//
// [Firewood]: https://github.com/ava-labs/firewood
package ffi

// // Note that -lm is required on Linux but not on Mac.
// #include <stdlib.h>
// #include "firewood.h"
import "C"

import (
	"bytes"
)

type ProofClient struct {
	db *Database
}

func NewProofClient(db *Database) *ProofClient {
	return &ProofClient{db: db}
}

// CommitRangeProof verifies and commits a range proof to the database.
// It returns the next key to retrieve if successful, or an error if the operation fails.
// If the request doesn't match the expected root or is malformed, it returns a `ErrRequest`.
// If there is a database error, it returns an `ErrDB`.
func (c *ProofClient) CommitRangeProof(targetRoot, proofBytes []byte) ([]byte, error) {
	if c.db.handle == nil {
		return nil, errDBClosed
	}

	if len(targetRoot) == 0 || bytes.Equal(targetRoot, EmptyRoot) {
		return nil, nil
	}

	values, cleanup := newValueFactory()
	defer cleanup()

	proofResult := C.fwd_commit_range_proof(
		c.db.handle,
		values.from(targetRoot),
		values.from(proofBytes),
	)

	return parseProofResponse(&proofResult)
}

// CommitChangeProof verifies and commits a change proof to the database.
// It returns the next key to retrieve if successful, or an error if the operation fails.
// If the request doesn't match the expected root or is malformed, it returns a `ErrRequest`.
// If there is a database error, it returns an `ErrDB`.
func (c *ProofClient) CommitChangeProof(targetStartRoot, targetEndRoot, proofBytes []byte) ([]byte, error) {
	if c.db.handle == nil {
		return nil, errDBClosed
	}

	if len(targetEndRoot) == 0 || bytes.Equal(targetEndRoot, EmptyRoot) {
		return nil, nil
	}

	values, cleanup := newValueFactory()
	defer cleanup()

	proofResult := C.fwd_commit_change_proof(
		c.db.handle,
		values.from(targetStartRoot),
		values.from(targetEndRoot),
		values.from(proofBytes),
	)

	return parseProofResponse(&proofResult)
}
