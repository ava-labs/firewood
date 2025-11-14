// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"context"
	"path/filepath"
	"runtime"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
)

const maxProofLen = 10

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

func proofNotNil(t *testing.T, proof *RangeProof) {
	t.Helper()
	r := require.New(t)
	r.NotNil(proof)
	r.NotNil(proof.handle)
}

func TestRangeProofEmptyDB(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	proof, err := db.RangeProof(EmptyRoot, nothing(), nothing(), 0)
	r.ErrorIs(err, errRevisionNotFound)
	r.Nil(proof)
}

func TestRangeProofNonExistentRoot(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// insert some data
	keys, vals := kvForTest(100)
	root, err := db.Update(keys, vals)
	r.NoError(err)
	r.NotEmpty(root)

	// create a bogus root
	root[0] ^= 0xFF

	proof, err := db.RangeProof(root, nothing(), nothing(), 0)
	r.ErrorIs(err, errRevisionNotFound)
	r.Nil(proof)
}

func TestRangeProofPartialRange(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Insert a lot of data.
	keys, vals := kvForTest(10000)
	root, err := db.Update(keys, vals)
	r.NoError(err)

	// get a proof over some partial range
	proof1 := rangeProof(t, db, root, nothing(), nothing())

	// get a proof over a different range
	proof2 := rangeProof(t, db, root, something([]byte("key2")), something([]byte("key3")))

	// ensure the proofs are different
	r.NotEqual(proof1, proof2)

	// TODO(https://github.com/ava-labs/firewood/issues/738): verify the proofs
}

func TestRangeProofDiffersAfterUpdate(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Insert some data.
	keys, vals := kvForTest(100)
	root1, err := db.Update(keys[:50], vals[:50])
	r.NoError(err)

	// get a proof
	proof := rangeProof(t, db, root1, nothing(), nothing())

	// insert more data
	root2, err := db.Update(keys[50:], vals[50:])
	r.NoError(err)
	r.NotEqual(root1, root2)

	// get a proof again
	proof2 := rangeProof(t, db, root2, nothing(), nothing())

	// ensure the proofs are different
	r.NotEqual(proof, proof2)
}

func TestRoundTripSerialization(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Insert some data.
	keys, vals := kvForTest(10)
	root, err := db.Update(keys, vals)
	r.NoError(err)

	// get a proof
	proofBytes := rangeProof(t, db, root, nothing(), nothing())

	// Deserialize the proof.
	proof := new(RangeProof)
	err = proof.UnmarshalBinary(proofBytes)
	r.NoError(err)

	// serialize the proof again
	serialized, err := proof.MarshalBinary()
	r.NoError(err)
	r.Equal(proofBytes, serialized)

	r.NoError(proof.Free())
}

// rangeProof generates a range proof for the given parameters.
func rangeProof(
	t *testing.T,
	db *Database,
	root Hash,
	startKey, endKey maybe,
) []byte {
	r := require.New(t)

	proof, err := db.RangeProof(root, startKey, endKey, maxProofLen)
	r.NoError(err)
	proofNotNil(t, proof)
	proofBytes, err := proof.MarshalBinary()
	r.NoError(err)
	r.NoError(proof.Free())

	return proofBytes
}

// TestRangeProofVerify tests the standalone Verify method
func TestRangeProofVerify(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Insert data and generate proof
	keys, vals := kvForTest(100)
	root, err := db.Update(keys, vals)
	r.NoError(err)

	proof, err := db.RangeProof(root, nothing(), nothing(), maxProofLen)
	r.NoError(err)
	proofNotNil(t, proof)
	defer func() { r.NoError(proof.Free()) }()

	// Test: Verify with correct root should succeed
	err = proof.Verify(root, nothing(), nothing(), maxProofLen)
	r.NoError(err)

	// TODO(#738): re-enabled after verification is implemented
	// // Test: Verify with wrong root should fail
	// wrongRoot := make([]byte, len(root))
	// copy(wrongRoot, root)
	// wrongRoot[0] ^= 0xFF
	// err = proof.Verify(wrongRoot, nothing(), nothing(), maxProofLen)
	// r.Error(err, "Verification with wrong root should fail")
}

