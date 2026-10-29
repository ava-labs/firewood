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

// newVerifiedRangeProof generates a range proof for the given parameters and
// verifies using [RangeProof.Verify] which does not prepare a proposal.
func newVerifiedRangeProof(
	t *testing.T,
	db *Database,
	root Hash,
	startKey, endKey maybe,
	proofLen uint32,
) *RangeProof {
	r := require.New(t)

	proof, err := db.RangeProof(root, startKey, endKey, proofLen)
	r.NoError(err)
	assertProofNotNil(t, proof)

	r.NoError(proof.Verify(root, startKey, endKey, proofLen))

	return proof
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
	r := require.New(t)

	proof := newVerifiedRangeProof(t, db, root, startKey, endKey, proofLen)

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
	r := require.New(t)

	proof, err := db.ChangeProof(startRoot, endRoot, startKey, endKey, changeProofLenUnbounded)
	r.NoError(err)

	proofBytes, err := proof.Marshal()
	r.NoError(err)
	r.NoError(proof.Drop())

	return proofBytes
}

// newTransferredChangeProof seeds src and dst with the first half of
// kvForTest(100), advances src with the second half, and returns the change
// proof for [startKey, endKey] parsed on dst together with src's new root.
func newTransferredChangeProof(
	t *testing.T,
	src, dst *Database,
	startKey, endKey maybe,
) (*ChangeProof, Hash) {
	t.Helper()
	r := require.New(t)

	_, _, batch := kvForTest(100)
	startRoot, err := src.Update(batch[:50])
	r.NoError(err)
	dstRoot, err := dst.Update(batch[:50])
	r.NoError(err)
	r.Equal(startRoot, dstRoot)

	endRoot, err := src.Update(batch[50:])
	r.NoError(err)

	changeProof, err := src.ChangeProof(startRoot, endRoot, startKey, endKey, changeProofLenUnbounded)
	r.NoError(err)
	return transferChangeProof(t, changeProof, dst), endRoot
}

// newVerifiedChangeProof is [newTransferredChangeProof] over the whole
// keyspace, verified on dst. The helper never calls Drop or registers a
// t.Cleanup on the verified proof, so the caller controls the release path
// (Drop, Commit then Drop, or GC); TestChangeProofProposalKeepAlive exercises
// all three.
func newVerifiedChangeProof(
	t *testing.T,
	src, dst *Database,
) (*VerifiedChangeProof, Hash) {
	t.Helper()
	r := require.New(t)

	changeProof, endRoot := newTransferredChangeProof(t, src, dst, nothing(), nothing())
	verified, err := changeProof.Verify(endRoot, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	r.NoError(changeProof.Drop())

	return verified, endRoot
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

	// Insert some data.
	_, _, batch := kvForTest(10)
	root, err := db.Update(batch)
	r.NoError(err)

	// get a proof
	proofBytes := newSerializedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenUnbounded)

	// Deserialize the proof.
	proof, err := db.UnmarshalRangeProof(proofBytes)
	r.NoError(err)

	// serialize the proof again
	serialized, err := proof.Marshal()
	r.NoError(err)
	r.Equal(proofBytes, serialized)
}

func TestRangeProofVerify(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	root, err := db.Update(batch)
	r.NoError(err)

	// not using `newVerifiedRangeProof` so we can test Verify separately
	proof, err := db.RangeProof(root, nothing(), nothing(), rangeProofLenTruncated)
	r.NoError(err)

	// Verify with wrong root should fail
	root[0] ^= 0xFF
	err = proof.Verify(root, nothing(), nothing(), rangeProofLenTruncated)
	r.Error(err, "Verification with wrong root should fail")
}

