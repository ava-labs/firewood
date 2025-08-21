// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// Package ffi provides a Go wrapper around the [Firewood] database.
//
// [Firewood]: https://github.com/ava-labs/firewood
package ffi

// // Note that -lm is required on Linux but not on Mac.
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
	errNilStruct     = errors.New("nil struct pointer cannot be freed")
	errBadValue      = errors.New("value from cgo formatted incorrectly")
	errKeysAndValues = errors.New("keys and values must have the same length")
)

type Pinner interface {
	Pin(ptr any)
	Unpin()
}

// newBorrowedBytes creates a new BorrowedBytes from a Go byte slice.
//
// Provide a Pinner to ensure the memory is pinned while the BorrowedBytes is in use.
func newBorrowedBytes(slice []byte, pinner Pinner) C.BorrowedBytes {
	sliceLen := len(slice)
	if sliceLen == 0 {
		return C.BorrowedBytes{ptr: nil, len: 0}
	}

	ptr := unsafe.SliceData(slice)
	if ptr == nil {
		return C.BorrowedBytes{ptr: nil, len: 0}
	}

	pinner.Pin(ptr)

	return C.BorrowedBytes{
		ptr: (*C.uint8_t)(ptr),
		len: C.size_t(sliceLen),
	}
}

// newKeyValuePair creates a new KeyValuePair from Go byte slices for key and value.
//
// Provide a Pinner to ensure the memory is pinned while the KeyValuePair is in use.
func newKeyValuePair(key, value []byte, pinner Pinner) C.KeyValuePair {
	return C.KeyValuePair{
		key:   newBorrowedBytes(key, pinner),
		value: newBorrowedBytes(value, pinner),
	}
}

// newBorrowedKeyValuePairs creates a new BorrowedKeyValuePairs from a slice of KeyValuePair.
//
// Provide a Pinner to ensure the memory is pinned while the BorrowedKeyValuePairs is
// in use.
func newBorrowedKeyValuePairs(pairs []C.KeyValuePair, pinner Pinner) C.BorrowedKeyValuePairs {
	sliceLen := len(pairs)
	if sliceLen == 0 {
		return C.BorrowedKeyValuePairs{ptr: nil, len: 0}
	}

	ptr := unsafe.SliceData(pairs)
	if ptr == nil {
		return C.BorrowedKeyValuePairs{ptr: nil, len: 0}
	}

	pinner.Pin(ptr)

	return C.BorrowedKeyValuePairs{
		ptr: ptr,
		len: C.size_t(sliceLen),
	}
}

// newKeyValuePairs creates a new BorrowedKeyValuePairs from slices of keys and values.
//
// The keys and values must have the same length.
//
// Provide a Pinner to ensure the memory is pinned while the BorrowedKeyValuePairs is
// in use.
func newKeyValuePairs(keys, vals [][]byte, pinner Pinner) (C.BorrowedKeyValuePairs, error) {
	if len(keys) != len(vals) {
		return C.BorrowedKeyValuePairs{}, fmt.Errorf("%w: %d != %d", errKeysAndValues, len(keys), len(vals))
	}

	pairs := make([]C.KeyValuePair, len(keys))
	for i := range keys {
		pairs[i] = newKeyValuePair(keys[i], vals[i], pinner)
	}

	return newBorrowedKeyValuePairs(pairs, pinner), nil
}

// Close releases the memory associated with the Database.
//
// This is safe to call if the pointer is nil, in which case it does nothing. The
// pointer will be set to nil after freeing to prevent double free.
func (db *Database) Close() error {
	if db == nil {
		return nil
	}

	ptr := db.handle
	if ptr == nil {
		return nil
	}

	// Clear the handle before freeing to prevent double free
	db.handle = nil

	if err := fromVoidResult(C.fwd_close_db(ptr)); err != nil {
		return fmt.Errorf("unexpected error when closing database: %w", err)
	}

	return nil
}

// RustOwnedBytes is a wrapper around C.OwnedBytes that provides a Go interface
// for Rust-owned byte slices.
type RustOwnedBytes struct {
	owned C.OwnedBytes
}

// Free releases the memory associated with the RustOwnedBytes.
//
// It is safe to call this method multiple times, and it will do nothing if the
// RustOwnedBytes is nil or has already been freed.
func (b *RustOwnedBytes) Free() error {
	if b == nil {
		return nil
	}

	this := *b               // copy so we can reset before freeing,
	b.owned = C.OwnedBytes{} // reset to clear the pointer

	if this.owned.ptr == nil {
		return nil
	}

	if err := fromVoidResult(C.fwd_free_owned_bytes(this.owned)); err != nil {
		return fmt.Errorf("unexpected error when freeing owned bytes: %w", err)
	}

	return nil
}