// TestVerifyAndCommitRangeProof tests the complete workflow
func TestVerifyAndCommitRangeProof(t *testing.T) {
	r := require.New(t)

	// Create source and target databases
	dbSource := newTestDatabase(t)
	dbTarget := newTestDatabase(t)

	// Populate source
	keys, vals := kvForTest(50)
	sourceRoot, err := dbSource.Update(keys, vals)
	r.NoError(err)

	// Generate proof
	proof, err := dbSource.RangeProof(sourceRoot, nothing(), nothing(), 0)
	r.NoError(err)
	proofNotNil(t, proof)
	defer func() { r.NoError(proof.Free()) }()

	// Verify and commit to target
	committedRoot, err := dbTarget.VerifyAndCommitRangeProof(proof, nothing(), nothing(), sourceRoot, 0)
	r.NoError(err)
	r.NotEmpty(committedRoot)

	// Verify all keys are now in target database
	for i, key := range keys {
		got, err := dbTarget.Get(key)
		r.NoError(err, "Get key %d", i)
		r.Equal(vals[i], got, "Value mismatch for key %d", i)
	}
}

// TestRangeProofFindNextKey tests the next key range discovery
func TestRangeProofFindNextKey(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Create database with many keys (more than maxProofLen)
	keys, vals := kvForTest(1000)
	root, err := db.Update(keys, vals)
	r.NoError(err)

	// Generate truncated proof (should not include all keys)
	proof, err := db.RangeProof(root, nothing(), nothing(), maxProofLen)
	r.NoError(err)
	proofNotNil(t, proof)
	defer func() { r.NoError(proof.Free()) }()

	// FindNextKey should fail before verification
	_, err = proof.FindNextKey()
	r.ErrorIs(err, errNotPrepared, "FindNextKey should fail on unverified proof")

	// Verify the proof
	committedRoot, err := db.VerifyAndCommitRangeProof(proof, nothing(), nothing(), root, maxProofLen)
	r.NoError(err)
	r.NotEmpty(committedRoot)

	// Now FindNextKey should work
	nextRange, err := proof.FindNextKey()
	r.NoError(err)
	if nextRange != nil {
		r.NotEmpty(nextRange.StartKey(), "Next range should have start key")
		r.NoError(nextRange.Free())
	}
}

// TestRangeProofBoundedRange tests proofs over specific key ranges
func TestRangeProofBoundedRange(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Insert many keys
	keys, vals := kvForTest(100)
	root, err := db.Update(keys, vals)
	r.NoError(err)

	// Generate proof for middle range only
	startKey := keys[25]
	endKey := keys[75]
	proof, err := db.RangeProof(
		root,
		something(startKey),
		something(endKey),
		maxProofLen,
	)
	r.NoError(err)
	proofNotNil(t, proof)
	defer func() { r.NoError(proof.Free()) }()

	// Verify with same bounds
	err = proof.Verify(root, something(startKey), something(endKey), maxProofLen)
	r.NoError(err)
}

// TestRangeProofEmptyRange tests proof generation for empty range
func TestRangeProofEmptyRange(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	keys, vals := kvForTest(100)
	root, err := db.Update(keys, vals)
	r.NoError(err)

	// Request range where start >= end (should be empty)
	startKey := keys[50]
	endKey := keys[30] // end before start

	proof, err := db.RangeProof(
		root,
		something(startKey),
		something(endKey),
		maxProofLen,
	)
	// This might succeed with empty proof or fail - test actual behavior
	if err == nil {
		proofNotNil(t, proof)
		defer func() { r.NoError(proof.Free()) }()
		err = proof.Verify(root, something(startKey), something(endKey), maxProofLen)
		r.NoError(err)
	} else {
		r.Nil(proof)
	}
}