func TestVerifyAndCommitRangeProof(t *testing.T) {
	r := require.New(t)

	// Create source and target databases
	dbSource := newTestDatabase(t)
	dbTarget := newTestDatabase(t)

	// Populate source
	keys, vals, batch := kvForTest(50)
	sourceRoot, err := dbSource.Update(batch)
	r.NoError(err)

	proof := newVerifiedRangeProof(t, dbSource, sourceRoot, nothing(), nothing(), rangeProofLenUnbounded)

	// Verify and commit to target without previously calling db.VerifyRangeProof
	committedRoot, err := dbTarget.VerifyAndCommitRangeProof(proof, nothing(), nothing(), sourceRoot, rangeProofLenUnbounded)
	r.NoError(err)
	r.Equal(sourceRoot, committedRoot)

	// Verify all keys are now in target database
	for i, key := range keys {
		got, err := dbTarget.Get(key)
		r.NoError(err, "Get key %d", i)
		r.Equal(vals[i], got, "Value mismatch for key %d", i)
	}
}

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

	// The proof was taken from db's own state, so merging it over the proven
	// range is a no-op and the proposal keeps `root`. That equality is the
	// FFI's "already caught up" short-circuit, so there is nothing left to
	// fetch — before commit here and after commit below. A NotNil here means
	// the apply path deleted outside the proven range and moved the root.
	// TestRangeProofFindNextKeyDivergentReceiver covers a receiver that is
	// genuinely behind.
	nextRange, err := proof.FindNextKey()
	r.NoError(err)
	r.Nil(nextRange)

	_, err = db.VerifyAndCommitRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated)
	r.NoError(err)

	nextRange, err = proof.FindNextKey()
	r.NoError(err)
	r.Nil(nextRange)
}

// TestRangeProofFindNextKeyDivergentReceiver applies a truncated proof to an
// empty target: a receiver genuinely behind the proof's root must report more
// to fetch.
func TestRangeProofFindNextKeyDivergentReceiver(t *testing.T) {
	r := require.New(t)

	dbSource := newTestDatabase(t)
	dbTarget := newTestDatabase(t)

	keys, vals, batch := kvForTest(100)
	sourceRoot, err := dbSource.Update(batch)
	r.NoError(err)

	// dbTarget starts empty, so it is materially behind sourceRoot.
	proof := newVerifiedRangeProof(t, dbSource, sourceRoot, nothing(), nothing(), rangeProofLenTruncated)

	_, err = dbTarget.VerifyAndCommitRangeProof(proof, nothing(), nothing(), sourceRoot, rangeProofLenTruncated)
	r.NoError(err)

	// The proven prefix is written. Against an empty target this says nothing
	// about where the bound falls — only the sibling tests named in
	// TestRangeProofTruncatedDeletesStaleKeyWithinProvenEdge do.
	for i := range rangeProofLenTruncated {
		got, err := dbTarget.Get(keys[i])
		r.NoError(err, "Get key %d", i)
		r.Equal(vals[i], got, "key %d from the proven prefix was not applied", i)
	}

	// The truncated proof only proved a prefix, and dbTarget started with
	// none of the data, so there is genuinely more to fetch.
	nextRange, err := proof.FindNextKey()
	r.NoError(err)
	r.NotNil(nextRange)
	r.False(nextRange.EndKey.HasValue(), "the request was unbounded above")

	// The next request resumes after the last proven key without skipping the
	// first unproven one. The keys are Go-owned, so they outlive the proof.
	r.NoError(proof.Drop())
	r.Positive(bytes.Compare(nextRange.StartKey, keys[rangeProofLenTruncated-1]), "start key repeats a proven key")
	r.LessOrEqual(bytes.Compare(nextRange.StartKey, keys[rangeProofLenTruncated]), 0, "start key skips an unproven key")
}

