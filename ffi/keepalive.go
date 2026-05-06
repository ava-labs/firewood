// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
import "C"

import (
	"fmt"
	"sync"
)

type handle[T any] struct {
	// handle is an opaque pointer to the underlying Rust object. It should be
	// passed to the C FFI functions that operate on this type.
	//
	// It is not safe to call these methods with a nil handle.
	//
	// Calls to `C.fwd_free_X` will invalidate this handle, so it should not be
	// used after those calls.
	ptr     T
	dropped bool

	// keepAliveHandle is used to keep the database alive while this object is
	// in use. It is initialized when the object is created and disowned after
	// [X.Close] is called.
	keepAliveHandle databaseKeepAliveHandle

	free func(T) C.VoidResult
}

func createHandle[T any](ptr T, registry *keepAliveRegistry, free func(T) C.VoidResult) *handle[T] {
	h := &handle[T]{
		ptr:     ptr,
		free:    free,
		dropped: false,
	}
	h.keepAliveHandle.init(registry)
	return h
}

func drop[T any](h *handle[T]) {
	_ = h.Drop()
}

func (h *handle[T]) Drop() error {
	return h.keepAliveHandle.disown(false /* evenOnError */, func() error {
		if h.dropped {
			return nil
		}

		if err := getErrorFromVoidResult(h.free(h.ptr)); err != nil {
			return fmt.Errorf("%w: %w", errFreeingValue, err)
		}

		// Prevent double free
		var zero T
		h.ptr = zero
		h.dropped = true

		return nil
	})
}

// keepAliveRegistry tracks every outstanding handle that holds a lease on a
// database. It carries the WaitGroup that powers [Database.Close]'s graceful
// wait, alongside a map of drop callbacks that [WithForceCloseHandles] uses
// to release handles forcibly.
//
// All fields are owned by the registry; nothing else mutates them. The map
// values are the outer-type Drop functions (e.g. [Iterator.Drop], whose
// implementation also frees any borrowed batch).
type keepAliveRegistry struct {
	mu      sync.Mutex
	wg      sync.WaitGroup
	handles map[*databaseKeepAliveHandle]func() error
}

func newKeepAliveRegistry() *keepAliveRegistry {
	return &keepAliveRegistry{
		handles: make(map[*databaseKeepAliveHandle]func() error),
	}
}

// register records a handle's outer Drop function. The caller must register
// before whatever lock prevents [Database.Close] from racing with the
// handle's construction is released — in practice, before the constructor
// returns from under [Database.handleLock] (RLock).
func (r *keepAliveRegistry) register(h *databaseKeepAliveHandle, dropFn func() error) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.handles[h] = dropFn
}

func (r *keepAliveRegistry) unregister(h *databaseKeepAliveHandle) {
	r.mu.Lock()
	defer r.mu.Unlock()
	delete(r.handles, h)
}

// snapshot returns the currently registered drop callbacks.
func (r *keepAliveRegistry) snapshot() []func() error {
	r.mu.Lock()
	defer r.mu.Unlock()
	out := make([]func() error, 0, len(r.handles))
	for _, fn := range r.handles {
		out = append(out, fn)
	}
	return out
}

// databaseKeepAliveHandle is added to types that hold a lease on the database
// to ensure it is not closed while those types are still in use.
//
// This is necessary to prevent use-after-free bugs where a type holding a
// reference to the database outlives the database itself. Even attempting to
// free those objects after the database has been closed will lead to undefined
// behavior, as a part of the underling Rust object will have already been freed.
type databaseKeepAliveHandle struct {
	mu sync.RWMutex
	// registry is the parent database's keep-alive registry. It is set in
	// [init] and cleared in [disownLocked]; nil indicates the handle has
	// already been disowned.
	registry *keepAliveRegistry
}

// init initializes the keep-alive handle to track a new outstanding handle.
func (h *databaseKeepAliveHandle) init(registry *keepAliveRegistry) {
	// lock not necessary today, but will be necessary in the future for types
	// that initialize the handle at some point after construction (#1429).
	h.mu.Lock()
	defer h.mu.Unlock()

	if h.registry != nil {
		// setting the finalizer twice will also panic, so we're panicking
		// early to provide better context
		panic("keep-alive handle already initialized")
	}

	h.registry = registry
	h.registry.wg.Add(1)
}

// disown indicates that the object owning this handle is no longer keeping the
// database alive. If [attemptDisown] returns an error, disowning will only occur
// if [disownEvenOnErr] is true.
//
// This method is safe to call multiple times; subsequent calls after the first
// will continue to invoke [attemptDisown] but will not decrement the wait group
// unless [databaseKeepAliveHandle.init] was called again in the meantime.
func (h *databaseKeepAliveHandle) disown(disownEvenOnErr bool, attemptDisown func() error) error {
	h.mu.Lock()
	defer h.mu.Unlock()

	err := attemptDisown()
	if err == nil || disownEvenOnErr {
		h.disownLocked()
	}
	return err
}

// disownLocked performs the disown bookkeeping (Done on the WaitGroup,
// unregister from the registry, clear the back-pointer) and assumes the
// caller already holds h.mu. It exists for callers like
// [Reconstructed.Reconstruct] that hold mu.Lock for a wider critical section
// and would otherwise deadlock through [disown].
//
// Idempotent: calls after the first are no-ops until [init] runs again.
func (h *databaseKeepAliveHandle) disownLocked() {
	if h.registry == nil {
		return
	}
	h.registry.wg.Done()
	h.registry.unregister(h)
	h.registry = nil
}
