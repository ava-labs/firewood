// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// Proof correctness (adversarial inputs, structural invariants, boundary
// conditions) is tested extensively on the Rust side in:
//   - firewood/src/merkle/tests/change.rs  (change proof verification)
//   - firewood/src/merkle/tests/range.rs   (range proof verification)
//   - firewood/src/merkle/tests/proof.rs   (single-key proofs)
//   - ffi/src/proofs/change.rs             (FFI-layer integration)
//
// Go-side tests focus on FFI concerns: error codes, serialization round-trips,
// handle lifecycle, keep-alive semantics, and parameter passing across the
// CGo boundary.

package ffi

import (
	"encoding/hex"
	"runtime"
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
	proofLen uint32,
) []byte {
	r := require.New(t)

	proof, err := db.ChangeProof(startRoot, endRoot, startKey, endKey, proofLen)
	r.NoError(err)

	proofBytes, err := proof.MarshalBinary()
	r.NoError(err)

	return proofBytes
}

// newProposedChangeProof creates a ProposedChangeProof from two databases that
// share the same initial state. It inserts additional data into dbA, creates a
// change proof, verifies it, and proposes it on dbB. No cleanup is registered
// so callers can control when the proof is freed (important for keep-alive tests).
func newProposedChangeProof(
	t *testing.T,
	dbA, dbB *Database,
) (*ProposedChangeProof, Hash) {
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

	proposed, err := dbB.VerifyAndProposeChangeProof(changeProof, rootB, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)

	return proposed, rootAUpdated
}

func TestRangeProofEmptyDB(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	proof, err := db.RangeProof(EmptyRoot, nothing(), nothing(), rangeProofLenUnbounded)
	r.ErrorIs(err, errRevisionNotFound)
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
	r.ErrorIs(err, errRevisionNotFound)
	r.Nil(proof)
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

func TestVerifyAndCommitRangeProof(t *testing.T) {
	r := require.New(t)

	// Create source and target databases
	dbSource := newTestDatabase(t)
	dbTarget := newTestDatabase(t)

	// Populate source
	_, _, batch := kvForTest(50)
	sourceRoot, err := dbSource.Update(batch)
	r.NoError(err)

	proof := newVerifiedRangeProof(t, dbSource, sourceRoot, nothing(), nothing(), rangeProofLenUnbounded)

	// Verify and commit to target without previously calling db.VerifyRangeProof
	committedRoot, err := dbTarget.VerifyAndCommitRangeProof(proof, nothing(), nothing(), sourceRoot, rangeProofLenUnbounded)
	r.NoError(err)
	r.Equal(sourceRoot, committedRoot)
}

func TestRangeProofCodeHashes(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// RLP encoded account with code hash
	key := [32]byte{0x12, 0x34, 0x56} // key must be length 32
	val, err := hex.DecodeString("f8440164a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a0044852b2a670ade5407e78fb2863c51de9fcb96542a07186fe3aeda6bb8a116d")
	r.NoError(err)
	codeHash := stringToHash(t, "044852b2a670ade5407e78fb2863c51de9fcb96542a07186fe3aeda6bb8a116d")

	root, err := db.Update([]BatchOp{Put(key[:], val)})
	r.NoError(err)

	proof := newVerifiedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenUnbounded)

	i := 0
	mode, err := inferHashingMode(t.Context())
	r.NoError(err)
	for h, err := range proof.CodeHashes() {
		i++
		if mode == ethhashKey {
			r.NoError(err, "%T.CodeHashes()", proof)
			r.Equal(codeHash, h)
		} else {
			require.ErrorContains(t, err, "feature not supported in this build: ethhash code hash iterator")
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

func TestRoundTripChangeProofSerialization(t *testing.T) {
	tests := []struct {
		name      string
		emptyDiff bool
	}{
		{"normal proof", false},
		{"empty diff proof", true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			r := require.New(t)
			db := newTestDatabase(t)

			// Insert some data.
			_, _, batch := kvForTest(10)
			root1, err := db.Update(batch[:5])
			r.NoError(err)

			var root2 Hash
			if tt.emptyDiff {
				// Re-insert the same data to create a second revision
				// with the same root hash (no actual changes).
				root2, err = db.Update(batch[:5])
				r.NoError(err)
			} else {
				root2, err = db.Update(batch[5:])
				r.NoError(err)
			}

			// get a proof
			proofBytes := newSerializedChangeProof(t, db, root1, root2, nothing(), nothing(), changeProofLenUnbounded)

			// Deserialize the proof.
			proof := new(ChangeProof)
			err = proof.UnmarshalBinary(proofBytes)
			r.NoError(err)
			t.Cleanup(func() { r.NoError(proof.Free()) })

			// serialize the proof again
			serialized, err := proof.MarshalBinary()
			r.NoError(err)
			r.Equal(proofBytes, serialized)
		})
	}
}

func TestVerifyAndProposeChangeProof(t *testing.T) {
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

	// Create a bounded change proof from dbA. Use end_key beyond all keys
	// so the proof covers the full range in one round.
	endKey := something([]byte("key9"))
	changeProof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), endKey, changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(changeProof.Free()) })

	// Verify and propose the change proof on dbB.
	proposedChangeProof, err := dbB.VerifyAndProposeChangeProof(changeProof, rootB, rootAUpdated, nothing(), endKey, changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proposedChangeProof.Free()) })
}

