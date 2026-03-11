// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"context"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/require"
)

func newTestMultiDatabase(tb testing.TB, maxValidators uint, opts ...Option) *MultiDatabase {
	tb.Helper()
	r := require.New(tb)

	db, err := newMultiDatabase(tb.TempDir(), maxValidators, opts...)
	r.NoError(err)
	tb.Cleanup(func() {
		err := db.Close(oneSecCtx(tb))
		if errors.Is(err, ErrActiveKeepAliveHandles) {
			runtime.GC()
			err = db.Close(oneSecCtx(tb))
		}
		assert.NoError(tb, err)
	})
	return db
}

func newMultiDatabase(dbDir string, maxValidators uint, opts ...Option) (*MultiDatabase, error) {
	// Reuse the detection logic from single-head tests.
	// newDatabase triggers detectedNodeHashAlgorithmOnce which is
	// also triggered by TestMain -> inferHashingMode -> newDatabase.
	detectedNodeHashAlgorithmOnce.Do(func() {
		tempDir := filepath.Join(os.TempDir(), fmt.Sprintf("firewood-multi-hash-detection-%d", time.Now().UnixNano()))
		defer os.RemoveAll(tempDir)

		_, err := New(tempDir, EthereumNodeHashing, WithTruncate(true))
		if err == nil {
			detectedNodeHashAlgorithm = EthereumNodeHashing
		} else {
			detectedNodeHashAlgorithm = MerkleDBNodeHashing
		}
	})

	f, err := NewMulti(dbDir, detectedNodeHashAlgorithm, maxValidators, opts...)
	if err != nil {
		return nil, err
	}
	return f, nil
}

// TestMultiNewAndClose tests basic lifecycle of a multi-validator database.
func TestMultiNewAndClose(t *testing.T) {
	r := require.New(t)

	db, err := newMultiDatabase(t.TempDir(), 4)
	r.NoError(err)

	err = db.RegisterValidator(0)
	r.NoError(err)

	ctx, cancel := context.WithTimeout(t.Context(), time.Second)
	defer cancel()
	r.NoError(db.Close(ctx))
}

// TestMultiRegisterDeregister tests registering and deregistering validators.
func TestMultiRegisterDeregister(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)

	// Register 4 validators
	for i := range uint64(4) {
		r.NoError(db.RegisterValidator(ValidatorID(i)))
	}

	// Deregister first two
	r.NoError(db.DeregisterValidator(0))
	r.NoError(db.DeregisterValidator(1))

	// Remaining validators should still work
	_, err := db.Root(2)
	r.NoError(err)
	_, err = db.Root(3)
	r.NoError(err)
}

// TestMultiMaxValidators tests that exceeding max validators returns an error.
func TestMultiMaxValidators(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 2)

	r.NoError(db.RegisterValidator(0))
	r.NoError(db.RegisterValidator(1))

	// Third should fail
	err := db.RegisterValidator(2)
	r.Error(err)
}

// TestMultiGet tests reading a value from a validator's head.
func TestMultiGet(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))

	_, err := db.Update(0, []BatchOp{Put([]byte("key"), []byte("value"))})
	r.NoError(err)

	val, err := db.Get(0, []byte("key"))
	r.NoError(err)
	r.Equal([]byte("value"), val)

	// Missing key
	val, err = db.Get(0, []byte("missing"))
	r.NoError(err)
	r.Nil(val)
}

// TestMultiRoot tests getting the root hash of a validator's head.
func TestMultiRoot(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))

	_, err := db.Update(0, []BatchOp{Put([]byte("k"), []byte("v"))})
	r.NoError(err)

	root, err := db.Root(0)
	r.NoError(err)
	r.NotEqual(EmptyRoot, root)
}

// TestMultiLatestRevision tests getting a revision from a validator's head.
func TestMultiLatestRevision(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))

	_, err := db.Update(0, []BatchOp{Put([]byte("k"), []byte("v"))})
	r.NoError(err)

	rev, err := db.LatestRevision(0)
	r.NoError(err)
	defer func() { r.NoError(rev.Drop()) }()

	val, err := rev.Get([]byte("k"))
	r.NoError(err)
	r.Equal([]byte("v"), val)
}

// TestMultiProposeAndCommit tests the propose/commit workflow.
func TestMultiProposeAndCommit(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))

	p, err := db.Propose(0, []BatchOp{Put([]byte("k"), []byte("v"))})
	r.NoError(err)

	r.NotEqual(EmptyRoot, p.Root())

	r.NoError(p.Commit())

	val, err := db.Get(0, []byte("k"))
	r.NoError(err)
	r.Equal([]byte("v"), val)
}