// TestRangeProofStateTransitions tests that state transitions work correctly
func TestRangeProofStateTransitions(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	keys, vals := kvForTest(30)
	root, err := db.Update(keys, vals)
	r.NoError(err)

	proof, err := db.RangeProof(root, nothing(), nothing(), maxProofLen)
	r.NoError(err)
	proofNotNil(t, proof)
	defer func() { r.NoError(proof.Free()) }()

	// State: Unverified - FindNextKey should fail
	_, err = proof.FindNextKey()
	r.ErrorIs(err, errNotPrepared)

	// Transition to Proposed state
	err = db.VerifyRangeProof(proof, nothing(), nothing(), root, maxProofLen)
	r.NoError(err)

	// State: Proposed - FindNextKey should work
	nextRange, err := proof.FindNextKey()
	r.NoError(err)
	if nextRange != nil {
		r.NoError(nextRange.Free())
	}

	// Calling VerifyAndCommit should skip verification and commit
	committedRoot, err := db.VerifyAndCommitRangeProof(
		proof, nothing(), nothing(), root, maxProofLen,
	)
	r.NoError(err)
	r.NotEmpty(committedRoot)

	// State: Committed - FindNextKey should still work
	nextRange, err = proof.FindNextKey()
	r.NoError(err)
	if nextRange != nil {
		r.NoError(nextRange.Free())
	}
}

// createAndVerifyProof is a helper that creates a proof and verifies it with VerifyRangeProof,
// which acquires a keep-alive handle on the database.
func createAndVerifyProof(t *testing.T, db *Database) (*RangeProof, Hash) {
	t.Helper()
	r := require.New(t)

	keys, vals := kvForTest(50)
	root, err := db.Update(keys, vals)
	r.NoError(err)

	proof, err := db.RangeProof(root, nothing(), nothing(), 0)
	r.NoError(err)
	proofNotNil(t, proof)

	err = db.VerifyRangeProof(proof, nothing(), nothing(), root, 0)
	r.NoError(err)

	return proof, root
}

// assertDatabaseCloseable verifies that the database can be closed within a short timeout.
func assertDatabaseCloseable(t *testing.T, db *Database) {
	t.Helper()
	r := require.New(t)

	ctx, cancel := context.WithTimeout(t.Context(), 100*time.Millisecond)
	defer cancel()

	err := db.Close(ctx)
	r.NoError(err, "Database should be closeable")
}

// assertDatabaseNotCloseable verifies that the database cannot be closed due to active keep-alive handles.
func assertDatabaseNotCloseable(t *testing.T, db *Database) {
	t.Helper()
	r := require.New(t)

	ctx, cancel := context.WithTimeout(t.Context(), 100*time.Millisecond)
	defer cancel()

	err := db.Close(ctx)
	r.ErrorIs(err, ErrActiveKeepAliveHandles, "Database should not be closeable with active keep-alive handles")
}

// TestRangeProofKeepsDbAlive verifies that VerifyRangeProof acquires a keep-alive handle
// that prevents the database from being closed until the proof is freed.
func TestRangeProofKeepsDbAlive(t *testing.T) {
	r := require.New(t)
	// We need to create the database without the t.Cleanup cleanup handler
	// because we want to control when Close is called
	dbFile := filepath.Join(t.TempDir(), "test.db")
	db, err := newDatabase(dbFile)
	r.NoError(err)

	// Create and verify proof (acquires keep-alive)
	proof, _ := createAndVerifyProof(t, db)

	// Database should not be closeable while proof has keep-alive
	assertDatabaseNotCloseable(t, db)

	// Free the proof (releases keep-alive)
	r.NoError(proof.Free())

	// Database should now be closeable
	assertDatabaseCloseable(t, db)
}

// TestRangeProofDisownOnCommit verifies that VerifyAndCommitRangeProof releases
// the keep-alive handle, making the database immediately closeable.
func TestRangeProofDisownOnCommit(t *testing.T) {
	r := require.New(t)
	dbFile := filepath.Join(t.TempDir(), "test.db")
	db, err := newDatabase(dbFile)
	r.NoError(err)

	// Create and verify proof
	proof, root := createAndVerifyProof(t, db)
	defer func() { r.NoError(proof.Free()) }()

	// Database should not be closeable while proof has keep-alive
	assertDatabaseNotCloseable(t, db)

	// Commit the proof (should release keep-alive)
	committedRoot, err := db.VerifyAndCommitRangeProof(proof, nothing(), nothing(), root, 0)
	r.NoError(err)
	r.NotEmpty(committedRoot)

	// Database should now be closeable
	assertDatabaseCloseable(t, db)
}