func TestVerifyAndProposeEmptyChangeProofRange(t *testing.T) {
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

	// Verify and propose the change proof on dbB.
	proposedChangeProof, err := dbB.VerifyAndProposeChangeProof(changeProof, rootB, rootAUpdated, startKey, endKey, 5)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proposedChangeProof.Free()) })
}

func TestVerifyAndCommitChangeProof(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	// Insert some data.
	_, _, batch := kvForTest(100)
	root, err := dbA.Update(batch[:50])
	r.NoError(err)
	_, err = dbB.Update(batch[:50])
	r.NoError(err)

	// Insert more data into dbA but not dbB.
	rootAUpdated, err := dbA.Update(batch[50:])
	r.NoError(err)

	// Create a bounded change proof from dbA. Use end_key beyond all keys
	// so the proof covers the full range in one round.
	endKey := something([]byte("key99"))
	changeProof, err := dbA.ChangeProof(root, rootAUpdated, nothing(), endKey, changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(changeProof.Free()) })

	// Verify and propose change proof on dbB.
	proposedChangeProof, err := dbB.VerifyAndProposeChangeProof(changeProof, root, rootAUpdated, nothing(), endKey, changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proposedChangeProof.Free()) })

	// Commit the proposal on dbB.
	rootBUpdated, err := proposedChangeProof.CommitChangeProof()
	r.NoError(err)
	r.Equal(rootAUpdated, rootBUpdated)
}

func TestProposedChangeProofKeepAlive(t *testing.T) {
	tests := []struct {
		name    string
		release func(*require.Assertions, *ProposedChangeProof)
	}{
		{
			// Free the proof (releases keep-alive)
			"free", func(r *require.Assertions, p *ProposedChangeProof) {
				r.NoError(p.Free())
			},
		},
		{
			// Commit the proof (releases keep-alive)
			"commit", func(r *require.Assertions, p *ProposedChangeProof) {
				_, err := p.CommitChangeProof()
				r.NoError(err)
			},
		},
		{
			// GC finalizer releases keep-alive
			"gc", func(_ *require.Assertions, p *ProposedChangeProof) {
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

			proposed, _ := newProposedChangeProof(t, dbA, dbB)

			// Database should not be closeable while proof has keep-alive
			r.ErrorIs(dbB.Close(oneSecCtx(t)), ErrActiveKeepAliveHandles)

			tt.release(r, proposed)

			// Database should now be closeable
			r.NoError(dbB.Close(oneSecCtx(t)))
		})
	}
}