// TestRangeProofMethodFreeRace is a regression test for ava-labs/firewood#2137.
//
// Every RangeProof method that passes the handle into cgo must hold lease.mu,
// which serializes it against Drop freeing the Rust RangeProofContext. In
// production the racing Drop is the GC cleanup, which can run while a cgo call
// is in flight; a method that skips the lock is a use-after-free, reported in
// the issue as "SIGSEGV ... signal arrived during cgo execution" inside
// fwd_range_proof_find_next_key.
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

	ops := []struct {
		name string
		call func(*RangeProof)
	}{
		{
			name: "FindNextKey",
			call: func(p *RangeProof) { _, _ = p.FindNextKey() },
		},
		{
			name: "Verify",
			call: func(p *RangeProof) {
				_ = p.Verify(root, nothing(), nothing(), rangeProofLenTruncated)
			},
		},
		{
			name: "VerifyRangeProof",
			call: func(p *RangeProof) {
				_ = db.VerifyRangeProof(p, nothing(), nothing(), root, rangeProofLenTruncated)
			},
		},
		{
			name: "Marshal",
			call: func(p *RangeProof) { _, _ = p.Marshal() },
		},
		{
			name: "CodeHashes",
			call: func(p *RangeProof) { _, _ = p.CodeHashes() },
		},
	}

	for _, op := range ops {
		t.Run(op.name, func(t *testing.T) {
			// -race flags the conflicting p.handle access as soon as one
			// iteration's method and Drop overlap, which the start barrier makes
			// near-certain per iteration; a few thousand iterations is ample
			// margin while keeping the -race CI run fast.
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
}

// TestChangeProofMethodDropRace is the ChangeProof counterpart of
// TestRangeProofMethodFreeRace: under -race, any method that passes the handle
// into cgo without holding lease.mu is reported as racing Drop.
func TestChangeProofMethodDropRace(t *testing.T) {
	db := newTestDatabase(t)
	_, _, batch := kvForTest(100)
	startRoot, err := db.Update(batch[:50])
	require.NoError(t, err)
	endRoot, err := db.Update(batch[50:])
	require.NoError(t, err)
	proofBytes := newSerializedChangeProof(t, db, startRoot, endRoot, nothing(), nothing())

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
				p, err := db.UnmarshalChangeProof(proofBytes)
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
}

// TestVerifiedChangeProofMethodDropRace is the VerifiedChangeProof counterpart
// of [TestChangeProofMethodDropRace]. The proof is verified against the
// database it was generated on, whose latest revision is already the end root.
func TestVerifiedChangeProofMethodDropRace(t *testing.T) {
	db := newTestDatabase(t)
	_, _, batch := kvForTest(100)
	startRoot, err := db.Update(batch[:50])
	require.NoError(t, err)
	endRoot, err := db.Update(batch[50:])
	require.NoError(t, err)
	proofBytes := newSerializedChangeProof(t, db, startRoot, endRoot, nothing(), nothing())

	ops := []struct {
		name string
		call func(*VerifiedChangeProof)
	}{
		{
			name: "Commit",
			call: func(p *VerifiedChangeProof) { _, _ = p.Commit() },
		},
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
			// Each iteration verifies, which builds a proposal, so fewer
			// iterations than the unverified case keep the -race run bounded.
			const iterations = 2_000
			for range iterations {
				p, err := db.UnmarshalChangeProof(proofBytes)
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
}

// TestRangeProofVerifyTwiceIdempotent verifies that preparing the same proof
// against the same database more than once is a no-op that leaves the proof
// usable.
func TestRangeProofVerifyTwiceIdempotent(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	_, _, batch := kvForTest(100)
	root, err := db.Update(batch)
	r.NoError(err)

	proof, err := db.RangeProof(root, nothing(), nothing(), rangeProofLenTruncated)
	r.NoError(err)

	r.NoError(db.VerifyRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated))
	r.NoError(db.VerifyRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated))

	// The proof is still usable after the redundant verify.
	_, err = proof.FindNextKey()
	r.NoError(err)
}

// TestVerifiedChangeProofCommitAfterDatabaseAdvanced checks that Commit fails,
// and leaves the target unchanged, when the target changes a key inside the
// proven range after Verify. A retry fails the same way.
func TestVerifiedChangeProofCommitAfterDatabaseAdvanced(t *testing.T) {
	r := require.New(t)
	src := newTestDatabase(t)
	dst := newTestDatabase(t)

	verified, _ := newVerifiedChangeProof(t, src, dst)

	// newVerifiedChangeProof seeds both databases with the first half of
	// kvForTest(100); the proof's operations never touch that half, so the
	// tampered value survives them and is caught by the end-root check.
	keys, _, _ := kvForTest(100)
	advanced, err := dst.Update([]BatchOp{Put(keys[0], []byte("tampered"))})
	r.NoError(err)

	_, err = verified.Commit()
	r.ErrorContains(err, "doesn't match the expected end root", "a stale change proof must be re-verified, not rebased")
	r.Equal(advanced, dst.Root(), "a failed Commit leaves the database as it was")

	// A retry fails the same way.
	_, err = verified.Commit()
	r.ErrorContains(err, "doesn't match the expected end root")
	r.Equal(advanced, dst.Root())

	r.NoError(verified.Drop())
}

// TestVerifiedChangeProofCommitAfterUnrelatedAdvance checks the rebuild path
// for change proofs: a proposal made stale by a commit outside the proven
// range is verified again against the latest revision and committed.
func TestVerifiedChangeProofCommitAfterUnrelatedAdvance(t *testing.T) {
	r := require.New(t)
	src := newTestDatabase(t)
	dst := newTestDatabase(t)

	keys, vals, _ := kvForTest(100)
	half := len(keys) / 2
	startKey, endKey := something(keys[half]), something(keys[len(keys)-1])
	proof, endRoot := newTransferredChangeProof(t, src, dst, startKey, endKey)
	verified, err := proof.Verify(endRoot, startKey, endKey, changeProofLenUnbounded)
	r.NoError(err)
	r.NoError(proof.Drop())

	// Sorts after every kvForTest key, so it is outside the proven range.
	unrelated := []byte{0xff, 0xff}
	advanced, err := dst.Update([]BatchOp{Put(unrelated, []byte("value"))})
	r.NoError(err)

	committed, err := verified.Commit()
	r.NoError(err, "a stale proposal is rebuilt when the proven range is untouched")
	r.NotEqual(advanced, committed)
	r.Equal(committed, dst.Root())
	for i := half; i < len(keys); i++ {
		got, err := dst.Get(keys[i])
		r.NoError(err, "Get key %d", i)
		r.Equal(vals[i], got)
	}
	got, err := dst.Get(unrelated)
	r.NoError(err)
	r.Equal([]byte("value"), got, "the key outside the proven range survives the commit")
	r.NoError(verified.Drop())
}

// TestChangeProofVerifyTwice checks that two verifications of one proof
// commit independently: the second, stale once the first commits, rebuilds
// its proposal and reaches the same root.
func TestChangeProofVerifyTwice(t *testing.T) {
	r := require.New(t)
	src := newTestDatabase(t)
	dst := newTestDatabase(t)

	proof, endRoot := newTransferredChangeProof(t, src, dst, nothing(), nothing())
	first, err := proof.Verify(endRoot, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	second, err := proof.Verify(endRoot, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)

	firstRoot, err := first.Commit()
	r.NoError(err)
	// The second proposal is stale once the first commits; re-verifying it
	// against the committed state reproduces the end root.
	secondRoot, err := second.Commit()
	r.NoError(err)
	r.Equal(endRoot, firstRoot)
	r.Equal(firstRoot, secondRoot)

	r.NoError(first.Drop())
	r.NoError(second.Drop())
	r.NoError(proof.Drop())
}

// TestChangeProofVerifyFailureLeavesNoLease checks that a failed Verify
// returns no verified proof and leaves no keep-alive lease on the database.
func TestChangeProofVerifyFailureLeavesNoLease(t *testing.T) {
	r := require.New(t)
	src := newTestDatabase(t)
	dst := newTestDatabase(t)

	proof, endRoot := newTransferredChangeProof(t, src, dst, nothing(), nothing())

	wrongRoot := endRoot
	wrongRoot[0] ^= 0xFF
	verified, err := proof.Verify(wrongRoot, nothing(), nothing(), changeProofLenUnbounded)
	r.Error(err, "Verification with wrong root should fail")
	r.Nil(verified)

	r.NoError(proof.Drop())
	r.NoError(dst.Close(oneSecCtx(t)), "a failed Verify must leave no lease behind")
}

func TestRangeProofCodeHashes(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	key, val, codeHash := ethAccountWithCodeHash(t)
	root, err := db.Update([]BatchOp{Put(key[:], val)})
	r.NoError(err)

	proof := newVerifiedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenUnbounded)

	hashes, err := proof.CodeHashes()
	if selectedHashMode == ethhashKey {
		r.NoError(err, "%T.CodeHashes()", proof)
		r.Equal([]Hash{codeHash}, hashes)
	} else {
		r.ErrorContains(err, "code hash iteration requires an ethereum-mode proof")
	}
}

func TestVerifiedChangeProofCodeHashes(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)

	// A baseline revision so the proof's start root is non-empty.
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
		makeProof func(t *testing.T, db *Database, root Hash) *RangeProof
	}{
		{
			name: "prepared_range",
			makeProof: func(t *testing.T, db *Database, root Hash) *RangeProof {
				proof := newVerifiedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
				require.NoError(t, db.VerifyRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated))
				return proof
			},
		},
		{
			name: "prepared_unmarshaled",
			makeProof: func(t *testing.T, db *Database, root Hash) *RangeProof {
				data := newSerializedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
				proof, err := db.UnmarshalRangeProof(data)
				require.NoError(t, err)
				require.NoError(t, db.VerifyRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated))
				return proof
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			_, _, batch := kvForTest(50)
			db := newTestDatabase(t)

			root, err := db.Update(batch)
			require.NoError(t, err)

			proof := tt.makeProof(t, db, root)
			require.ErrorIs(t, db.Close(oneSecCtx(t)), ErrActiveKeepAliveHandles)

			// Free the proof and ensure Close can complete.
			require.NoError(t, proof.Drop())
			require.NoError(t, db.Close(oneSecCtx(t)))
		})
	}
}

