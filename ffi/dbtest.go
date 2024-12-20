package main

//#cgo LDFLAGS: ./../target/debug/libffi.a -ldl
//#include "firewood.h"
//#include <stdlib.h>
import "C"
import (
	"fmt"
	"unsafe"
)

//import "runtime"

type Firewood struct {
}

func main() {
	// do we need to pin? probably not but let's see...
	// var pin runtime.Pinner
	// pin.Pin(&key)
	// defer pin.Unpin()
	// pk := *((*[64]byte)(unsafe.Pointer(&key.pubkey)))
	//setup_globals()
	key := []byte{'a', 'b', 'c'}
	keyptr := (*C.uchar)(unsafe.Pointer(&key[0]))
	s := C.Value{C.size_t(len(key)), keyptr}
	value := C.get(s)

	slice := (*[1 << 30]C.char)(unsafe.Pointer(value.data))[:value.len:value.len]
	fmt.Println("slice:", slice)

	C.free_value(value)
}
