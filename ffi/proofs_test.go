// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

import (
	"bytes"
	"encoding/hex"
	"runtime"
	"sync"
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

// newRangeProof generates an unverified range proof for the given parameters.
func newRangeProof(
	t *testing.T,
	db *Database,
	root Hash,
	startKey, endKey maybe,
	proofLen uint32,
) *RangeProof {
	t.Helper()
	proof, err := db.RangeProof(root, startKey, endKey, proofLen)
	require.NoError(t, err)
	assertProofNotNil(t, proof)
	return proof
}

// verifyRangeProof verifies proof on the database it belongs to.
func verifyRangeProof(
	t *testing.T,
	proof *RangeProof,
	root Hash,
	startKey, endKey maybe,
	proofLen uint32,
) *VerifiedRangeProof {
	t.Helper()
	verified, err := proof.Verify(root, startKey, endKey, proofLen)
	require.NoError(t, err)
	require.NotNil(t, verified)
	return verified
}

// transferRangeProof moves proof to dst the way a proof travels over the
// network: serialize on the producer, parse on the consumer. The source proof
// is dropped.
func transferRangeProof(t *testing.T, proof *RangeProof, dst *Database) *RangeProof {
	t.Helper()
	r := require.New(t)
	data, err := proof.Marshal()
	r.NoError(err)
	r.NoError(proof.Drop())
	parsed, err := dst.UnmarshalRangeProof(data)
	r.NoError(err)
	return parsed
}

// transferChangeProof is [transferRangeProof] for change proofs.
func transferChangeProof(t *testing.T, proof *ChangeProof, dst *Database) *ChangeProof {
	t.Helper()
	r := require.New(t)
	data, err := proof.Marshal()
	r.NoError(err)
	r.NoError(proof.Drop())
	parsed, err := dst.UnmarshalChangeProof(data)
	r.NoError(err)
	return parsed
}

// ethAccountWithCodeHash returns a 32-byte account key, the RLP-encoded account
// stored under it, and the code hash embedded in that account. Only
// account-length keys reach the code-hash extractor, so this is the minimal
// fixture that makes a proof yield exactly one code hash.
func ethAccountWithCodeHash(t *testing.T) ([32]byte, []byte, Hash) {
	t.Helper()

	key := [32]byte{0x12, 0x34, 0x56} // account keys must be 32 bytes
	val, err := hex.DecodeString("f8440164a056e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421a0044852b2a670ade5407e78fb2863c51de9fcb96542a07186fe3aeda6bb8a116d")
	require.NoError(t, err)
	return key, val, stringToHash(t, "044852b2a670ade5407e78fb2863c51de9fcb96542a07186fe3aeda6bb8a116d")
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
	t.Helper()
	r := require.New(t)

	proof := newRangeProof(t, db, root, startKey, endKey, proofLen)

	proofBytes, err := proof.Marshal()
	r.NoError(err)
	r.NoError(proof.Drop())

	return proofBytes
}

func newSerializedChangeProof(
	t *testing.T,
	db *Database,
	startRoot, endRoot Hash,
	startKey, endKey maybe,
) []byte {
	t.Helper()
	r := require.New(t)

	proof, err := db.ChangeProof(startRoot, endRoot, startKey, endKey, changeProofLenUnbounded)
	r.NoError(err)

	proofBytes, err := proof.Marshal()
	r.NoError(err)
	r.NoError(proof.Drop())

	return proofBytes
}

// newVerifiedChangeProof seeds dbA and dbB with the same state, advances dbA,
// and returns the change proof verified on dbB together with dbA's new root.
// The helper never calls Drop or registers a t.Cleanup on the verified proof,
// so the caller controls the release path (Drop, Commit then Drop, or GC);
// TestVerifiedChangeProofKeepAlive exercises all three.
func newVerifiedChangeProof(
	t *testing.T,
	dbA, dbB *Database,
) (*VerifiedChangeProof, Hash) {
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
	changeProof = transferChangeProof(t, changeProof, dbB)

	verified, err := changeProof.Verify(rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)

	return verified, rootAUpdated
}

func TestRangeProofEmptyDB(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	proof, err := db.RangeProof(EmptyRoot, nothing(), nothing(), rangeProofLenUnbounded)
	r.ErrorIs(err, ErrRevisionNotFound)
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
	r.ErrorIs(err, ErrRevisionNotFound)
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

	_, _, batch := kvForTest(10)
	root, err := db.Update(batch)
	r.NoError(err)

	proofBytes := newSerializedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenUnbounded)

	proof, err := db.UnmarshalRangeProof(proofBytes)
	r.NoError(err)

	serialized, err := proof.Marshal()
	r.NoError(err)
	r.Equal(proofBytes, serialized)
}

// TestRangeProofVerifyFailureLeavesNoLease checks that a failed verification
// returns an error and builds nothing: the database closes without waiting on
// any lease.
func TestRangeProofVerifyFailureLeavesNoLease(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	root, err := db.Update(batch)
	r.NoError(err)

	proof := newRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)

	wrongRoot := root
	wrongRoot[0] ^= 0xFF
	verified, err := proof.Verify(wrongRoot, nothing(), nothing(), rangeProofLenTruncated)
	r.Error(err, "Verification with wrong root should fail")
	r.Nil(verified)

	r.NoError(proof.Drop())
	r.NoError(db.Close(oneSecCtx(t)), "a failed Verify must leave no lease behind")
}

