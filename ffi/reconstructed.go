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
// by calling [Reconstructed.Drop]. A finalizer is set to call Drop automatically
// if needed, but explicit calls are recommended.
type Reconstructed struct {
	handle  *C.ReconstructedHandle
	root    Hash
	rootMu  sync.Mutex
	rootSet bool

	keepAliveHandle databaseKeepAliveHandle
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
	if r.handle == nil {
		r.keepAliveHandle.mu.RUnlock()
		return EmptyRoot
	}
	result := C.fwd_reconstructed_root_hash(r.handle)
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

	if r.handle == nil {
		return nil, ErrDroppedReconstructed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	return getValueFromValueResult(C.fwd_get_from_reconstructed(
		r.handle,
		newBorrowedBytes(key, &pinner),
	))
}

// Iter creates an iterator over the reconstructed view.
func (r *Reconstructed) Iter(key []byte) (*Iterator, error) {
	r.keepAliveHandle.mu.RLock()
	defer r.keepAliveHandle.mu.RUnlock()

	if r.handle == nil {
		return nil, ErrDroppedReconstructed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	itResult := C.fwd_iter_on_reconstructed(r.handle, newBorrowedBytes(key, &pinner))
	return getIteratorFromIteratorResult(itResult)
}

// Reconstruct applies a new batch on top of this reconstructed view.
//
// On success, the receiver is updated to point at the newly reconstructed view.
// On error, the receiver is no longer usable.
func (r *Reconstructed) Reconstruct(batch []BatchOp) error {
	r.keepAliveHandle.mu.RLock()
	wg := r.keepAliveHandle.outstandingHandles
	r.keepAliveHandle.mu.RUnlock()

	var newHandle *C.ReconstructedHandle
	if err := r.keepAliveHandle.disown(true /* evenOnError */, func() error {
		if r.handle == nil {
			return ErrDroppedReconstructed
		}

		var pinner runtime.Pinner
		defer pinner.Unpin()
		kvp := newKeyValuePairsFromBatch(batch, &pinner)

		result := C.fwd_reconstruct_on_reconstructed(r.handle, kvp)
		r.handle = nil

		var err error
		newHandle, err = getReconstructedHandleFromResult(
			result,
			r.keepAliveHandle.outstandingHandles,
		)
		if err != nil {
			return err
		}
		return nil
	}); err != nil {
		return err
	}

	r.keepAliveHandle.mu.Lock()
	if wg != nil {
		wg.Add(1)
		r.keepAliveHandle.outstandingHandles = wg
	}
	// Guard handle swap under the keep-alive lock to avoid races with Get/Root.
	r.handle = newHandle
	r.keepAliveHandle.mu.Unlock()

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

	if r.handle == nil {
		return "", ErrDroppedReconstructed
	}

	bytes, err := getValueFromValueResult(C.fwd_reconstructed_dump(r.handle))
	if err != nil {
		return "", err
	}

	return string(bytes), nil
}

// Drop releases this reconstructed handle.
func (r *Reconstructed) Drop() error {
	return r.keepAliveHandle.disown(false /* evenOnError */, func() error {
		if r.handle == nil {
			return nil
		}

		if err := getErrorFromVoidResult(C.fwd_free_reconstructed(r.handle)); err != nil {
			return fmt.Errorf("%w: %w", errFreeingValue, err)
		}

		r.handle = nil
		return nil
	})
}

func getReconstructedFromResult(result C.ReconstructedResult, wg *sync.WaitGroup) (*Reconstructed, error) {
	switch result.tag {
	case C.ReconstructedResult_NullHandlePointer:
		return nil, errDBClosed
	case C.ReconstructedResult_Ok:
		body := (*C.ReconstructedResult_Ok_Body)(unsafe.Pointer(&result.anon0))
		reconstructed := &Reconstructed{
			handle: body.handle,
			root:   EmptyRoot,
		}
		reconstructed.keepAliveHandle.init(wg)
		runtime.SetFinalizer(reconstructed, (*Reconstructed).Drop)
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