// TestRangeProofCommitKeepsLease checks that committing a proof does not
// release its lease: the proof stays usable, and Close waits until it is
// dropped.
func TestRangeProofCommitKeepsLease(t *testing.T) {
	r := require.New(t)
	db := newTestDatabase(t)
	_, _, batch := kvForTest(50)
	root, err := db.Update(batch)
	r.NoError(err)

	proof := newVerifiedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
	marshalledBeforeCommit, err := proof.Marshal()
	r.NoError(err)

	r.NoError(db.VerifyRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated))
	_, err = db.VerifyAndCommitRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated)
	r.NoError(err)

	r.ErrorIs(db.Close(oneSecCtx(t)), ErrActiveKeepAliveHandles, "a committed proof must keep its lease")

	marshalledAfterCommit, err := proof.Marshal()
	r.NoError(err)
	r.Equal(marshalledBeforeCommit, marshalledAfterCommit)

	r.NoError(proof.Drop())
	r.NoError(db.Close(oneSecCtx(t)))
}

// TestRangeProofCleanup verifies that the GC cleanup releases the keep-alive handle
// when the proof goes out of scope, for every way a proof can take its lease.
func TestRangeProofCleanup(t *testing.T) {
	tests := []struct {
		name      string
		makeProof func(*testing.T, *Database, Hash) *RangeProof
	}{
		{
			name: "prepared_range",
			makeProof: func(t *testing.T, db *Database, root Hash) *RangeProof {
				proof, err := db.RangeProof(root, nothing(), nothing(), rangeProofLenTruncated)
				require.NoError(t, err)
				require.NoError(t, db.VerifyRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated))
				return proof
			},
		},
		{
			name: "committed_range",
			makeProof: func(t *testing.T, db *Database, root Hash) *RangeProof {
				proof := newVerifiedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
				_, err := db.VerifyAndCommitRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated)
				require.NoError(t, err)
				return proof
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

			proof := tt.makeProof(t, db, root)

			require.ErrorIs(t, db.Close(oneSecCtx(t)), ErrActiveKeepAliveHandles)

			runtime.KeepAlive(proof)
			proof = nil //nolint:ineffassign // necessary to drop the reference for GC
			runtime.GC()

			require.NoError(t, db.Close(t.Context()), "Database should be closeable after proof is garbage collected")
		})
	}
}

