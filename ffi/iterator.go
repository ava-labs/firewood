// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
import "C"
import "errors"

type Iterator struct {
	// The database this iterator is associated with. We hold onto this to ensure
	// the database handle outlives the iterator handle, which is required for
	// the iterator to be valid.
	db *Database

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

	// currentPair is the current pair retrieved from the iterator
	currentPair *ownedKeyValue

	// currentKey is the current pair retrieved from the iterator
	currentKey []byte

	// currentValue is the current pair retrieved from the iterator
	currentValue []byte

	// err is the error from the iterator, if any
	err error

	// currentResource is a reference to a freeable resource to clean up
	currentResource interface{ Free() error }
}

func (it *Iterator) freeCurrentAllocation() error {
	if it.currentResource == nil {
		return nil
	}
	return it.currentResource.Free()
}

func (it *Iterator) nextInternal() error {
	if len(it.loadedPairs) == 0 {
		if e := it.freeCurrentAllocation(); e != nil {
			return e
		}
		if it.batchSize <= 1 {
			kv, e := getKeyValueFromKeyValueResult(C.fwd_iter_next(it.handle))
			if e != nil {
				return e
			}
			if kv != nil {
				// kv is nil when done
				it.loadedPairs = append(it.loadedPairs, kv)
			}
			it.currentResource = kv
		} else {
			batch, e := getKeyValueBatchFromKeyValueBatchResult(C.fwd_iter_next_n(it.handle, C.size_t(it.batchSize)))
			if e != nil {
				return e
			}
			it.loadedPairs = batch.Copied()
			it.currentResource = batch
		}
	}
	if len(it.loadedPairs) > 0 {
		it.currentPair, it.loadedPairs = it.loadedPairs[0], it.loadedPairs[1:]
	} else {
		it.currentPair = nil
	}
	return nil
}

// SetBatchSize sets the max number of pairs to be retrieved in one ffi call.
func (it *Iterator) SetBatchSize(batchSize int) {
	it.batchSize = batchSize
}

// Next proceeds to the next item on the iterator, and returns true
// if succeeded and there is a pair available.
// The new pair could be retrieved with Key and Value methods.
func (it *Iterator) Next() bool {
	it.err = it.nextInternal()
	if it.currentPair == nil || it.err != nil {
		return false
	}
	k, v := it.currentPair.Copy()
	it.currentKey = k
	it.currentValue = v
	return true
}

// NextBorrowed retrieves the next item on the iterator similar to Next
// the difference is that returned bytes in Key and Value are not copied
// and will be freed on next call to Next or NextBorrowed
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

// Key returns the key of the current pair
func (it *Iterator) Key() []byte {
	if it.currentPair == nil || it.err != nil {
		return nil
	}
	return it.currentKey
}

// Value returns the value of the current pair
func (it *Iterator) Value() []byte {
	if it.currentPair == nil || it.err != nil {
		return nil
	}
	return it.currentValue
}

// Err returns the error if Next failed
func (it *Iterator) Err() error {
	return it.err
}

// Drop drops the iterator and releases the resources
func (it *Iterator) Drop() error {
	e1 := it.freeCurrentAllocation()
	if it.handle != nil {
		return errors.Join(
			e1,
			getErrorFromVoidResult(C.fwd_free_iterator(it.handle)))
	}
	return e1
}
