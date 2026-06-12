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
// #cgo noescape fwd_get_work
// #cgo nocallback fwd_get_work
// #cgo noescape fwd_submit_work
// #cgo nocallback fwd_submit_work
import "C"

import (
	"context"
	"errors"
	"fmt"
	"runtime"
	"sync"
	"sync/atomic"
	"unsafe"
)

// Lock ordering for the state-sync worker pool. Never acquire a
// lower-numbered lock while holding a higher-numbered one:
//
//	| Order         | Lock                       | Held where                                          |
//	|---------------|----------------------------|-----------------------------------------------------|
//	| 1 (outermost) | pool mu (syncPool.mu)      | acquire (incl. fwd_get_work via Sync.getWork),      |
//	|               |                            | every Signal/Broadcast site, the done/err flags     |
//	| 2             | keepAliveHandle.mu (RLock) | inside the getWork/submitWork wrappers;             |
//	|               |                            | write-locked by Finish/Drop                         |
//	| 3 (innermost) | Rust SyncState mutex +     | inside the C calls                                  |
//	|               | revision read locks        |                                                     |
//
// fwd_get_work IS called under the pool mu (lock 1 -> 2 -> 3); fwd_submit_work
// must NOT be called under the pool mu (lock 2 -> 3 only) — it does real
// verification + commit work and would serialize the pool. Finish/Drop take
// only lock 2 (write), so they are safe against in-flight getWork/submitWork
// calls, but they must not run while Run is executing: parked workers do not
// hold lock 2 and would not be woken by Drop — cancelling Run's ctx is the
// only sanctioned way to stop a running sync. A Drop racing Run anyway
// degrades gracefully: the next getWork/submitWork returns errSyncDropped,
// which aborts the pool cleanly.

// SyncMaxProofKeys is the maximum number of key/value pairs the verifier
// accepts per submitted proof; proofs exceeding this are rejected as invalid
// (a peer fault). Callers should pass this as the key limit in peer requests
// so the transport's byte budget, not the key count, is the binding limit.
const SyncMaxProofKeys = uint32(C.FWD_SYNC_MAX_PROOF_KEYS)

// defaultSyncTaskLimit is the default worker count for [Sync.Run] and the
// default cap on concurrently outstanding work regions.
const defaultSyncTaskLimit = 16

var (
	errSyncDropped = errors.New("sync already finished or dropped")

	// ErrCoverageRootMismatch is returned (wrapped, test with [errors.Is])
	// from [Sync.Run] when coverage tiles the whole keyspace but the
	// committed root does not match the target: an internal invariant
	// violation, never a peer fault. The recommended caller reaction is to
	// restart the entire sync.
	ErrCoverageRootMismatch = errors.New("sync coverage complete but root does not match target")
)

// FetchFunc fetches the serialized range proof for one inclusive key range
// from a peer. Firewood never sees the network; this callback IS the network.
//
//   - startKey/endKey: INCLUSIVE proof-request bounds; a Maybe with no value
//     means unbounded in that direction. Pass them through to the peer
//     request unchanged: an absent bound must be forwarded as the protocol's
//     "no bound", never as a present empty key (the proof generator emits a
//     different, unverifiable shape for an explicit empty start key).
//   - attempt: 0 on the first try for this exact range; incremented each
//     time the previous response for this range failed verification
//     (InvalidProof). attempt > 0 means: pick a DIFFERENT peer, and
//     optionally down-score the previous one.
//   - prevErr: nil when attempt == 0; otherwise the (opaque, v1) rejection
//     detail for the previous attempt, for logging or peer scoring.
//
// Requests should quote at most [SyncMaxProofKeys] keys. Returning an error
// aborts the entire sync; "no peers available" style give-up policy belongs
// to the caller, not the pool.
type FetchFunc func(ctx context.Context, startKey, endKey Maybe[[]byte], attempt int, prevErr error) ([]byte, error)

// syncConfig collects the [SyncOption] settings for [Database.StartSync].
type syncConfig struct {
	taskLimit uint32
}

// SyncOption configures [Database.StartSync].
type SyncOption func(*syncConfig)

// WithTaskLimit sets the number of pool workers, which is also the cap on
// concurrently outstanding work regions. Zero is rejected by [Database.StartSync].
// Default: 16.
func WithTaskLimit(n uint32) SyncOption {
	return func(c *syncConfig) {
		c.taskLimit = n
	}
}

// Sync is an in-progress state sync of a [Database] toward a fixed target
// root. Create with [Database.StartSync], drive with [Sync.Run], and release
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

	// taskLimit is the worker count for Run, fixed at StartSync time.
	taskLimit uint32

	// ran makes Run single-shot.
	ran atomic.Bool
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
	return getSyncFromSyncStartResult(result, &db.outstandingHandles, conf.taskLimit)
}