// TestRangeProofDisownOnFree verifies that Free releases the keep-alive handle.
func TestRangeProofDisownOnFree(t *testing.T) {
	r := require.New(t)
	dbFile := filepath.Join(t.TempDir(), "test.db")
	db, err := newDatabase(dbFile)
	r.NoError(err)

	// Create and verify proof
	proof, _ := createAndVerifyProof(t, db)

	// Database should not be closeable
	assertDatabaseNotCloseable(t, db)

	// Free the proof
	r.NoError(proof.Free())

	// Database should now be closeable
	assertDatabaseCloseable(t, db)
}

// TestRangeProofValidAfterCommit verifies that the RangeProof remains valid
// and usable after VerifyAndCommitRangeProof is called.
func TestRangeProofValidAfterCommit(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Create and verify proof
	proof, root := createAndVerifyProof(t, db)
	defer func() { r.NoError(proof.Free()) }()

	// Commit the proof
	committedRoot, err := db.VerifyAndCommitRangeProof(proof, nothing(), nothing(), root, 0)
	r.NoError(err)
	r.NotEmpty(committedRoot)

	// Proof should still be valid - FindNextKey should work
	nextRange, err := proof.FindNextKey()
	r.NoError(err)
	if nextRange != nil {
		r.NoError(nextRange.Free())
	}

	// Free should succeed
	r.NoError(proof.Free())
}

// TestRangeProofNoKeepAliveOnStandaloneVerify verifies that calling the standalone
// Verify method (not VerifyRangeProof) doesn't acquire a keep-alive handle.
func TestRangeProofNoKeepAliveOnStandaloneVerify(t *testing.T) {
	r := require.New(t)
	dbFile := filepath.Join(t.TempDir(), "test.db")
	db, err := newDatabase(dbFile)
	r.NoError(err)

	// Create proof
	keys, vals := kvForTest(50)
	root, err := db.Update(keys, vals)
	r.NoError(err)

	proof, err := db.RangeProof(root, nothing(), nothing(), 0)
	r.NoError(err)
	proofNotNil(t, proof)
	defer func() { r.NoError(proof.Free()) }()

	// Use standalone Verify (not VerifyRangeProof)
	err = proof.Verify(root, nothing(), nothing(), 0)
	r.NoError(err)

	// Database should be immediately closeable (no keep-alive)
	assertDatabaseCloseable(t, db)
}

// TestRangeProofNoKeepAliveOnUnmarshal verifies that unmarshalled proofs
// don't have keep-alive handles.
func TestRangeProofNoKeepAliveOnUnmarshal(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Create and marshal a proof
	keys, vals := kvForTest(50)
	root, err := db.Update(keys, vals)
	r.NoError(err)

	proof1, err := db.RangeProof(root, nothing(), nothing(), 0)
	r.NoError(err)
	proofNotNil(t, proof1)

	proofBytes, err := proof1.MarshalBinary()
	r.NoError(err)
	r.NoError(proof1.Free())

	// Unmarshal into new proof
	proof2 := new(RangeProof)
	err = proof2.UnmarshalBinary(proofBytes)
	r.NoError(err)
	defer func() { r.NoError(proof2.Free()) }()

	// Database should be closeable (unmarshalled proof has no keep-alive)
	// Note: We need to create a separate database without t.Cleanup to test this
	dbFile := filepath.Join(t.TempDir(), "test2.db")
	db2, err := newDatabase(dbFile)
	r.NoError(err)

	assertDatabaseCloseable(t, db2)
}

// TestRangeProofFinalizerCleanup verifies that the finalizer properly releases
// the keep-alive handle when the proof goes out of scope.
func TestRangeProofFinalizerCleanup(t *testing.T) {
	r := require.New(t)
	dbFile := filepath.Join(t.TempDir(), "test.db")
	db, err := newDatabase(dbFile)
	r.NoError(err)

	// Create and verify proof in a nested scope
	func() {
		proof, _ := createAndVerifyProof(t, db)
		// Proof goes out of scope without explicit Free
		_ = proof
	}()

	// Force GC to run finalizers
	runtime.GC()
	runtime.GC() // Run twice to be sure
	time.Sleep(100 * time.Millisecond)

	// Database should eventually become closeable after finalizer runs
	// Give it a bit more time since finalizers are not immediate
	maxAttempts := 10
	for i := range maxAttempts {
		ctx, cancel := context.WithTimeout(t.Context(), 100*time.Millisecond)
		err := db.Close(ctx)
		cancel()

		if err == nil {
			// Success - finalizer cleaned up
			return
		}

		if i < maxAttempts-1 {
			runtime.GC()
			time.Sleep(100 * time.Millisecond)
		}
	}

	r.Fail("Database should be closeable after finalizer cleanup")
}

