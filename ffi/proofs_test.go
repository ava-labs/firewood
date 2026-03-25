// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"bytes"
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

	// TODO(#738): re-enable after verification is implemented
	// r.Error(err, "Verification with wrong root should fail")
	r.NoError(err)
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

	// Now FindNextKey should work
	nextRange, err := proof.FindNextKey()
	r.NoError(err)
	r.NotNil(nextRange)
	startKey := nextRange.StartKey()
	r.NotEmpty(startKey)
	startKey = append([]byte{}, startKey...) // copy to new slice to avoid use-after-free
	r.NoError(nextRange.Free())

	_, err = db.VerifyAndCommitRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated)
	r.NoError(err)

	// FindNextKey should still work after commit
	nextRange, err = proof.FindNextKey()
	r.NoError(err)
	r.NotNil(nextRange)
	r.Equal(nextRange.StartKey(), startKey)
	r.NoError(nextRange.Free())
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
	proof1 := newSerializedChangeProof(t, db, root1, root2, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)

	// Insert more data
	root3, err := db.Update(batch[5000:])
	r.NoError(err)
	r.NotEqual(root2, root3)

	// Get a proof again
	proof2 := newSerializedChangeProof(t, db, root2, root3, nothing(), nothing(), changeProofLenUnbounded)
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

	// Create a change proof from dbA.
	changeProof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(changeProof.Free()) })

	// Verify and propose the change proof on dbB.
	proposedChangeProof, err := dbB.VerifyAndProposeChangeProof(changeProof, rootB, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
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

	// Verify and propose change proof on dbB.
	proposedChangeProof, err := dbB.VerifyAndProposeChangeProof(changeProof, root, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proposedChangeProof.Free()) })

	// Commit the proposal on dbB.
	rootBUpdated, err := proposedChangeProof.CommitChangeProof()
	r.NoError(err)
	r.Equal(rootAUpdated, rootBUpdated)

	// Verify all keys are now in db2
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

	rootB, err := dbB.Update(batch[:5000])
	r.NoError(err)

	// Insert the rest in the second batch
	rootAUpdated, err := dbA.Update(batch[5000:])
	r.NoError(err)

	proof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenTruncated)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proof.Free()) })

	// Verify and propose change proof
	proposedChangeProof, err := dbB.VerifyAndProposeChangeProof(proof, rootB, rootAUpdated, nothing(), nothing(), changeProofLenTruncated)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proposedChangeProof.Free()) })

	// FindNextKey is available after creating a proposal.
	nextRange, err := proposedChangeProof.FindNextKey()
	r.NoError(err)
	r.NotNil(nextRange)
	startKey := nextRange.StartKey()
	r.NotEmpty(startKey)
	r.NoError(nextRange.Free())

	// Commit the proposal on dbB.
	_, err = proposedChangeProof.CommitChangeProof()
	r.NoError(err)

	// FindNextKey should still work after commit
	nextRange, err = proposedChangeProof.FindNextKey()
	r.NoError(err)
	r.NotNil(nextRange)
	r.Equal(nextRange.StartKey(), startKey)
	r.NoError(nextRange.Free())
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

// TestSubTrieHashCheckWithMismatchedSource creates a change proof on dbA, then
// attempts to apply it on dbB which has DIFFERENT initial data (same keys,
// different values). The proposal's sub-trie hashes won't match the boundary
// proof's claims, so we expect an error.
func TestSubTrieHashCheckWithMismatchedSource(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	changeProof, rootB, rootAUpdated := newMismatchedChangeProof(t, dbA, dbB)

	// Attempt to verify and propose on dbB — should fail because dbB's
	// initial state differs from dbA's
	_, err := dbB.VerifyAndProposeChangeProof(changeProof, rootB, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.Error(err, "should fail: dbB has different initial data than dbA")
}

// TestSubTrieHashCheckTruncatedProof creates a valid truncated change proof
// and verifies that VerifyAndProposeChangeProof succeeds — both sub-trie
// and boundary value checks pass for valid truncated proofs.
func TestSubTrieHashCheckTruncatedProof(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	// Insert shared initial data
	_, _, batch := kvForTest(100)
	rootA, err := dbA.Update(batch[:50])
	r.NoError(err)
	rootB, err := dbB.Update(batch[:50])
	r.NoError(err)
	r.Equal(rootA, rootB)

	// Insert more data into dbA
	rootAUpdated, err := dbA.Update(batch[50:])
	r.NoError(err)

	// Create a truncated change proof (fewer items than total changes)
	changeProof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenTruncated)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(changeProof.Free()) })

	// Verify and propose should succeed for valid truncated proof
	proposed, err := dbB.VerifyAndProposeChangeProof(changeProof, rootB, rootAUpdated, nothing(), nothing(), changeProofLenTruncated)
	r.NoError(err, "valid truncated proof should pass both sub-trie and boundary value checks")
	t.Cleanup(func() { r.NoError(proposed.Free()) })
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

				// Verify and propose the proof
				proposedProof, err := dbB.VerifyAndProposeChangeProof(proof, rootB, rootAUpdated, startKey, nothing(), changeProofLenTruncated)
				r.NoError(err)
				t.Cleanup(func() { r.NoError(proposedProof.Free()) })

				// Commit the proof
				rootB, err = proposedProof.CommitChangeProof()
				r.NoError(err)

				// Find the next start key
				nextRange, err := proposedProof.FindNextKey()
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

