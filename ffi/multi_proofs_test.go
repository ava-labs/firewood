// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"testing"

	"github.com/stretchr/testify/require"
)

// TestMultiRangeProof creates a range proof from a multi-head database.
func TestMultiRangeProof(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))

	_, _, batch := kvForTest(50)
	root, err := db.Update(0, batch)
	r.NoError(err)

	proof, err := db.RangeProof(root, nothing(), nothing(), rangeProofLenUnbounded)
	r.NoError(err)
	r.NotNil(proof)
	t.Cleanup(func() { r.NoError(proof.Free()) })
}

// TestMultiRangeProofEmptyDB verifies that requesting a range proof with
// EmptyRoot on an empty multi-head database returns errRevisionNotFound.
func TestMultiRangeProofEmptyDB(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))

	proof, err := db.RangeProof(EmptyRoot, nothing(), nothing(), rangeProofLenUnbounded)
	r.ErrorIs(err, errRevisionNotFound)
	r.Nil(proof)
}

// TestMultiChangeProof creates a change proof between two committed revisions.
func TestMultiChangeProof(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))

	_, _, batch := kvForTest(50)
	root1, err := db.Update(0, batch[:25])
	r.NoError(err)

	root2, err := db.Update(0, batch[25:])
	r.NoError(err)
	r.NotEqual(root1, root2)

	proof, err := db.ChangeProof(root1, root2, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	r.NotNil(proof)
	t.Cleanup(func() { r.NoError(proof.Free()) })
}

// TestMultiVerifyAndCommitRangeProof does a full round-trip: create a proof
// from one validator, verify+commit it for another.
func TestMultiVerifyAndCommitRangeProof(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))
	r.NoError(db.RegisterValidator(1))

	// Populate validator 0
	keys, vals, batch := kvForTest(50)
	sourceRoot, err := db.Update(0, batch)
	r.NoError(err)

	// Create proof from validator 0's state
	proof, err := db.RangeProof(sourceRoot, nothing(), nothing(), rangeProofLenUnbounded)
	r.NoError(err)
	r.NotNil(proof)

	// Verify and commit for validator 1
	committedRoot, err := db.VerifyAndCommitRangeProof(
		proof, nothing(), nothing(), sourceRoot, rangeProofLenUnbounded, 1,
	)
	r.NoError(err)
	r.Equal(sourceRoot, committedRoot)

	// Verify all keys are now visible from validator 1
	r.NoError(db.AdvanceToHash(1, committedRoot))
	for i, key := range keys {
		got, err := db.Get(1, key)
		r.NoError(err, "Get key %d", i)
		r.Equal(vals[i], got, "Value mismatch for key %d", i)
	}
}

// TestMultiProposeAndCommitChangeProof does a full round-trip for change proofs.
func TestMultiProposeAndCommitChangeProof(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))
	r.NoError(db.RegisterValidator(1))

	_, _, batch := kvForTest(50)
	root1, err := db.Update(0, batch[:25])
	r.NoError(err)

	root2, err := db.Update(0, batch[25:])
	r.NoError(err)

	// Advance validator 1 to root1 so the change proof applies cleanly
	r.NoError(db.AdvanceToHash(1, root1))

	// Create change proof
	proof, err := db.ChangeProof(root1, root2, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	r.NotNil(proof)
	t.Cleanup(func() { r.NoError(proof.Free()) })

	// Verify the change proof (standalone, no DB needed)
	verifiedProof, err := proof.VerifyChangeProof(root1, root2, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	r.NotNil(verifiedProof)
	t.Cleanup(func() { r.NoError(verifiedProof.Free()) })

	// Propose for validator 1
	proposedProof, err := db.ProposeChangeProof(verifiedProof, 1)
	r.NoError(err)
	r.NotNil(proposedProof)
	t.Cleanup(func() { r.NoError(proposedProof.Free()) })

	// Commit
	committedHash, err := proposedProof.CommitChangeProof()
	r.NoError(err)
	r.NotEqual(EmptyRoot, committedHash)
}

// TestMultiProofCrossValidator verifies that a proof created from one
// validator's state can be committed by another validator.
func TestMultiProofCrossValidator(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))
	r.NoError(db.RegisterValidator(1))
	r.NoError(db.RegisterValidator(2))

	// Build state on validator 0
	_, _, batch := kvForTest(30)
	root, err := db.Update(0, batch)
	r.NoError(err)

	// Create a range proof (read-only, no validator needed)
	proof, err := db.RangeProof(root, nothing(), nothing(), rangeProofLenUnbounded)
	r.NoError(err)
	r.NotNil(proof)

	// Commit for validator 1
	committedRoot1, err := db.VerifyAndCommitRangeProof(
		proof, nothing(), nothing(), root, rangeProofLenUnbounded, 1,
	)
	r.NoError(err)
	r.Equal(root, committedRoot1)

	// Create another proof and commit for validator 2
	proof2, err := db.RangeProof(root, nothing(), nothing(), rangeProofLenUnbounded)
	r.NoError(err)
	r.NotNil(proof2)

	committedRoot2, err := db.VerifyAndCommitRangeProof(
		proof2, nothing(), nothing(), root, rangeProofLenUnbounded, 2,
	)
	r.NoError(err)
	r.Equal(root, committedRoot2)
}
