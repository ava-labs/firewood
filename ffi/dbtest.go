package main

/*
#cgo LDFLAGS: -L ../target/debug/ -lffi
#include "firewood.h"
#include <stdlib.h>
*/
import "C"
import "fmt"

type Firewood struct {
}

func main() {
	fmt.Println(C.add(1, 2))
}
