// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"context"
	"errors"
	"fmt"
	"os"
	"runtime/pprof"
	"strconv"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
)

// poolWedgeTimeout is only a failure detector: causality in every test in
// this file is channel-enforced, never sleep-based. A healthy pool finishes
// these tests in milliseconds.
const poolWedgeTimeout = time.Minute

// syncFetcher returns a [FetchFunc] serving real serialized range proofs
// from src at root, truncated to maxLen entries.
func syncFetcher(src *Database, root Hash, maxLen uint32) FetchFunc {
	return func(_ context.Context, startKey, endKey Maybe[[]byte], _ int, _ error) ([]byte, error) {
		proof, err := src.RangeProof(root, startKey, endKey, maxLen)
		if err != nil {
			return nil, err
		}
		bytes, err := proof.MarshalBinary()
		return bytes, errors.Join(err, proof.Free())
	}
}

// awaitPool waits for a pool driven on the done channel, failing the test if
// it wedges. On a wedge, the goroutine stacks are dumped first (cancelling
// unblocks ctx-aware fetches but also destroys the evidence of whether
// workers were parked in cond.Wait or blocked in fetch), then ctx is
// cancelled so the pool goroutine can exit before the test reports.
func awaitPool(t *testing.T, cancel context.CancelFunc, done <-chan error) error {
	t.Helper()
	select {
	case err := <-done:
		return err
	case <-time.After(poolWedgeTimeout):
		_ = pprof.Lookup("goroutine").WriteTo(os.Stderr, 2)
		cancel()
		err := <-done
		require.Fail(t, "sync pool wedged (lost wakeup?)", "after-cancel result: %v", err)
		return err
	}
}

func TestSyncEndToEnd(t *testing.T) {
	r := require.New(t)
	src := newTestDatabase(t)
	keys, vals, batch := kvForTest(1000)
	srcRoot, err := src.Update(batch)
	r.NoError(err)

	dst := newTestDatabase(t)
	s, err := dst.StartSync(srcRoot, WithTaskLimit(4))
	r.NoError(err)
	t.Cleanup(func() { _ = s.Drop() })

	// maxLen 64 forces truncation: the Continue and shed paths are exercised.
	fetch := syncFetcher(src, srcRoot, 64)
	r.NoError(s.Run(t.Context(), fetch))

	// Run is single-shot.
	r.ErrorContains(s.Run(t.Context(), fetch), "once")

	root, err := s.Finish()
	r.NoError(err)
	r.Equal(srcRoot, root)

	// Root equality is the cryptographic check; spot-check a sample anyway.
	for i := 0; i < len(keys); i += 97 {
		got, err := dst.Get(keys[i])
		r.NoError(err)
		r.Equal(vals[i], got)
	}
}

func TestSyncTaskLimitOne(t *testing.T) {
	r := require.New(t)
	src := newTestDatabase(t)
	_, _, batch := kvForTest(300)
	srcRoot, err := src.Update(batch)
	r.NoError(err)

	dst := newTestDatabase(t)
	s, err := dst.StartSync(srcRoot, WithTaskLimit(1))
	r.NoError(err)
	t.Cleanup(func() { _ = s.Drop() })

	r.NoError(s.Run(t.Context(), syncFetcher(src, srcRoot, 32)))

	root, err := s.Finish()
	r.NoError(err)
	r.Equal(srcRoot, root)
}