// ---------------------------------------------------------------------------
// Adversarial change proof verification tests
//
// Each test crafts an invalid proof (or invalid verification parameters) and
// asserts that the verifier rejects it with the expected error.
// ---------------------------------------------------------------------------

// TestChangeProofInvertedRange verifies that passing start_key > end_key is
// rejected as an invalid range before any other validation runs.
func TestChangeProofInvertedRange(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	_, _, batch := kvForTest(20)
	rootA, err := dbA.Update(batch[:10])
	r.NoError(err)
	rootB, err := dbB.Update(batch[:10])
	r.NoError(err)
	r.Equal(rootA, rootB)

	rootAUpdated, err := dbA.Update(batch[10:])
	r.NoError(err)

	proof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proof.Free()) })

	// start_key "z" > end_key "a" → InvalidRange
	_, err = dbB.VerifyAndProposeChangeProof(proof, rootB, rootAUpdated,
		something([]byte("z")), something([]byte("a")), changeProofLenUnbounded)
	r.Error(err)
	r.ErrorContains(err, "Invalid range")
}

// TestChangeProofDeleteRangeRejected injects a DeleteRange operation into a
// serialized proof by flipping a Delete discriminant byte and verifies that
// the verifier rejects it.
func TestChangeProofDeleteRangeRejected(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Insert initial data, then delete a key so the change proof contains a
	// Delete op whose serialized discriminant (0x01) we can flip to
	// DeleteRange (0x02).
	_, _, batch := kvForTest(20)
	root1, err := db.Update(batch)
	r.NoError(err)

	root2, err := db.Update([]BatchOp{Delete([]byte("key0"))})
	r.NoError(err)

	proof, err := db.ChangeProof(root1, root2, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)

	proofBytes, err := proof.MarshalBinary()
	r.NoError(err)
	r.NoError(proof.Free())

	// The Delete op for "key0" (4 bytes) is serialized as:
	//   0x01 (Delete) | 0x04 (varint key length) | 'k' 'e' 'y' '0'
	target := []byte{0x01, 0x04, 'k', 'e', 'y', '0'}
	idx := bytes.Index(proofBytes, target)
	r.GreaterOrEqual(idx, 0, "should find Delete('key0') in serialized proof")

	// Flip discriminant from Delete (0x01) to DeleteRange (0x02).
	// The two variants share the same wire format (tag + key), so
	// deserialization succeeds but verification rejects DeleteRange.
	mutated := append([]byte{}, proofBytes...)
	mutated[idx] = 0x02

	mutatedProof := new(ChangeProof)
	err = mutatedProof.UnmarshalBinary(mutated)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(mutatedProof.Free()) })

	_, err = db.VerifyAndProposeChangeProof(mutatedProof, root1, root2,
		nothing(), nothing(), changeProofLenUnbounded)
	r.Error(err)
	r.ErrorContains(err, "proof error")
}

// TestChangeProofExceedsMaxLength verifies that a proof with more batch ops
// than the specified max_length is rejected.
func TestChangeProofExceedsMaxLength(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	rootA, err := dbA.Update(batch[:50])
	r.NoError(err)
	rootB, err := dbB.Update(batch[:50])
	r.NoError(err)
	r.Equal(rootA, rootB)

	rootAUpdated, err := dbA.Update(batch[50:])
	r.NoError(err)

	// Create a proof with ~50 batch ops
	proof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proof.Free()) })

	// Verify with max_length=1, which is less than the number of ops
	_, err = dbB.VerifyAndProposeChangeProof(proof, rootB, rootAUpdated,
		nothing(), nothing(), 1)
	r.Error(err)
	r.ErrorContains(err, "proof error")
}