// getSyncFromSyncStartResult converts a C.SyncStartResult to a Sync or error.
func getSyncFromSyncStartResult(result C.SyncStartResult, wg *sync.WaitGroup, taskLimit uint32) (*Sync, error) {
	switch result.tag {
	case C.SyncStartResult_NullHandlePointer:
		return nil, errDBClosed
	case C.SyncStartResult_Ok:
		ptr := *(**C.SyncHandle)(unsafe.Pointer(&result.anon0))
		s := &Sync{
			handle:    createHandle(ptr, wg, func(p *C.SyncHandle) C.VoidResult { return C.fwd_free_sync(p) }),
			taskLimit: taskLimit,
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
// the caller to compare against the target. Meaningful once [Sync.Run] has
// returned nil, but callable anytime. Idempotent with Drop: whichever runs
// first releases the handle, and a Finish after either returns an error.
//
// Finish must not be called while [Sync.Run] is executing; cancel Run's ctx
// and wait for it to return first.
func (s *Sync) Finish() (Hash, error) {
	var hash Hash
	err := s.keepAliveHandle.disown(true /* evenOnError */, func() error {
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

// work is one decoded C.SyncWorkItem: a region of the keyspace reserved for
// one worker. The bounds are the EXACT inclusive proof-request bounds to
// forward to a peer.
type work struct {
	// id correlates this region with its eventual submitWork call.
	id uint64
	// startKey is the inclusive lower request bound; no value = unbounded.
	// Decoded from an EMPTY start_key on the wire (lossless: a work range
	// never has an empty-but-present lower bound).
	startKey Maybe[[]byte]
	// endKey is the inclusive upper request bound; no value = unbounded.
	endKey Maybe[[]byte]
	// wakeupNeighbor is true iff shareable cold work remains right now: the
	// pool should wake exactly one parked worker (Signal, not Broadcast).
	wakeupNeighbor bool
}

// someBytes and noBytes are concrete Maybe[[]byte] implementations for
// non-test code. NOTE: proofs_test.go defines test-only `maybe`/`something`/
// `nothing` in this package — these use distinct names to avoid colliding in
// the test build.
type someBytes []byte

// HasValue implements [Maybe].
func (someBytes) HasValue() bool { return true }

// Value implements [Maybe].
func (s someBytes) Value() []byte { return s }

type noBytes struct{}

// HasValue implements [Maybe].
func (noBytes) HasValue() bool { return false }

// Value implements [Maybe].
func (noBytes) Value() []byte { return nil }

// newWorkFromC copies a C.SyncWorkItem into Go memory and frees the C key
// allocations immediately (no per-item finalizers; this runs once per
// region, not per key).
func newWorkFromC(item C.SyncWorkItem) (*work, error) {
	w := &work{id: uint64(item.id), wakeupNeighbor: bool(item.wakeup_neighbor)}

	start := newOwnedBytes(item.start_key)
	startBytes := start.CopiedBytes()
	err := start.Free()
	if len(startBytes) > 0 {
		w.startKey = someBytes(startBytes)
	} else {
		// EMPTY start_key means "no lower bound"; it MUST cross to the
		// protocol as an absent bound, never as a present empty key.
		w.startKey = noBytes{}
	}

	if item.end_key.tag == C.Maybe_OwnedBytes_Some_OwnedBytes {
		end := newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&item.end_key.anon0)))
		w.endKey = someBytes(end.CopiedBytes())
		err = errors.Join(err, end.Free())
	} else {
		w.endKey = noBytes{}
	}

	return w, err
}

// getWorkOutcome is the decoded tag of a C.GetWorkResult.
type getWorkOutcome uint8

const (
	// gotWait is the zero value so error paths default to the inert outcome;
	// callers must check the error before the outcome.
	gotWait getWorkOutcome = iota
	gotWork
	gotDone
)

// submitOutcome is the decoded tag of a C.SubmitResult.
type submitOutcome uint8

const (
	// submitFailed is the zero value so error paths default to the fatal
	// outcome; callers must check the error before the outcome.
	submitFailed submitOutcome = iota
	submitContinue
	submitExhausted
	submitInvalid
)

// syncWorkSource abstracts the FFI work/submit pair so the pool can be
// unit-tested against a scripted in-Go fake (no cgo in the loop). *Sync is
// the production implementation.
type syncWorkSource interface {
	getWork() (getWorkOutcome, *work, error)
	submitWork(id uint64, proof []byte) (submitOutcome, *work, error)
}

// getWork asks Firewood for a new region (the only C.fwd_get_work callsite).
// It never parks and is safe and idempotent to re-call, so the pool calls it
// while holding the pool mutex (see the lock-ordering table).
func (s *Sync) getWork() (getWorkOutcome, *work, error) {
	s.keepAliveHandle.mu.RLock()
	defer s.keepAliveHandle.mu.RUnlock()
	if s.dropped {
		return gotWait, nil, errSyncDropped
	}

	result := C.fwd_get_work(s.ptr)
	switch result.tag {
	case C.GetWorkResult_NullHandlePointer:
		return gotWait, nil, errSyncDropped
	case C.GetWorkResult_Work:
		w, err := newWorkFromC(*(*C.SyncWorkItem)(unsafe.Pointer(&result.anon0)))
		if err != nil {
			return gotWait, nil, err
		}
		return gotWork, w, nil
	case C.GetWorkResult_Wait:
		return gotWait, nil, nil
	case C.GetWorkResult_Done:
		return gotDone, nil, nil
	case C.GetWorkResult_CoverageRootMismatch:
		detail := newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
		return gotWait, nil, fmt.Errorf("%w: %w", ErrCoverageRootMismatch, detail)
	case C.GetWorkResult_Err:
		return gotWait, nil, newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
	default:
		return gotWait, nil, fmt.Errorf("unknown C.GetWorkResult tag: %d", result.tag)
	}
}

// submitWork submits serialized proof bytes for the region reserved under id
// (the only C.fwd_submit_work callsite). It does real verification + commit
// work, so it must NOT be called under the pool mutex.
//
// Outcomes: submitInvalid is NON-fatal (peer fault; same region, SAME id —
// re-fetch from a different peer; the error carries the rejection detail).
// submitFailed is fatal to the sync: a stale id also lands there, and
// resubmitting a stale id can never succeed.
func (s *Sync) submitWork(id uint64, proof []byte) (submitOutcome, *work, error) {
	s.keepAliveHandle.mu.RLock()
	defer s.keepAliveHandle.mu.RUnlock()
	if s.dropped {
		return submitFailed, nil, errSyncDropped
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	result := C.fwd_submit_work(s.ptr, C.uint64_t(id), newBorrowedBytes(proof, &pinner))
	switch result.tag {
	case C.SubmitResult_NullHandlePointer:
		return submitFailed, nil, errSyncDropped
	case C.SubmitResult_Continue:
		w, err := newWorkFromC(*(*C.SyncWorkItem)(unsafe.Pointer(&result.anon0)))
		if err != nil {
			return submitFailed, nil, err
		}
		return submitContinue, w, nil
	case C.SubmitResult_Exhausted:
		return submitExhausted, nil, nil
	case C.SubmitResult_InvalidProof:
		return submitInvalid, nil, newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
	case C.SubmitResult_Err:
		return submitFailed, nil, newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
	default:
		return submitFailed, nil, fmt.Errorf("unknown C.SubmitResult tag: %d", result.tag)
	}
}

// Run spawns the configured number of worker goroutines up front and drives
// the sync to completion. It blocks until the sync is done (returns nil), an
// internal error occurs (first error wins; see [ErrCoverageRootMismatch]),
// or ctx is cancelled (returns ctx's error). Run may be called at most once,
// and [Sync.Finish]/Drop must not be called until Run has returned;
// cancelling ctx is the only sanctioned way to stop a running sync.
//
// Sync commits happen Rust-side (rebase-on-commit) and deliberately do not
// take this package's database commit lock: other Go-side write traffic
// remains safe but contends with sync commits inside Rust. Callers should
// not write to the database during a sync.
func (s *Sync) Run(ctx context.Context, fetch FetchFunc) error {
	if fetch == nil {
		return errors.New("nil FetchFunc")
	}
	if !s.ran.CompareAndSwap(false, true) {
		return errors.New("sync Run may only be called once")
	}
	return runPool(ctx, s, fetch, s.taskLimit)
}

// syncPool is the fixed worker pool driving one sync run. One mutex, one
// condvar, two flags — see the lock-ordering table at the top of this file
// and `docs/plans/state-sync.md` ("Lost-wakeup correctness").
type syncPool struct {
	src    syncWorkSource
	fetch  FetchFunc
	cancel context.CancelFunc

	mu   sync.Mutex
	cond *sync.Cond // over mu
	done bool       // a worker observed Done
	err  error      // first internal/fetch error; non-nil only when aborting
}

// runPool is [Sync.Run] minus the single-shot guard, split out so tests can
// drive the pool against a scripted syncWorkSource. taskLimit must be >= 1:
// with 0 workers there is nothing to wait for and the result is a vacuous
// nil. The public API guarantees this (fwd_start_sync rejects a zero
// task_limit, so a [Sync] always carries a nonzero one).
func runPool(ctx context.Context, src syncWorkSource, fetch FetchFunc, taskLimit uint32) error {
	ctx, cancel := context.WithCancel(ctx)
	defer cancel()

	p := &syncPool{src: src, fetch: fetch, cancel: cancel}
	p.cond = sync.NewCond(&p.mu)

	// ctx cancellation must wake parked workers. AfterFunc takes mu before
	// Broadcast, so the same two-lock argument as the shed Signal applies: a
	// worker is either pre-park (it re-checks ctx.Err() under mu) or already
	// parked (it receives the Broadcast). No lost cancellation.
	stop := context.AfterFunc(ctx, func() {
		p.mu.Lock()
		p.cond.Broadcast()
		p.mu.Unlock()
	})
	defer stop()

	var wg sync.WaitGroup
	for range taskLimit { // taskLimit goroutines, up front
		wg.Go(func() {
			p.worker(ctx)
		})
	}
	wg.Wait()

	p.mu.Lock()
	defer p.mu.Unlock()
	switch {
	case p.err != nil:
		return p.err // first error wins (incl. ErrCoverageRootMismatch)
	case p.done:
		return nil // sync complete
	default:
		return ctx.Err() // parent cancellation
	}
}

// fail records the first error and wakes everyone. The caller must hold mu.
func (p *syncPool) fail(err error) {
	if p.err == nil {
		p.err = err
	}
	p.cancel()         // unblocks in-flight fetches; AfterFunc also Broadcasts
	p.cond.Broadcast() // wake parked workers immediately
}

// worker is one pool goroutine: acquire a region, sweep it to exhaustion,
// repeat until done/error/cancellation.
func (p *syncPool) worker(ctx context.Context) {
	for {
		w, ok := p.acquire(ctx)
		if !ok {
			return
		}
		if !p.sweep(ctx, w) {
			return
		}
		// Region exhausted — go help a neighbor (reacquire).
	}
}

// acquire returns the next region, or ok=false when the worker should exit
// (done, error, or cancellation).
func (p *syncPool) acquire(ctx context.Context) (*work, bool) {
	p.mu.Lock()
	defer p.mu.Unlock()
	for { // ── REQUIREMENT 3: re-check LOOP, never a single if. ──
		if p.done || p.err != nil || ctx.Err() != nil {
			return nil, false
		}
		// ── REQUIREMENT 1: fwd_get_work is called while holding mu, so the
		// predicate read (C1) and the park (C2) share one mu critical
		// section. fwd_get_work's documented contract (pure function of the
		// durable SyncState, never parks, idempotent to re-call) is what
		// makes this legal.
		outcome, w, err := p.src.getWork()
		if err != nil {
			p.fail(err)
			return nil, false
		}
		switch outcome {
		case gotDone:
			p.done = true
			p.cond.Broadcast() // wake every parked worker so it can exit
			return nil, false
		case gotWork:
			if w.wakeupNeighbor {
				p.cond.Signal() // more cold work exists: wake ONE helper
			}
			return w, true
		case gotWait:
			p.cond.Wait() // park: atomically releases mu, reacquires on wake
		}
	}
}

// sweep drives one region to exhaustion. It returns false when the worker
// should exit (done/abort/cancellation), true to reacquire.
func (p *syncPool) sweep(ctx context.Context, w *work) bool {
	attempt, prevErr := 0, error(nil)
	for {
		// Exit early between chunks if the sync finished or aborted
		// (benign-straggler window: an in-flight submit still lands).
		p.mu.Lock()
		stop := p.done || p.err != nil
		p.mu.Unlock()
		if stop || ctx.Err() != nil {
			return false
		}

		proof, err := p.fetch(ctx, w.startKey, w.endKey, attempt, prevErr)
		if err != nil {
			p.mu.Lock()
			p.fail(err)
			p.mu.Unlock()
			return false
		}

		// NOT under mu (documented FFI contract): submit does real verify +
		// commit work and holds no Go lock while doing it.
		outcome, next, err := p.src.submitWork(w.id, proof)
		switch outcome {
		case submitContinue:
			w, attempt, prevErr = next, 0, nil
			if w.wakeupNeighbor {
				// ── REQUIREMENT 2: the producer publishes the shed cold
				// chunk to SyncState under the Rust mutex INSIDE submit (F1),
				// and submitWork has already returned — releasing that mutex
				// — before we take mu to Signal (F2). F1 happens-before F2
				// by construction; nothing more is needed.
				p.mu.Lock()
				p.cond.Signal()
				p.mu.Unlock()
			}
		case submitExhausted:
			return true // region done — reacquire
		case submitInvalid:
			// Hostile or buggy peer: same region, SAME id, zero Rust-side
			// state change. Re-fetch from a different peer; no retry cap
			// here — give-up policy belongs to the fetcher (it returns an
			// error to abort the sync).
			attempt++
			prevErr = err
		case submitFailed:
			p.mu.Lock()
			p.fail(err)
			p.mu.Unlock()
			return false
		}
	}
}
