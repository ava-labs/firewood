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

// GetRangeProof retrieves a range proof for the specified root and key range.
// It returns the proof bytes if successful, or an error if the operation fails.
// If the root isn't available, it returns an `ErrRequest`.
// If there is a database error, it returns an `ErrDB`.
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
	return parseProofResponse(&proofResult)
}

// GetChangeProof retrieves a change proof for the specified start and end roots and key range.
// It returns the proof bytes if successful, or an error if the operation fails.
// If the start and/or end roots aren't available, it returns an `ErrRequest`.
// If there is a database error, it returns an `ErrDB`.
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

	return parseProofResponse(&proofResult)
}