// TestMultiUpdate tests the propose+commit convenience method.
func TestMultiUpdate(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))

	hash, err := db.Update(0, []BatchOp{Put([]byte("k"), []byte("v"))})
	r.NoError(err)
	r.NotEqual(EmptyRoot, hash)

	val, err := db.Get(0, []byte("k"))
	r.NoError(err)
	r.Equal([]byte("v"), val)
}

// TestMultiProposeOnProposal tests chaining proposals.
func TestMultiProposeOnProposal(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))

	p1, err := db.Propose(0, []BatchOp{Put([]byte("k1"), []byte("v1"))})
	r.NoError(err)

	p2, err := p1.Propose([]BatchOp{Put([]byte("k2"), []byte("v2"))})
	r.NoError(err)

	// Read from child proposal
	val, err := p2.Get([]byte("k1"))
	r.NoError(err)
	r.Equal([]byte("v1"), val)

	val, err = p2.Get([]byte("k2"))
	r.NoError(err)
	r.Equal([]byte("v2"), val)

	// Commit parent first, then child
	r.NoError(p1.Commit())
	r.NoError(p2.Commit())

	// Both values should be readable
	val, err = db.Get(0, []byte("k1"))
	r.NoError(err)
	r.Equal([]byte("v1"), val)

	val, err = db.Get(0, []byte("k2"))
	r.NoError(err)
	r.Equal([]byte("v2"), val)
}

// TestMultiProposalGet tests reading from a proposal before commit.
func TestMultiProposalGet(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))

	p, err := db.Propose(0, []BatchOp{Put([]byte("k"), []byte("v"))})
	r.NoError(err)
	defer func() { _ = p.Drop() }()

	val, err := p.Get([]byte("k"))
	r.NoError(err)
	r.Equal([]byte("v"), val)
}

// TestMultiProposalIter tests iterating over a proposal.
func TestMultiProposalIter(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))

	batch := []BatchOp{
		Put([]byte("a"), []byte("1")),
		Put([]byte("b"), []byte("2")),
		Put([]byte("c"), []byte("3")),
	}
	p, err := db.Propose(0, batch)
	r.NoError(err)
	defer func() { _ = p.Drop() }()

	it, err := p.Iter(nil)
	r.NoError(err)
	defer func() { r.NoError(it.Drop()) }()

	var count int
	for it.Next() {
		count++
	}
	r.NoError(it.Err())
	r.Equal(3, count)
}

// TestMultiRevision tests getting a committed revision by hash.
func TestMultiRevision(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))

	hash, err := db.Update(0, []BatchOp{Put([]byte("k"), []byte("v"))})
	r.NoError(err)

	rev, err := db.Revision(hash)
	r.NoError(err)
	defer func() { r.NoError(rev.Drop()) }()

	val, err := rev.Get([]byte("k"))
	r.NoError(err)
	r.Equal([]byte("v"), val)
}

// TestMultiDump tests dumping a validator's trie.
func TestMultiDump(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))

	_, err := db.Update(0, []BatchOp{Put([]byte("k"), []byte("v"))})
	r.NoError(err)

	dump, err := db.Dump(0)
	r.NoError(err)
	r.NotEmpty(dump)
}

// TestMultiValidatorIsolation tests that validators' data is isolated.
func TestMultiValidatorIsolation(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))
	r.NoError(db.RegisterValidator(1))

	_, err := db.Update(0, []BatchOp{Put([]byte("v0key"), []byte("v0val"))})
	r.NoError(err)

	_, err = db.Update(1, []BatchOp{Put([]byte("v1key"), []byte("v1val"))})
	r.NoError(err)

	// v0 should not see v1's data
	val, err := db.Get(0, []byte("v1key"))
	r.NoError(err)
	r.Nil(val)

	// v1 should not see v0's data
	val, err = db.Get(1, []byte("v0key"))
	r.NoError(err)
	r.Nil(val)
}

// TestMultiValidatorDivergentCommits tests that validators can have different heads.
func TestMultiValidatorDivergentCommits(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))
	r.NoError(db.RegisterValidator(1))

	hash0, err := db.Update(0, []BatchOp{Put([]byte("k"), []byte("v0"))})
	r.NoError(err)

	hash1, err := db.Update(1, []BatchOp{Put([]byte("k"), []byte("v1"))})
	r.NoError(err)

	r.NotEqual(hash0, hash1)

	val0, err := db.Get(0, []byte("k"))
	r.NoError(err)
	r.Equal([]byte("v0"), val0)

	val1, err := db.Get(1, []byte("k"))
	r.NoError(err)
	r.Equal([]byte("v1"), val1)
}

