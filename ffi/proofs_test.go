// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"bytes"
	"encoding/hex"
	"runtime"
	"sync"
	"testing"

	"github.com/stretchr/testify/require"
)

const (
	rangeProofLenUnbounded  = 0
	rangeProofLenTruncated  = 10
	changeProofLenUnbounded = 0
	changeProofLenTruncated = 10
)

type maybe struct {
	value    []byte
	hasValue bool
}

func (m maybe) HasValue() bool {
	return m.hasValue
}

func (m maybe) Value() []byte {
	return m.value
}

func something(b []byte) maybe {
	return maybe{
		hasValue: true,
		value:    b,
	}
}

func nothing() maybe {
	return maybe{
		hasValue: false,
	}
}

// assertProofNotNil verifies that the given proof and its inner handle are not nil.
func assertProofNotNil(t *testing.T, proof *RangeProof) {
	t.Helper()
	r := require.New(t)
	r.NotNil(proof)
	r.NotNil(proof.handle)
}

// newVerifiedRangeProof generates a range proof for the given parameters and
// verifies using [RangeProof.Verify] which does not prepare a proposal. A
// cleanup is registered to free the proof when the test ends.
func newVerifiedRangeProof(
	t *testing.T,
	db *Database,
	root Hash,
	startKey, endKey maybe,
	proofLen uint32,
) *RangeProof {
	r := require.New(t)

	proof, err := db.RangeProof(root, startKey, endKey, proofLen)
	r.NoError(err)
	assertProofNotNil(t, proof)
	t.Cleanup(func() { r.NoError(proof.Free()) })

	r.NoError(proof.Verify(root, startKey, endKey, proofLen))

	return proof
}

// ethAccountWithCodeHash returns a 32-byte account key, the RLP-encoded account
// stored under it, and the code hash embedded in that account. Only
// account-length keys reach the code-hash extractor, so this is the minimal
// fixture that makes a proof yield exactly one code hash.
func ethAccountWithCodeHash(t *testing.T) ([32]byte, []byte, Hash) {
	t.Helper()

	key := [32]byte{0x12, 0x34, 0x56} // account keys must be 32 bytes
	val, err := hex.DecodeString("f8440164a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a0044852b2a670ade5407e78fb2863c51de9fcb96542a07186fe3aeda6bb8a116d")
	require.NoError(t, err)
	return key, val, stringToHash(t, "044852b2a670ade5407e78fb2863c51de9fcb96542a07186fe3aeda6bb8a116d")
}

// newSerializedRangeProof generates a range proof for the given parameters and
// returns its serialized bytes.
func newSerializedRangeProof(
	t *testing.T,
	db *Database,
	root Hash,
	startKey, endKey maybe,
	proofLen uint32,
) []byte {
	r := require.New(t)

	proof := newVerifiedRangeProof(t, db, root, startKey, endKey, proofLen)

	proofBytes, err := proof.MarshalBinary()
	r.NoError(err)

	return proofBytes
}

func newSerializedChangeProof(
	t *testing.T,
	db *Database,
	startRoot, endRoot Hash,
	startKey, endKey maybe,
) []byte {
	r := require.New(t)

	proof, err := db.ChangeProof(startRoot, endRoot, startKey, endKey, changeProofLenUnbounded)
	r.NoError(err)

	proofBytes, err := proof.MarshalBinary()
	r.NoError(err)

	return proofBytes
}

// newVerifiedChangeProof creates a Proposal from two databases that share the
// same initial state. It inserts additional data into dbA, creates a change
// proof, verifies it on dbB, and returns the proposal. No cleanup is registered
// on the proposal so callers can control when it is freed (important for
// keep-alive tests).
func newVerifiedChangeProof(
	t *testing.T,
	dbA, dbB *Database,
) (*Proposal, Hash) {
	t.Helper()
	r := require.New(t)

	_, _, batch := kvForTest(100)
	rootA, err := dbA.Update(batch[:50])
	r.NoError(err)
	rootB, err := dbB.Update(batch[:50])
	r.NoError(err)
	r.Equal(rootA, rootB)

	rootAUpdated, err := dbA.Update(batch[50:])
	r.NoError(err)

	changeProof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(changeProof.Free()) })

	proposal, err := dbB.VerifyChangeProof(changeProof, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)

	return proposal, rootAUpdated
}

func TestRangeProofEmptyDB(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	proof, err := db.RangeProof(EmptyRoot, nothing(), nothing(), rangeProofLenUnbounded)
	r.ErrorIs(err, ErrRevisionNotFound)
	r.Nil(proof)
}

func TestRangeProofNonExistentRoot(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// insert some data
	_, _, batch := kvForTest(100)
	root, err := db.Update(batch)
	r.NoError(err)

	// create a bogus root
	root[0] ^= 0xFF

	proof, err := db.RangeProof(root, nothing(), nothing(), rangeProofLenUnbounded)
	r.ErrorIs(err, ErrRevisionNotFound)
	r.Nil(proof)
}

func TestRangeProofPartialRange(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Insert a lot of data.
	_, _, batch := kvForTest(10000)
	root, err := db.Update(batch)
	r.NoError(err)

	// get a proof over some partial range
	proof1 := newSerializedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)

	// get a proof over a different range
	proof2 := newSerializedRangeProof(t, db, root, something([]byte("key2")), something([]byte("key3")), rangeProofLenTruncated)

	// ensure the proofs are different
	r.NotEqual(proof1, proof2)
}