// TestChangeProofKeysNotSorted swaps two key values in a serialized proof to
// break the sort invariant and verifies the verifier catches it.
func TestChangeProofKeysNotSorted(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Create a proof with exactly two Put ops using short, known keys.
	// "mmm" is only in root1 so it won't appear in the diff.
	root1, err := db.Update([]BatchOp{Put([]byte("mmm"), []byte("v1"))})
	r.NoError(err)

	root2, err := db.Update([]BatchOp{
		Put([]byte("aaa"), []byte("va")),
		Put([]byte("bbb"), []byte("vb")),
	})
	r.NoError(err)

	proof, err := db.ChangeProof(root1, root2, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)

	proofBytes, err := proof.MarshalBinary()
	r.NoError(err)
	r.NoError(proof.Free())

	// For a complete proof (no bounds), boundary proofs are empty, so "aaa"
	// and "bbb" only appear in the batch_ops section. Swapping them in place
	// reverses the sort order: ["bbb","aaa"] is not sorted.
	idxA := bytes.Index(proofBytes, []byte("aaa"))
	idxB := bytes.Index(proofBytes, []byte("bbb"))
	r.Greater(idxA, 0, "should find 'aaa' in proof bytes")
	r.Greater(idxB, idxA, "'bbb' should come after 'aaa' in sorted proof")

	mutated := append([]byte{}, proofBytes...)
	copy(mutated[idxA:idxA+3], []byte("bbb"))
	copy(mutated[idxB:idxB+3], []byte("aaa"))

	mutatedProof := new(ChangeProof)
	err = mutatedProof.UnmarshalBinary(mutated)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(mutatedProof.Free()) })

	_, err = db.VerifyAndProposeChangeProof(mutatedProof, root1, root2,
		nothing(), nothing(), changeProofLenUnbounded)
	r.Error(err)
	r.ErrorContains(err, "proof error")
}

// TestChangeProofStartKeyOutOfBounds verifies that a start_key greater than
// the first batch op key is rejected.
func TestChangeProofStartKeyOutOfBounds(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	rootA, err := dbA.Update(batch[:50])
	r.NoError(err)
	rootB, err := dbB.Update(batch[:50])
	r.NoError(err)
	r.Equal(rootA, rootB)

	rootAUpdated, err := dbA.Update(batch[50:])
	r.NoError(err)

	proof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proof.Free()) })

	// The proof's first key is the smallest added key (a "key*" value).
	// "zzz" is lexicographically greater than any "key*" key.
	_, err = dbB.VerifyAndProposeChangeProof(proof, rootB, rootAUpdated,
		something([]byte("zzz")), nothing(), changeProofLenUnbounded)
	r.Error(err)
	r.ErrorContains(err, "proof error")
}

// TestChangeProofEndKeyOutOfBounds verifies that an end_key less than the
// last batch op key is rejected.
func TestChangeProofEndKeyOutOfBounds(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	rootA, err := dbA.Update(batch[:50])
	r.NoError(err)
	rootB, err := dbB.Update(batch[:50])
	r.NoError(err)
	r.Equal(rootA, rootB)

	rootAUpdated, err := dbA.Update(batch[50:])
	r.NoError(err)

	proof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proof.Free()) })

	// The proof's last key is the largest added key (a "key*" value).
	// "a" is lexicographically less than any "key*" key.
	_, err = dbB.VerifyAndProposeChangeProof(proof, rootB, rootAUpdated,
		nothing(), something([]byte("a")), changeProofLenUnbounded)
	r.Error(err)
	r.ErrorContains(err, "proof error")
}

// TestChangeProofMissingBoundaryProof creates a complete proof (no key
// bounds, so boundary proofs are empty) and then verifies it with bounds.
// The verifier should reject it because non-empty batch ops require at
// least one boundary proof when key bounds are specified.
func TestChangeProofMissingBoundaryProof(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	rootA, err := dbA.Update(batch[:50])
	r.NoError(err)
	rootB, err := dbB.Update(batch[:50])
	r.NoError(err)
	r.Equal(rootA, rootB)

	rootAUpdated, err := dbA.Update(batch[50:])
	r.NoError(err)

	// Complete proof: no key bounds → empty boundary proofs
	proof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proof.Free()) })

	// Verify with bounds — triggers MissingBoundaryProof because the proof
	// has non-empty batch ops but no start/end proofs.
	_, err = dbB.VerifyAndProposeChangeProof(proof, rootB, rootAUpdated,
		something([]byte("a")), something([]byte("z")), changeProofLenUnbounded)
	r.Error(err)
	r.ErrorContains(err, "proof error")
}

