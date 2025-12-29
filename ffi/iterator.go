// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
import "C"

import (
	"errors"
	"fmt"
	"unsafe"
)

// Iterator provides sequential access to key-value pairs within a [Revision] or [Proposal].
// Iterators traverse the trie in lexicographic key order, starting from a specified key.
//
// Instances are created via [Revision.Iter] or [Proposal.Iter], and must be released
// with [Iterator.Drop] when no longer needed to free the underlying resources.
//
// An Iterator holds a reference to the underlying view (Revision or Proposal), preventing
// it from being garbage collected. This means the Iterator can safely outlive the
// Revision or Proposal it was created from, but the underlying state will not be
// released until the Iterator is dropped.
//
// Iterator supports two modes of accessing key-value pairs:
//   - [Iterator.Next]: Copies the key and value into Go-managed memory. Safer but slower.
//   - [Iterator.NextBorrowed]: Returns slices that borrow Rust-owned memory. Faster but
//     requires careful lifetime management - borrowed slices are only valid until the
//     next iteration or drop.
//
// Batching can be enabled via [Iterator.SetBatchSize] to reduce FFI call overhead
// when iterating over many items.
//
// All operations on an Iterator are NOT thread-safe. Do not use the same Iterator
// concurrently from multiple goroutines.
type Iterator struct {
	// handle is an opaque pointer to the iterator within Firewood. It should be
	// passed to the C FFI functions that operate on iterators
	//
	// It is not safe to call these methods with a nil handle.
	handle *C.IteratorHandle

	// batchSize is the number of items that are loaded at once
	// to reduce ffi call overheads
	batchSize int

	// loadedPairs is the latest loaded key value pairs retrieved
	// from the iterator, not yet consumed by user
	loadedPairs []*ownedKeyValue

	// current* fields correspond to the current cursor state
	// nil/empty if not started or exhausted; refreshed on each Next().
	currentPair  *ownedKeyValue
	currentKey   []byte
	currentValue []byte
	// FFI resource for current pair or batch to free on advance or drop
	currentResource interface{ free() error }

	// err is the error from the iterator, if any
	err error
}

func (it *Iterator) freeCurrentAllocation() error {
	if it.currentResource == nil {
		return nil
	}
	e := it.currentResource.free()
	it.currentResource = nil
	return e
}

func (it *Iterator) nextInternal() error {
	if len(it.loadedPairs) > 0 {
		it.currentPair, it.loadedPairs = it.loadedPairs[0], it.loadedPairs[1:]
		return nil
	}

	// current resources should **only** be freed, on the next call to the FFI
	// this is to make sure we don't invalidate a batch in between iteration
	if e := it.freeCurrentAllocation(); e != nil {
		return e
	}
	if it.batchSize <= 1 {
		kv, e := getKeyValueFromResult(C.fwd_iter_next(it.handle))
		if e != nil {
			return e
		}
		it.currentPair = kv
		it.currentResource = kv
	} else {
		batch, e := getKeyValueBatchFromResult(C.fwd_iter_next_n(it.handle, C.size_t(it.batchSize)))
		if e != nil {
			return e
		}
		pairs := batch.copy()
		if len(pairs) > 0 {
			it.currentPair, it.loadedPairs = pairs[0], pairs[1:]
		} else {
			it.currentPair = nil
		}
		it.currentResource = batch
	}

	return nil
}

// SetBatchSize configures the number of key-value pairs to retrieve per FFI call.
// Setting a batch size greater than 1 reduces FFI overhead by fetching multiple
// pairs in a single call, which can significantly improve iteration performance
// over large datasets.
//
// A batch size of 0 or 1 disables batching, fetching one pair per FFI call.
// The default batch size is 0 (no batching).
//
// This method can be called at any point during iteration; the new batch size
// takes effect on the next FFI call.
func (it *Iterator) SetBatchSize(batchSize int) {
	it.batchSize = batchSize
}

// Next advances the iterator to the next key-value pair and returns true if
// a pair is available. After Next returns true, call [Iterator.Key] and
// [Iterator.Value] to retrieve the current pair.
//
// Next copies the key and value into Go-managed memory, making them safe to
// retain after subsequent Next calls or after the iterator is dropped. For
// better performance when you don't need to retain the data, use [Iterator.NextBorrowed].
//
// Returns false when the iterator is exhausted or an error occurs. After Next
// returns false, call [Iterator.Err] to check for errors. It is safe to call
// Next again after it returns false; it will continue to return false.
//
// Example usage:
//
//	it, err := rev.Iter(nil) // start from beginning
//	if err != nil { ... }
//	defer it.Drop()
//	for it.Next() {
//	    fmt.Printf("key=%x value=%x\n", it.Key(), it.Value())
//	}
//	if err := it.Err(); err != nil { ... }
func (it *Iterator) Next() bool {
	it.err = it.nextInternal()
	if it.currentPair == nil || it.err != nil {
		return false
	}
	k, v := it.currentPair.copy()
	it.currentKey = k
	it.currentValue = v
	return true
}