func TestRangeProofDiffersAfterUpdate(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Insert some data.
	_, _, batch := kvForTest(100)
	root1, err := db.Update(batch[:50])
	r.NoError(err)

	// get a proof
	proof := newSerializedRangeProof(t, db, root1, nothing(), nothing(), rangeProofLenTruncated)

	// insert more data
	root2, err := db.Update(batch[50:])
	r.NoError(err)
	r.NotEqual(root1, root2)

	// get a proof again
	proof2 := newSerializedRangeProof(t, db, root2, nothing(), nothing(), rangeProofLenTruncated)

	// ensure the proofs are different
	r.NotEqual(proof, proof2)
}

func TestRoundTripSerialization(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Insert some data.
	_, _, batch := kvForTest(10)
	root, err := db.Update(batch)
	r.NoError(err)

	// get a proof
	proofBytes := newSerializedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenUnbounded)

	// Deserialize the proof.
	proof := new(RangeProof)
	err = proof.UnmarshalBinary(proofBytes)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proof.Free()) })

	// serialize the proof again
	serialized, err := proof.MarshalBinary()
	r.NoError(err)
	r.Equal(proofBytes, serialized)
}

func TestRangeProofVerify(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	root, err := db.Update(batch)
	r.NoError(err)

	// not using `newVerifiedRangeProof` so we can test Verify separately
	proof, err := db.RangeProof(root, nothing(), nothing(), rangeProofLenTruncated)
	r.NoError(err)

	// Database should be immediately closeable (no keep-alive)
	r.NoError(db.Close(oneSecCtx(t)))

	// Verify with wrong root should fail
	root[0] ^= 0xFF
	err = proof.Verify(root, nothing(), nothing(), rangeProofLenTruncated)
	r.Error(err, "Verification with wrong root should fail")
}

func TestVerifyAndCommitRangeProof(t *testing.T) {
	r := require.New(t)

	// Create source and target databases
	dbSource := newTestDatabase(t)
	dbTarget := newTestDatabase(t)

	// Populate source
	keys, vals, batch := kvForTest(50)
	sourceRoot, err := dbSource.Update(batch)
	r.NoError(err)

	proof := newVerifiedRangeProof(t, dbSource, sourceRoot, nothing(), nothing(), rangeProofLenUnbounded)

	// Verify and commit to target without previously calling db.VerifyRangeProof
	committedRoot, err := dbTarget.VerifyAndCommitRangeProof(proof, nothing(), nothing(), sourceRoot, rangeProofLenUnbounded)
	r.NoError(err)
	r.Equal(sourceRoot, committedRoot)

	// Verify all keys are now in target database
	for i, key := range keys {
		got, err := dbTarget.Get(key)
		r.NoError(err, "Get key %d", i)
		r.Equal(vals[i], got, "Value mismatch for key %d", i)
	}
}

func TestRangeProofFindNextKey(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	root, err := db.Update(batch)
	r.NoError(err)

	proof := newVerifiedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)

	// FindNextKey should fail before preparing a proposal or committing
	_, err = proof.FindNextKey()
	r.ErrorIs(err, errNotPrepared, "FindNextKey should fail on unverified proof")

	// Verify the proof
	r.NoError(db.VerifyRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated))

	// This proof was taken from db's own current state and verified against
	// itself, so db already held everything at `root` before any merge ran.
	// Bounding the merge to the proven range (rather than the full requested
	// range) means merging the truncated key-values changes nothing: they
	// already match, and the untouched tail past the proven edge is left
	// alone. The resulting proposal's root is therefore exactly `root`,
	// which is the FFI short-circuit's signal that the receiver is fully
	// caught up, so FindNextKey correctly returns nil — both here, before
	// commit, and after commit below, since the committed proposal is the
	// same already-matching one.
	//
	// This assertion previously read NotNil and only held because the
	// pre-fix apply path merged over the caller's unbounded end_key instead
	// of the proven edge, deleting the 90 keys past the proof's coverage and
	// corrupting db's root away from `root`. That corruption is what kept
	// this short-circuit from ever firing, so the assertion was passing for
	// the wrong reason. If this goes back to NotNil, the apply path is
	// deleting data outside the proven range again.
	nextRange, err := proof.FindNextKey()
	r.NoError(err)
	r.Nil(nextRange)

	_, err = db.VerifyAndCommitRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated)
	r.NoError(err)

	nextRange, err = proof.FindNextKey()
	r.NoError(err)
	r.Nil(nextRange)
}

