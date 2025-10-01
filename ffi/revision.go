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
	"fmt"
	"runtime"
	"unsafe"
)

var (
	errRevisionNotFound  = errors.New("revision not found")
	errInvalidRootLength = fmt.Errorf("root hash must be %d bytes", RootLength)
	errDroppedRevision   = errors.New("revision already dropped")
)

// Revision is an immutable snapshot view over the database at a specific root hash.
// Instances are created via Database.Revision and provide read-only access to the
// state at that revision.
type Revision struct {
	handle *C.RevisionHandle
	// The revision root
	root []byte
}

// Get reads the value stored at the provided key within the revision.
//
// It returns errDroppedRevision if the underlying native handle has already been
// released.
func (r *Revision) Get(key []byte) ([]byte, error) {
	if r.handle == nil {
		return nil, errDroppedRevision
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	return getValueFromValueResult(C.fwd_get_from_revision(
		r.handle,
		newBorrowedBytes(key, &pinner),
	))
}

// Drop releases the memory associated with the Revision.
//
// This is safe to call if the pointer is nil, in which case it does nothing.
//
// The pointer will be set to nil after freeing to prevent double free.
func (r *Revision) Drop() error {
	if r.handle == nil {
		return nil
	}

	if err := getErrorFromVoidResult(C.fwd_free_revision(r.handle)); err != nil {
		return fmt.Errorf("%w: %w", errFreeingValue, err)
	}

	r.handle = nil // Prevent double free

	return nil
}

// getRevisionFromResult converts a C.RevisionResult to a Revision or error.
func getRevisionFromResult(result C.RevisionResult) (*Revision, error) {
	switch result.tag {
	case C.RevisionResult_NullHandlePointer:
		return nil, errDBClosed
	case C.RevisionResult_RevisionNotFound:
		return nil, errRevisionNotFound
	case C.RevisionResult_Ok:
		body := (*C.RevisionResult_Ok_Body)(unsafe.Pointer(&result.anon0))
		proposal := &Revision{
			handle: body.handle,
		}
		return proposal, nil
	case C.RevisionResult_Err:
		err := newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
		return nil, err
	default:
		return nil, fmt.Errorf("unknown C.RevisionResult tag: %d", result.tag)
	}
}
