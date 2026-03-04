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

// WitnessProof is a Go wrapper around a witness proof handle returned from
// Firewood. A witness proof contains the minimal set of trie nodes needed for
// a client to independently re-execute batch operations and verify the
// resulting root hash.
//
// Instances are created via [Database.GenerateWitness] and must be freed with
// [WitnessProof.Free] when no longer needed.
type WitnessProof struct {
	handle *C.WitnessProofHandle
}

// Free releases the resources associated with this WitnessProof.
//
// It is safe to call Free more than once; subsequent calls after the first
// will be no-ops.
func (w *WitnessProof) Free() error {
	if w.handle == nil {
		return nil
	}

	if err := getErrorFromVoidResult(C.fwd_free_witness_proof(w.handle)); err != nil {
		return err
	}

	w.handle = nil
	return nil
}

// MarshalBinary serializes the WitnessProof into a binary format suitable
// for wire transport (e.g., gRPC). Implements [encoding.BinaryMarshaler].
func (w *WitnessProof) MarshalBinary() ([]byte, error) {
	if w.handle == nil {
		return nil, fmt.Errorf("witness proof already freed")
	}
	result := C.fwd_witness_proof_to_bytes(w.handle)
	return getValueFromValueResult(result)
}

// UnmarshalBinary deserializes bytes produced by [WitnessProof.MarshalBinary]
// into this WitnessProof, replacing any existing state.
//
// If the receiver already holds a handle, it is freed first.
// Implements [encoding.BinaryUnmarshaler].
func (w *WitnessProof) UnmarshalBinary(data []byte) error {
	// Free any existing handle
	if w.handle != nil {
		if err := w.Free(); err != nil {
			return err
		}
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	result := C.fwd_witness_proof_from_bytes(newBorrowedBytes(data, &pinner))
	newWitness, err := getWitnessProofFromResult(result)
	if err != nil {
		return err
	}
	*w = *newWitness
	return nil
}

// ValidateOps checks that the witness proof's embedded batch_ops match the
// expected operations. Put and Delete require exact match; PrefixDelete
// consumes zero or more consecutive Delete ops whose keys start with the prefix.
func (w *WitnessProof) ValidateOps(expected []BatchOp) error {
	if w.handle == nil {
		return fmt.Errorf("witness proof already freed")
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	cOps := newKeyValuePairsFromBatch(expected, &pinner)
	return getErrorFromVoidResult(C.fwd_validate_witness_ops(w.handle, cOps))
}

// getWitnessProofFromResult converts a C.WitnessResult to a WitnessProof or
// error.
func getWitnessProofFromResult(result C.WitnessResult) (*WitnessProof, error) {
	switch result.tag {
	case C.WitnessResult_NullHandlePointer:
		return nil, errDBClosed
	case C.WitnessResult_Ok:
		ptr := *(**C.WitnessProofHandle)(unsafe.Pointer(&result.anon0))
		return &WitnessProof{handle: ptr}, nil
	case C.WitnessResult_Err:
		return nil, newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
	default:
		return nil, fmt.Errorf("unknown C.WitnessResult tag: %d", result.tag)
	}
}
