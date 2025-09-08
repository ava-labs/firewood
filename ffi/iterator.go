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

	// currentKey is the current key retrieved from the iterator
	currentKey []byte
	// currentVal is the current value retrieved from the iterator
	currentVal []byte
	// err is the error from the iterator, if any
	err error
}

// Next proceeds to the next item on the iterator, and returns true
// if succeeded and there is a pair available.
// The new pair could be retrieved with Key and Value methods.
func (it *Iterator) Next() bool {
	kv, e := getKeyValueFromKeyValueResult(C.fwd_iter_next(it.handle))
	it.err = e
	if kv == nil || e != nil {
		return false
	}
	k, v, e := kv.Consume()
	it.currentKey = k
	it.currentVal = v
	it.err = e
	return e == nil
}

// Key returns the key of the current pair
func (it *Iterator) Key() []byte {
	if (it.currentKey == nil && it.currentVal == nil) || it.err != nil {
		return nil
	}
	return it.currentKey
}

// Value returns the value of the current pair
func (it *Iterator) Value() []byte {
	if (it.currentKey == nil && it.currentVal == nil) || it.err != nil {
		return nil
	}
	return it.currentVal
}

// Err returns the error if Next failed
func (it *Iterator) Err() error {
	return it.err
}