func TestVerifiedRangeProofCommitAppliesValues(t *testing.T) {
	r := require.New(t)

	dbSource := newTestDatabase(t)
	dbTarget := newTestDatabase(t)

	keys, vals, batch := kvForTest(50)
	sourceRoot, err := dbSource.Update(batch)
	r.NoError(err)

	proof := newRangeProof(t, dbSource, sourceRoot, nothing(), nothing(), rangeProofLenUnbounded)
	proof = transferRangeProof(t, proof, dbTarget)
	verified := verifyRangeProof(t, proof, sourceRoot, nothing(), nothing(), rangeProofLenUnbounded)

	committedRoot, err := verified.Commit()
	r.NoError(err)
	r.Equal(sourceRoot, committedRoot)

	for i, key := range keys {
		got, err := dbTarget.Get(key)
		r.NoError(err, "Get key %d", i)
		r.Equal(vals[i], got, "Value mismatch for key %d", i)
	}
	r.NoError(verified.Drop())
}

func TestRangeProofNextKeyRanges(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	root, err := db.Update(batch)
	r.NoError(err)

	proof := newRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
	verified := verifyRangeProof(t, proof, root, nothing(), nothing(), rangeProofLenTruncated)

	// The proof was taken from db's own state, so merging it over the proven
	// range is a no-op and the proposal keeps `root`. That equality is the
	// FFI's "already caught up" short-circuit, so there is nothing left to
	// fetch — before commit here and after commit below. A non-empty result
	// means the apply path deleted outside the proven range and moved the
	// root. TestRangeProofNextKeyRangesDivergentReceiver covers a receiver
	// that is genuinely behind.
	ranges, err := verified.NextKeyRanges()
	r.NoError(err)
	r.Empty(ranges)

	_, err = verified.Commit()
	r.NoError(err)

	ranges, err = verified.NextKeyRanges()
	r.NoError(err)
	r.Empty(ranges)
	r.NoError(verified.Drop())
}

// TestRangeProofNextKeyRangesDivergentReceiver applies a truncated proof to an
// empty target: a receiver genuinely behind the proof's root must report more
// to fetch.
func TestRangeProofNextKeyRangesDivergentReceiver(t *testing.T) {
	r := require.New(t)

	dbSource := newTestDatabase(t)
	dbTarget := newTestDatabase(t)

	keys, vals, batch := kvForTest(100)
	sourceRoot, err := dbSource.Update(batch)
	r.NoError(err)

	proof := newRangeProof(t, dbSource, sourceRoot, nothing(), nothing(), rangeProofLenTruncated)
	proof = transferRangeProof(t, proof, dbTarget)
	verified := verifyRangeProof(t, proof, sourceRoot, nothing(), nothing(), rangeProofLenTruncated)

	_, err = verified.Commit()
	r.NoError(err)

	for i := range rangeProofLenTruncated {
		got, err := dbTarget.Get(keys[i])
		r.NoError(err, "Get key %d", i)
		r.Equal(vals[i], got, "key %d from the proven prefix was not applied", i)
	}

	// The truncated proof only proved a prefix, and dbTarget started with
	// none of the data, so there is genuinely more to fetch.
	ranges, err := verified.NextKeyRanges()
	r.NoError(err)
	r.Len(ranges, 1)
	// The start key is inclusive, so it is the smallest key strictly above the
	// last proven one: that key with a zero byte appended.
	lastProven := keys[rangeProofLenTruncated-1]
	r.Equal(append(bytes.Clone(lastProven), 0x00), ranges[0].StartKey)
	r.False(ranges[0].EndKey.HasValue(), "the request was unbounded above")

	// The keys are Go-owned: they survive the proof.
	r.NoError(verified.Drop())
	r.NoError(proof.Drop())
	ranges[0].StartKey[0] ^= 0xFF
}

// TestRangeProofMethodFreeRace is a regression test for ava-labs/firewood#2137.
//
// Every method that passes a handle into cgo must hold lease.mu, which
// serializes it against Drop freeing the Rust context. In production the
// racing Drop is the GC cleanup, which can run while a cgo call is in flight;
// a method that skips the lock is a use-after-free, reported in the issue as
// "SIGSEGV ... signal arrived during cgo execution".
//
// The cleanup's timing cannot be forced from Go, so this test races each method
// against an explicit Drop on the same proof. Under -race, which CI runs the ffi
// suite under, a method that skips the lock is reported deterministically.
// Without -race the window is too narrow to crash reliably in a bounded run.
func TestRangeProofMethodFreeRace(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	root, err := db.Update(batch)
	r.NoError(err)

	// Cheaply mint fresh, independent RangeProofContexts by unmarshalling the
	// same serialized proof each iteration.
	proofBytes := newSerializedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)

	t.Run("unverified", func(t *testing.T) {
		ops := []struct {
			name string
			call func(*RangeProof)
		}{
			{
				name: "Verify",
				call: func(p *RangeProof) {
					v, err := p.Verify(root, nothing(), nothing(), rangeProofLenTruncated)
					if err == nil {
						_ = v.Drop()
					}
				},
			},
			{
				name: "Marshal",
				call: func(p *RangeProof) { _, _ = p.Marshal() },
			},
		}

		for _, op := range ops {
			t.Run(op.name, func(t *testing.T) {
				// -race flags the conflicting p.handle access as soon as one
				// iteration's method and Drop overlap, which the start barrier
				// makes near-certain per iteration; a few thousand iterations is
				// ample margin while keeping the -race CI run fast.
				const iterations = 10_000
				for range iterations {
					p, err := db.UnmarshalRangeProof(proofBytes)
					require.NoError(t, err)

					start := make(chan struct{})
					var wg sync.WaitGroup
					wg.Go(func() {
						<-start
						op.call(p) // reads p.handle across cgo
					})
					wg.Go(func() {
						<-start
						_ = p.Drop() // frees the Rust context (the GC cleanup's code path)
					})
					close(start)
					wg.Wait()
				}
			})
		}
	})

	t.Run("verified", func(t *testing.T) {
		ops := []struct {
			name string
			call func(*VerifiedRangeProof)
		}{
			{
				name: "NextKeyRanges",
				call: func(p *VerifiedRangeProof) { _, _ = p.NextKeyRanges() },
			},
			{
				name: "CodeHashes",
				call: func(p *VerifiedRangeProof) { _, _ = p.CodeHashes() },
			},
		}

		for _, op := range ops {
			t.Run(op.name, func(t *testing.T) {
				// Each iteration verifies, which builds a proposal, so fewer
				// iterations than the unverified case keep the -race run bounded.
				const iterations = 2_000
				for range iterations {
					p, err := db.UnmarshalRangeProof(proofBytes)
					require.NoError(t, err)
					v, err := p.Verify(root, nothing(), nothing(), rangeProofLenTruncated)
					require.NoError(t, err)
					require.NoError(t, p.Drop())

					start := make(chan struct{})
					var wg sync.WaitGroup
					wg.Go(func() {
						<-start
						op.call(v)
					})
					wg.Go(func() {
						<-start
						_ = v.Drop()
					})
					close(start)
					wg.Wait()
				}
			})
		}
	})
}

