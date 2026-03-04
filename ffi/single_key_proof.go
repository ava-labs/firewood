// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
import "C"

import (
	"fmt"
	"runtime"
	"unsafe"
)

// GetWithProofResult holds the result of a [Database.GetWithProof] call.
type GetWithProofResult struct {
	// Value is the value associated with the key, or nil if the key was not found.
	Value []byte
	// Proof is the serialized single-key Merkle proof.
	Proof []byte
	// Found is true if the key was found in the trie.
	Found bool
}

// GetWithProof retrieves a value and its single-key Merkle proof from the
// revision identified by root.
//
// If the key exists, the returned [GetWithProofResult] has Found=true and
// Value set to the stored value. If the key does not exist, Found=false and
// Proof contains an exclusion proof.
func (db *Database) GetWithProof(root Hash, key []byte) (*GetWithProofResult, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	result := C.fwd_get_with_proof(
		db.handle,
		newCHashKey(root),
		newBorrowedBytes(key, &pinner),
	)

	return getGetWithProofFromResult(result)
}

// VerifySingleKeyProof verifies a single-key Merkle proof against a known root
// hash. No database handle is needed; verification is purely cryptographic.
//
// Set value to nil for exclusion proofs (proving a key does not exist).
func VerifySingleKeyProof(root Hash, key, value, proof []byte) error {
	var pinner runtime.Pinner
	defer pinner.Unpin()

	valueIsPresent := value != nil

	return getErrorFromVoidResult(C.fwd_verify_single_key_proof(
		newCHashKey(root),
		newBorrowedBytes(key, &pinner),
		newBorrowedBytes(value, &pinner),
		C.bool(valueIsPresent),
		newBorrowedBytes(proof, &pinner),
	))
}

// getGetWithProofFromResult converts a C.GetWithProofResult to a
// GetWithProofResult or error.
func getGetWithProofFromResult(result C.GetWithProofResult) (*GetWithProofResult, error) {
	switch result.tag {
	case C.GetWithProofResult_NullHandlePointer:
		return nil, errDBClosed
	case C.GetWithProofResult_Ok:
		body := (*C.GetWithProofResult_Ok_Body)(unsafe.Pointer(&result.anon0))
		valueOwned := newOwnedBytes(body.value)
		proofOwned := newOwnedBytes(body.proof)
		valueCopy := valueOwned.CopiedBytes()
		proofCopy := proofOwned.CopiedBytes()
		if err := valueOwned.Free(); err != nil {
			_ = proofOwned.Free()
			return nil, err
		}
		if err := proofOwned.Free(); err != nil {
			return nil, err
		}
		return &GetWithProofResult{
			Value: valueCopy,
			Proof: proofCopy,
			Found: true,
		}, nil
	case C.GetWithProofResult_NotFound:
		body := (*C.GetWithProofResult_NotFound_Body)(unsafe.Pointer(&result.anon0))
		proofOwned := newOwnedBytes(body.proof)
		proofCopy := proofOwned.CopiedBytes()
		if err := proofOwned.Free(); err != nil {
			return nil, err
		}
		return &GetWithProofResult{
			Value: nil,
			Proof: proofCopy,
			Found: false,
		}, nil
	case C.GetWithProofResult_Err:
		return nil, newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
	default:
		return nil, fmt.Errorf("unknown C.GetWithProofResult tag: %d", result.tag)
	}
}