func TestSyncInvalidProofRetry(t *testing.T) {
	// testRetry syncs 200 keys while `bad` replaces the very first fetched
	// proof; the pool must retry the same region (attempt > 0, prevErr set)
	// against the unmodified fetcher and still converge.
	testRetry := func(t *testing.T, bad func(good []byte, startKey, endKey Maybe[[]byte]) []byte) {
		r := require.New(t)
		src := newTestDatabase(t)
		keys, vals, batch := kvForTest(200)
		srcRoot, err := src.Update(batch)
		r.NoError(err)

		dst := newTestDatabase(t)
		s, err := dst.StartSync(srcRoot, WithTaskLimit(2))
		r.NoError(err)
		t.Cleanup(func() { _ = s.Drop() })

		inner := syncFetcher(src, srcRoot, 64)
		var calls, retries atomic.Int64
		fetch := func(ctx context.Context, startKey, endKey Maybe[[]byte], attempt int, prevErr error) ([]byte, error) {
			if attempt > 0 && prevErr != nil {
				retries.Add(1)
			}
			good, err := inner(ctx, startKey, endKey, attempt, prevErr)
			if err != nil {
				return nil, err
			}
			if calls.Add(1) == 1 {
				return bad(good, startKey, endKey), nil
			}
			return good, nil
		}

		r.NoError(s.Run(t.Context(), fetch))
		r.Positive(retries.Load(), "expected at least one InvalidProof retry")

		root, err := s.Finish()
		r.NoError(err)
		r.Equal(srcRoot, root)

		got, err := dst.Get(keys[0])
		r.NoError(err)
		r.Equal(vals[0], got)
	}

	t.Run("corrupted bytes", func(t *testing.T) {
		// Truncated serialization: exercises the parse -> InvalidProof path.
		testRetry(t, func(good []byte, _, _ Maybe[[]byte]) []byte {
			return good[:len(good)-1]
		})
	})

	t.Run("wrong root", func(t *testing.T) {
		// A structurally valid proof against a DIFFERENT root: exercises the
		// verify -> InvalidProof path.
		r := require.New(t)
		wrongSrc := newTestDatabase(t)
		keys, _, _ := kvForTest(200)
		wrongVals := make([][]byte, len(keys))
		for i := range wrongVals {
			wrongVals[i] = []byte("wrong" + strconv.Itoa(i))
		}
		wrongRoot, err := wrongSrc.Update(makeBatch(keys, wrongVals))
		r.NoError(err)

		testRetry(t, func(_ []byte, startKey, endKey Maybe[[]byte]) []byte {
			proof, err := wrongSrc.RangeProof(wrongRoot, startKey, endKey, 64)
			if err != nil {
				// Unparseable garbage also routes to InvalidProof, keeping
				// the retry assertion intact without failing from a non-test
				// goroutine.
				return []byte("unfetchable")
			}
			bytes, err := proof.MarshalBinary()
			if err2 := errors.Join(err, proof.Free()); err2 != nil {
				return []byte("unmarshalable")
			}
			return bytes
		})
	})
}

// TestSyncRestartAfterCancel is the Go-side crash-restart lifecycle through
// the real FFI: a fetcher cancels the sync's ctx after a fixed number of
// fetches, Run returns the ctx error, and a NEW handle on the same database
// restarts from committed revisions alone and completes to the target root.
// (Restart-from-partial-coverage nuances are owned by the Rust suite; this
// proves the handle lifecycle.)
func TestSyncRestartAfterCancel(t *testing.T) {
	r := require.New(t)
	src := newTestDatabase(t)
	keys, vals, batch := kvForTest(500)
	srcRoot, err := src.Update(batch)
	r.NoError(err)

	dst := newTestDatabase(t)
	s1, err := dst.StartSync(srcRoot, WithTaskLimit(2))
	r.NoError(err)
	t.Cleanup(func() { _ = s1.Drop() })

	// The fetcher cancels the ctx on the 5th fetch: a deterministic
	// mid-sync interruption (500 keys at 16 per proof need far more than 5
	// fetches, so the first run can never complete instead).
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()
	inner := syncFetcher(src, srcRoot, 16)
	var fetches atomic.Int64
	fetch := func(fctx context.Context, startKey, endKey Maybe[[]byte], attempt int, prevErr error) ([]byte, error) {
		if fetches.Add(1) == 5 {
			cancel()
			return nil, fctx.Err()
		}
		return inner(fctx, startKey, endKey, attempt, prevErr)
	}
	r.ErrorIs(s1.Run(ctx, fetch), context.Canceled)
	// Abandon the interrupted sync; only committed revisions survive.
	r.NoError(s1.Drop())

	s2, err := dst.StartSync(srcRoot, WithTaskLimit(2))
	r.NoError(err)
	t.Cleanup(func() { _ = s2.Drop() })
	r.NoError(s2.Run(t.Context(), syncFetcher(src, srcRoot, 16)))

	root, err := s2.Finish()
	r.NoError(err)
	r.Equal(srcRoot, root)

	// Root equality is the cryptographic check; spot-check a sample anyway.
	for i := 0; i < len(keys); i += 83 {
		got, err := dst.Get(keys[i])
		r.NoError(err)
		r.Equal(vals[i], got)
	}
}