// TestMultiValidatorAdvanceToHash tests advancing a validator to another's hash.
func TestMultiValidatorAdvanceToHash(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))
	r.NoError(db.RegisterValidator(1))

	hash0, err := db.Update(0, []BatchOp{Put([]byte("k"), []byte("v"))})
	r.NoError(err)

	r.NoError(db.AdvanceToHash(1, hash0))

	val, err := db.Get(1, []byte("k"))
	r.NoError(err)
	r.Equal([]byte("v"), val)
}

// TestMultiGetClosedDatabase tests that operations on a closed database return errors.
func TestMultiGetClosedDatabase(t *testing.T) {
	r := require.New(t)
	db, err := newMultiDatabase(t.TempDir(), 4)
	r.NoError(err)

	ctx, cancel := context.WithTimeout(t.Context(), time.Second)
	defer cancel()
	r.NoError(db.Close(ctx))

	_, err = db.Get(0, []byte("k"))
	r.Error(err)
}

// TestMultiOperationsOnUnregisteredValidator tests that operations on an
// unregistered validator return errors.
func TestMultiOperationsOnUnregisteredValidator(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)

	_, err := db.Get(99, []byte("k"))
	r.Error(err)

	_, err = db.Root(99)
	r.Error(err)

	_, err = db.Propose(99, []BatchOp{Put([]byte("k"), []byte("v"))})
	r.Error(err)

	_, err = db.Update(99, []BatchOp{Put([]byte("k"), []byte("v"))})
	r.Error(err)
}

// TestMultiProposalDrop tests that dropping a proposal doesn't leak.
func TestMultiProposalDrop(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4)
	r.NoError(db.RegisterValidator(0))

	p, err := db.Propose(0, []BatchOp{Put([]byte("k"), []byte("v"))})
	r.NoError(err)

	r.NoError(p.Drop())
}

// TestMultiCloseWithActiveHandles tests that Close blocks until handles are dropped.
func TestMultiCloseWithActiveHandles(t *testing.T) {
	r := require.New(t)
	db, err := newMultiDatabase(t.TempDir(), 4)
	r.NoError(err)

	r.NoError(db.RegisterValidator(0))
	_, err = db.Update(0, []BatchOp{Put([]byte("k"), []byte("v"))})
	r.NoError(err)

	rev, err := db.LatestRevision(0)
	r.NoError(err)

	// Close should block because revision is active
	ctx, cancel := context.WithTimeout(t.Context(), 100*time.Millisecond)
	defer cancel()
	err = db.Close(ctx)
	r.Error(err) // Should timeout

	// Drop the revision
	r.NoError(rev.Drop())

	// Now close should succeed
	ctx2, cancel2 := context.WithTimeout(context.WithoutCancel(t.Context()), time.Second)
	defer cancel2()
	r.NoError(db.Close(ctx2))
}

// TestMultiDivergentReapingSafety verifies that when two validators diverge,
// reaping on one chain does not corrupt the other chain's data.
func TestMultiDivergentReapingSafety(t *testing.T) {
	r := require.New(t)
	db := newTestMultiDatabase(t, 4, WithRevisions(5))

	r.NoError(db.RegisterValidator(0))
	r.NoError(db.RegisterValidator(1))

	// Shared base
	baseRoot, err := db.Update(0, []BatchOp{Put([]byte("shared"), []byte("base"))})
	r.NoError(err)
	r.NoError(db.AdvanceToHash(1, baseRoot))

	// Diverge: v0 and v1 commit different data
	_, err = db.Update(0, []BatchOp{Put([]byte("v0key"), []byte("v0val"))})
	r.NoError(err)

	_, err = db.Update(1, []BatchOp{Put([]byte("v1key"), []byte("v1val"))})
	r.NoError(err)

	// V0 commits many revisions to trigger reaping
	for i := range 15 {
		_, err = db.Update(0, []BatchOp{Put(
			[]byte(fmt.Sprintf("v0extra%d", i)),
			[]byte(fmt.Sprintf("v0eval%d", i)),
		)})
		r.NoError(err)
	}

	// V1's data should be intact despite V0's reaping
	val, err := db.Get(1, []byte("shared"))
	r.NoError(err)
	r.Equal([]byte("base"), val, "shared ancestor data should survive reaping")

	val, err = db.Get(1, []byte("v1key"))
	r.NoError(err)
	r.Equal([]byte("v1val"), val, "v1's own data should survive reaping")

	// V0 should also work
	val, err = db.Get(0, []byte("v0extra14"))
	r.NoError(err)
	r.Equal([]byte("v0eval14"), val)
}
