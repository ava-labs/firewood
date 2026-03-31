// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"testing"

	"github.com/stretchr/testify/require"
)

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
