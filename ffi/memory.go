// Package firewood provides a Go wrapper around the [Firewood] database.
//
// [Firewood]: https://github.com/ava-labs/firewood
package firewood

// // Note that -lm is required on Linux but not on Mac.
// #cgo LDFLAGS: -L${SRCDIR}/../target/release -L/usr/local/lib -lfirewood_ffi -lm
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
	errNilBuffer = errors.New("firewood error: nil value returned from cgo")
	errBadValue  = errors.New("firewood error: value from cgo formatted incorrectly")
)

// KeyValue is a key-value pair.
type KeyValue struct {
	Key   []byte
	Value []byte
}

// extractBytesAndErrorThenFree converts the cgo `Value` payload into either:
// 1. a 32 byte slice, an id and nil error
// 2. a nil byte slice, 0 id, and a non-nil error
// This should only be called when the `Value` is expected to only contain an error or
// an ID and a hash, otherwise the behavior is undefined.
func extractBytesAndErrorThenFree(v *C.struct_Value) ([]byte, uint32, error) {
	// Pin the returned value to prevent it from being garbage collected.
	defer runtime.KeepAlive(v)

	if v == nil {
		return nil, 0, errNilBuffer
	}

	// Expected empty case for Rust's `()`
	// Ignores the length.
	if v.data == nil {
		return nil, uint32(v.len), nil
	}

	// If the value is an error string, it should be freed and an error
	// returned.
	if v.len == 0 {
		errStr := C.GoString((*C.char)(unsafe.Pointer(v.data)))
		C.fwd_free_value(v)
		return nil, 0, fmt.Errorf("firewood error: %s", errStr)
	}

	// We must assume that the byte slice is a valid root slice.
	id := uint32(v.len)
	buf := C.GoBytes(unsafe.Pointer(v.data), RootLength)
	v.len = C.size_t(RootLength) // set the length to clean
	C.fwd_free_value(v)
	return buf, id, nil
}

// extractErrorThenFree converts the cgo `Value` payload to either:
// 1. a nil value, indicating no error, or
// 2. a non-nil error, indicating an error occurred.
// This should only be called when the `Value` is expected to only contain an error.
// Otherwise, an error is returned.
func extractErrorThenFree(v *C.struct_Value) error {
	// Pin the returned value to prevent it from being garbage collected.
	defer runtime.KeepAlive(v)

	if v == nil {
		return errNilBuffer
	}

	// Expected empty case for Rust's `()`
	// Ignores the length.
	if v.data == nil {
		return nil
	}

	// If the value is an error string, it should be freed and an error
	// returned.
	if v.len == 0 {
		errStr := C.GoString((*C.char)(unsafe.Pointer(v.data)))
		C.fwd_free_value(v)
		return fmt.Errorf("firewood error: %s", errStr)
	}

	// The value is formatted incorrectly.
	// We should still attempt to free the value.
	C.fwd_free_value(v)
	return errBadValue
}

// extractBytesThenFree converts the cgo `Value` payload to either:
// 1. a non-nil byte slice and nil error, indicating a valid byte slice
// 2. a nil byte slice and nil error, indicating an empty byte slice
// 3. a nil byte slice and a non-nil error, indicating an error occurred.
// This should only be called when the `Value` is expected to only contain an error or a byte slice.
// Otherwise, an error is returned.
func extractBytesThenFree(v *C.struct_Value) ([]byte, error) {
	// Pin the returned value to prevent it from being garbage collected.
	defer runtime.KeepAlive(v)

	if v == nil {
		return nil, errNilBuffer
	}

	// Expected behavior - no data and length is zero.
	if v.len == 0 && v.data == nil {
		return nil, nil
	}

	// Normal case, data is non-nil and length is non-zero.
	if v.len != 0 && v.data != nil {
		buf := C.GoBytes(unsafe.Pointer(v.data), C.int(v.len))
		C.fwd_free_value(v)
		return buf, nil
	}

	// Data non-nil but length is zero indcates an error.
	if v.len == 0 {
		errStr := C.GoString((*C.char)(unsafe.Pointer(v.data)))
		C.fwd_free_value(v)
		return nil, fmt.Errorf("firewood error: %s", errStr)
	}

	// The value is formatted incorrectly.
	// We should still attempt to free the value.
	C.fwd_free_value(v)
	return nil, errBadValue
}

// newValueFactory returns a factory for converting byte slices into cgo `Value`
// structs that can be passed as arguments to cgo functions. The returned
// cleanup function MUST be called when the constructed values are no longer
// required, after which they can no longer be used as cgo arguments.
func newValueFactory() (*valueFactory, func()) {
	f := new(valueFactory)
	return f, func() { f.pin.Unpin() }
}

type valueFactory struct {
	pin runtime.Pinner
}

func (f *valueFactory) from(data []byte) C.struct_Value {
	if len(data) == 0 {
		return C.struct_Value{0, nil}
	}
	ptr := (*C.uchar)(unsafe.SliceData(data))
	f.pin.Pin(ptr)
	return C.struct_Value{C.size_t(len(data)), ptr}
}
