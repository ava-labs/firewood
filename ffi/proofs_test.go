// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"testing"

	"github.com/stretchr/testify/require"
)

type maybe struct {
	hasValue bool
	value    []byte
}

func (m maybe) HasValue() bool {
	return m.hasValue
}

func (m maybe) Value() []byte {
	return m.value
}

func TestPartialRangeProof(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// check empty database
	proof, err := db.RangeProof(nil, nil, nil, 0)
	r.ErrorIs(err, errEmptyTrie)
	r.Nil(proof)

	// Insert a lot of data.
	keys, vals := kvForTest(10000)
	root, err := db.Update(keys, vals)
	r.NoError(err)

	proof, err = db.RangeProof(nil, nil, nil, 10)
	r.NoError(err)
	r.NotNil(proof)
	proof1Bytes, err := proof.MarshalBinary()
	r.NoError(err)
	r.NoError(proof.Free())

	proof, err = db.RangeProof(maybe{true, root}, nil, nil, 10)
	r.NoError(err)
	r.NotNil(proof)
	proof2Bytes, err := proof.MarshalBinary()
	r.NoError(err)
	r.NoError(proof.Free())

	r.Equal(proof1Bytes, proof2Bytes)

	// Deserialize the proof.
	proof3 := new(RangeProof)
	err = proof3.UnmarshalBinary(proof1Bytes)
	r.NoError(err)
	r.NoError(proof3.Free())

	proof, err = db.RangeProof(nil, maybe{true, []byte("key2")}, maybe{true, []byte("key3")}, 10)
	r.NoError(err)
	r.NotNil(proof)
	proof4Bytes, err := proof.MarshalBinary()
	r.NoError(err)
	r.NoError(proof.Free())

	proof, err = db.RangeProof(maybe{true, root}, maybe{true, []byte("key2")}, maybe{true, []byte("key3")}, 10)
	r.NoError(err)
	r.NotNil(proof)
	proof5Bytes, err := proof.MarshalBinary()
	r.NoError(err)
	r.NoError(proof.Free())

	r.Equal(proof4Bytes, proof5Bytes)
	r.NotEqual(proof1Bytes, proof4Bytes)

	// Deserialize the proof.
	proof6 := new(RangeProof)
	err = proof6.UnmarshalBinary(proof4Bytes)
	r.NoError(err)
	r.NoError(proof6.Free())
}

func TestRoundTripSerialization(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// check empty database
	proof, err := db.RangeProof(nil, nil, nil, 0)
	r.ErrorIs(err, errEmptyTrie)
	r.Nil(proof)

	// check for non-existent root
	proof, err = db.RangeProof(maybe{
		hasValue: true,
		value:    []byte{0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31},
	}, nil, nil, 0)
	r.ErrorIs(err, errRevisionNotFound)
	r.Nil(proof)

	// Insert some data.
	keys, vals := kvForTest(10)
	root, err := db.Update(keys, vals)
	r.NoError(err)

	proof1, err := db.RangeProof(nil, nil, nil, 0)
	r.NoError(err)
	r.NotNil(proof1)
	t.Cleanup(func() {
		r.NoError(proof1.Free())
	})

	proof1Bytes, err := proof1.MarshalBinary()
	r.NoError(err)

	proof2, err := db.RangeProof(maybe{true, root}, nil, nil, 0)
	r.NoError(err)
	r.NotNil(proof2)
	t.Cleanup(func() {
		r.NoError(proof2.Free())
	})

	proof2Bytes, err := proof2.MarshalBinary()
	r.NoError(err)

	r.Equal(proof1Bytes, proof2Bytes)

	// Deserialize the proof.
	proof3 := new(RangeProof)
	err = proof3.UnmarshalBinary(proof1Bytes)
	r.NoError(err)
	r.NoError(proof3.Free())
}
