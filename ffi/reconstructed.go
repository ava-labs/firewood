// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
// #cgo noescape fwd_reconstructed_root_hash
// #cgo nocallback fwd_reconstructed_root_hash
// #cgo noescape fwd_get_from_reconstructed
// #cgo nocallback fwd_get_from_reconstructed
// #cgo noescape fwd_iter_on_reconstructed
// #cgo nocallback fwd_iter_on_reconstructed
// #cgo noescape fwd_reconstruct_on_reconstructed
// #cgo nocallback fwd_reconstruct_on_reconstructed
// #cgo noescape fwd_reconstructed_dump
// #cgo nocallback fwd_reconstructed_dump
// #cgo noescape fwd_free_reconstructed
// #cgo nocallback fwd_free_reconstructed
import "C"

import (
	"errors"
	"fmt"
	"runtime"
	"sync"
	"unsafe"
)

var ErrDroppedReconstructed = errors.New("reconstructed view already dropped")

// Reconstructed is a linear, read-only reconstructed view over a historical
// revision.
//
// Unlike [Proposal], a Reconstructed view cannot be committed and does not
// participate in proposal branching semantics. Calling [Reconstructed.Reconstruct]
// updates this instance in place.
//
// Reconstructed handles must be released before the associated database is closed
// by calling [Reconstructed.Drop]. A cleanup function is registered to call Drop
// automatically if needed, but explicit calls are recommended.
type Reconstructed struct {
	*handle[*C.ReconstructedHandle]

	root    Hash
	rootMu  sync.Mutex
	rootSet bool
}

// Root returns the root hash of the reconstructed view.
func (r *Reconstructed) Root() Hash {
	r.rootMu.Lock()
	if r.rootSet {
		root := r.root
		r.rootMu.Unlock()
		return root
	}
	r.rootMu.Unlock()

	r.keepAliveHandle.mu.RLock()
	if r.ptr == nil {
		r.keepAliveHandle.mu.RUnlock()
		return EmptyRoot
	}
	result := C.fwd_reconstructed_root_hash(r.ptr)
	r.keepAliveHandle.mu.RUnlock()

	root, err := getHashKeyFromHashResult(result)
	if err != nil {
		return EmptyRoot
	}

	r.rootMu.Lock()
	if !r.rootSet {
		r.root = root
		r.rootSet = true
	}
	root = r.root
	r.rootMu.Unlock()

	return root
}

// Get retrieves the value for the given key in this reconstructed view.
func (r *Reconstructed) Get(key []byte) ([]byte, error) {
	r.keepAliveHandle.mu.RLock()
	defer r.keepAliveHandle.mu.RUnlock()

	if r.dropped {
		return nil, ErrDroppedReconstructed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	return getValueFromValueResult(C.fwd_get_from_reconstructed(
		r.ptr,
		newBorrowedBytes(key, &pinner),
	))
}

// Iter creates an iterator over the reconstructed view.
func (r *Reconstructed) Iter(key []byte) (*Iterator, error) {
	r.keepAliveHandle.mu.RLock()
	defer r.keepAliveHandle.mu.RUnlock()

	if r.dropped {
		return nil, ErrDroppedReconstructed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	itResult := C.fwd_iter_on_reconstructed(r.ptr, newBorrowedBytes(key, &pinner))
	return getIteratorFromIteratorResult(itResult)
}

// Reconstruct applies a new batch on top of this reconstructed view.
//
// On success, the receiver is updated to point at the newly reconstructed view.
// On error, the receiver is no longer usable.
func (r *Reconstructed) Reconstruct(batch []BatchOp) error {
	r.keepAliveHandle.mu.Lock()
	defer r.keepAliveHandle.mu.Unlock()

	if r.dropped {
		return ErrDroppedReconstructed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()
	kvp := newKeyValuePairsFromBatch(batch, &pinner)

	result := C.fwd_reconstruct_on_reconstructed(r.ptr, kvp)
	// The old handle is consumed by the FFI call regardless of outcome.
	r.ptr = nil

	newHandle, err := getReconstructedHandleFromResult(result, nil)
	if err != nil {
		r.dropped = true
		return err
	}

	r.ptr = newHandle

	// Safe to lock rootMu here: Root() takes rootMu then keepAliveHandle.mu.RLock(),
	// so it cannot be between rootMu.Unlock() and keepAliveHandle.mu.RLock() while
	// we hold the write lock.
	r.rootMu.Lock()
	r.root = EmptyRoot
	r.rootSet = false
	r.rootMu.Unlock()

	return nil
}

// Dump returns a DOT (Graphviz) representation of this reconstructed view.
func (r *Reconstructed) Dump() (string, error) {
	r.keepAliveHandle.mu.RLock()
	defer r.keepAliveHandle.mu.RUnlock()

	if r.dropped {
		return "", ErrDroppedReconstructed
	}

	bytes, err := getValueFromValueResult(C.fwd_reconstructed_dump(r.ptr))
	if err != nil {
		return "", err
	}

	return string(bytes), nil
}

func getReconstructedFromResult(result C.ReconstructedResult, wg *sync.WaitGroup) (*Reconstructed, error) {
	switch result.tag {
	case C.ReconstructedResult_NullHandlePointer:
		return nil, errDBClosed
	case C.ReconstructedResult_Ok:
		body := (*C.ReconstructedResult_Ok_Body)(unsafe.Pointer(&result.anon0))
		reconstructed := &Reconstructed{
			handle: createHandle(body.handle, wg, func(h *C.ReconstructedHandle) C.VoidResult {
				return C.fwd_free_reconstructed(h)
			}),
			root: EmptyRoot,
		}
		runtime.AddCleanup(reconstructed, drop[*C.ReconstructedHandle], reconstructed.handle)
		return reconstructed, nil
	case C.ReconstructedResult_Err:
		err := newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
		return nil, err
	default:
		return nil, fmt.Errorf("unknown C.ReconstructedResult tag: %d", result.tag)
	}
}

func getReconstructedHandleFromResult(
	result C.ReconstructedResult,
	_ *sync.WaitGroup,
) (*C.ReconstructedHandle, error) {
	switch result.tag {
	case C.ReconstructedResult_NullHandlePointer:
		return nil, errDBClosed
	case C.ReconstructedResult_Ok:
		body := (*C.ReconstructedResult_Ok_Body)(unsafe.Pointer(&result.anon0))
		return body.handle, nil
	case C.ReconstructedResult_Err:
		err := newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
		return nil, err
	default:
		return nil, fmt.Errorf("unknown C.ReconstructedResult tag: %d", result.tag)
	}
}