// TestRangeProofFindNextKeyDivergentReceiver covers the case
// TestRangeProofFindNextKey no longer can: a receiver genuinely behind the
// proof's target root, rather than one self-proving state it already has.
// A truncated proof applied to an empty target must report more to fetch.
func TestRangeProofFindNextKeyDivergentReceiver(t *testing.T) {
	r := require.New(t)

	dbSource := newTestDatabase(t)
	dbTarget := newTestDatabase(t)

	// kvForTest sorts keys (and their paired vals) lexicographically, so
	// keys[:rangeProofLenTruncated] are exactly the trie-order prefix the
	// truncated proof below proves.
	keys, vals, batch := kvForTest(100)
	sourceRoot, err := dbSource.Update(batch)
	r.NoError(err)

	// dbTarget starts empty, so it is materially behind sourceRoot.
	proof := newVerifiedRangeProof(t, dbSource, sourceRoot, nothing(), nothing(), rangeProofLenTruncated)

	_, err = dbTarget.VerifyAndCommitRangeProof(proof, nothing(), nothing(), sourceRoot, rangeProofLenTruncated)
	r.NoError(err)

	// Smoke check that the merge actually writes the proven prefix into a
	// divergent target, not just that it avoids over-deleting (that guard is
	// TestRangeProofTruncatedDoesNotDeleteBeyondProvenEdge). This does NOT
	// exercise proven_end's bound at all: dbTarget starts empty, so its trie
	// iterator is exhausted before merge.rs's bound check ever runs, and
	// every key-value is applied unconditionally regardless of the bound's
	// value — see TestRangeProofTruncatedDeletesStaleKeyWithinProvenEdge for
	// a test that actually discriminates a too-tight proven_end.
	for i := range rangeProofLenTruncated {
		got, err := dbTarget.Get(keys[i])
		r.NoError(err, "Get key %d", i)
		r.Equal(vals[i], got, "key %d from the proven prefix was not applied", i)
	}

	// The truncated proof only proved a prefix, and dbTarget started with
	// none of the data, so there is genuinely more to fetch.
	nextRange, err := proof.FindNextKey()
	r.NoError(err)
	r.NotNil(nextRange)
	startKey := nextRange.StartKey()
	r.NotEmpty(startKey)
	r.NoError(nextRange.Free())
}

// TestRangeProofMethodFreeRace is a regression test for ava-labs/firewood#2137.
//
// Several (*RangeProof) methods read p.handle and pass it into a cgo call
// without holding p.lease.mu, so they are not serialized against
// (*RangeProof).Free, which frees the underlying Rust RangeProofContext under
// p.lease.mu. In production the racing Free is the GC finalizer that
// (*RangeProof) registers: once the proof becomes unreachable — which it is for
// the duration of the cgo call, since these methods never touch p again after
// loading the handle — the finalizer may run concurrently and free the context
// out from under the in-flight call. The result is a use-after-free: the issue
// reported "SIGSEGV ... signal arrived during cgo execution" inside
// fwd_range_proof_find_next_key (FindNextKey); Verify and MarshalBinary share
// the identical defect.
//
// The finalizer's exact timing cannot be forced from Go — the proof must be
// reachable to call a method on it, yet unreachable for the finalizer to run —
// so this test drives the identical concurrent code paths directly: the method
// and Free (the very method the finalizer calls) race on the same proof. Under
// -race — which CI runs the ffi suite under — the unsynchronized p.handle
// access is reported deterministically. Without -race the same race can surface
// as the reported SIGSEGV, but the timing window is narrow and does not
// reliably crash in a bounded run, so -race is the signal this test relies on.
// Guarding each method with p.lease.mu fixes all of them.
func TestRangeProofMethodFreeRace(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	root, err := db.Update(batch)
	r.NoError(err)

	// Cheaply mint fresh, independent RangeProofContexts by unmarshalling the
	// same serialized proof each iteration. UnmarshalBinary arms the Free
	// finalizer but attaches no database lease, so nothing keeps the handle
	// alive across the cgo call — exactly the production shape.
	proofBytes := newSerializedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)

	// Each op reads p.handle and passes it into a single cgo call. None may race
	// (*RangeProof).Free on p.handle. (CodeHashes is intentionally excluded: its
	// Rust iterator borrows from the proof and is consumed across multiple calls,
	// so it needs an iteration-spanning guard rather than this single-call one.)
	ops := []struct {
		name string
		call func(*RangeProof)
	}{
		{"FindNextKey", func(p *RangeProof) { _, _ = p.FindNextKey() }},
		{"Verify", func(p *RangeProof) {
			_ = p.Verify(root, nothing(), nothing(), rangeProofLenTruncated)
		}},
		{"MarshalBinary", func(p *RangeProof) { _, _ = p.MarshalBinary() }},
	}

	for _, op := range ops {
		t.Run(op.name, func(t *testing.T) {
			r := require.New(t)
			// -race flags the conflicting p.handle access as soon as one
			// iteration's method and Free overlap, which the start barrier makes
			// near-certain per iteration; a few thousand iterations is ample
			// margin while keeping the -race CI run fast.
			const iterations = 10_000
			for range iterations {
				p := new(RangeProof)
				r.NoError(p.UnmarshalBinary(proofBytes))

				start := make(chan struct{})
				var wg sync.WaitGroup
				wg.Add(2)
				go func() {
					defer wg.Done()
					<-start
					op.call(p) // reads p.handle across cgo, unsynchronized
				}()
				go func() {
					defer wg.Done()
					<-start
					_ = p.Free() // frees the Rust context (the finalizer's code path)
				}()
				close(start)
				wg.Wait()
			}
		})
	}
}

