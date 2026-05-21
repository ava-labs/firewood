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
// registry.mu. Going the other way (registry first, then lease.mu) is
// what [lease.attach] does on the closed-registry path — but it drops
// registry.mu before invoking dropFn, so there is no simultaneous nested
// acquisition. [closeAndForceDrop] takes registry.mu only for the
// snapshot, releases it, and then invokes dropFns, which re-enter
// registry.mu via remove. No path holds both locks at once.

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

// newHandle constructs an inactive handle: the C pointer and free
// function are set, but the lease is not yet attached to any registry.
// Callers must invoke [lease.attach] before the handle becomes visible,
// or the wg/dropFn invariants will be broken.
func newHandle[T any](ptr T, free func(T) C.VoidResult) *handle[T] {
	return &handle[T]{
		ptr:     ptr,
		free:    free,
		dropped: false,
	}
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
	// are added in [lease.attach] and removed in [remove] (called from
	// [lease.releaseLocked]).
	handles map[*lease]func() error
	// closed is set by [closeAndForceDrop] and prevents any subsequent
	// registration. Once closed, [lease.attach] invokes the dropFn itself
	// and returns errDBClosed so the caller does not need to clean up.
	closed bool
}

func newKeepAliveRegistry() *keepAliveRegistry {
	return &keepAliveRegistry{
		handles: make(map[*lease]func() error),
	}
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
// same critical section, so any [lease.attach] call that observes the
// registry as still open will be visible in the snapshot, and any call
// that observes it as closed will refuse and Drop itself. This removes
// the race a snapshot-and-retry loop would have with derived-handle
// constructors (e.g. [Proposal.Propose], [Revision.Iter]) that hold only
// the parent's lease.mu.RLock rather than [Database.handleLock].
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

// attach atomically registers this lease with both the registry's
// WaitGroup and its drop map under a single registry.mu critical
// section. Either both succeed, or — on a registry that has already
// been closed via [closeAndForceDrop] — dropFn is invoked, the wg is
// NOT incremented, and errDBClosed is returned (joined with any error
// dropFn produced).
//
// This is the single entry point for activating a new lease. Holding
// the wg without being in the drop map is the latent failure mode that
// [closeAndForceDrop] cannot recover from, so the two-step init+register
// sequence is unsafe; attach does both atomically.
//
// Caller does not need to hold lease.mu — a fresh lease isn't visible
// to anyone else until attach returns. Caller does not need to clean
// up on error: on the closed-registry path attach invokes dropFn itself.
func (l *lease) attach(registry *keepAliveRegistry, dropFn func() error) error {
	registry.mu.Lock()
	if registry.closed {
		registry.mu.Unlock()
		// Call dropFn outside registry.mu to preserve the lease.mu →
		// registry.mu lock order (dropFn → handle.Drop → lease.release
		// takes lease.mu, which would then take registry.mu via remove).
		// No wg.Add ran, so no wg.Done is needed.
		if err := dropFn(); err != nil {
			return fmt.Errorf("%w: %w", errDBClosed, err)
		}
		return errDBClosed
	}
	if l.registry != nil {
		registry.mu.Unlock()
		// attach must be called exactly once per lease. Reaching this is
		// always a package bug — either a constructor re-used a lease or
		// two goroutines raced on a lease that should be single-owner.
		panic("lease already attached")
	}
	registry.wg.Add(1)
	registry.handles[l] = dropFn
	l.registry = registry
	registry.mu.Unlock()
	return nil
}

// attachUnregistered increments the registry's WaitGroup but does NOT
// add this lease to the drop map. It exists for [RangeProof], which
// intentionally stays outside the registry so its runtime.SetFinalizer
// can collect the proof — a bound Free in the registry map would keep
// the proof reachable forever. Consequently [WithForceCloseHandles]
// will not auto-drop a still-referenced RangeProof; graceful
// [Database.Close] still waits on the wg.
//
// Callers must serialize against [Database.Close] (e.g. via
// db.handleLock.RLock) before invoking this — there is no closed-registry
// guard here. The change-proof family is being redesigned, so a
// handle[T] migration here is deferred.
func (l *lease) attachUnregistered(registry *keepAliveRegistry) {
	registry.mu.Lock()
	defer registry.mu.Unlock()

	if l.registry != nil {
		panic("lease already attached")
	}
	registry.wg.Add(1)
	l.registry = registry
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
// unless [attach] or [attachUnregistered] runs again in between.
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
// [release]. Idempotent: calls after the first are no-ops until
// [lease.attach] or [lease.attachUnregistered] runs again.
func (l *lease) releaseLocked() {
	if l.registry == nil {
		return
	}
	l.registry.wg.Done()
	l.registry.remove(l)
	l.registry = nil
}