// TestChangeProofMethodDropRace is the ChangeProof counterpart of
// TestRangeProofMethodFreeRace.
func TestChangeProofMethodDropRace(t *testing.T) {
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)
	_, _, batch := kvForTest(100)
	startRoot, err := dbA.Update(batch[:50])
	require.NoError(t, err)
	_, err = dbB.Update(batch[:50])
	require.NoError(t, err)
	endRoot, err := dbA.Update(batch[50:])
	require.NoError(t, err)
	proofBytes := newSerializedChangeProof(t, dbA, startRoot, endRoot, nothing(), nothing())

	t.Run("unverified", func(t *testing.T) {
		ops := []struct {
			name string
			call func(*ChangeProof)
		}{
			{
				name: "Verify",
				call: func(p *ChangeProof) {
					v, err := p.Verify(endRoot, nothing(), nothing(), changeProofLenUnbounded)
					if err == nil {
						_ = v.Drop()
					}
				},
			},
			{
				name: "Marshal",
				call: func(p *ChangeProof) { _, _ = p.Marshal() },
			},
		}

		for _, op := range ops {
			t.Run(op.name, func(t *testing.T) {
				// See [TestRangeProofMethodFreeRace] for the iteration count.
				const iterations = 10_000
				for range iterations {
					p, err := dbB.UnmarshalChangeProof(proofBytes)
					require.NoError(t, err)

					start := make(chan struct{})
					var wg sync.WaitGroup
					wg.Go(func() {
						<-start
						op.call(p)
					})
					wg.Go(func() {
						<-start
						_ = p.Drop()
					})
					close(start)
					wg.Wait()
				}
			})
		}
	})

	t.Run("verified", func(t *testing.T) {
		ops := []struct {
			name string
			call func(*VerifiedChangeProof)
		}{
			{
				name: "NextKeyRanges",
				call: func(p *VerifiedChangeProof) { _, _ = p.NextKeyRanges() },
			},
			{
				name: "CodeHashes",
				call: func(p *VerifiedChangeProof) { _, _ = p.CodeHashes() },
			},
		}

		for _, op := range ops {
			t.Run(op.name, func(t *testing.T) {
				const iterations = 2_000
				for range iterations {
					p, err := dbB.UnmarshalChangeProof(proofBytes)
					require.NoError(t, err)
					v, err := p.Verify(endRoot, nothing(), nothing(), changeProofLenUnbounded)
					require.NoError(t, err)
					require.NoError(t, p.Drop())

					start := make(chan struct{})
					var wg sync.WaitGroup
					wg.Go(func() {
						<-start
						op.call(v)
					})
					wg.Go(func() {
						<-start
						_ = v.Drop()
					})
					close(start)
					wg.Wait()
				}
			})
		}
	})
}

// TestRangeProofVerifyTwice checks that Verify does not consume the proof: two
// calls yield two independent verified proofs that both commit to the same
// root.
func TestRangeProofVerifyTwice(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	root, err := db.Update(batch)
	r.NoError(err)

	proof := newRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
	first := verifyRangeProof(t, proof, root, nothing(), nothing(), rangeProofLenTruncated)
	second := verifyRangeProof(t, proof, root, nothing(), nothing(), rangeProofLenTruncated)

	firstRoot, err := first.Commit()
	r.NoError(err)
	secondRoot, err := second.Commit()
	r.NoError(err)
	r.Equal(root, firstRoot)
	r.Equal(firstRoot, secondRoot)

	r.NoError(first.Drop())
	r.NoError(second.Drop())
	r.NoError(proof.Drop())
}

// TestVerifiedRangeProofCommitTwice checks that a second Commit returns the
// cached root and does not touch the database.
func TestVerifiedRangeProofCommitTwice(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	_, _, batch := kvForTest(50)
	root, err := db.Update(batch)
	r.NoError(err)

	proof := newRangeProof(t, db, root, nothing(), nothing(), rangeProofLenUnbounded)
	verified := verifyRangeProof(t, proof, root, nothing(), nothing(), rangeProofLenUnbounded)

	first, err := verified.Commit()
	r.NoError(err)
	second, err := verified.Commit()
	r.NoError(err)
	r.Equal(first, second)
	r.Equal(root, db.Root())
	r.NoError(verified.Drop())
}