func TestUnboundRangeProofOutlivesDatabase(t *testing.T) {
	var wrongRoot Hash
	wrongRoot[0] = 0xff

	tests := []struct {
		name      string
		makeProof func(*testing.T, *Database, Hash) *RangeProof
	}{
		{
			name: "from_range",
			makeProof: func(t *testing.T, db *Database, root Hash) *RangeProof {
				return newVerifiedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
			},
		},
		{
			name: "unmarshaled_range",
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
				proof := newVerifiedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
				require.ErrorContains(t, db.VerifyRangeProof(proof, nothing(), nothing(), wrongRoot, rangeProofLenTruncated), "proof error")
				return proof
			},
		},
		{
			name: "failed_commit",
			makeProof: func(t *testing.T, db *Database, root Hash) *RangeProof {
				proof := newVerifiedRangeProof(t, db, root, nothing(), nothing(), rangeProofLenTruncated)
				_, err := db.VerifyAndCommitRangeProof(proof, nothing(), nothing(), wrongRoot, rangeProofLenTruncated)
				require.ErrorContains(t, err, "proof error")
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
			require.NoError(t, proof.Verify(root, nothing(), nothing(), rangeProofLenTruncated))
			_, err = proof.FindNextKey()
			require.ErrorIs(t, err, errNotPrepared)
			require.NoError(t, proof.Drop())
		})
	}
}

