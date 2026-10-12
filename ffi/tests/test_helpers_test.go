// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi_test

import (
	"context"
	"errors"
	"fmt"
	"runtime"
	"testing"
	"time"

	"github.com/ava-labs/firewood/ffi"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func oneSecCtx(tb testing.TB) context.Context {
	ctx := context.WithoutCancel(tb.Context())
	ctx, cancel := context.WithTimeout(ctx, time.Second)
	tb.Cleanup(cancel)
	return ctx
}

func newTestDatabaseWithHashMode(tb testing.TB, algorithm ffi.NodeHashAlgorithm) *ffi.Database {
	tb.Helper()
	r := require.New(tb)

	dbDir := tb.TempDir()
	db, err := ffi.New(dbDir, algorithm)
	if err != nil {
		err = fmt.Errorf("failed to create new database in directory %q: %w", dbDir, err)
	}
	r.NoError(err)
	tb.Cleanup(func() {
		err := db.Close(oneSecCtx(tb))
		if errors.Is(err, ffi.ErrActiveKeepAliveHandles) {
			// Collect dangling handles once before retrying the close.
			runtime.GC()
			err = db.Close(oneSecCtx(tb))
		}
		assert.NoError(tb, err) // don't mask an earlier test failure
	})
	return db
}