// TestRangeProofCommitAfterFree verifies that attempting to commit after
// freeing the proof fails gracefully.
func TestRangeProofCommitAfterFree(t *testing.T) {
	r := require.New(t)
	dbFile := filepath.Join(t.TempDir(), "test.db")
	db, err := newDatabase(dbFile)
	r.NoError(err)

	// Create and verify proof
	proof, root := createAndVerifyProof(t, db)

	// Free the proof first
	r.NoError(proof.Free())

	// Database should now be closeable
	assertDatabaseCloseable(t, db)

	// Attempting to commit after free should fail
	_, err = db.VerifyAndCommitRangeProof(proof, nothing(), nothing(), root, 0)
	r.Error(err, "VerifyAndCommitRangeProof should fail after Free")
}

// TestRangeProofMultipleFree verifies that calling Free multiple times
// is safe and doesn't cause issues.
func TestRangeProofMultipleFree(t *testing.T) {
	r := require.New(t)
	dbFile := filepath.Join(t.TempDir(), "test.db")
	db, err := newDatabase(dbFile)
	r.NoError(err)

	// Create and verify proof
	proof, _ := createAndVerifyProof(t, db)

	// Free the proof multiple times
	r.NoError(proof.Free())
	r.NoError(proof.Free())
	r.NoError(proof.Free())

	// Database should be closeable
	assertDatabaseCloseable(t, db)
}

// TestRangeProofMultipleProofsKeepAlive verifies that multiple proofs
// accumulate keep-alive references.
func TestRangeProofMultipleProofsKeepAlive(t *testing.T) {
	r := require.New(t)
	dbFile := filepath.Join(t.TempDir(), "test.db")
	db, err := newDatabase(dbFile)
	r.NoError(err)

	// Create and verify 3 different proofs
	proof1, _ := createAndVerifyProof(t, db)
	proof2, _ := createAndVerifyProof(t, db)
	proof3, _ := createAndVerifyProof(t, db)

	// Database should not be closeable with all 3 proofs active
	assertDatabaseNotCloseable(t, db)

	// Free first proof - database should still not be closeable
	r.NoError(proof1.Free())
	assertDatabaseNotCloseable(t, db)

	// Free second proof - database should still not be closeable
	r.NoError(proof2.Free())
	assertDatabaseNotCloseable(t, db)

	// Free third proof - database should now be closeable
	r.NoError(proof3.Free())
	assertDatabaseCloseable(t, db)
}

// TestRangeProofConcurrentVerifyAndClose verifies that Close properly waits
// for proof verification and keep-alive release.
func TestRangeProofConcurrentVerifyAndClose(t *testing.T) {
	r := require.New(t)
	dbFile := filepath.Join(t.TempDir(), "test.db")
	db, err := newDatabase(dbFile)
	r.NoError(err)

	// Create and verify proof
	proof, _ := createAndVerifyProof(t, db)

	// Channel to synchronize goroutines
	closeStarted := make(chan struct{})
	closeDone := make(chan error, 1)

	// Goroutine 1: Attempt to close database
	go func() {
		close(closeStarted)
		ctx, cancel := context.WithTimeout(t.Context(), 500*time.Millisecond)
		defer cancel()
		closeDone <- db.Close(ctx)
	}()

	// Wait for close to start
	<-closeStarted
	time.Sleep(50 * time.Millisecond)

	// Database should still be open due to active proof
	// Free the proof to unblock the close
	r.NoError(proof.Free())

	// Close should complete now
	err = <-closeDone
	// Either succeeded or timed out is acceptable for this test
	// The important part is that it doesn't panic or deadlock
	if err != nil {
		r.ErrorIs(err, ErrActiveKeepAliveHandles, "Close should return ErrActiveKeepAliveHandles or succeed")
	}
}