// TestRangeProofSetFinalizerReset guards against a double-SetFinalizer panic.
//
// runtime.SetFinalizer fatally panics ("finalizer already set") if it is called
// with a non-nil finalizer on an object that already has one. A RangeProof can
// legitimately reach a finalizer-setting path more than once: UnmarshalBinary
// documents that it overwrites existing contents (so it may be called again),
// and a from-bytes proof can then be prepared with Database.VerifyRangeProof —
// which also sets the finalizer. Because Free does not clear the finalizer,
// each of these sequences re-set the finalizer on the unfixed code and crashed
// the process. They must instead leave the proof with exactly one finalizer.
//
// (Calling VerifyRangeProof twice is a separate matter: it panics earlier on
// lease.attachUnregistered, so it is not exercised here.)
func TestRangeProofSetFinalizerReset(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	root, err := db.Update(batch)
	r.NoError(err)

	proofBytes := newSerializedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)

	tests := []struct {
		name string
		// run performs a sequence that reaches a finalizer-setting path twice
		// and returns the resulting proof for cleanup. It must not panic.
		run func(*require.Assertions) *RangeProof
	}{
		{"unmarshal_then_unmarshal", func(r *require.Assertions) *RangeProof {
			p := new(RangeProof)
			r.NoError(p.UnmarshalBinary(proofBytes))
			r.NoError(p.UnmarshalBinary(proofBytes))
			return p
		}},
		{"unmarshal_then_verify", func(r *require.Assertions) *RangeProof {
			p := new(RangeProof)
			r.NoError(p.UnmarshalBinary(proofBytes))
			r.NoError(db.VerifyRangeProof(p, nothing(), nothing(), root, rangeProofLenTruncated))
			return p
		}},
		{"verify_then_unmarshal", func(r *require.Assertions) *RangeProof {
			p, err := db.RangeProof(root, nothing(), nothing(), rangeProofLenTruncated)
			r.NoError(err)
			r.NoError(db.VerifyRangeProof(p, nothing(), nothing(), root, rangeProofLenTruncated))
			r.NoError(p.UnmarshalBinary(proofBytes))
			return p
		}},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			r := require.New(t)
			p := tt.run(r) // must not fatally panic with "finalizer already set"
			t.Cleanup(func() { _ = p.Free() })
		})
	}
}

// TestRangeProofVerifyTwiceIdempotent verifies that preparing the same proof
// against the same database more than once is a safe no-op rather than a panic
// ("lease already attached"), and that the redundant call does not leak a
// second keep-alive lease (which would leave Database.Close waiting on a
// phantom outstanding handle).
func TestRangeProofVerifyTwiceIdempotent(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	root, err := db.Update(batch)
	r.NoError(err)

	proof, err := db.RangeProof(root, nothing(), nothing(), rangeProofLenTruncated)
	r.NoError(err)

	r.NoError(db.VerifyRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated))
	r.NoError(db.VerifyRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated))

	// The proof is still usable after the redundant verify.
	nkr, err := proof.FindNextKey()
	r.NoError(err)
	if nkr != nil {
		r.NoError(nkr.Free())
	}

	// Exactly one lease was taken: a single Free drops the outstanding-handle
	// count to zero, so a graceful Close finds nothing to wait on. A leaked
	// second lease would make this Close return ErrActiveKeepAliveHandles.
	// (db.Close is idempotent, so newTestDatabase's cleanup close is a no-op.)
	r.NoError(proof.Free())
	r.NoError(db.Close(oneSecCtx(t)))
}

// TestCodeIteratorRetainsProof pins the lifetime contract between a code-hash
// iterator and the proof it borrows. Rust's CodeIteratorHandle<'p> wraps
// Box<dyn Iterator + 'p> over the proof's key-values, so the iterator must hold
// a Go reference to that proof; without one the proof is collectible the moment
// CodeHashes stops mentioning it, and its finalizer frees the data the iterator
// is still reading.
//
// The assertion is the reference itself rather than an observation of the
// garbage collector. A reachable object is never collected, so proving the
// reference exists proves the lifetime property outright -- deterministically,
// with no forced collections, no timing, and no tuned constants. Probing the GC
// can only ever fail to disprove the property within some arbitrary budget.
func TestCodeIteratorRetainsProof(t *testing.T) {
	if selectedHashMode != ethhashKey {
		t.Skip("code hash iterators are only created for ethereum-mode proofs")
	}

	tests := []struct {
		name string
		// newProof builds a proof over the account fixture and registers its
		// own cleanup. The concrete pointer is returned as the interface so
		// the assertion below compares what the iterator actually stored.
		newProof func(*testing.T, *require.Assertions, *Database) codeHashSource
	}{
		{
			"range",
			func(t *testing.T, r *require.Assertions, db *Database) codeHashSource {
				key, val, _ := ethAccountWithCodeHash(t)
				root, err := db.Update([]BatchOp{Put(key[:], val)})
				r.NoError(err)
				return newVerifiedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenUnbounded)
			},
		},
		{
			"change",
			func(t *testing.T, r *require.Assertions, db *Database) codeHashSource {
				// Baseline insert so the change proof's start root is non-empty.
				startRoot, err := db.Update([]BatchOp{Put([]byte("baseline"), []byte("v"))})
				r.NoError(err)
				key, val, _ := ethAccountWithCodeHash(t)
				endRoot, err := db.Update([]BatchOp{Put(key[:], val)})
				r.NoError(err)

				proof, err := db.ChangeProof(startRoot, endRoot, nothing(), nothing(), changeProofLenUnbounded)
				r.NoError(err)
				t.Cleanup(func() { r.NoError(proof.Free()) })
				return proof
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			r := require.New(t)
			db := newTestDatabase(t)
			proof := tt.newProof(t, r, db)

			it, err := proof.codeIterator()
			r.NoError(err)
			// Registered after the proof's own cleanup, so it runs first: the
			// borrow is released before the borrowed-from proof is freed.
			t.Cleanup(func() { r.NoError(it.Free()) })

			// A nil owner means the Rust borrow is backed by no Go reference
			// at all, which is the regression this test exists to catch;
			// check it separately so the failure says so.
			r.NotNil(it.owner, "%T.codeIterator() must retain the proof it borrows", proof)
			r.Same(proof, it.owner, "%T.codeIterator() retained the wrong proof", proof)
		})
	}
}