func TestSyncFetchErrorAborts(t *testing.T) {
	r := require.New(t)
	src := newTestDatabase(t)
	_, _, batch := kvForTest(500)
	srcRoot, err := src.Update(batch)
	r.NoError(err)

	dst := newTestDatabase(t)
	s, err := dst.StartSync(srcRoot, WithTaskLimit(2))
	r.NoError(err)
	t.Cleanup(func() { _ = s.Drop() })

	errBoom := errors.New("no peers available")
	inner := syncFetcher(src, srcRoot, 16)
	var calls atomic.Int64
	fetch := func(ctx context.Context, startKey, endKey Maybe[[]byte], attempt int, prevErr error) ([]byte, error) {
		// 500 keys at 16 per proof need far more than two fetches, so the
		// error is reached on every schedule.
		if calls.Add(1) > 2 {
			return nil, errBoom
		}
		return inner(ctx, startKey, endKey, attempt, prevErr)
	}

	// Run returning (with the sentinel) is itself the proof that every
	// worker exited.
	r.ErrorIs(s.Run(t.Context(), fetch), errBoom)
}

func TestSyncWarmStartIsDoneImmediately(t *testing.T) {
	r := require.New(t)
	src := newTestDatabase(t)
	_, _, batch := kvForTest(100)
	srcRoot, err := src.Update(batch)
	r.NoError(err)

	// Pre-populate the destination to the target root.
	dst := newTestDatabase(t)
	dstRoot, err := dst.Update(batch)
	r.NoError(err)
	r.Equal(srcRoot, dstRoot)

	s, err := dst.StartSync(srcRoot)
	r.NoError(err)
	t.Cleanup(func() { _ = s.Drop() })

	var calls atomic.Int64
	fetch := func(context.Context, Maybe[[]byte], Maybe[[]byte], int, error) ([]byte, error) {
		calls.Add(1)
		return nil, errors.New("fetch must not be called on a warm start")
	}

	r.NoError(s.Run(t.Context(), fetch))
	r.Zero(calls.Load())

	root, err := s.Finish()
	r.NoError(err)
	r.Equal(srcRoot, root)
}

func TestSyncStartZeroTaskLimit(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	s, err := db.StartSync(Hash{0x01}, WithTaskLimit(0))
	r.Nil(s)
	r.ErrorContains(err, "task_limit must be nonzero")
}

func TestSyncKeepAliveBlocksClose(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	s, err := db.StartSync(Hash{0x01})
	r.NoError(err)

	err = db.Close(oneSecCtx(t))
	r.ErrorIs(err, ErrActiveKeepAliveHandles)

	r.NoError(s.Drop())
	r.NoError(db.Close(oneSecCtx(t)))
}

func TestSyncFinishIdempotentAndDropAfterFinish(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)
	_, _, batch := kvForTest(10)
	root, err := db.Update(batch)
	r.NoError(err)

	s, err := db.StartSync(root)
	r.NoError(err)

	got, err := s.Finish()
	r.NoError(err)
	r.Equal(root, got)

	// Drop after Finish is a no-op (no double free).
	r.NoError(s.Drop())

	// A second Finish reports the handle as gone.
	_, err = s.Finish()
	r.ErrorIs(err, errSyncDropped)

	// And Drop stays a no-op.
	r.NoError(s.Drop())
}

