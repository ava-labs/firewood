// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
import "C"

import (
	"context"
	"errors"
	"fmt"
	"sync"
)

// Lock ordering inside the keep-alive infrastructure:
//
//	lease.mu (per-handle)  before  keepAliveRegistry.mu (per-database)
//
// Concretely: every [lease.release] / [lease.releaseLocked] call holds
// lease.mu while invoking [keepAliveRegistry.remove], which briefly takes
// registry.mu. Going the other way (register first, then lease.mu) is
// what [keepAliveRegistry.register] does on the closed-registry path —
// but it drops registry.mu before invoking dropFn, so there is no
// simultaneous nested acquisition. [closeAndForceDrop] takes registry.mu
// only for the snapshot, releases it, and then invokes dropFns, which
// re-enter registry.mu via remove. No path holds both locks at once.

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

	// lease keeps the parent database alive while this handle is in use. It is
	// initialized when the handle is created and released when [Drop] is called.
	lease lease

	free func(T) C.VoidResult
}

func createHandle[T any](ptr T, registry *keepAliveRegistry, free func(T) C.VoidResult) *handle[T] {
	h := &handle[T]{
		ptr:     ptr,
		free:    free,
		dropped: false,
	}
	h.lease.init(registry)
	return h
}

func drop[T any](h *handle[T]) {
	_ = h.Drop()
}

// Drop releases the C-side resource and releases this handle's lease on
// the parent database.
//
// Release is unconditional: even if the underlying free call returns an
// error, the lease is still released. The C pointer is cleared before the
// free call, so a subsequent Drop is a no-op rather than a retry against
// a possibly-corrupt pointer. Any free error is surfaced to the caller,
// which is then responsible for deciding what to do — but the caller does
// not need to worry about the database hanging on shutdown because a free
// errored.
//
// Idempotent: subsequent calls after the first return nil.
func (h *handle[T]) Drop() error {
	return h.lease.release(true /* releaseOnError */, func() error {
		if h.dropped {
			return nil
		}

		// Mark dropped and clear the pointer *before* calling free, so that
		// even if free errors we never call it twice on the same pointer.
		ptr := h.ptr
		var zero T
		h.ptr = zero
		h.dropped = true

		if err := getErrorFromVoidResult(h.free(ptr)); err != nil {
			return fmt.Errorf("%w: %w", errFreeingValue, err)
		}
		return nil
	})
}

// keepAliveRegistry tracks every outstanding handle that holds a lease on a
// database. It owns the WaitGroup that powers [Database.Close]'s graceful
// wait alongside a map of drop callbacks that [WithForceCloseHandles] uses
// to release leases forcibly.
//
// All bookkeeping (WaitGroup + map) lives here; the lease type holds only
// a back-reference for release. Map values are outer-type Drop functions
// (e.g. [Iterator.Drop], whose implementation also frees any borrowed batch).
type keepAliveRegistry struct {
	mu sync.Mutex
	wg sync.WaitGroup
	// handles maps a registered lease to its outer Drop function. Entries
	// are added in [register] and removed in [remove] (called from
	// [lease.releaseLocked]).
	handles map[*lease]func() error
	// closed is set by [closeAndForceDrop] and prevents any subsequent
	// registration. Once closed, [register] invokes the dropFn itself and
	// returns errDBClosed so the caller does not need to clean up.
	closed bool
}

func newKeepAliveRegistry() *keepAliveRegistry {
	return &keepAliveRegistry{
		handles: make(map[*lease]func() error),
	}
}

// register records the outer Drop callback for an already-initialized
// lease.
//
// If the registry has been closed via [closeAndForceDrop], register
// invokes dropFn itself (so the WaitGroup increment from [lease.init] is
// matched and the C handle is freed) and returns errDBClosed joined with
// any error the synchronous drop produced. Callers do not need to clean
// up on register failure — they just propagate the error.
func (r *keepAliveRegistry) register(l *lease, dropFn func() error) error {
	r.mu.Lock()
	if r.closed {
		r.mu.Unlock()
		if dropErr := dropFn(); dropErr != nil {
			return errors.Join(errDBClosed, dropErr)
		}
		return errDBClosed
	}
	r.handles[l] = dropFn
	r.mu.Unlock()
	return nil
}

// remove deletes a lease from the registry without invoking its dropFn.
// Called from [lease.releaseLocked] on explicit disown.
func (r *keepAliveRegistry) remove(l *lease) {
	r.mu.Lock()
	delete(r.handles, l)
	r.mu.Unlock()
}