func TestRangeProofCodeHashes(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	key, val, codeHash := ethAccountWithCodeHash(t)
	root, err := db.Update([]BatchOp{Put(key[:], val)})
	r.NoError(err)

	proof := newVerifiedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenUnbounded)

	i := 0
	for h, err := range proof.CodeHashes() {
		i++
		if selectedHashMode == ethhashKey {
			r.NoError(err, "%T.CodeHashes()", proof)
			r.Equal(codeHash, h)
		} else {
			require.ErrorContains(t, err, "code hash iteration requires an ethereum-mode proof")
		}
	}

	require.Equalf(t, 1, i, "expected one yield from %T.CodeHashes()", proof)
}

func TestChangeProofCodeHashes(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Baseline insert so the change proof's start root is non-empty. The key
	// is shorter than 32 bytes so it is naturally skipped by the code-hash
	// extractor's account-key filter even if it were ever present in
	// batch_ops (it should not be, since it is unchanged in endRoot).
	startRoot, err := db.Update([]BatchOp{Put([]byte("baseline"), []byte("v"))})
	r.NoError(err)

	key, val, codeHash := ethAccountWithCodeHash(t)
	endRoot, err := db.Update([]BatchOp{Put(key[:], val)})
	r.NoError(err)

	proof, err := db.ChangeProof(startRoot, endRoot, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proof.Free()) })

	i := 0
	for h, err := range proof.CodeHashes() {
		i++
		if selectedHashMode == ethhashKey {
			r.NoError(err, "%T.CodeHashes()", proof)
			r.Equal(codeHash, h)
		} else {
			require.ErrorContains(t, err, "code hash iteration requires an ethereum-mode proof")
		}
	}

	require.Equalf(t, 1, i, "expected one yield from %T.CodeHashes()", proof)
}

func TestRangeProofFreeReleasesKeepAlive(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)
	_, _, batch := kvForTest(50)
	root, err := db.Update(batch)
	r.NoError(err)

	proof := newVerifiedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
	r.NoError(err)

	// prepare proposal (acquires keep-alive)
	r.NoError(db.VerifyRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated))

	// Database should not be closeable while proof has keep-alive
	r.ErrorIs(db.Close(oneSecCtx(t)), ErrActiveKeepAliveHandles)

	// Free the proof (releases keep-alive)
	r.NoError(proof.Free())

	// Database should now be closeable
	r.NoError(db.Close(oneSecCtx(t)))
}

func TestRangeProofCommitReleasesKeepAlive(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)
	_, _, batch := kvForTest(50)
	root, err := db.Update(batch)
	r.NoError(err)

	proof := newVerifiedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
	marshalledBeforeCommit, err := proof.MarshalBinary()
	r.NoError(err)

	// prepare proposal (acquires keep-alive)
	r.NoError(db.VerifyRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated))

	// Database should not be closeable while proof has keep-alive
	r.ErrorIs(db.Close(oneSecCtx(t)), ErrActiveKeepAliveHandles)

	// Commit the proof (releases keep-alive)
	_, err = db.VerifyAndCommitRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated)
	r.NoError(err)

	// Database should now be closeable
	r.NoError(db.Close(oneSecCtx(t)))

	marshalledAfterCommit, err := proof.MarshalBinary()
	r.NoError(err)

	// methods like MarshalBinary should still work after commit and closing the database
	r.Equal(marshalledBeforeCommit, marshalledAfterCommit)
}

// TestRangeProofFinalizerCleanup verifies that the finalizer properly releases
// the keep-alive handle when the proof goes out of scope.
func TestRangeProofFinalizerCleanup(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)
	_, _, batch := kvForTest(50)
	root, err := db.Update(batch)
	r.NoError(err)

	// note: this does not use newVerifiedRangeProof because it sets a cleanup
	// which retains a handle to the proof blocking our ability to wait for the
	// finalizer to run
	proof, err := db.RangeProof(root, nothing(), nothing(), rangeProofLenTruncated)
	r.NoError(err)
	assertProofNotNil(t, proof)

	// prepare proposal (acquires keep-alive)
	r.NoError(db.VerifyRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated))

	// Database should not be closeable while proof has keep-alive
	r.ErrorIs(db.Close(oneSecCtx(t)), ErrActiveKeepAliveHandles)

	runtime.KeepAlive(proof)
	proof = nil //nolint:ineffassign // necessary to drop the reference for GC
	runtime.GC()

	r.NoError(db.Close(t.Context()), "Database should be closeable after proof is garbage collected")
}

