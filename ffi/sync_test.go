// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"testing"

	"github.com/stretchr/testify/require"
)

func TestMultiRoundChangeProof(t *testing.T) {
	type TestStruct struct {
		name       string
		hasDeletes bool
		deleteOnly bool
	}

	tests := []TestStruct{
		{"Multi-round change proofs with no deletes", false, false},
		{"Multi-round change proofs With deletes", true, false},
		{"Multi-round change proofs delete-only update", false, true},
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

			if tt.deleteOnly {
				// Delete ALL keys from the second batch, producing a proof
				// with only Delete batch_ops and no Put ops.
				delKeys := make([]BatchOp, 50)
				for i := range delKeys {
					delKeys[i] = Delete(keys[50+i])
					keys[50+i] = nil
				}
				rootAUpdated, err = dbA.Update(delKeys)
				r.NoError(err)
			} else if tt.hasDeletes {
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

				// Sync is complete when root hashes match. The caller
				// determines completion, not find_next_key.
				if rootB == rootAUpdated {
					break
				}

				// Not done — find next key range to continue syncing.
				nextRange, err := proposedProof.FindNextKey()
				r.NoError(err)
				r.NotNil(nextRange, "find_next_key returned nil but root hashes don't match")
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

// TestChangeProofBoundaryValueMismatchDeferred verifies that when two
// databases have different values at a key, a multi-round sync detects
// the mismatch during verify_root_hash (not just at final completion).
//
// Setup:
//   - dbA: \x10=v0, \x20=valA, \x30=v2 (prover)
//   - dbB: \x10=v0, \x20=valB, \x30=v2 (verifier, different at \x20)
//   - Changes on dbA: Put(\x10, changed), Put(\x30, changed)
//
// Round 1 (truncated, max_length=1): batch_ops=[Put(\x10, changed)].
// End proof for \x10 (nibble 1 at depth 0). The mismatch at \x20
// (nibble 2) is beyond end_bn — not checked. find_next_key returns
// continuation.
//
// Round 2 (full): batch_ops=[Put(\x10, changed), Put(\x30, changed)].
// Start proof for \x10 (nibble 1), end proof for \x30 (nibble 3).
// At the divergence parent, nibble 2 (\x20) is between start_bn (1)
// and end_bn (3) — in-range. The proposal has \x20=valB (from dbB),
// the proof has \x20=valA (from end_root). InRangeChildMismatch.
func TestChangeProofBoundaryValueMismatchDeferred(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	// Same keys, different values at \x20
	initialA := []BatchOp{
		Put([]byte("\x10"), []byte("v0")),
		Put([]byte("\x20"), []byte("valA")),
		Put([]byte("\x30"), []byte("v2")),
	}
	initialB := []BatchOp{
		Put([]byte("\x10"), []byte("v0")),
		Put([]byte("\x20"), []byte("valB")),
		Put([]byte("\x30"), []byte("v2")),
	}
	rootA, err := dbA.Update(initialA)
	r.NoError(err)
	rootB, err := dbB.Update(initialB)
	r.NoError(err)
	r.NotEqual(rootA, rootB, "roots should differ at \\x20")

	// Change \x10 and \x30 on dbA
	rootAUpdated, err := dbA.Update([]BatchOp{
		Put([]byte("\x10"), []byte("changed0")),
		Put([]byte("\x30"), []byte("changed2")),
	})
	r.NoError(err)

	startKey := something([]byte("\x00"))
	endKey := something([]byte("\x30"))

	// --- Round 1: truncated proof (max_length=1) ---
	// Only includes Put(\x10). End proof for \x10 (nibble 1).
	// \x20 (nibble 2) is beyond end_bn — not checked.
	proof1, err := dbA.ChangeProof(rootA, rootAUpdated, startKey, endKey, 1)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proof1.Free()) })

	proposed1, err := dbB.VerifyAndProposeChangeProof(
		proof1, rootB, rootAUpdated, startKey, endKey, 1)
	r.NoError(err, "round 1: should pass (\\x20 mismatch is off-path)")
	t.Cleanup(func() { r.NoError(proposed1.Free()) })

	// find_next_key: last_op (\x10) < end_key (\x30) → continuation
	next, err := proposed1.FindNextKey()
	r.NoError(err)
	r.NotNil(next, "round 1: should return continuation")

	// Commit round 1
	rootB, err = proposed1.CommitChangeProof()
	r.NoError(err)
	nextStart := maybe{hasValue: true, value: next.StartKey()}
	r.NoError(next.Free())

	// --- Round 2: full proof for continuation range ---
	// Includes Put(\x10) again (idempotent) and Put(\x30).
	// Start proof for \x10 (nibble 1), end proof for \x30 (nibble 3).
	// \x20 (nibble 2) is in-range at the divergence parent.
	// Proposal has \x20=valB, proof has \x20=valA → mismatch.
	proof2, err := dbA.ChangeProof(rootA, rootAUpdated, nextStart, endKey, changeProofLenUnbounded)
	r.NoError(err)
	t.Cleanup(func() { r.NoError(proof2.Free()) })

	_, err = dbB.VerifyAndProposeChangeProof(
		proof2, rootB, rootAUpdated, nextStart, endKey, changeProofLenUnbounded)
	r.Error(err, "round 2: verify_root_hash should catch \\x20 mismatch")
	r.ErrorContains(err, "proof error:")
}