// TestRangeProofLeasesVerifyingDatabase checks that a range proof's lease is
// on the database that verified it, not the one that created it, and that a
// bound proof is rejected by every other database.
func TestRangeProofLeasesVerifyingDatabase(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)
	dbC := newTestDatabase(t)

	_, _, batch := kvForTest(50)
	root, err := dbA.Update(batch)
	r.NoError(err)

	proof := newVerifiedRangeProof(t, dbA, root, nothing(), nothing(), rangeProofLenTruncated)
	r.NoError(dbB.VerifyRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated))

	r.NoError(dbA.Close(oneSecCtx(t)), "the creating database holds no lease")
	r.ErrorIs(dbB.Close(oneSecCtx(t)), ErrActiveKeepAliveHandles, "the verifying database holds the lease")

	r.ErrorIs(dbC.VerifyRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated), errBoundToOtherDatabase)
	_, err = dbC.VerifyAndCommitRangeProof(proof, nothing(), nothing(), root, rangeProofLenTruncated)
	r.ErrorIs(err, errBoundToOtherDatabase)

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

	// Insert some data.
	_, _, batch := kvForTest(10)
	root1, err := db.Update(batch[:5])
	r.NoError(err)

	root2, err := db.Update(batch[5:])
	r.NoError(err)

	// get a proof
	proofBytes := newSerializedChangeProof(t, db, root1, root2, nothing(), nothing())

	// Deserialize the proof.
	proof, err := db.UnmarshalChangeProof(proofBytes)
	r.NoError(err)

	// serialize the proof again
	serialized, err := proof.Marshal()
	r.NoError(err)
	r.Equal(proofBytes, serialized)
}

func TestVerifyChangeProof(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	// Insert some data.
	_, _, batch := kvForTest(10)
	rootA, err := dbA.Update(batch[:5])
	r.NoError(err)
	rootB, err := dbB.Update(batch[:5])
	r.NoError(err)
	r.Equal(rootA, rootB)

	// Insert more data into dbA but not dbB.
	rootAUpdated, err := dbA.Update(batch[5:])
	r.NoError(err)

	// Create a change proof from dbA.
	changeProof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)

	// Verify the change proof on dbB.
	changeProof = transferChangeProof(t, changeProof, dbB)
	verified, err := changeProof.Verify(rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	r.NoError(verified.Drop())
}

func TestVerifyEmptyChangeProofRange(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	// Insert some data.
	_, _, batch := kvForTest(9)
	rootA, err := dbA.Update(batch[:5])
	r.NoError(err)
	rootB, err := dbB.Update(batch[:5])
	r.NoError(err)
	r.Equal(rootA, rootB)

	// Insert more data into dbA but not dbB.
	rootAUpdated, err := dbA.Update(batch[5:])
	r.NoError(err)

	startKey := maybe{
		hasValue: true,
		value:    []byte("key0"),
	}

	endKey := maybe{
		hasValue: true,
		value:    []byte("key1"),
	}

	// Create a change proof from dbA. This should create an empty changeProof because
	// the start and end keys are both from the first insert.
	changeProof, err := dbA.ChangeProof(rootA, rootAUpdated, startKey, endKey, 5)
	r.NoError(err)

	// Verify the change proof and create an empty proposal on dbB.
	changeProof = transferChangeProof(t, changeProof, dbB)
	verified, err := changeProof.Verify(rootAUpdated, startKey, endKey, 5)
	r.NoError(err)
	r.NoError(verified.Drop())
}

