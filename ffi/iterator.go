// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
import "C"

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

	// batchSize is the number of items that are loaded at once from ffi
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
}

func (it *Iterator) nextInternal() error {
	if len(it.loadedPairs) == 0 {
		if it.batchSize < 1 {
			kv, e := getKeyValueFromKeyValueResult(C.fwd_iter_next(it.handle))
			if e != nil {
				return e
			}
			if kv != nil {
				// kv is nil when done
				it.loadedPairs = append(it.loadedPairs, kv)
			}
		} else {
			batch, e := getKeyValueBatchFromKeyValueBatchResult(C.fwd_iter_next_n(it.handle, C.size_t(it.batchSize)))
			if e != nil {
				return e
			}
			it.loadedPairs = batch.Copied()
			if e = batch.Free(); e != nil {
				return e
			}
		}
	}
	if len(it.loadedPairs) > 0 {
		it.currentPair, it.loadedPairs = it.loadedPairs[0], it.loadedPairs[1:]
	} else {
		it.currentPair = nil
	}
	return nil
}

func (it *Iterator) SetBatchSize(batchSize int) {
	it.batchSize = batchSize
}

func (it *Iterator) Next() bool {
	it.err = it.nextInternal()
	if it.currentPair == nil || it.err != nil {
		return false
	}
	k, v, e := it.currentPair.Consume()
	it.currentKey = k
	it.currentValue = v
	it.err = e
	return e == nil
}

func (it *Iterator) Key() []byte {
	if it.currentPair == nil || it.err != nil {
		return nil
	}
	return it.currentKey
}

func (it *Iterator) Value() []byte {
	if it.currentPair == nil || it.err != nil {
		return nil
	}
	return it.currentValue
}

func (it *Iterator) Err() error {
	return it.err
}