// BorrowedBytes returns the underlying byte slice. It may return nil if the
// data has already been freed was never set.
//
// The returned slice is valid only as long as the RustOwnedBytes is valid.
//
// It does not copy the data; however, the slice is valid only as long as the
// RustOwnedBytes is valid. If the RustOwnedBytes is freed, the slice will
// become invalid.
func (b *RustOwnedBytes) BorrowedBytes() []byte {
	if b == nil {
		return nil
	}

	if b.owned.ptr == nil {
		return nil
	}

	return unsafe.Slice((*byte)(b.owned.ptr), b.owned.len)
}

// intoError converts the RustOwnedBytes into an error. This is used for methods
// that return a RustOwnedBytes as an error type.
//
// If the RustOwnedBytes is nil or has already been freed, it returns nil.
// Otherwise, the bytes will be copied into Go memory and converted into an
// error. The original RustOwnedBytes will be freed after this operation.
func (b *RustOwnedBytes) intoError() error {
	if b == nil {
		return nil
	}

	if b.owned.ptr == nil {
		return nil
	}

	// Convert the owned bytes to a string and create an error from it.
	errMsg := string(b.CopiedBytes())

	if err := b.Free(); err != nil {
		return fmt.Errorf("failed to free owned bytes: %w while handling original error %s", err, errMsg)
	}

	return errors.New(errMsg)
}

// CopiedBytes returns a copy of the underlying byte slice. It may return nil if the
// data has already been freed or was never set.
//
// The returned slice is a copy of the data and is valid independently of the
// RustOwnedBytes. It is safe to use after the RustOwnedBytes is freed and will
// be freed by the Go garbage collector.
func (b *RustOwnedBytes) CopiedBytes() []byte {
	if b == nil {
		return nil
	}

	if b.owned.ptr == nil {
		return nil
	}

	return C.GoBytes(unsafe.Pointer(b.owned.ptr), C.int(b.owned.len))
}

// fromOwnedBytes creates a RustOwnedBytes from a C.OwnedBytes.
//
// The caller is responsible for calling Free() on the returned RustOwnedBytes
// when it is no longer needed otherwise memory will leak.
func fromOwnedBytes(owned C.OwnedBytes) *RustOwnedBytes {
	if owned.ptr == nil {
		return nil
	}

	rustBytes := &RustOwnedBytes{owned: owned}

	return rustBytes
}

// fromHashResult creates a byte slice or error from a C.HashResult.
//
// It returns nil, nil if the result is None.
// It returns nil, err if the result is an error.
// It returns a byte slice, nil if the result is Some.
func fromHashResult(result C.HashResult) ([]byte, error) {
	switch result.tag {
	case C.HashResult_NullHandlePointer:
		return nil, errDBClosed
	case C.HashResult_None:
		return nil, nil
	case C.HashResult_Some:
		// copy the bytes from the C.HashResult to a fixed-size array
		hashKey := fromHashKey((*C.HashKey)(unsafe.Pointer(&result.anon0)))
		return hashKey, nil
	case C.HashResult_Err:
		ownedBytes := fromOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0)))
		return nil, ownedBytes.intoError()
	default:
		return nil, fmt.Errorf("unknown C.HashResult tag: %d", result.tag)
	}
}

// fromVoidResult converts a C.VoidResult to an error.
//
// It return nil if the result is Ok, otherwise it return an error.
func fromVoidResult(result C.VoidResult) error {
	switch result.tag {
	case C.VoidResult_NullHandlePointer:
		return errDBClosed
	case C.VoidResult_Ok:
		return nil
	case C.VoidResult_Err:
		return fromOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
	default:
		return fmt.Errorf("unknown C.VoidResult tag: %d", result.tag)
	}
}

// fromHandleResult converts a C.HandleResult to a Database or error.
//
// It sets a finalizer to free the memory when the Database is no longer
// referenced.
//
// If the C.HandleResult is an error, it returns an error instead of a Database.
func fromHandleResult(result C.HandleResult) (*Database, error) {
	switch result.tag {
	case C.HandleResult_Ok:
		ptr := *(**C.DatabaseHandle)(unsafe.Pointer(&result.anon0))
		db := &Database{
			handle: ptr,
		}
		return db, nil
	case C.HandleResult_Err:
		ownedBytes := fromOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0)))
		return nil, ownedBytes.intoError()
	default:
		return nil, fmt.Errorf("unknown C.HandleResult tag: %d", result.tag)
	}
}