// TestVerifiedRangeProofCommitAfterDatabaseAdvanced checks the rebuild path:
// a proposal prepared by Verify is stale once the database moves on, and
// Commit rebuilds it from the proof instead of failing.
func TestVerifiedRangeProofCommitAfterDatabaseAdvanced(t *testing.T) {
	r := require.New(t)
	dbSource := newTestDatabase(t)
	dbTarget := newTestDatabase(t)

	keys, vals, batch := kvForTest(50)
	sourceRoot, err := dbSource.Update(batch)
	r.NoError(err)

	proof := newRangeProof(t, dbSource, sourceRoot, nothing(), nothing(), rangeProofLenUnbounded)
	proof = transferRangeProof(t, proof, dbTarget)
	verified := verifyRangeProof(t, proof, sourceRoot, nothing(), nothing(), rangeProofLenUnbounded)

	// Advance dbTarget underneath the prepared proposal.
	_, err = dbTarget.Update([]BatchOp{Put([]byte("unrelated"), []byte("value"))})
	r.NoError(err)

	committed, err := verified.Commit()
	r.NoError(err)
	// The unbounded proof replaces the whole keyspace, so the unrelated key is
	// gone and the root matches the source again.
	r.Equal(sourceRoot, committed)
	for i, key := range keys {
		got, err := dbTarget.Get(key)
		r.NoError(err, "Get key %d", i)
		r.Equal(vals[i], got)
	}
	r.NoError(verified.Drop())
}

// TestVerifiedChangeProofCommitAfterDatabaseAdvanced checks that a stale
// change proof is verified again rather than rebased. The target changes a key
// inside the proven range after Verify; re-applying the proof's operations on
// top of that no longer reproduces the verified end root over the range, so
// Commit fails and leaves the target untouched. A rebase would have committed
// the operations and returned a root the proof never vouched for.
func TestVerifiedChangeProofCommitAfterDatabaseAdvanced(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	verified, _ := newVerifiedChangeProof(t, dbA, dbB)

	// newVerifiedChangeProof seeds both databases with the first half of
	// kvForTest(100); the proof's operations never touch that half, so the
	// tampered value survives them and is caught by the end-root check.
	keys, _, _ := kvForTest(100)
	advanced, err := dbB.Update([]BatchOp{Put(keys[0], []byte("tampered"))})
	r.NoError(err)

	_, err = verified.Commit()
	r.Error(err, "a stale change proof must be re-verified, not rebased")
	r.Equal(advanced, dbB.Root(), "a failed Commit leaves the database as it was")

	// The failed commit left the state Pending; the retry re-verifies against
	// the same advanced revision and fails the same way.
	_, err = verified.Commit()
	r.Error(err)
	r.Equal(advanced, dbB.Root())

	r.NoError(verified.Drop())
}

func TestRangeProofCodeHashes(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	key, val, codeHash := ethAccountWithCodeHash(t)
	root, err := db.Update([]BatchOp{Put(key[:], val)})
	r.NoError(err)

	proof := newRangeProof(t, db, root, nothing(), nothing(), rangeProofLenUnbounded)
	verified := verifyRangeProof(t, proof, root, nothing(), nothing(), rangeProofLenUnbounded)

	hashes, err := verified.CodeHashes()
	if selectedHashMode == ethhashKey {
		r.NoError(err, "%T.CodeHashes()", verified)
		r.Equal([]Hash{codeHash}, hashes)
	} else {
		r.ErrorContains(err, "code hash iteration requires an ethereum-mode proof")
	}
	r.NoError(verified.Drop())
}

func TestChangeProofCodeHashes(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// Baseline insert so the change proof's start root is non-empty. The key
	// is shorter than 32 bytes so it is naturally skipped by the code-hash
	// extractor's account-key filter even if it were ever present in the
	// change proof's operations (it should not be, since it is unchanged in
	// endRoot).
	startRoot, err := db.Update([]BatchOp{Put([]byte("baseline"), []byte("v"))})
	r.NoError(err)

	key, val, codeHash := ethAccountWithCodeHash(t)
	endRoot, err := db.Update([]BatchOp{Put(key[:], val)})
	r.NoError(err)

	proof, err := db.ChangeProof(startRoot, endRoot, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)

	// db's latest revision is endRoot, so applying the proof reproduces it.
	verified, err := proof.Verify(endRoot, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)

	hashes, err := verified.CodeHashes()
	if selectedHashMode == ethhashKey {
		r.NoError(err, "%T.CodeHashes()", verified)
		r.Equal([]Hash{codeHash}, hashes)
	} else {
		r.ErrorContains(err, "code hash iteration requires an ethereum-mode proof")
	}
	r.NoError(verified.Drop())
}

func TestRangeProofBlocksClose(t *testing.T) {
	tests := []struct {
		name      string
		makeProof func(t *testing.T, db *Database, root Hash) *VerifiedRangeProof
	}{
		{
			name: "verified_generated",
			makeProof: func(t *testing.T, db *Database, root Hash) *VerifiedRangeProof {
				proof := newRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
				return verifyRangeProof(t, proof, root, nothing(), nothing(), rangeProofLenTruncated)
			},
		},
		{
			name: "verified_unmarshaled",
			makeProof: func(t *testing.T, db *Database, root Hash) *VerifiedRangeProof {
				data := newSerializedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
				proof, err := db.UnmarshalRangeProof(data)
				require.NoError(t, err)
				return verifyRangeProof(t, proof, root, nothing(), nothing(), rangeProofLenTruncated)
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			_, _, batch := kvForTest(50)
			db := newTestDatabase(t)

			root, err := db.Update(batch)
			require.NoError(t, err)

			verified := tt.makeProof(t, db, root)
			require.ErrorIs(t, db.Close(oneSecCtx(t)), ErrActiveKeepAliveHandles)

			require.NoError(t, verified.Drop())
			require.NoError(t, db.Close(oneSecCtx(t)))
		})
	}
}

