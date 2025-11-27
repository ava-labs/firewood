// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
import "C"

func FlushStuff() error {
	return getErrorFromVoidResult(C.fwd_block_replay_flush())
}