// TestChangeProofSetFinalizerReset guards against a double-SetFinalizer panic on
// ChangeProof, mirroring TestRangeProofSetFinalizerReset. Both
// Database.ChangeProof and UnmarshalBinary register the Free finalizer, and Free
// does not clear it, so a proof that reaches a finalizer-setting path twice (a
// from-create proof re-loaded via UnmarshalBinary, or a repeated UnmarshalBinary)
// fatally panicked with "finalizer already set". Each sequence must leave the
// proof with exactly one finalizer.
//
// (VerifyChangeProof does not set a finalizer, so there is no verify variant, and
// ChangeProof has no lease, so there is no lease-attach variant.)
func TestChangeProofSetFinalizerReset(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	startRoot, err := db.Update(batch[:50])
	r.NoError(err)
	endRoot, err := db.Update(batch)
	r.NoError(err)
	proofBytes := newSerializedChangeProof(t, db, startRoot, endRoot, nothing(), nothing())

	t.Run("create_then_unmarshal", func(t *testing.T) {
		r := require.New(t)
		p, err := db.ChangeProof(startRoot, endRoot, nothing(), nothing(), changeProofLenUnbounded)
		r.NoError(err)
		r.NoError(p.UnmarshalBinary(proofBytes))

		t.Cleanup(func() { _ = p.Free() })
	})

	t.Run("unmarshal_then_unmarshal", func(t *testing.T) {
		r := require.New(t)
		p := new(ChangeProof)
		r.NoError(p.UnmarshalBinary(proofBytes))
		r.NoError(p.UnmarshalBinary(proofBytes))

		t.Cleanup(func() { _ = p.Free() })
	})
}

func TestChangeProofEmptyDB(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	proof, err := db.ChangeProof(EmptyRoot, EmptyRoot, nothing(), nothing(), changeProofLenUnbounded)
	r.ErrorIs(err, ErrEndRevisionNotFound)
	r.Nil(proof)
}

func TestChangeProofCreation(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Insert first half of data in the first batch
	_, _, batch := kvForTest(10000)
	root1, err := db.Update(batch[:5000])
	r.NoError(err)

	// Insert the rest in the second batch
	root2, err := db.Update(batch[5000:])
	r.NoError(err)

	_, err = db.ChangeProof(root1, root2, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
}

func TestChangeProofDiffersAfterUpdate(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Insert 2500 entries in the first batch
	_, _, batch := kvForTest(10000)
	root1, err := db.Update(batch[:2500])
	r.NoError(err)

	// Insert 2500 more entries in the second batch
	root2, err := db.Update(batch[2500:5000])
	r.NoError(err)
	r.NotEqual(root1, root2)

	// Get a proof
	proof1 := newSerializedChangeProof(t, db, root1, root2, nothing(), nothing())
	r.NoError(err)

	// Insert more data
	root3, err := db.Update(batch[5000:])
	r.NoError(err)
	r.NotEqual(root2, root3)

	// Get a proof again
	proof2 := newSerializedChangeProof(t, db, root2, root3, nothing(), nothing())
	// Ensure the proofs are different
	r.NotEqual(proof1, proof2)
}

func TestRoundTripChangeProofSerialization(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Insert some data.
	_, _, batch := kvForTest(10)
	root1, err := db.Update(batch[:5])
	r.NoError(err)

	root2, err := db.Update(batch[5:])
	r.NoError(err)

	// get a proof
	proofBytes := newSerializedChangeProof(t, db, root1, root2, nothing(), nothing())

	// Deserialize the proof.
	proof := new(ChangeProof)
	err = proof.UnmarshalBinary(proofBytes)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proof.Free()) })

	// serialize the proof again
	serialized, err := proof.MarshalBinary()
	r.NoError(err)
	r.Equal(proofBytes, serialized)
}

func TestVerifyChangeProof(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	// Insert some data.
	_, _, batch := kvForTest(10)
	rootA, err := dbA.Update(batch[:5])
	r.NoError(err)
	rootB, err := dbB.Update(batch[:5])
	r.NoError(err)
	r.Equal(rootA, rootB)

	// Insert more data into dbA but not dbB.
	rootAUpdated, err := dbA.Update(batch[5:])
	r.NoError(err)

	// Create a change proof from dbA.
	changeProof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(changeProof.Free()) })

	// Verify the change proof and create a proposal on dbB.
	proposal, err := dbB.VerifyChangeProof(changeProof, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proposal.Drop()) })
}

func TestVerifyEmptyChangeProofRange(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	// Insert some data.
	_, _, batch := kvForTest(9)
	rootA, err := dbA.Update(batch[:5])
	r.NoError(err)
	rootB, err := dbB.Update(batch[:5])
	r.NoError(err)
	r.Equal(rootA, rootB)

	// Insert more data into dbA but not dbB.
	rootAUpdated, err := dbA.Update(batch[5:])
	r.NoError(err)

	startKey := maybe{
		hasValue: true,
		value:    []byte("key0"),
	}

	endKey := maybe{
		hasValue: true,
		value:    []byte("key1"),
	}

	// Create a change proof from dbA. This should create an empty changeProof because
	// the start and end keys are both from the first insert.
	changeProof, err := dbA.ChangeProof(rootA, rootAUpdated, startKey, endKey, 5)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(changeProof.Free()) })

	// Verify the change proof and create an empty proposal on dbB.
	proposal, err := dbB.VerifyChangeProof(changeProof, rootAUpdated, startKey, endKey, 5)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proposal.Drop()) })
}

func TestVerifyAndCommitChangeProof(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	// Insert some data.
	keys, vals, batch := kvForTest(100)
	root, err := dbA.Update(batch[:50])
	r.NoError(err)
	_, err = dbB.Update(batch[:50])
	r.NoError(err)

	// Insert more data into dbA but not dbB.
	rootAUpdated, err := dbA.Update(batch[50:])
	r.NoError(err)

	// Create a change proof from dbA.
	changeProof, err := dbA.ChangeProof(root, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(changeProof.Free()) })

	// Verify the change proof and create a proposal on dbB.
	proposal, err := dbB.VerifyChangeProof(changeProof, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)

	// Commit the proposal on dbB.
	rootBUpdated, err := proposal.CommitWithRebase()
	r.NoError(err)
	r.Equal(rootAUpdated, rootBUpdated)

	// Verify all keys are now in dbB
	for i, key := range keys {
		got, err := dbB.Get(key)
		r.NoError(err, "Get key %d", i)
		r.Equal(vals[i], got, "Value mismatch for key %d", i)
	}
}

