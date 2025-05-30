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

// intoHashAndId converts a Value to a 32 byte slice and an id.
// This should only be called when the `Value` is expected to only contain an error or
// an ID and a hash, otherwise the behavior is undefined.
//         data    | len   | meaning
// 1.    | nil     | 0     | invalid
// 2.    | nil     | non-0 | proposal deleted everything
// 3.    | non-nil | 0     | error string
// 4.    | non-nil | non-0 | hash and id
// TODO: case 2 should be invalid as well, since the proposal should be returning
// the nil hash, not None.

func (v *Value) intoHashAndId() ([]byte, uint32, error) {
	defer runtime.KeepAlive(v)

	if v.V == nil {
		return nil, 0, errNilBuffer
	}

	if v.V.data != nil {
		if v.V.len != 0 {
			// Case 4 - valid bytes returned
			id := v.id()
			v.V.len = C.size_t(RootLength)
			return v.bytes(), id, nil
		}

		// Case 3 - error string returned
		return nil, 0, v.error()
	}

	// Case 2 - proposal deleted everything
	if v.id() > 0 {
		return nil, v.id(), nil
	}

	// Case 1 - invalid value
	return nil, 0, errBadValue
}

// Value is a wrapper around the cgo `Value` struct.
// It is used so that the conversions are methods on the Value struct.
type Value struct {
	V *C.struct_Value
}

func (v *Value) intoError() error {
	defer runtime.KeepAlive(v)

	if v.V == nil {
		return errNilBuffer
	}

	if v.V.data == nil {
		return nil
	}

	return v.error()
}

// error returns the error string from the cgo `Value` payload.
// This avoids code duplication
// input must be validated -- v not nil and v.V.data must be non-nil.
func (v *Value) error() error {
	errStr := C.GoString((*C.char)(unsafe.Pointer(v.V.data)))
	C.fwd_free_value(v.V)
	return fmt.Errorf("firewood error: %s", errStr)
}

// bytes returns the byte slice from the cgo `Value` payload.
// This avoids code duplication
// input must be validated -- v not nil and v.V.data must be non-nil, and v.V.len must be non-zero.
func (v *Value) bytes() []byte {
	// This assertion should have been done in the caller
	// if v.V.data == nil || v.V.len == 0 {
	// 	panic("firewood error: invalid value")
	// }

	buf := C.GoBytes(unsafe.Pointer(v.V.data), C.int(v.V.len))
	C.fwd_free_value(v.V)
	return buf
}

func (v *Value) id() uint32 {
	return uint32(v.V.len)
}

// intoBytes converts a Value to a byte or an error
// based on the following table:
//
//	| data    | len   | meaning
//
// 1.    | nil     | 0     | empty
// 2.    | nil     | non-0 | invalid
// 3.    | non-nil | 0     | error string
// 4.    | non-nil | non-0 | bytes (most common)
func (v *Value) intoBytes() ([]byte, error) {
	defer runtime.KeepAlive(v)

	if v.V == nil {
		return nil, errNilBuffer
	}

	// Case 4 - valid bytes returned
	if v.V.len != 0 && v.V.data != nil {
		return v.bytes(), nil
	}

	// Case 1 - empty value
	if v.V.len == 0 && v.V.data == nil {
		return nil, nil
	}

	// Case 3 - error string returned
	if v.V.data != nil {
		return nil, v.error()
	}

	// Case 2 - invalid value
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