// scriptedSyncSource is an in-Go syncWorkSource fake. The closures run under
// its mutex; they must not block and must not reach back into the pool.
type scriptedSyncSource struct {
	mu        sync.Mutex
	onGetWork func() (getWorkOutcome, *work, error)
	onSubmit  func(id uint64, proof []byte) (submitOutcome, *work, error)
}

var _ syncWorkSource = (*scriptedSyncSource)(nil)

func (s *scriptedSyncSource) getWork() (getWorkOutcome, *work, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.onGetWork()
}

func (s *scriptedSyncSource) submitWork(id uint64, proof []byte) (submitOutcome, *work, error) {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.onSubmit(id, proof)
}

var fakeProof = []byte("proof")

// fakeWork builds a pool work item whose startKey doubles as a label the
// fake fetcher can dispatch on.
func fakeWork(id uint64, key string, wakeup bool) *work {
	return &work{id: id, startKey: someBytes(key), endKey: noBytes{}, wakeupNeighbor: wakeup}
}

// TestPoolParkWakeLiveness is the non-flaky lost-wakeup test. Worker 1 takes
// region A but its fetch is gated on worker 2 parking, so the park strictly
// precedes the submit (the fake's getWork closes the gate under the pool
// mutex immediately before cond.Wait's atomic release — the same guarantee
// TestPoolRecheckLoopAbsorbsBareSignal relies on). The submit publishes a
// shed region (C) and returns Continue(B, wakeup_neighbor=true), so worker 1
// Signals at a worker that is provably parked. The fetch for B blocks on a
// channel closed only by the fetch for C: if the Signal->wake chain works,
// worker 2 wakes, fetches C, and unblocks worker 1; if the Signal is lost,
// worker 2 stays parked, worker 1 is wedged in fetch(B), C is never fetched,
// and the test times out. Nothing else can wake worker 2 (no Done, no error,
// live ctx), which is what makes a lost shed-Signal wedge deterministically
// rather than schedule-dependently.
func TestPoolParkWakeLiveness(t *testing.T) {
	r := require.New(t)
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	var (
		handedFirst bool
		shed        bool
		handedShed  bool
		exhausted   int
		waitCount   int
	)
	parkedOnce := make(chan struct{})
	src := &scriptedSyncSource{}
	src.onGetWork = func() (getWorkOutcome, *work, error) {
		switch {
		case !handedFirst:
			handedFirst = true
			return gotWork, fakeWork(1, "A", false), nil
		case shed && !handedShed:
			handedShed = true
			return gotWork, fakeWork(3, "C", false), nil
		case exhausted == 2:
			return gotDone, nil, nil
		default:
			waitCount++
			if waitCount == 1 {
				close(parkedOnce)
			}
			return gotWait, nil, nil
		}
	}
	src.onSubmit = func(id uint64, _ []byte) (submitOutcome, *work, error) {
		if id == 1 {
			// Publish the shed region BEFORE returning, mirroring the Rust
			// side, which publishes to SyncState under its mutex inside
			// submit.
			shed = true
			return submitContinue, fakeWork(2, "B", true), nil
		}
		exhausted++
		return submitExhausted, nil, nil
	}

	thirdFetched := make(chan struct{})
	fetch := func(fctx context.Context, startKey, _ Maybe[[]byte], _ int, _ error) ([]byte, error) {
		switch string(startKey.Value()) {
		case "A":
			// Hold the first submit until worker 2 is parked: without this
			// gate, a fast worker 1 finishes A->B->C alone before worker 2
			// first asks for work, and a lost Signal goes undetected.
			select {
			case <-parkedOnce:
				return fakeProof, nil
			case <-fctx.Done():
				return nil, fctx.Err()
			}
		case "B":
			select {
			case <-thirdFetched:
				return fakeProof, nil
			case <-fctx.Done():
				return nil, fctx.Err()
			}
		case "C":
			close(thirdFetched)
			return fakeProof, nil
		default:
			return fakeProof, nil
		}
	}

	done := make(chan error, 1)
	go func() { done <- runPool(ctx, src, fetch, 2) }()
	r.NoError(awaitPool(t, cancel, done))
}