func TestChangeProofFindNextKey(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	// Insert first half of data in the first batch
	_, _, batch := kvForTest(10000)
	rootA, err := dbA.Update(batch[:5000])
	r.NoError(err)

	_, err = dbB.Update(batch[:5000])
	r.NoError(err)

	// Insert the rest in the second batch
	rootAUpdated, err := dbA.Update(batch[5000:])
	r.NoError(err)

	proof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenTruncated)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proof.Free()) })

	// Verify the change proof and create a proposal on dbB.
	proposal, err := dbB.VerifyChangeProof(proof, rootAUpdated, nothing(), nothing(), changeProofLenTruncated)
	r.NoError(err)

	// FindNextKey is on the proof, not the proposal.
	nextRange, err := proof.FindNextKey(nothing())
	r.NoError(err)
	r.NotNil(nextRange)
	startKey := nextRange.StartKey()
	r.NotEmpty(startKey)
	r.NoError(nextRange.Free())

	// Commit the proposal on dbB.
	_, err = proposal.CommitWithRebase()
	r.NoError(err)

	// FindNextKey still works — it reads from the proof, not the proposal.
	nextRange, err = proof.FindNextKey(nothing())
	r.NoError(err)
	r.NotNil(nextRange)
	r.Equal(nextRange.StartKey(), startKey)
	r.NoError(nextRange.Free())
}

func TestChangeProofProposalKeepAlive(t *testing.T) {
	tests := []struct {
		name    string
		release func(*require.Assertions, *Proposal)
	}{
		{
			// Drop the proposal (releases keep-alive)
			"drop", func(r *require.Assertions, p *Proposal) {
				r.NoError(p.Drop())
			},
		},
		{
			// Commit the proposal (releases keep-alive)
			"commit", func(r *require.Assertions, p *Proposal) {
				_, err := p.CommitWithRebase()
				r.NoError(err)
			},
		},
		{
			// GC cleanup releases keep-alive
			"gc", func(_ *require.Assertions, p *Proposal) {
				runtime.KeepAlive(p)
				//nolint:ineffassign // necessary to drop the reference for GC
				p = nil
				runtime.GC()
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			r := require.New(t)
			dbA := newTestDatabase(t)
			dbB := newTestDatabase(t)

			proposal, _ := newVerifiedChangeProof(t, dbA, dbB)

			// Database should not be closeable while proposal has keep-alive
			r.ErrorIs(dbB.Close(oneSecCtx(t)), ErrActiveKeepAliveHandles)

			tt.release(r, proposal)

			// Database should now be closeable
			r.NoError(dbB.Close(oneSecCtx(t)))
		})
	}
}

func TestMultiRoundChangeProof(t *testing.T) {
	type TestStruct struct {
		name       string
		hasDeletes bool
	}

	tests := []TestStruct{
		{"Multi-round change proofs with no deletes", false},
		{"Multi-round change proofs With deletes", true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			r := require.New(t)
			dbA := newTestDatabase(t)
			dbB := newTestDatabase(t)

			// Insert first half of data in the first batch
			keys, vals, batch := kvForTest(100)
			rootA, err := dbA.Update(batch[:50])
			r.NoError(err)

			rootB, err := dbB.Update(batch[:50])
			r.NoError(err)

			// Insert the rest in the second batch
			rootAUpdated, err := dbA.Update(batch[50:])
			r.NoError(err)

			if tt.hasDeletes {
				// Delete some of the keys. This will create Delete BatchOps in the
				// change proof.
				delKeys := make([]BatchOp, 20)
				for i := range delKeys {
					keyIdx := i * 2
					delKeys[i] = Delete(keys[keyIdx])
					keys[keyIdx] = nil
				}
				rootAUpdated, err = dbA.Update(delKeys)
				r.NoError(err)
			}

			// Create and commit multiple change proofs to update dbB to match dbA.
			startKey := nothing()

			// Loop limit to help with debugging
			for range 10 {
				proof, err := dbA.ChangeProof(rootA, rootAUpdated, startKey, nothing(), changeProofLenTruncated)
				r.NoError(err)
				t.Cleanup(func() { r.NoError(proof.Free()) })

				// Verify the proof and create a proposal on dbB.
				proposal, err := dbB.VerifyChangeProof(proof, rootAUpdated, startKey, nothing(), changeProofLenTruncated)
				r.NoError(err)

				// Commit the proposal.
				rootB, err = proposal.CommitWithRebase()
				r.NoError(err)

				// Find the next start key from the proof.
				nextRange, err := proof.FindNextKey(nothing())
				r.NoError(err)
				if nextRange == nil {
					break
				}
				startKey = maybe{
					hasValue: true,
					value:    nextRange.StartKey(),
				}
				r.NoError(nextRange.Free())
			}

			// Verify that the root hashes match
			r.Equal(rootAUpdated, rootB)

			// Verify all keys are now in dbB. Skip over any keys that has been deleted.
			for i, key := range keys {
				if key == nil {
					continue
				}
				got, err := dbB.Get(key)
				r.NoError(err, "Get key %d", i)
				r.Equal(vals[i], got, "Value mismatch for %s", string(key))
			}
		})
	}
}

