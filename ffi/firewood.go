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
	Db *C.void
}

func NewFirewood(path string) Firewood {
	db := C.fwd_create_db(C.CString(path), C.size_t(1000000), C.size_t(100))
	ptr := (*C.void)(db)
	return Firewood{Db: ptr}
}

const (
	OP_PUT = iota
	OP_DELETE
)

type KeyValue struct {
	Key   []byte
	Value []byte
}

func (f *Firewood) Batch(ops []KeyValue) []byte {
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
	hash := C.fwd_batch(unsafe.Pointer(f.Db), C.size_t(len(ops)), ptr)
	hash_bytes := C.GoBytes(unsafe.Pointer(hash.data), C.int(hash.len))
	C.fwd_free_value(&hash)
	return hash_bytes
}

func (f *Firewood) Get(input_key []byte) []byte {
	var pin runtime.Pinner
	defer pin.Unpin()
	ffi_key := make_value(&pin, input_key)

	value := C.fwd_get(unsafe.Pointer(f.Db), ffi_key)
	ffi_bytes := C.GoBytes(unsafe.Pointer(value.data), C.int(value.len))
	C.fwd_free_value(&value)
	return ffi_bytes
}
func make_value(pin *runtime.Pinner, data []byte) C.struct_Value {
	ptr := (*C.uchar)(unsafe.Pointer(&data[0]))
	pin.Pin(ptr)
	return C.struct_Value{C.size_t(len(data)), ptr}
}

func (f *Firewood) RootHash() []byte {
	hash := C.fwd_root_hash(unsafe.Pointer(f.Db))
	hash_bytes := C.GoBytes(unsafe.Pointer(hash.data), C.int(hash.len))
	return hash_bytes
}

func (f *Firewood) Close() {
	C.fwd_close_db(unsafe.Pointer(f.Db))
}