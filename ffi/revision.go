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

type Revision struct {
	handle *C.RevisionHandle
	// The revision root
	root []byte
}

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
