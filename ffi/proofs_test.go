// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"testing"

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

func TestRangeProofEmptyDB(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	proof, err := db.RangeProof(nothing(), nothing(), nothing(), 0)
	r.ErrorIs(err, errEmptyTrie)
	r.Nil(proof)
}

func TestRangeProofNonExistentRoot(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// insert some data
	keys, vals := kvForTest(100)
	root, err := db.Update(keys, vals)
	r.NoError(err)
	r.NotNil(root)

	// create a bogus root
	bogusRoot := make([]byte, len(root))
	copy(bogusRoot, root)
	bogusRoot[0] ^= 0xFF

	proof, err := db.RangeProof(something(bogusRoot), nothing(), nothing(), 0)
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
	proof1, proof1Bytes := rangeProofWithAndWithoutRoot(t, db, root, nothing(), nothing())

	// get a proof over a different range
	startKey := something([]byte("key2"))
	endKey := something([]byte("key3"))
	proof2, proof2Bytes := rangeProofWithAndWithoutRoot(t, db, root, startKey, endKey)

	// ensure the proofs are different
	r.NotEqual(proof1Bytes, proof2Bytes)

	r.NoError(proof1.Verify(root, nothing(), nothing(), maxProofLen))
	r.NoError(proof2.Verify(root, startKey, endKey, maxProofLen))
}

func TestRangeProofDiffersAfterUpdate(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Insert some data.
	keys, vals := kvForTest(100)
	root1, err := db.Update(keys[:50], vals[:50])
	r.NoError(err)

	// get a proof
	proof1, proof1Bytes := rangeProofWithAndWithoutRoot(t, db, root1, nothing(), nothing())

	// insert more data
	root2, err := db.Update(keys[50:], vals[50:])
	r.NoError(err)
	r.NotEqual(root1, root2)

	// get a proof again
	proof2, proof2Bytes := rangeProofWithAndWithoutRoot(t, db, root2, nothing(), nothing())

	// ensure the proofs are different
	r.NotEqual(proof1Bytes, proof2Bytes)

	r.NoError(proof1.Verify(root1, nothing(), nothing(), maxProofLen))
	r.NoError(proof2.Verify(root2, nothing(), nothing(), maxProofLen))
}

func TestRoundTripSerialization(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Insert some data.
	keys, vals := kvForTest(10)
	root, err := db.Update(keys, vals)
	r.NoError(err)

	// get a proof
	_, proofBytes := rangeProofWithAndWithoutRoot(t, db, root, nothing(), nothing())

	// Deserialize the proof.
	proof := new(RangeProof)
	err = proof.UnmarshalBinary(proofBytes)
	r.NoError(err)

	// serialize the proof again
	serialized, err := proof.MarshalBinary()
	r.NoError(err)
	r.Equal(proofBytes, serialized)

	r.NoError(proof.Verify(root, nothing(), nothing(), maxProofLen))

	r.NoError(proof.Free())
}

// rangeProofWithAndWithoutRoot checks that requesting a range proof with and
// without the root, when the default root is the same as the provided root,
// yields the same proof and returns the proof bytes.
func rangeProofWithAndWithoutRoot(
	t *testing.T,
	db *Database,
	root []byte,
	startKey, endKey maybe,
) (*RangeProof, []byte) {
	r := require.New(t)

	proof1, err := db.RangeProof(maybe{hasValue: false}, startKey, endKey, maxProofLen)
	r.NoError(err)
	r.NotNil(proof1)
	proof1Bytes, err := proof1.MarshalBinary()
	r.NoError(err)
	t.Cleanup(func() {
		r.NoError(proof1.Free())
	})

	proof2, err := db.RangeProof(maybe{hasValue: true, value: root}, startKey, endKey, maxProofLen)
	r.NoError(err)
	r.NotNil(proof2)
	proof2Bytes, err := proof2.MarshalBinary()
	r.NoError(err)
	t.Cleanup(func() {
		r.NoError(proof2.Free())
	})

	r.Equal(proof1Bytes, proof2Bytes)

	return proof1, proof1Bytes
}
