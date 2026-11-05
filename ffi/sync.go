// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
// #cgo noescape fwd_start_sync
// #cgo nocallback fwd_start_sync
// #cgo noescape fwd_free_sync
// #cgo nocallback fwd_free_sync
// #cgo noescape fwd_finish_sync
// #cgo nocallback fwd_finish_sync
import "C"

import (
	"errors"
	"fmt"
	"runtime"
	"unsafe"
)

// defaultSyncTaskLimit is the default cap on concurrently outstanding work
// regions.
const defaultSyncTaskLimit = 16

var errSyncDropped = errors.New("sync already finished or dropped")

// syncConfig collects the [SyncOption] settings for [Database.StartSync].
type syncConfig struct {
	taskLimit uint32
}

// SyncOption configures [Database.StartSync].
type SyncOption func(*syncConfig)

// WithTaskLimit sets the cap on concurrently outstanding work regions. Zero is rejected by [Database.StartSync].
// Default: 16.
func WithTaskLimit(n uint32) SyncOption {
	return func(c *syncConfig) {
		c.taskLimit = n
	}
}

// Sync is an in-progress state sync of a [Database] toward a fixed target
// root. Create with [Database.StartSync] and release
// with [Sync.Finish] (returns the final root) or Drop (promoted from the
// underlying handle; for abandoned syncs). An abandoned sync leaves only
// ordinary committed revisions behind.
//
// The Sync holds a keep-alive lease on the database: [Database.Close] blocks
// (and then fails) until Finish or Drop is called.
type Sync struct {
	// handle is an opaque pointer to the sync within Firewood. It should be
	// passed to the C FFI functions that operate on syncs.
	//
	// It is not safe to call these methods with a nil handle.
	//
	// Calls to `C.fwd_finish_sync` and `C.fwd_free_sync` will invalidate
	// this handle, so it should not be used after those calls.
	*handle[*C.SyncHandle]

	// taskLimit is the cap on outstanding work regions, fixed at StartSync
	// time.
	taskLimit uint32
}

// StartSync begins a state sync of this database toward target, with at most
// the configured task limit of concurrently outstanding work regions.
// The returned [Sync] must be released with [Sync.Finish] or Drop before the
// database is closed.
func (db *Database) StartSync(target Hash, opts ...SyncOption) (*Sync, error) {
	conf := &syncConfig{taskLimit: defaultSyncTaskLimit}
	for _, opt := range opts {
		opt(conf)
	}

	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	result := C.fwd_start_sync(db.handle, newCHashKey(target), C.uint32_t(conf.taskLimit))
	return getSyncFromSyncStartResult(result, db.keepAlives, conf.taskLimit)
}

// getSyncFromSyncStartResult converts a C.SyncStartResult to a Sync or error.
func getSyncFromSyncStartResult(result C.SyncStartResult, registry *keepAliveRegistry, taskLimit uint32) (*Sync, error) {
	switch result.tag {
	case C.SyncStartResult_NullHandlePointer:
		return nil, errDBClosed
	case C.SyncStartResult_Ok:
		ptr := *(**C.SyncHandle)(unsafe.Pointer(&result.anon0))
		s := &Sync{
			handle:    newHandle(ptr, func(p *C.SyncHandle) C.VoidResult { return C.fwd_free_sync(p) }),
			taskLimit: taskLimit,
		}
		// attach calls s.Drop on the closed-registry path; nothing else to clean up.
		if err := s.lease.attach(registry, s.Drop); err != nil {
			return nil, err
		}
		runtime.AddCleanup(s, drop[*C.SyncHandle], s.handle)
		return s, nil
	case C.SyncStartResult_Err:
		return nil, newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
	default:
		return nil, fmt.Errorf("unknown C.SyncStartResult tag: %d", result.tag)
	}
}

// Finish releases the sync handle and returns the latest committed root, for
// the caller to compare against the target. Idempotent with Drop: whichever
// runs first releases the handle, and a Finish after either returns an error.
func (s *Sync) Finish() (Hash, error) {
	var hash Hash
	err := s.lease.release(func() error {
		if s.dropped {
			return errSyncDropped
		}

		var err error
		hash, err = getHashKeyFromHashResult(C.fwd_finish_sync(s.ptr))

		// Prevent double free
		s.ptr = nil
		s.dropped = true

		return err
	})
	return hash, err
}