// TestRangeProofCommitKeepsLease checks that committing a verified proof does
// not release its lease: the proof stays usable, and Close waits until it is
// dropped. The unverified proof it came from is unaffected throughout.
func TestRangeProofCommitKeepsLease(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)
	_, _, batch := kvForTest(50)
	root, err := db.Update(batch)
	r.NoError(err)

	proof := newRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
	marshalledBeforeCommit, err := proof.Marshal()
	r.NoError(err)

	verified := verifyRangeProof(t, proof, root, nothing(), nothing(), rangeProofLenTruncated)
	_, err = verified.Commit()
	r.NoError(err)

	r.ErrorIs(db.Close(oneSecCtx(t)), ErrActiveKeepAliveHandles, "a committed proof must keep its lease")

	marshalledAfterCommit, err := proof.Marshal()
	r.NoError(err)
	r.Equal(marshalledBeforeCommit, marshalledAfterCommit)

	r.NoError(verified.Drop())
	r.NoError(proof.Drop())
	r.NoError(db.Close(oneSecCtx(t)))
}

// TestRangeProofCleanup verifies that the GC cleanup releases the keep-alive
// lease when a verified proof goes out of scope, before and after commit.
func TestRangeProofCleanup(t *testing.T) {
	tests := []struct {
		name      string
		makeProof func(*testing.T, *Database, Hash) *VerifiedRangeProof
	}{
		{
			name: "verified",
			makeProof: func(t *testing.T, db *Database, root Hash) *VerifiedRangeProof {
				proof := newRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
				return verifyRangeProof(t, proof, root, nothing(), nothing(), rangeProofLenTruncated)
			},
		},
		{
			name: "committed",
			makeProof: func(t *testing.T, db *Database, root Hash) *VerifiedRangeProof {
				proof := newRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
				verified := verifyRangeProof(t, proof, root, nothing(), nothing(), rangeProofLenTruncated)
				_, err := verified.Commit()
				require.NoError(t, err)
				return verified
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()

			db := newTestDatabase(t)
			_, _, batch := kvForTest(50)
			root, err := db.Update(batch)
			require.NoError(t, err)

			verified := tt.makeProof(t, db, root)

			require.ErrorIs(t, db.Close(oneSecCtx(t)), ErrActiveKeepAliveHandles)

			runtime.KeepAlive(verified)
			verified = nil //nolint:ineffassign // necessary to drop the reference for GC
			runtime.GC()

			require.NoError(t, db.Close(t.Context()), "Database should be closeable after proof is garbage collected")
		})
	}
}

// TestUnboundRangeProofOutlivesDatabase checks that an unverified range proof
// never holds a lease: it survives a force-close, still serializes, and only
// Verify, which needs the database, reports the closure.
func TestUnboundRangeProofOutlivesDatabase(t *testing.T) {
	var wrongRoot Hash
	wrongRoot[0] = 0xff

	tests := []struct {
		name      string
		makeProof func(*testing.T, *Database, Hash) *RangeProof
	}{
		{
			name: "generated",
			makeProof: func(t *testing.T, db *Database, root Hash) *RangeProof {
				return newRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
			},
		},
		{
			name: "unmarshaled",
			makeProof: func(t *testing.T, db *Database, root Hash) *RangeProof {
				data := newSerializedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
				proof, err := db.UnmarshalRangeProof(data)
				require.NoError(t, err)
				return proof
			},
		},
		{
			name: "failed_verify",
			makeProof: func(t *testing.T, db *Database, root Hash) *RangeProof {
				proof := newRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
				_, err := proof.Verify(wrongRoot, nothing(), nothing(), rangeProofLenTruncated)
				require.ErrorContains(t, err, "proof error")
				return proof
			},
		},
		{
			name: "verified_then_dropped",
			makeProof: func(t *testing.T, db *Database, root Hash) *RangeProof {
				proof := newRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
				verified := verifyRangeProof(t, proof, root, nothing(), nothing(), rangeProofLenTruncated)
				require.NoError(t, verified.Drop())
				return proof
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			db := newTestDatabase(t)
			_, _, batch := kvForTest(50)
			root, err := db.Update(batch)
			require.NoError(t, err)

			proof := tt.makeProof(t, db, root)
			require.NoError(t, db.Close(oneSecCtx(t), WithForceCloseHandles()))

			_, err = proof.Marshal()
			require.NoError(t, err)
			_, err = proof.Verify(root, nothing(), nothing(), rangeProofLenTruncated)
			require.ErrorIs(t, err, errDBClosed)
			require.NoError(t, proof.Drop())
		})
	}
}

// TestRangeProofBoundToParsingDatabase checks that a verified proof's lease is
// on the database that parsed and verified it, not the one that generated it.
func TestRangeProofBoundToParsingDatabase(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	_, _, batch := kvForTest(50)
	root, err := dbA.Update(batch)
	r.NoError(err)

	proof := newRangeProof(t, dbA, root, nothing(), nothing(), rangeProofLenTruncated)
	proof = transferRangeProof(t, proof, dbB)
	verified := verifyRangeProof(t, proof, root, nothing(), nothing(), rangeProofLenTruncated)

	r.NoError(dbA.Close(oneSecCtx(t)), "the generating database holds no lease")
	r.ErrorIs(dbB.Close(oneSecCtx(t)), ErrActiveKeepAliveHandles, "the verifying database holds the lease")

	r.NoError(verified.Drop())
	r.NoError(proof.Drop())
	r.NoError(dbB.Close(oneSecCtx(t)))
}