// TestPoolAcquireCascadeSignal pins the acquire-site cascade Signal: a
// region handed out by getWork with wakeup_neighbor set must wake a parked
// worker. In production losing this Signal only degrades to serialization
// (reacquiring workers eventually drain the cold work), so this script
// manufactures a schedule where completion REQUIRES the cascade: worker 2 is
// provably parked before worker 1 receives region A with wakeup set (the
// fake hands A out only after the first Wait, whose gate closes under the
// pool mutex just before cond.Wait's atomic release), and worker 1's fetch
// of A blocks until region B — reachable only by the woken worker 2 — has
// been fetched.
func TestPoolAcquireCascadeSignal(t *testing.T) {
	r := require.New(t)
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	var (
		aHanded   bool
		bHanded   bool
		exhausted int
		waitCount int
	)
	src := &scriptedSyncSource{}
	src.onGetWork = func() (getWorkOutcome, *work, error) {
		switch {
		case waitCount == 0:
			// The first asker parks; region A is not handed out until then,
			// and since getWork runs under the pool mutex (released only by
			// cond.Wait's atomic park), the cascade Signal always lands on
			// a worker that is already parked.
			waitCount++
			return gotWait, nil, nil
		case !aHanded:
			aHanded = true
			return gotWork, fakeWork(1, "A", true), nil
		case !bHanded:
			bHanded = true
			return gotWork, fakeWork(2, "B", false), nil
		case exhausted == 2:
			return gotDone, nil, nil
		default:
			waitCount++
			return gotWait, nil, nil
		}
	}
	src.onSubmit = func(_ uint64, _ []byte) (submitOutcome, *work, error) {
		exhausted++
		return submitExhausted, nil, nil
	}

	bFetched := make(chan struct{})
	fetch := func(fctx context.Context, startKey, _ Maybe[[]byte], _ int, _ error) ([]byte, error) {
		switch string(startKey.Value()) {
		case "A":
			// Completion requires worker 2 to wake and fetch B: if the
			// acquire-site Signal is lost, worker 2 stays parked, this
			// fetch never returns, and the test wedges.
			select {
			case <-bFetched:
				return fakeProof, nil
			case <-fctx.Done():
				return nil, fctx.Err()
			}
		case "B":
			close(bFetched)
			return fakeProof, nil
		default:
			return fakeProof, nil
		}
	}

	done := make(chan error, 1)
	go func() { done <- runPool(ctx, src, fetch, 2) }()
	r.NoError(awaitPool(t, cancel, done))
	r.True(bHanded, "the woken worker must have picked up region B")
	r.Equal(2, exhausted)
}

