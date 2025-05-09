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
	"runtime"
	"unsafe"
)

var errNilValue = errors.New("firewood error: nil value returned from cgo")

// KeyValue is a key-value pair.
type KeyValue struct {
	Key   []byte
	Value []byte
}

// extractValueThenFree extracts the value from a cgo `Value` struct and frees
// the memory allocated for it. The caller must ensure that the returned value
// is not used after this function returns. The caller must also ensure that
// the `Value` returned is interpreted correctly - i.e. the format matches the
// expected information in the documentation.
func extractValueThenFree(v *C.struct_Value) (uint32, []byte, error) {
	// Pin the returned value to prevent it from being garbage collected.
	defer runtime.KeepAlive(v)
	if v == nil {
		return 0, nil, errNilValue
	}
	if v.data == nil {
		return uint32(v.len), nil, nil
	}

	if v.len == 0 {
		errStr := C.GoString((*C.char)(unsafe.Pointer(v.data)))
		C.fwd_free_value(v)
		return 0, nil, errors.New(errStr)
	}

	buf := C.GoBytes(unsafe.Pointer(v.data), C.int(v.len))
	C.fwd_free_value(v)

	return 0, buf, nil
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

// WARNING!
// The following function and type should NOT be used in production code.
// They are only used for testing and debugging purposes only.
type testValue = C.struct_Value

func newCValueLen(len int) *testValue {
	return &C.struct_Value{
		len:  C.size_t(len),
		data: nil,
	}
}

func newCValueData(len int, data string) *testValue {
	// convert to C string
	cstr := C.CString(data)
	defer C.free(unsafe.Pointer(cstr))
	val := C.fwd_alloc_test(cstr)
	val.len = C.size_t(len)
	return &val
}