// TestChangeProofEndRootMismatch creates a valid complete proof and verifies
// it with a wrong end_root hash, so the computed root after applying the
// batch ops doesn't match the expected end root.
func TestChangeProofEndRootMismatch(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	rootA, err := dbA.Update(batch[:50])
	r.NoError(err)
	rootB, err := dbB.Update(batch[:50])
	r.NoError(err)
	r.Equal(rootA, rootB)

	rootAUpdated, err := dbA.Update(batch[50:])
	r.NoError(err)

	// Complete proof (no bounds)
	proof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proof.Free()) })

	// Flip a byte in end_root to make it wrong
	wrongEndRoot := rootAUpdated
	wrongEndRoot[0] ^= 0xFF

	_, err = dbB.VerifyAndProposeChangeProof(proof, rootB, wrongEndRoot,
		nothing(), nothing(), changeProofLenUnbounded)
	r.Error(err)
	r.ErrorContains(err, "proof error")
}

// TestChangeProofAsymmetricDepth exercises the (None, Some(e)) code path in
// verify_subtrie_hashes by choosing a short start_key and long end_key that
// resolve at different trie depths. The start proof has fewer nodes than the
// end proof, so the zip terminates early and the asymmetric arm fires.
func TestChangeProofAsymmetricDepth(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	// Insert keys at varying depths to create asymmetric proofs.
	// Short keys resolve at shallow depth; long keys resolve deeper
	// through extension nodes.
	initialBatch := []BatchOp{
		Put([]byte("\x00"), []byte("v0")),
		Put([]byte("\x00\x00\x01"), []byte("v1")),
		Put([]byte("\x00\x00\xff"), []byte("v2")),
		Put([]byte("\x01"), []byte("v3")),
		Put([]byte("\x01\x00\x01"), []byte("v4")),
	}
	rootA, err := dbA.Update(initialBatch)
	r.NoError(err)
	rootB, err := dbB.Update(initialBatch)
	r.NoError(err)
	r.Equal(rootA, rootB)

	// Add more data to dbA in the range between start_key and end_key
	extraBatch := []BatchOp{
		Put([]byte("\x00\x00\x02"), []byte("v5")),
		Put([]byte("\x00\x00\x80"), []byte("v6")),
	}
	rootAUpdated, err := dbA.Update(extraBatch)
	r.NoError(err)

	// Create a bounded change proof with start_key="\x00" (shallow)
	// and end_key="\x00\x00\xFF" (deep). The start proof should have
	// fewer nodes than the end proof, exercising the asymmetric arm.
	startKey := something([]byte("\x00"))
	endKey := something([]byte("\x00\x00\xff"))

	proof, err := dbA.ChangeProof(rootA, rootAUpdated, startKey, endKey, changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proof.Free()) })

	proposed, err := dbB.VerifyAndProposeChangeProof(proof, rootB, rootAUpdated, startKey, endKey, changeProofLenUnbounded)
	r.NoError(err, "asymmetric depth proof should pass verification")
	t.Cleanup(func() { r.NoError(proposed.Free()) })
}

// TestChangeProofAsymmetricDepthMismatch is the same setup as
// TestChangeProofAsymmetricDepth but with mismatched initial states (different
// values for the same keys). Verifies that the sub-trie hash check catches
// mismatches even with asymmetric proof depths.
func TestChangeProofAsymmetricDepthMismatch(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	// Same keys, different values → different initial root hashes
	initialBatchA := []BatchOp{
		Put([]byte("\x00"), []byte("vA0")),
		Put([]byte("\x00\x00\x01"), []byte("vA1")),
		Put([]byte("\x00\x00\xff"), []byte("vA2")),
		Put([]byte("\x01"), []byte("vA3")),
	}
	initialBatchB := []BatchOp{
		Put([]byte("\x00"), []byte("vB0")),
		Put([]byte("\x00\x00\x01"), []byte("vB1")),
		Put([]byte("\x00\x00\xff"), []byte("vB2")),
		Put([]byte("\x01"), []byte("vB3")),
	}
	rootA, err := dbA.Update(initialBatchA)
	r.NoError(err)
	rootB, err := dbB.Update(initialBatchB)
	r.NoError(err)
	r.NotEqual(rootA, rootB, "roots should differ because values differ")

	// Add more data to dbA
	extraBatch := []BatchOp{
		Put([]byte("\x00\x00\x02"), []byte("v5")),
		Put([]byte("\x00\x00\x80"), []byte("v6")),
	}
	rootAUpdated, err := dbA.Update(extraBatch)
	r.NoError(err)

	startKey := something([]byte("\x00"))
	endKey := something([]byte("\x00\x00\xff"))

	proof, err := dbA.ChangeProof(rootA, rootAUpdated, startKey, endKey, changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proof.Free()) })

	// Should fail because dbB's initial state differs from dbA's
	_, err = dbB.VerifyAndProposeChangeProof(proof, rootB, rootAUpdated, startKey, endKey, changeProofLenUnbounded)
	r.Error(err, "should fail: dbB has different initial data than dbA")
}
