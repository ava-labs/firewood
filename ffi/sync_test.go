// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"testing"

	"github.com/stretchr/testify/require"
)

func TestSyncStartZeroTaskLimit(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	s, err := db.StartSync(Hash{0x01}, WithTaskLimit(0))
	r.Nil(s)
	r.ErrorContains(err, "task_limit must be nonzero")
}

func TestSyncStartZeroTarget(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	s, err := db.StartSync(Hash{})
	r.Nil(s)
	r.ErrorContains(err, "all-zero hash")
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