// hashAndIDFromValue converts the cgo `Value` payload into:
//
//	case | data    | len   | meaning
//
// 1.    | nil     | 0     | invalid
// 2.    | nil     | non-0 | proposal deleted everything
// 3.    | non-nil | 0     | error string
// 4.    | non-nil | non-0 | hash and id
//
// The value should never be nil.
func hashAndIDFromValue(v *C.struct_Value) ([]byte, uint32, error) {
	// Pin the returned value to prevent it from being garbage collected.
	defer runtime.KeepAlive(v)

	if v == nil {
		return nil, 0, errNilStruct
	}

	if v.data == nil {
		// Case 2
		if v.len != 0 {
			return nil, uint32(v.len), nil
		}

		// Case 1
		return nil, 0, errBadValue
	}

	// Case 3
	if v.len == 0 {
		errStr := C.GoString((*C.char)(unsafe.Pointer(v.data)))
		if err := fromVoidResult(C.fwd_free_value(v)); err != nil {
			return nil, 0, fmt.Errorf("unexpected error while freeing value: %w", err)
		}
		return nil, 0, errors.New(errStr)
	}

	// Case 4
	id := uint32(v.len)
	buf := C.GoBytes(unsafe.Pointer(v.data), RootLength)
	v.len = C.size_t(RootLength) // set the length to free
	if err := fromVoidResult(C.fwd_free_value(v)); err != nil {
		return nil, 0, fmt.Errorf("unexpected error while freeing value: %w", err)
	}
	return buf, id, nil
}

// errorFromValue converts the cgo `Value` payload into:
//
//	case | data    | len   | meaning
//
// 1.    | nil     | 0     | empty
// 2.    | nil     | non-0 | invalid
// 3.    | non-nil | 0     | error string
// 4.    | non-nil | non-0 | invalid
//
// The value should never be nil.
func errorFromValue(v *C.struct_Value) error {
	// Pin the returned value to prevent it from being garbage collected.
	defer runtime.KeepAlive(v)

	if v == nil {
		return errNilStruct
	}

	// Case 1
	if v.data == nil && v.len == 0 {
		return nil
	}

	// Case 3
	if v.len == 0 {
		errStr := C.GoString((*C.char)(unsafe.Pointer(v.data)))
		if err := fromVoidResult(C.fwd_free_value(v)); err != nil {
			return fmt.Errorf("unexpected error while freeing value: %w", err)
		}
		return errors.New(errStr)
	}

	// Case 2 and 4
	if err := fromVoidResult(C.fwd_free_value(v)); err != nil {
		return fmt.Errorf("unexpected error while freeing value: %w", err)
	}
	return errBadValue
}

// bytesFromValue converts the cgo `Value` payload to:
//
//	case | data    | len   | meaning
//
// 1.    | nil     | 0     | empty
// 2.    | nil     | non-0 | invalid
// 3.    | non-nil | 0     | error string
// 4.    | non-nil | non-0 | bytes (most common)
//
// The value should never be nil.
func bytesFromValue(v *C.struct_Value) ([]byte, error) {
	// Pin the returned value to prevent it from being garbage collected.
	defer runtime.KeepAlive(v)

	if v == nil {
		return nil, errNilStruct
	}

	// Case 4
	if v.len != 0 && v.data != nil {
		buf := C.GoBytes(unsafe.Pointer(v.data), C.int(v.len))
		if err := fromVoidResult(C.fwd_free_value(v)); err != nil {
			return nil, fmt.Errorf("unexpected error while freeing value: %w", err)
		}
		return buf, nil
	}

	// Case 1
	if v.len == 0 && v.data == nil {
		return nil, nil
	}

	// Case 3
	if v.len == 0 {
		errStr := C.GoString((*C.char)(unsafe.Pointer(v.data)))
		if err := fromVoidResult(C.fwd_free_value(v)); err != nil {
			return nil, fmt.Errorf("unexpected error while freeing value: %w", err)
		}
		return nil, errors.New(errStr)
	}

	// Case 2
	return nil, errBadValue
}

func fromHashKey(key *C.HashKey) []byte {
	if key == nil {
		return nil
	}
	hashKey := make([]byte, 32)
	copy(hashKey, unsafe.Slice((*byte)(unsafe.Pointer(key)), 32))
	return hashKey
}