func TestVerifiedChangeProofCommit(t *testing.T) {
	r := require.New(t)
	dbA := newTestDatabase(t)
	dbB := newTestDatabase(t)

	// Insert some data.
	keys, vals, batch := kvForTest(100)
	root, err := dbA.Update(batch[:50])
	r.NoError(err)
	_, err = dbB.Update(batch[:50])
	r.NoError(err)

	// Insert more data into dbA but not dbB.
	rootAUpdated, err := dbA.Update(batch[50:])
	r.NoError(err)

	// Create a change proof from dbA.
	changeProof, err := dbA.ChangeProof(root, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)

	// Verify the change proof on dbB.
	changeProof = transferChangeProof(t, changeProof, dbB)
	verified, err := changeProof.Verify(rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)

	// Commit the proof on dbB.
	rootBUpdated, err := verified.Commit()
	r.NoError(err)
	r.Equal(rootAUpdated, rootBUpdated)

	// A second Commit returns the same root.
	again, err := verified.Commit()
	r.NoError(err)
	r.Equal(rootBUpdated, again)

	// Verify all keys are now in dbB
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

// TestChangeProofProposalKeepAlive checks that a verified change proof, which
// owns a proposal, holds its database's lease until it is dropped or
// collected, and that Commit does not release it.
func TestChangeProofProposalKeepAlive(t *testing.T) {
	tests := []struct {
		name    string
		release func(*testing.T, *VerifiedChangeProof)
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
			// GC cleanup releases keep-alive
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

			// Database should not be closeable while the verified proof has keep-alive
			r.ErrorIs(dbB.Close(oneSecCtx(t)), ErrActiveKeepAliveHandles)

			tt.release(t, verified)

			// Database should now be closeable
			r.NoError(dbB.Close(oneSecCtx(t)))
		})
	}
}