// TestChangeProofMarshalWorksAfterVerify verifies that MarshalBinary on a
// ChangeProof still works after verification, since VerifyChangeProof
// borrows the proof rather than consuming it.
func TestChangeProofMarshalWorksAfterVerify(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	// Insert some data.
	_, _, batch := kvForTest(10)
	rootA, err := dbA.Update(batch[:5])
	r.NoError(err)
	_, err = dbB.Update(batch[:5])
	r.NoError(err)

	// Insert more data into dbA.
	rootAUpdated, err := dbA.Update(batch[5:])
	r.NoError(err)

	// Create a change proof.
	changeProof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(changeProof.Free()) })

	// Marshal before verify — should succeed.
	marshalledBefore, err := changeProof.MarshalBinary()
	r.NoError(err)
	r.NotEmpty(marshalledBefore)

	// Verify the change proof.
	proposal, err := dbB.VerifyChangeProof(changeProof, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proposal.Drop()) })

	// Marshal after verify — should still succeed and produce the same bytes.
	marshalledAfter, err := changeProof.MarshalBinary()
	r.NoError(err)
	r.Equal(marshalledBefore, marshalledAfter)
}

// A truncated range proof proves only a prefix of the requested range. Applying
// it must not delete local keys past the proven edge — they are covered by no
// proof. Regression test for the apply path bounding its write to the proven
// edge rather than the requested end_key.
func TestRangeProofTruncatedDoesNotDeleteBeyondProvenEdge(t *testing.T) {
	r := require.New(t)

	dbSource := newTestDatabase(t)
	dbTarget := newTestDatabase(t)

	keys, vals, batch := kvForTest(50)

	sourceRoot, err := dbSource.Update(batch)
	r.NoError(err)

	// Seed the target with identical data, so it holds keys past the edge a
	// truncated reply will prove. Nothing here should be deleted.
	_, err = dbTarget.Update(batch)
	r.NoError(err)

	// Request the whole keyspace but cap the reply, forcing truncation.
	proof := newVerifiedRangeProof(t, dbSource, sourceRoot, nothing(), nothing(), rangeProofLenTruncated)

	_, err = dbTarget.VerifyAndCommitRangeProof(proof, nothing(), nothing(), sourceRoot, rangeProofLenTruncated)
	r.NoError(err)

	// Source and target held the same data, and the proof covered a prefix of
	// it, so every original key must survive. Before the fix the apply path
	// replaced the whole keyspace with the truncated reply and deleted the
	// tail.
	for i, key := range keys {
		got, err := dbTarget.Get(key)
		r.NoError(err, "Get key %d", i)
		r.Equal(vals[i], got, "key %d was deleted or altered by a proof that never covered it", i)
	}
}

// A truncated range proof proves that, within the proven prefix, only the
// key-values the proof carries exist — anything else there must be deleted.
// This is the mirror of TestRangeProofTruncatedDoesNotDeleteBeyondProvenEdge:
// that test guards against a bound looser than the proof justifies
// (over-deletion past the proven edge); this one guards against a bound
// tighter than the proof justifies (under-deletion within it). A too-tight
// proven_end stops the merge's trie scan before it reaches the synthetic
// stale key below, leaving it in place.
//
// TestRangeProofFindNextKeyDivergentReceiver's positive-apply check cannot
// catch this: merge.rs's bound only ever gates the trie-side scan
// (MergeKeyValueIter::new's stop_after_key), never the key-value side, and an
// empty target's trie iterator is exhausted before that scan even starts —
// every key-value gets applied regardless of the bound's value. A stale local
// key is the only way to observe the bound at all.
func TestRangeProofTruncatedDeletesStaleKeyWithinProvenEdge(t *testing.T) {
	r := require.New(t)

	dbSource := newTestDatabase(t)
	dbTarget := newTestDatabase(t)

	keys, _, batch := kvForTest(50)

	sourceRoot, err := dbSource.Update(batch)
	r.NoError(err)

	_, err = dbTarget.Update(batch)
	r.NoError(err)

	// A key present only in dbTarget: a proper extension of keys[0], which
	// therefore sorts strictly after it, and — verified below rather than
	// assumed, since kvForTest's lexicographic order need not match
	// insertion order — strictly before the proven edge at
	// keys[rangeProofLenTruncated-1]. It is absent from the proof's
	// key-values, so a correctly-bounded merge must delete it.
	staleKey := append(append([]byte{}, keys[0]...), 0)
	r.Negative(bytes.Compare(keys[0], staleKey), "synthetic key must sort after keys[0]")
	r.Negative(bytes.Compare(staleKey, keys[rangeProofLenTruncated-1]),
		"synthetic key must sort inside the proven range")
	_, err = dbTarget.Update([]BatchOp{Put(staleKey, []byte("stale"))})
	r.NoError(err)

	proof := newVerifiedRangeProof(t, dbSource, sourceRoot, nothing(), nothing(), rangeProofLenTruncated)

	_, err = dbTarget.VerifyAndCommitRangeProof(proof, nothing(), nothing(), sourceRoot, rangeProofLenTruncated)
	r.NoError(err)

	got, err := dbTarget.Get(staleKey)
	r.NoError(err)
	r.Nil(got, "stale key inside the proven range should have been deleted")
}