// TestUnmarshalRejectsOtherHashMode checks that a proof encoded for one hash
// mode is rejected when parsed for a database in the other mode, before any
// verification.
func TestUnmarshalRejectsOtherHashMode(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	otherAlgorithm := MerkleDBNodeHashing
	if selectedHashMode != ethhashKey {
		otherAlgorithm = EthereumNodeHashing
	}
	other, err := New(t.TempDir(), otherAlgorithm)
	r.NoError(err)
	t.Cleanup(func() { require.NoError(t, other.Close(oneSecCtx(t))) })

	_, _, batch := kvForTest(100)
	startRoot, err := db.Update(batch[:50])
	r.NoError(err)
	endRoot, err := db.Update(batch[50:])
	r.NoError(err)

	rangeBytes := newSerializedRangeProof(t, db, endRoot, nothing(), nothing(), rangeProofLenTruncated)
	_, err = other.UnmarshalRangeProof(rangeBytes)
	r.ErrorContains(err, "hash mode")

	changeBytes := newSerializedChangeProof(t, db, startRoot, endRoot, nothing(), nothing())
	_, err = other.UnmarshalChangeProof(changeBytes)
	r.ErrorContains(err, "hash mode")
}

// TestUnmarshalOnClosedDatabase checks that parsing needs an open database.
func TestUnmarshalOnClosedDatabase(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	startRoot, err := db.Update(batch[:50])
	r.NoError(err)
	endRoot, err := db.Update(batch[50:])
	r.NoError(err)
	rangeBytes := newSerializedRangeProof(t, db, endRoot, nothing(), nothing(), rangeProofLenTruncated)
	changeBytes := newSerializedChangeProof(t, db, startRoot, endRoot, nothing(), nothing())

	r.NoError(db.Close(oneSecCtx(t)))

	_, err = db.UnmarshalRangeProof(rangeBytes)
	r.ErrorIs(err, errDBClosed)
	_, err = db.UnmarshalChangeProof(changeBytes)
	r.ErrorIs(err, errDBClosed)
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
	proof1 := newSerializedChangeProof(t, db, root1, root2, nothing(), nothing())
	r.NoError(err)

	// Insert more data
	root3, err := db.Update(batch[5000:])
	r.NoError(err)
	r.NotEqual(root2, root3)

	// Get a proof again
	proof2 := newSerializedChangeProof(t, db, root2, root3, nothing(), nothing())
	// Ensure the proofs are different
	r.NotEqual(proof1, proof2)
}

func TestRoundTripChangeProofSerialization(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	_, _, batch := kvForTest(10)
	root1, err := db.Update(batch[:5])
	r.NoError(err)

	root2, err := db.Update(batch[5:])
	r.NoError(err)

	proofBytes := newSerializedChangeProof(t, db, root1, root2, nothing(), nothing())

	proof, err := db.UnmarshalChangeProof(proofBytes)
	r.NoError(err)

	serialized, err := proof.Marshal()
	r.NoError(err)
	r.Equal(proofBytes, serialized)
}