// closeAndForceDrop closes the registry to new registrations and invokes
// the Drop callback for every currently-registered lease. It is the
// implementation behind [WithForceCloseHandles].
//
// Atomicity: the closed flag is set and the snapshot is taken under the
// same critical section, so any [register] call that observes the registry
// as still open will be visible in the snapshot, and any call that observes
// it as closed will refuse and Drop itself. This removes the race a
// snapshot-and-retry loop would have with derived-handle constructors
// (e.g. [Proposal.Propose], [Revision.Iter]) that hold only the parent's
// lease.mu.RLock rather than [Database.handleLock].
//
// Drop callbacks are invoked without the registry lock held; they
// re-enter the lock via [remove] inside releaseLocked, which is why
// the snapshot is taken eagerly.
//
// Drainable on retry: if ctx expires partway through, accumulated drop
// errors are returned and the registry stays closed, but handles that
// were not yet dropped remain in the map. A subsequent call re-snapshots
// the remainder and continues draining, so a caller that hits a context
// deadline can retry [Database.Close] with a fresh context to finish.
func (r *keepAliveRegistry) closeAndForceDrop(ctx context.Context) error {
	r.mu.Lock()
	r.closed = true
	drops := make([]func() error, 0, len(r.handles))
	for _, fn := range r.handles {
		drops = append(drops, fn)
	}
	r.mu.Unlock()

	var dropErr error
	for _, fn := range drops {
		if ctx.Err() != nil {
			// Caller (Close) detects ctx cancellation itself and wraps it
			// with ErrActiveKeepAliveHandles; returning ctx.Err() here would
			// double-wrap when Close joins the two.
			return dropErr
		}
		if err := fn(); err != nil {
			dropErr = errors.Join(dropErr, err)
		}
	}
	return dropErr
}

// lease represents a single outstanding handle's claim on the parent
// database. It exists so that operations on the wrapping handle can be
// serialized against [Database.Close] / [WithForceCloseHandles]: every
// public method on a Proposal, Revision, Reconstructed, or Iterator
// takes mu.RLock for the duration of the call, and force-drop takes
// mu.Lock via [release] before tearing down the C handle.
//
// All bookkeeping (the WaitGroup, the registry map) lives in
// [keepAliveRegistry]; the lease holds only a back-reference so that
// [releaseLocked] can find the registry it needs to decrement.
type lease struct {
	mu sync.RWMutex
	// registry is the parent database's keep-alive registry. Set in
	// [init], cleared in [releaseLocked]; nil indicates the lease has
	// already been released.
	registry *keepAliveRegistry
}

// init attaches this lease to the parent registry's WaitGroup. The
// registry's wg counter is incremented here; [release] / [releaseLocked]
// decrement it.
func (l *lease) init(registry *keepAliveRegistry) {
	// Lock not strictly necessary today (a lease is owned exclusively by
	// its constructing goroutine until init returns) but harmless, and
	// required for types that initialize the lease at some point after
	// construction (#1429).
	l.mu.Lock()
	defer l.mu.Unlock()

	if l.registry != nil {
		// init must be called exactly once per lease. Reaching this is
		// always a package bug — either a constructor re-used a lease or
		// two goroutines raced on a lease that should be single-owner.
		panic("lease already initialized")
	}

	l.registry = registry
	l.registry.wg.Add(1)
}

// release runs attemptDisown and, if it returns nil (or
// releaseOnError is true), releases the lease via [releaseLocked].
//
// Most callers should pass releaseOnError=true: keeping the lease alive
// on a free error means the parent database can never gracefully close,
// since [Database.Close] waits on a WaitGroup whose counter only
// decrements via release. Pass false only when the caller has a concrete
// recovery path for the failed free.
//
// Safe to call multiple times; subsequent calls after the first continue
// to invoke attemptDisown but do not double-decrement the WaitGroup
// unless [init] runs again in between.
func (l *lease) release(releaseOnError bool, attemptDisown func() error) error {
	l.mu.Lock()
	defer l.mu.Unlock()

	err := attemptDisown()
	if err == nil || releaseOnError {
		l.releaseLocked()
	}
	return err
}

// releaseLocked is the single bookkeeping point: decrements the
// registry's WaitGroup, removes this lease from the registry, and
// clears the registry back-pointer. Caller must hold l.mu.
//
// Exists for callers like [Reconstructed.Reconstruct] that hold mu.Lock
// for a wider critical section and would otherwise deadlock through
// [release]. Idempotent: calls after the first are no-ops until [init]
// runs again.
func (l *lease) releaseLocked() {
	if l.registry == nil {
		return
	}
	l.registry.wg.Done()
	l.registry.remove(l)
	l.registry = nil
}