func TestMultiRoundChangeProof(t *testing.T) {
	tests := []struct {
		name       string
		hasDeletes bool
	}{
		{
			name:       "Multi-round change proofs with no deletes",
			hasDeletes: false,
		},
		{
			name:       "Multi-round change proofs With deletes",
			hasDeletes: true,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			dbA := newTestDatabase(t)
			dbB := newTestDatabase(t)

			// Insert first half of data in the first batch
			keys, vals, batch := kvForTest(100)
			rootA, err := dbA.Update(batch[:50])
			require.NoError(t, err)

			rootB, err := dbB.Update(batch[:50])
			require.NoError(t, err)

			// Insert the rest in the second batch
			rootAUpdated, err := dbA.Update(batch[50:])
			require.NoError(t, err)

			if tt.hasDeletes {
				// Delete some of the keys. This will create Delete BatchOps in the
				// change proof.
				delKeys := make([]BatchOp, 20)
				for i := range delKeys {
					keyIdx := i * 2
					delKeys[i] = Delete(keys[keyIdx])
					keys[keyIdx] = nil
				}
				rootAUpdated, err = dbA.Update(delKeys)
				require.NoError(t, err)
			}

			// Create and commit multiple change proofs to update dbB to match dbA.
			startKey := nothing()

			// Loop limit to help with debugging
			for range 10 {
				proof, err := dbA.ChangeProof(rootA, rootAUpdated, startKey, nothing(), changeProofLenTruncated)
				require.NoError(t, err)

				// Verify the proof on dbB.
				proof = transferChangeProof(t, proof, dbB)
				verified, err := proof.Verify(rootAUpdated, startKey, nothing(), changeProofLenTruncated)
				require.NoError(t, err)

				// Commit the proof.
				rootB, err = verified.Commit()
				require.NoError(t, err)

				// Find the next start key from the proof.
				ranges, err := verified.NextKeyRanges()
				require.NoError(t, err)
				require.NoError(t, verified.Drop())
				require.NoError(t, proof.Drop())
				if len(ranges) == 0 {
					break
				}
				startKey = something(ranges[0].StartKey)
			}

			// Verify that the root hashes match
			require.Equal(t, rootAUpdated, rootB)

			// Verify all keys are now in dbB. Skip over any keys that has been deleted.
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

	// Insert some data.
	_, _, batch := kvForTest(10)
	rootA, err := dbA.Update(batch[:5])
	r.NoError(err)
	_, err = dbB.Update(batch[:5])
	r.NoError(err)

	// Insert more data into dbA.
	rootAUpdated, err := dbA.Update(batch[5:])
	r.NoError(err)

	// Create a change proof and parse it on dbB.
	changeProof, err := dbA.ChangeProof(rootA, rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)
	changeProof = transferChangeProof(t, changeProof, dbB)

	// Marshal before verify — should succeed.
	marshalledBefore, err := changeProof.Marshal()
	r.NoError(err)
	r.NotEmpty(marshalledBefore)

	// Verify the change proof.
	verified, err := changeProof.Verify(rootAUpdated, nothing(), nothing(), changeProofLenUnbounded)
	r.NoError(err)

	// Marshal after verify — should still succeed and produce the same bytes.
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
	proof := newVerifiedRangeProof(t, dbSource, sourceRoot, nothing(), nothing(), rangeProofLenTruncated)

	_, err = dbTarget.VerifyAndCommitRangeProof(proof, nothing(), nothing(), sourceRoot, rangeProofLenTruncated)
	r.NoError(err)

	// Source and target held the same data and the proof covered a prefix of
	// it, so every original key must survive.
	for i, key := range keys {
		got, err := dbTarget.Get(key)
		r.NoError(err, "Get key %d", i)
		r.Equal(vals[i], got, "key %d was deleted or altered by a proof that never covered it", i)
	}
}

// Within the proven prefix, a range proof proves that only the key-values it
// carries exist, so anything else there must be deleted. This is the mirror of
// TestRangeProofTruncatedDoesNotDeleteBeyondProvenEdge, which guards a bound
// looser than the proof justifies; a bound tighter than it justifies stops the
// merge's trie scan short and strands the synthetic stale key below.
//
// Only a stale local key can observe the bound: it gates the trie-side scan
// alone, so against an empty target (as in
// TestRangeProofFindNextKeyDivergentReceiver) every key-value is applied
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

	proof := newVerifiedRangeProof(t, dbSource, sourceRoot, nothing(), nothing(), rangeProofLenTruncated)

	_, err = dbTarget.VerifyAndCommitRangeProof(proof, nothing(), nothing(), sourceRoot, rangeProofLenTruncated)
	r.NoError(err)

	got, err := dbTarget.Get(staleKey)
	r.NoError(err)
	r.Nil(got, "stale key inside the proven range should have been deleted")
}

// TestUnboundChangeProofOutlivesDatabase checks that an unverified change proof
// never holds a lease, whatever was done with it.
func TestUnboundChangeProofOutlivesDatabase(t *testing.T) {
	tests := []struct {
		name      string
		makeProof func(*testing.T, *Database, Hash, Hash) *ChangeProof
	}{
		{
			name: "from_change",
			makeProof: func(t *testing.T, db *Database, startRoot, endRoot Hash) *ChangeProof {
				proof, err := db.ChangeProof(startRoot, endRoot, nothing(), nothing(), changeProofLenUnbounded)
				require.NoError(t, err)
				return proof
			},
		},
		{
			name: "unmarshaled_change",
			makeProof: func(t *testing.T, db *Database, startRoot, endRoot Hash) *ChangeProof {
				data := newSerializedChangeProof(t, db, startRoot, endRoot, nothing(), nothing())
				proof, err := db.UnmarshalChangeProof(data)
				require.NoError(t, err)
				return proof
			},
		},
		{
			name: "verified_change",
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
			name: "committed_change",
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
