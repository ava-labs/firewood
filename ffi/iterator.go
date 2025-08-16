// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// Package ffi provides a Go wrapper around the [Firewood] database.
//
// [Firewood]: https://github.com/ava-labs/firewood
package ffi

// #include <stdlib.h>
// #include "firewood.h"
import "C"

type DbIterator struct {
	dbHandle   *C.DatabaseHandle
	id         uint32
	currentKey []byte
	currentVal []byte
	err        error
}

// newIterator creates a new DbIterator from the given DatabaseHandle and Value.
// The Value must be returned from a Firewood FFI function.
// An error can only occur from parsing the Value.
func newIterator(handle *C.DatabaseHandle, val *C.struct_Value) (*DbIterator, error) {
	id, err := u32FromValue(val)
	if err != nil {
		return nil, err
	}

	return &DbIterator{
		dbHandle:   handle,
		id:         id,
		currentKey: nil,
		currentVal: nil,
		err:        nil,
	}, nil
}

func (it *DbIterator) Next() bool {
	v := C.fwd_iter_next(it.dbHandle, C.uint32_t(it.id))
	key, value, e := keyValueFromValue(&v)
	it.currentKey = key
	it.currentVal = value
	it.err = e
	if (key == nil && value == nil) || e != nil {
		return false
	}
	return true
}

func (it *DbIterator) Key() []byte {
	if (it.currentKey == nil && it.currentVal == nil) || it.err != nil {
		return nil
	}
	return it.currentKey
}

func (it *DbIterator) Value() []byte {
	if (it.currentKey == nil && it.currentVal == nil) || it.err != nil {
		return nil
	}
	return it.currentVal
}

func (it *DbIterator) Err() error {
	return it.err
}
