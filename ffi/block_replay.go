// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
import "C"

// FlushBlockReplay flushes buffered block replay operations to disk.
//
// This function is only meaningful when the FFI library was compiled with
// the `block-replay` feature and the `FIREWOOD_BLOCK_REPLAY_PATH` environment
// variable is set. Otherwise, it is a no-op.
func FlushBlockReplay() error {
	return getErrorFromVoidResult(C.fwd_block_replay_flush())
}
