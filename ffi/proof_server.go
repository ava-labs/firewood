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

type ProofServer struct {
	db *Database
}

func NewProofServer(db *Database) *ProofServer {
	return &ProofServer{db: db}
}

func (s *ProofServer) GetRangeProof(root, startKey, endKey []byte) ([]byte, error) {
	if s.db.handle == nil {
		return nil, errDBClosed
	}

	if len(root) == 0 || bytes.Equal(root, EmptyRoot) {
		return nil, nil
	}

	values, cleanup := newValueFactory()
	defer cleanup()

	proofResult := C.fwd_get_range_proof(
		s.db.handle,
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

func (s *ProofServer) GetChangeProof(startRoot, endRoot, startKey, endKey []byte) ([]byte, error) {
	if s.db.handle == nil {
		return nil, errDBClosed
	}

	if len(endRoot) == 0 || bytes.Equal(endRoot, EmptyRoot) {
		return nil, nil
	}

	values, cleanup := newValueFactory()
	defer cleanup()

	proofResult := C.fwd_get_change_proof(
		s.db.handle,
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
