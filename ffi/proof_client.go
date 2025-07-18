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

func (c *ProofClient) CommitRangeProof(root, startKey, endKey []byte) ([]byte, error) {
	if c.db.handle == nil {
		return nil, errDBClosed
	}

	if len(root) == 0 || bytes.Equal(root, EmptyRoot) {
		return nil, nil
	}

	values, cleanup := newValueFactory()
	defer cleanup()

	proofResult := C.fwd_commit_range_proof(
		c.db.handle,
		values.from(root),
		values.from(startKey),
		values.from(endKey),
	)

	proofBytes, dbErr, reqErr := parseProofResponse(&proofResult)

	// Any fatal error should take precedence.
	if dbErr != nil {
		return nil, dbErr
	}
	if reqErr != nil {
		return nil, reqErr
	}
	return proofBytes, nil
}

func (c *ProofClient) CommitChangeProof(startRoot, endRoot, startKey, endKey []byte) ([]byte, error) {
	if c.db.handle == nil {
		return nil, errDBClosed
	}

	if len(endRoot) == 0 || bytes.Equal(endRoot, EmptyRoot) {
		return nil, nil
	}

	values, cleanup := newValueFactory()
	defer cleanup()

	proofResult := C.fwd_commit_change_proof(
		c.db.handle,
		values.from(startRoot),
		values.from(endRoot),
		values.from(startKey),
		values.from(endKey),
	)

	proofBytes, dbErr, reqErr := parseProofResponse(&proofResult)

	// Any fatal error should take precedence.
	if dbErr != nil {
		return nil, dbErr
	}
	if reqErr != nil {
		return nil, reqErr
	}
	return proofBytes, nil
}