func TestVerifyChangeProof(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	_, _, batch := kvForTest(10)
	rootA, err := dbA.Update(batch[:5])
	r.NoError(err)
	rootB, err := dbB.Update(batch[:5])
	r.NoError(err)
	r.Equal(rootA, rootB)

	rootAUpdated, err := dbA.Update(batch[5:])
	r.NoError(err)

	changeProof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	changeProof = transferChangeProof(t, changeProof, dbB)

	verified, err := changeProof.Verify(rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	r.NoError(verified.Drop())
}

func TestVerifyEmptyChangeProofRange(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	_, _, batch := kvForTest(9)
	rootA, err := dbA.Update(batch[:5])
	r.NoError(err)
	rootB, err := dbB.Update(batch[:5])
	r.NoError(err)
	r.Equal(rootA, rootB)

	rootAUpdated, err := dbA.Update(batch[5:])
	r.NoError(err)

	startKey := something([]byte("key0"))
	endKey := something([]byte("key1"))

	// Both keys are from the first insert, so the proof carries no changes.
	changeProof, err := dbA.ChangeProof(rootA, rootAUpdated, startKey, endKey, 5)
	r.NoError(err)
	changeProof = transferChangeProof(t, changeProof, dbB)

	verified, err := changeProof.Verify(rootAUpdated, startKey, endKey, 5)
	r.NoError(err)
	r.NoError(verified.Drop())
}

func TestVerifiedChangeProofCommitAppliesValues(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	keys, vals, batch := kvForTest(100)
	root, err := dbA.Update(batch[:50])
	r.NoError(err)
	_, err = dbB.Update(batch[:50])
	r.NoError(err)

	rootAUpdated, err := dbA.Update(batch[50:])
	r.NoError(err)

	changeProof, err := dbA.ChangeProof(root, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	changeProof = transferChangeProof(t, changeProof, dbB)

	verified, err := changeProof.Verify(rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)

	rootBUpdated, err := verified.Commit()
	r.NoError(err)
	r.Equal(rootAUpdated, rootBUpdated)

	// A second Commit returns the cached root.
	again, err := verified.Commit()
	r.NoError(err)
	r.Equal(rootBUpdated, again)

	for i, key := range keys {
		got, err := dbB.Get(key)
		r.NoError(err, "Get key %d", i)
		r.Equal(vals[i], got, "Value mismatch for key %d", i)
	}
	r.NoError(verified.Drop())
}

func TestChangeProofNextKeyRanges(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	_, _, batch := kvForTest(10000)
	rootA, err := dbA.Update(batch[:5000])
	r.NoError(err)

	_, err = dbB.Update(batch[:5000])
	r.NoError(err)

	rootAUpdated, err := dbA.Update(batch[5000:])
	r.NoError(err)

	proof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenTruncated)
	r.NoError(err)
	proof = transferChangeProof(t, proof, dbB)

	verified, err := proof.Verify(rootAUpdated, nothing(), nothing(), changeProofLenTruncated)
	r.NoError(err)

	ranges, err := verified.NextKeyRanges()
	r.NoError(err)
	r.Len(ranges, 1)
	startKey := ranges[0].StartKey
	r.NotEmpty(startKey)
	r.False(ranges[0].EndKey.HasValue())

	_, err = verified.Commit()
	r.NoError(err)

	// NextKeyRanges reads the proof, not the proposal, so commit changes nothing.
	ranges, err = verified.NextKeyRanges()
	r.NoError(err)
	r.Len(ranges, 1)
	r.Equal(startKey, ranges[0].StartKey)
	r.NoError(verified.Drop())
}

// TestVerifiedChangeProofKeepAlive checks that a verified change proof holds
// its database's lease until it is dropped or collected, and that Commit does
// not release it.
func TestVerifiedChangeProofKeepAlive(t *testing.T) {
	tests := []struct {
		name    string
		release func(t *testing.T, p *VerifiedChangeProof)
	}{
		{
			"drop", func(t *testing.T, p *VerifiedChangeProof) {
				require.NoError(t, p.Drop())
			},
		},
		{
			"commit_then_drop", func(t *testing.T, p *VerifiedChangeProof) {
				_, err := p.Commit()
				require.NoError(t, err)
				require.ErrorIs(t, p.db.Close(oneSecCtx(t)), ErrActiveKeepAliveHandles, "commit must not release the lease")
				require.NoError(t, p.Drop())
			},
		},
		{
			"gc", func(_ *testing.T, p *VerifiedChangeProof) {
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

			verified, _ := newVerifiedChangeProof(t, dbA, dbB)

			r.ErrorIs(dbB.Close(oneSecCtx(t)), ErrActiveKeepAliveHandles)

			tt.release(t, verified)

			r.NoError(dbB.Close(oneSecCtx(t)))
		})
	}
}

func TestMultiRoundChangeProof(t *testing.T) {
	tests := []struct {
		name       string
		hasDeletes bool
	}{
		{name: "Multi-round change proofs with no deletes", hasDeletes: false},
		{name: "Multi-round change proofs with deletes", hasDeletes: true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			dbA := newTestDatabase(t)
			dbB := newTestDatabase(t)

			keys, vals, batch := kvForTest(100)
			rootA, err := dbA.Update(batch[:50])
			require.NoError(t, err)

			rootB, err := dbB.Update(batch[:50])
			require.NoError(t, err)

			rootAUpdated, err := dbA.Update(batch[50:])
			require.NoError(t, err)

			if tt.hasDeletes {
				delKeys := make([]BatchOp, 20)
				for i := range delKeys {
					keyIdx := i * 2
					delKeys[i] = Delete(keys[keyIdx])
					keys[keyIdx] = nil
				}
				rootAUpdated, err = dbA.Update(delKeys)
				require.NoError(t, err)
			}

			startKey := nothing()

			// Bound the rounds so a non-converging proof fails fast on the rootB
			// mismatch below instead of looping forever.
			for range 10 {
				proof, err := dbA.ChangeProof(rootA, rootAUpdated, startKey, nothing(), changeProofLenTruncated)
				require.NoError(t, err)
				proof = transferChangeProof(t, proof, dbB)

				verified, err := proof.Verify(rootAUpdated, startKey, nothing(), changeProofLenTruncated)
				require.NoError(t, err)

				rootB, err = verified.Commit()
				require.NoError(t, err)

				ranges, err := verified.NextKeyRanges()
				require.NoError(t, err)
				require.NoError(t, verified.Drop())
				require.NoError(t, proof.Drop())
				if len(ranges) == 0 {
					break
				}
				startKey = something(ranges[0].StartKey)
			}

			require.Equal(t, rootAUpdated, rootB)

			for i, key := range keys {
				if key == nil {
					continue
				}
				got, err := dbB.Get(key)
				require.NoError(t, err, "Get key %d", i)
				require.Equal(t, vals[i], got, "Value mismatch for %s", string(key))
			}
		})
	}
}

// TestChangeProofMarshalWorksAfterVerify verifies that Marshal on a
// ChangeProof still works after verification, since Verify borrows the proof
// rather than consuming it.
func TestChangeProofMarshalWorksAfterVerify(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	_, _, batch := kvForTest(10)
	rootA, err := dbA.Update(batch[:5])
	r.NoError(err)
	_, err = dbB.Update(batch[:5])
	r.NoError(err)

	rootAUpdated, err := dbA.Update(batch[5:])
	r.NoError(err)

	changeProof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	changeProof = transferChangeProof(t, changeProof, dbB)

	marshalledBefore, err := changeProof.Marshal()
	r.NoError(err)
	r.NotEmpty(marshalledBefore)

	verified, err := changeProof.Verify(rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)

	marshalledAfter, err := changeProof.Marshal()
	r.NoError(err)
	r.Equal(marshalledBefore, marshalledAfter)
	r.NoError(verified.Drop())
}

