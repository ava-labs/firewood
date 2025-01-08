package firewood

// #cgo LDFLAGS: -L../target/release -L/usr/local/lib -lfirewood_ffi
// #include "firewood.h"
// #include <stdlib.h>
import "C"
import (
	"runtime"
	"unsafe"
)

type Firewood struct {
}

const (
	OP_PUT = iota
	OP_DELETE
)

type KeyValue struct {
	Key   []byte
	Value []byte
}

func (f *Firewood) Batch(ops []KeyValue) {
	var pin runtime.Pinner
	defer pin.Unpin()

	ffi_ops := make([]C.struct_KeyValue, len(ops))
	for i, op := range ops {
		ffi_ops[i] = C.struct_KeyValue{
			key:   make_value(&pin, op.Key),
			value: make_value(&pin, op.Value),
		}
	}
	ptr := (*C.struct_KeyValue)(unsafe.Pointer(&ffi_ops[0]))
	C.batch(C.size_t(len(ops)), ptr)

}

func (f *Firewood) Get(input_key []byte) []byte {
	var pin runtime.Pinner
	defer pin.Unpin()
	ffi_key := make_value(&pin, input_key)

	value := C.get(ffi_key)
	ffi_bytes := C.GoBytes(unsafe.Pointer(value.data), C.int(value.len))
	C.free_value(value)
	return ffi_bytes
}
func make_value(pin *runtime.Pinner, data []byte) C.struct_Value {
	ptr := (*C.uchar)(unsafe.Pointer(&data[0]))
	pin.Pin(ptr)
	return C.struct_Value{C.size_t(len(data)), ptr}
}