func TestProposedChangeProofMarshalBinary(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	proposed, _ := newProposedChangeProof(t, dbA, dbB)
	t.Cleanup(func() { r.NoError(proposed.Free()) })

	// MarshalBinary should work on a ProposedChangeProof
	bytes, err := proposed.MarshalBinary()
	r.NoError(err)
	r.NotEmpty(bytes)
}

// newMismatchedChangeProof creates two databases with the same keys but
// different values, adds extra data to dbA, and returns a change proof from
// dbA that will fail verification on dbB (because the initial states differ).
func newMismatchedChangeProof(
	t *testing.T,
	dbA, dbB *Database,
) (changeProof *ChangeProof, rootB Hash, rootAUpdated Hash) {
	t.Helper()
	r := require.New(t)

	// Populate dbA and dbB with the SAME keys but DIFFERENT values
	keysA := make([]BatchOp, 50)
	keysB := make([]BatchOp, 50)
	for i := range 50 {
		key := keyForTest(i)
		keysA[i] = Put(key, []byte("valueA"+string(key)))
		keysB[i] = Put(key, []byte("valueB"+string(key)))
	}
	rootA, err := dbA.Update(keysA)
	r.NoError(err)
	rootB, err = dbB.Update(keysB)
	r.NoError(err)
	r.NotEqual(rootA, rootB, "roots should differ because values differ")

	// Add more data to dbA
	moreKeys := make([]BatchOp, 50)
	for i := range 50 {
		key := keyForTest(50 + i)
		moreKeys[i] = Put(key, valForTest(50+i))
	}
	rootAUpdated, err = dbA.Update(moreKeys)
	r.NoError(err)

	// Create a change proof from dbA
	changeProof, err = dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(changeProof.Free()) })

	return changeProof, rootB, rootAUpdated
}

func TestChangeProofMarshalAfterPropose(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	_, _, batch := kvForTest(10)
	rootA, err := dbA.Update(batch[:5])
	r.NoError(err)
	rootB, err := dbB.Update(batch[:5])
	r.NoError(err)

	rootAUpdated, err := dbA.Update(batch[5:])
	r.NoError(err)

	changeProof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(changeProof.Free()) })

	// Marshal before propose — should succeed
	bytes, err := changeProof.MarshalBinary()
	r.NoError(err)
	r.NotEmpty(bytes)

	// Propose consumes the handle
	proposed, err := dbB.VerifyAndProposeChangeProof(changeProof, rootB, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proposed.Free()) })

	// Marshal on consumed ChangeProof — should return errProofFreed, not errDBClosed
	_, err = changeProof.MarshalBinary()
	r.ErrorIs(err, errProofFreed)

	// Marshal on ProposedChangeProof — should still work
	bytes2, err := proposed.MarshalBinary()
	r.NoError(err)
	r.Equal(bytes, bytes2)
}

// TestVerifyAndProposeFailureKeepsChangeProof verifies that on failed
// verification, the original ChangeProof handle is still valid (can be freed
// or marshalled). This exercises the ProposedChangeProofResult::VerificationFailed
// path which returns the original ChangeProofContext to the Go side.
func TestVerifyAndProposeFailureKeepsChangeProof(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	changeProof, rootB, rootAUpdated := newMismatchedChangeProof(t, dbA, dbB)

	// Marshal before failed propose — should succeed
	bytesBefore, err := changeProof.MarshalBinary()
	r.NoError(err)
	r.NotEmpty(bytesBefore)

	// Attempt to verify and propose on dbB — should fail because dbB's
	// initial state differs from dbA's
	_, err = dbB.VerifyAndProposeChangeProof(changeProof, rootB, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.Error(err, "should fail: dbB has different initial data than dbA")

	// The original ChangeProof handle should still be valid after failure
	r.NotNil(changeProof.handle, "handle should not be nil after failed propose")

	// Marshal after failed propose — should still work
	bytesAfter, err := changeProof.MarshalBinary()
	r.NoError(err)
	r.Equal(bytesBefore, bytesAfter, "marshalled bytes should be identical")
}