// A truncated range proof proves only a prefix of the requested range, so
// applying it must leave local keys past the proven edge alone: no proof covers
// them.
func TestRangeProofTruncatedDoesNotDeleteBeyondProvenEdge(t *testing.T) {
	r := require.New(t)

	dbSource := newTestDatabase(t)
	dbTarget := newTestDatabase(t)

	keys, vals, batch := kvForTest(50)

	sourceRoot, err := dbSource.Update(batch)
	r.NoError(err)

	// Seed the target with identical data, so it holds keys past the edge a
	// truncated reply will prove. Nothing here should be deleted.
	_, err = dbTarget.Update(batch)
	r.NoError(err)

	// Request the whole keyspace but cap the reply, forcing truncation.
	proof := newRangeProof(t, dbSource, sourceRoot, nothing(), nothing(), rangeProofLenTruncated)
	proof = transferRangeProof(t, proof, dbTarget)
	verified := verifyRangeProof(t, proof, sourceRoot, nothing(), nothing(), rangeProofLenTruncated)

	_, err = verified.Commit()
	r.NoError(err)

	for i, key := range keys {
		got, err := dbTarget.Get(key)
		r.NoError(err, "Get key %d", i)
		r.Equal(vals[i], got, "key %d was deleted or altered by a proof that never covered it", i)
	}
	r.NoError(verified.Drop())
}

// Within the proven prefix, a range proof proves that only the key-values it
// carries exist, so anything else there must be deleted. This is the mirror of
// TestRangeProofTruncatedDoesNotDeleteBeyondProvenEdge, which guards a bound
// looser than the proof justifies; a bound tighter than it justifies stops the
// merge's trie scan short and strands the synthetic stale key below.
//
// Only a stale local key can observe the bound: it gates the trie-side scan
// alone, so against an empty target (as in
// TestRangeProofNextKeyRangesDivergentReceiver) every key-value is applied
// whatever the bound is.
func TestRangeProofTruncatedDeletesStaleKeyWithinProvenEdge(t *testing.T) {
	r := require.New(t)

	dbSource := newTestDatabase(t)
	dbTarget := newTestDatabase(t)

	keys, _, batch := kvForTest(50)

	sourceRoot, err := dbSource.Update(batch)
	r.NoError(err)

	_, err = dbTarget.Update(batch)
	r.NoError(err)

	// A key present only in dbTarget, and absent from the proof's key-values,
	// so a correctly-bounded merge must delete it. The assertions pin it inside
	// the proven range rather than assuming it.
	staleKey := append(append([]byte{}, keys[0]...), 0)
	r.Negative(bytes.Compare(keys[0], staleKey), "synthetic key must sort after keys[0]")
	r.Negative(bytes.Compare(staleKey, keys[rangeProofLenTruncated-1]),
		"synthetic key must sort inside the proven range")
	_, err = dbTarget.Update([]BatchOp{Put(staleKey, []byte("stale"))})
	r.NoError(err)

	proof := newRangeProof(t, dbSource, sourceRoot, nothing(), nothing(), rangeProofLenTruncated)
	proof = transferRangeProof(t, proof, dbTarget)
	verified := verifyRangeProof(t, proof, sourceRoot, nothing(), nothing(), rangeProofLenTruncated)

	_, err = verified.Commit()
	r.NoError(err)

	got, err := dbTarget.Get(staleKey)
	r.NoError(err)
	r.Nil(got, "stale key inside the proven range should have been deleted")
	r.NoError(verified.Drop())
}

// TestUnboundChangeProofOutlivesDatabase checks that an unverified change proof
// never holds a lease, whatever was done with it.
func TestUnboundChangeProofOutlivesDatabase(t *testing.T) {
	tests := []struct {
		name      string
		makeProof func(*testing.T, *Database, Hash, Hash) *ChangeProof
	}{
		{
			name: "generated",
			makeProof: func(t *testing.T, db *Database, startRoot, endRoot Hash) *ChangeProof {
				proof, err := db.ChangeProof(startRoot, endRoot, nothing(), nothing(), changeProofLenUnbounded)
				require.NoError(t, err)
				return proof
			},
		},
		{
			name: "unmarshaled",
			makeProof: func(t *testing.T, db *Database, startRoot, endRoot Hash) *ChangeProof {
				data := newSerializedChangeProof(t, db, startRoot, endRoot, nothing(), nothing())
				proof, err := db.UnmarshalChangeProof(data)
				require.NoError(t, err)
				return proof
			},
		},
		{
			name: "verified_then_dropped",
			makeProof: func(t *testing.T, db *Database, startRoot, endRoot Hash) *ChangeProof {
				proof, err := db.ChangeProof(startRoot, endRoot, nothing(), nothing(), changeProofLenUnbounded)
				require.NoError(t, err)
				verified, err := proof.Verify(endRoot, nothing(), nothing(), changeProofLenUnbounded)
				require.NoError(t, err)
				require.NoError(t, verified.Drop())
				return proof
			},
		},
		{
			name: "committed_then_dropped",
			makeProof: func(t *testing.T, db *Database, startRoot, endRoot Hash) *ChangeProof {
				proof, err := db.ChangeProof(startRoot, endRoot, nothing(), nothing(), changeProofLenUnbounded)
				require.NoError(t, err)
				verified, err := proof.Verify(endRoot, nothing(), nothing(), changeProofLenUnbounded)
				require.NoError(t, err)
				_, err = verified.Commit()
				require.NoError(t, err)
				require.NoError(t, verified.Drop())
				return proof
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			db := newTestDatabase(t)
			_, _, batch := kvForTest(100)
			startRoot, err := db.Update(batch[:50])
			require.NoError(t, err)
			endRoot, err := db.Update(batch[50:])
			require.NoError(t, err)

			proof := tt.makeProof(t, db, startRoot, endRoot)
			require.NoError(t, db.Close(oneSecCtx(t), WithForceCloseHandles()))

			_, err = proof.Marshal()
			require.NoError(t, err)
			_, err = proof.Verify(endRoot, nothing(), nothing(), changeProofLenUnbounded)
			require.ErrorIs(t, err, errDBClosed)
			require.NoError(t, proof.Drop())
		})
	}
}