// NextBorrowed advances the iterator like [Iterator.Next], but returns slices that
// borrow Rust-owned memory instead of copying into Go-managed memory. This is
// faster than Next but requires careful lifetime management.
//
// WARNING: The slices returned by [Iterator.Key] and [Iterator.Value] after
// NextBorrowed are only valid until the next call to Next, NextBorrowed, or
// [Iterator.Drop]. They alias FFI-owned memory that will be freed or reused.
//
// Do NOT:
//   - Retain or store these slices beyond the current iteration
//   - Modify the contents of these slices
//   - Use these slices after calling Next, NextBorrowed, or Drop
//
// Misuse can cause reads from freed memory and undefined behavior.
// If you need to retain the data, use [Iterator.Next] instead or copy the slices.
//
// Returns false when the iterator is exhausted or an error occurs, same as Next.
func (it *Iterator) NextBorrowed() bool {
	it.err = it.nextInternal()
	if it.currentPair == nil || it.err != nil {
		return false
	}
	it.currentKey = it.currentPair.key.BorrowedBytes()
	it.currentValue = it.currentPair.value.BorrowedBytes()
	it.err = nil
	return true
}

// Key returns the key of the current key-value pair, or nil if the iterator
// has not been advanced or is exhausted.
//
// The lifetime of the returned slice depends on how the iterator was advanced:
//   - After [Iterator.Next]: The slice is a copy and safe to retain.
//   - After [Iterator.NextBorrowed]: The slice borrows Rust memory and is only
//     valid until the next advance or drop. See NextBorrowed for details.
func (it *Iterator) Key() []byte {
	if it.currentPair == nil || it.err != nil {
		return nil
	}
	return it.currentKey
}

// Value returns the value of the current key-value pair, or nil if the iterator
// has not been advanced or is exhausted.
//
// The lifetime of the returned slice depends on how the iterator was advanced:
//   - After [Iterator.Next]: The slice is a copy and safe to retain.
//   - After [Iterator.NextBorrowed]: The slice borrows Rust memory and is only
//     valid until the next advance or drop. See NextBorrowed for details.
func (it *Iterator) Value() []byte {
	if it.currentPair == nil || it.err != nil {
		return nil
	}
	return it.currentValue
}

// Err returns the error encountered during the last [Iterator.Next] or
// [Iterator.NextBorrowed] call, or nil if no error occurred.
//
// Always check Err after the iteration loop completes (when Next returns false)
// to distinguish between normal exhaustion and an error condition.
func (it *Iterator) Err() error {
	return it.err
}

// Drop releases all resources associated with the iterator, including the
// reference to the underlying [Revision] or [Proposal] view.
//
// After Drop is called, the iterator must not be used. Any slices previously
// obtained via [Iterator.NextBorrowed] become invalid and must not be accessed.
//
// Drop should always be called when done with an iterator, typically via defer:
//
//	it, err := rev.Iter(nil)
//	if err != nil { ... }
//	defer it.Drop()
//
// It is safe to call Drop multiple times; subsequent calls are no-ops.
func (it *Iterator) Drop() error {
	err := it.freeCurrentAllocation()
	if it.handle != nil {
		// Always free the iterator even if releasing the current KV/batch failed.
		// The iterator holds a NodeStore ref that must be dropped.
		return errors.Join(
			err,
			getErrorFromVoidResult(C.fwd_free_iterator(it.handle)))
	}
	return err
}

// getIteratorFromIteratorResult converts a C.IteratorResult to an Iterator or error.
func getIteratorFromIteratorResult(result C.IteratorResult) (*Iterator, error) {
	switch result.tag {
	case C.IteratorResult_NullHandlePointer:
		return nil, errDBClosed
	case C.IteratorResult_Ok:
		body := (*C.IteratorResult_Ok_Body)(unsafe.Pointer(&result.anon0))
		proposal := &Iterator{
			handle: body.handle,
		}
		return proposal, nil
	case C.IteratorResult_Err:
		err := newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
		return nil, err
	default:
		return nil, fmt.Errorf("unknown C.IteratorResult tag: %d", result.tag)
	}
}