// TestPoolRecheckLoopAbsorbsBareSignal pins REQUIREMENT 3: a Signal with no
// work published (wakeup_neighbor set, nothing handed out) must be absorbed
// by the re-check loop — the woken worker gets Wait again and re-parks. The
// channels are closed by the fake's getWork, which runs under the pool
// mutex immediately before the worker parks (cond.Wait releases the mutex
// atomically), so anything gated on them that later takes the pool mutex is
// guaranteed to run with that worker parked.
func TestPoolRecheckLoopAbsorbsBareSignal(t *testing.T) {
	r := require.New(t)
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	var (
		handedFirst bool
		covered     bool
		waitCount   int
	)
	parkedOnce := make(chan struct{})
	parkedTwice := make(chan struct{})
	src := &scriptedSyncSource{}
	src.onGetWork = func() (getWorkOutcome, *work, error) {
		switch {
		case !handedFirst:
			handedFirst = true
			return gotWork, fakeWork(1, "A", false), nil
		case covered:
			return gotDone, nil, nil
		default:
			waitCount++
			switch waitCount {
			case 1:
				close(parkedOnce)
			case 2:
				close(parkedTwice)
			}
			return gotWait, nil, nil
		}
	}
	src.onSubmit = func(id uint64, _ []byte) (submitOutcome, *work, error) {
		if id == 1 {
			// BARE wakeup: wakeup_neighbor is set but no new work is
			// published — the next getWork still returns Wait.
			return submitContinue, fakeWork(2, "B", true), nil
		}
		covered = true
		return submitExhausted, nil, nil
	}

	fetch := func(fctx context.Context, startKey, _ Maybe[[]byte], _ int, _ error) ([]byte, error) {
		var gate <-chan struct{}
		switch string(startKey.Value()) {
		case "A":
			gate = parkedOnce // bare Signal is sent at a parked worker
		default:
			gate = parkedTwice // bare Signal was absorbed: worker re-parked
		}
		select {
		case <-gate:
			return fakeProof, nil
		case <-fctx.Done():
			return nil, fctx.Err()
		}
	}

	done := make(chan error, 1)
	go func() { done <- runPool(ctx, src, fetch, 2) }()
	r.NoError(awaitPool(t, cancel, done))

	// Exactly two Waits: the initial park and the re-park after the bare
	// Signal (Go condvars have no spurious wakeups; the Done broadcast is
	// absorbed by the done flag, not another getWork).
	r.Equal(2, waitCount)
}

func TestPoolInvalidProofKeepsSameID(t *testing.T) {
	r := require.New(t)
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	var (
		handedFirst bool
		invalids    int
		covered     bool
		submitted   []uint64
	)
	src := &scriptedSyncSource{}
	src.onGetWork = func() (getWorkOutcome, *work, error) {
		switch {
		case !handedFirst:
			handedFirst = true
			return gotWork, fakeWork(1, "A", false), nil
		case covered:
			return gotDone, nil, nil
		default:
			return gotWait, nil, nil
		}
	}
	src.onSubmit = func(id uint64, _ []byte) (submitOutcome, *work, error) {
		submitted = append(submitted, id)
		if invalids < 2 {
			invalids++
			return submitInvalid, nil, errors.New("bad proof")
		}
		covered = true
		return submitExhausted, nil, nil
	}

	type try struct {
		attempt int
		prevErr error
	}
	var tries []try // taskLimit is 1: no concurrent appends
	//nolint:unparam // the error result is fixed by the FetchFunc signature
	fetch := func(_ context.Context, _, _ Maybe[[]byte], attempt int, prevErr error) ([]byte, error) {
		tries = append(tries, try{attempt: attempt, prevErr: prevErr})
		return fakeProof, nil
	}

	done := make(chan error, 1)
	go func() { done <- runPool(ctx, src, fetch, 1) }()
	r.NoError(awaitPool(t, cancel, done))

	// The SAME id is resubmitted after each rejection; the fetcher saw the
	// attempt count climb with the rejection detail attached.
	r.Equal([]uint64{1, 1, 1}, submitted)
	r.Len(tries, 3)
	r.Equal(0, tries[0].attempt)
	r.NoError(tries[0].prevErr)
	r.Equal(1, tries[1].attempt)
	r.ErrorContains(tries[1].prevErr, "bad proof")
	r.Equal(2, tries[2].attempt)
	r.ErrorContains(tries[2].prevErr, "bad proof")
}

