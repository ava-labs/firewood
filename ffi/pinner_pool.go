// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"runtime"
	"sync"
)

// pinnerPool is a sync.Pool for reusing runtime.Pinner instances.
// This reduces allocations in the hot path of FFI calls where
// each call otherwise creates a new Pinner.
var pinnerPool = sync.Pool{
	New: func() any {
		return new(runtime.Pinner)
	},
}

// getPinner retrieves a Pinner from the pool.
// The caller MUST call releasePinner when done.
func getPinner() *runtime.Pinner {
	return pinnerPool.Get().(*runtime.Pinner)
}

// releasePinner unpins all pointers and returns the Pinner to the pool.
func releasePinner(p *runtime.Pinner) {
	p.Unpin()
	pinnerPool.Put(p)
}