func TestPoolDoneBroadcastWakesAllParked(t *testing.T) {
	r := require.New(t)
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	var (
		handedFirst bool
		covered     bool
	)
	src := &scriptedSyncSource{}
	src.onGetWork = func() (getWorkOutcome, *work, error) {
		switch {
		case !handedFirst:
			handedFirst = true
			return gotWork, fakeWork(1, "A", false), nil
		case covered:
			return gotDone, nil, nil
		default:
			return gotWait, nil, nil
		}
	}
	src.onSubmit = func(uint64, []byte) (submitOutcome, *work, error) {
		covered = true
		return submitExhausted, nil, nil
	}
	fetch := func(context.Context, Maybe[[]byte], Maybe[[]byte], int, error) ([]byte, error) {
		return fakeProof, nil
	}

	// One region, eight workers: up to seven park and must all be woken by
	// the Done broadcast. Completion is the assertion.
	done := make(chan error, 1)
	go func() { done <- runPool(ctx, src, fetch, 8) }()
	r.NoError(awaitPool(t, cancel, done))
}

func TestPoolCtxCancelWakesParked(t *testing.T) {
	r := require.New(t)
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	var (
		handedFirst bool
		parkedOnce  bool
	)
	parked := make(chan struct{})
	src := &scriptedSyncSource{}
	src.onGetWork = func() (getWorkOutcome, *work, error) {
		if !handedFirst {
			handedFirst = true
			return gotWork, fakeWork(1, "A", false), nil
		}
		if !parkedOnce {
			parkedOnce = true
			close(parked)
		}
		return gotWait, nil, nil
	}
	src.onSubmit = func(uint64, []byte) (submitOutcome, *work, error) {
		return submitFailed, nil, errors.New("unexpected submit")
	}

	// The one region never completes: its fetch blocks until cancellation.
	fetch := func(fctx context.Context, _, _ Maybe[[]byte], _ int, _ error) ([]byte, error) {
		<-fctx.Done()
		return nil, fctx.Err()
	}

	done := make(chan error, 1)
	go func() { done <- runPool(ctx, src, fetch, 2) }()

	// Wait until the second worker has parked (Wait returned under the pool
	// mutex; the park completes before that mutex is released), then cancel.
	select {
	case <-parked:
	case <-time.After(poolWedgeTimeout):
		cancel()
		<-done
		r.Fail("worker never parked")
	}
	cancel()

	r.ErrorIs(awaitPool(t, cancel, done), context.Canceled)
}

func TestPoolErrAborts(t *testing.T) {
	r := require.New(t)
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	sentinel := errors.New("submit exploded")
	var handedFirst bool
	src := &scriptedSyncSource{}
	src.onGetWork = func() (getWorkOutcome, *work, error) {
		if !handedFirst {
			handedFirst = true
			return gotWork, fakeWork(1, "A", false), nil
		}
		return gotWait, nil, nil
	}
	src.onSubmit = func(uint64, []byte) (submitOutcome, *work, error) {
		return submitFailed, nil, sentinel
	}
	fetch := func(context.Context, Maybe[[]byte], Maybe[[]byte], int, error) ([]byte, error) {
		return fakeProof, nil
	}

	// First error wins and the second (parked) worker exits: Run returning
	// the sentinel is the assertion.
	done := make(chan error, 1)
	go func() { done <- runPool(ctx, src, fetch, 2) }()
	r.ErrorIs(awaitPool(t, cancel, done), sentinel)
}

func TestPoolCoverageRootMismatchSurfaced(t *testing.T) {
	r := require.New(t)
	ctx, cancel := context.WithCancel(t.Context())
	defer cancel()

	src := &scriptedSyncSource{}
	src.onGetWork = func() (getWorkOutcome, *work, error) {
		return gotWait, nil, fmt.Errorf("%w: target deadbeef", ErrCoverageRootMismatch)
	}
	src.onSubmit = func(uint64, []byte) (submitOutcome, *work, error) {
		return submitFailed, nil, errors.New("unexpected submit")
	}
	fetch := func(context.Context, Maybe[[]byte], Maybe[[]byte], int, error) ([]byte, error) {
		return fakeProof, nil
	}

	done := make(chan error, 1)
	go func() { done <- runPool(ctx, src, fetch, 1) }()
	r.ErrorIs(awaitPool(t, cancel, done), ErrCoverageRootMismatch)
}
