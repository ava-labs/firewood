// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
// #cgo noescape fwd_db_range_proof
// #cgo nocallback fwd_db_range_proof
// #cgo noescape fwd_db_range_proof_sized
// #cgo nocallback fwd_db_range_proof_sized
// #cgo noescape fwd_range_proof_verify
// #cgo nocallback fwd_range_proof_verify
// #cgo noescape fwd_db_verify_range_proof
// #cgo nocallback fwd_db_verify_range_proof
// #cgo noescape fwd_db_verify_and_commit_range_proof
// #cgo nocallback fwd_db_verify_and_commit_range_proof
// #cgo noescape fwd_range_proof_find_next_key
// #cgo nocallback fwd_range_proof_find_next_key
// #cgo noescape fwd_range_proof_code_hash_iter
// #cgo nocallback fwd_range_proof_code_hash_iter
// #cgo noescape fwd_code_hash_iter_next
// #cgo nocallback fwd_code_hash_iter_next
// #cgo noescape fwd_code_hash_iter_free
// #cgo nocallback fwd_code_hash_iter_free
// #cgo noescape fwd_range_proof_to_bytes
// #cgo nocallback fwd_range_proof_to_bytes
// #cgo noescape fwd_range_proof_from_bytes
// #cgo nocallback fwd_range_proof_from_bytes
// #cgo noescape fwd_db_change_proof
// #cgo nocallback fwd_db_change_proof
// #cgo noescape fwd_db_change_proof_sized
// #cgo nocallback fwd_db_change_proof_sized
// #cgo noescape fwd_db_verify_change_proof
// #cgo nocallback fwd_db_verify_change_proof
// #cgo noescape fwd_db_verify_and_commit_change_proof
// #cgo nocallback fwd_db_verify_and_commit_change_proof
// #cgo noescape fwd_change_proof_find_next_key
// #cgo nocallback fwd_change_proof_find_next_key
// #cgo noescape fwd_change_proof_code_hash_iter
// #cgo nocallback fwd_change_proof_code_hash_iter
// #cgo noescape fwd_change_proof_to_bytes
// #cgo nocallback fwd_change_proof_to_bytes
// #cgo noescape fwd_change_proof_from_bytes
// #cgo nocallback fwd_change_proof_from_bytes
// #cgo noescape fwd_free_range_proof
// #cgo nocallback fwd_free_range_proof
// #cgo noescape fwd_free_change_proof
// #cgo nocallback fwd_free_change_proof
import "C"

import (
	"errors"
	"fmt"
	"iter"
	"runtime"
	"time"
	"unsafe"
)

var (
	errNotPrepared          = errors.New("proof not prepared into a proposal or committed")
	errDroppedRangeProof    = fmt.Errorf("range proof %w", ErrDropped)
	errDroppedChangeProof   = fmt.Errorf("change proof %w", ErrDropped)
	errBoundToOtherDatabase = errors.New("range proof is bound to a different database")
)

// OverBudgetError is the [ErrProofOverBudget] error, carrying the smallest
// chunk's wire size in bytes.
type OverBudgetError struct {
	Wire uint64
}

func (e *OverBudgetError) Error() string {
	return fmt.Sprintf("%v: %d bytes", ErrProofOverBudget, e.Wire)
}

func (*OverBudgetError) Is(target error) bool {
	return target == ErrProofOverBudget
}

// RangeProof represents a proof that a range of keys and their values are
// included in a trie with a given root hash.
type RangeProof struct {
	// handle owns the Rust RangeProofContext and, once the proof is verified
	// or committed on a database, this proof's lease on that database.
	//
	// Every method that passes the handle to a C call must hold lease.mu for
	// the duration of that call. Drop — including the GC cleanup registered
	// in [newRangeProofFromPtr] — invalidates the handle under
	// lease.mu.Lock, so the lock serializes the call against the free.
	// Omitting it is a use-after-free of the Rust RangeProofContext (see
	// https://github.com/ava-labs/firewood/issues/2137). Methods whose Rust
	// function takes the context as `&mut` (Verify, FindNextKey,
	// Database.VerifyRangeProof, and Database.VerifyAndCommitRangeProof) hold
	// lease.mu.Lock, because Rust requires that reference to be exclusive; the
	// rest hold lease.mu.RLock.
	*handle[*C.RangeProofContext]
}

// ChangeProof represents a proof of changes between two roots for a range of keys.
type ChangeProof struct {
	// handle owns the Rust ChangeProofContext, under the same locking rule as
	// [RangeProof]. A change proof never borrows a database, so it never takes
	// a lease: it never blocks [Database.Close] and is never dropped by
	// [WithForceCloseHandles]. No change-proof FFI function takes the context
	// mutably, so every method holds lease.mu.RLock.
	*handle[*C.ChangeProofContext]
}

// SizingRatioScale is the fixed-point scale of [SizingHint.Ratio]; a ratio
// equal to it is 1:1.
const SizingRatioScale uint32 = C.SIZING_RATIO_SCALE

// SizingHint is what a sized proof measured about its chunk; pass it as the
// next request's hint. The zero value means no hint.
type SizingHint struct {
	// Ratio is the compressed/uncompressed body size ratio in fixed point.
	Ratio uint32
	// Edges is the body bytes the edge proofs took; zero means unknown.
	Edges uint64
}

// SizedProof is a size-targeted proof chunk with its serialized wire bytes.
type SizedProof[P interface{ Drop() error }] struct {
	// Proof is the proof handle, which the caller should Drop when done.
	Proof P
	// Wire is Proof's serialization, ready to send without calling Marshal.
	Wire []byte
	// Complete is true if no further request is needed. It is out-of-band
	// metadata: verify the proof before trusting it.
	Complete bool
	// Hint is the hint for the next request.
	Hint SizingHint
}

// SizedRangeProof is a size-targeted range proof; Complete means it reached
// the end of the keyspace.
type SizedRangeProof = SizedProof[*RangeProof]

// SizedChangeProof is a size-targeted change proof; see [SizedProof].
// Complete means the proof reached the end of the diff, and it is the only
// completion signal: [ChangeProof.FindNextKey] still returns a non-nil range
// after the final chunk, so a non-nil result does not mean more changes
// exist.
type SizedChangeProof = SizedProof[*ChangeProof]

func (h SizingHint) toC() C.SizingHint {
	return C.SizingHint{ratio: C.uint32_t(h.Ratio), edges: C.uint64_t(h.Edges)}
}

func newSizingHint(hint C.SizingHint) SizingHint {
	return SizingHint{Ratio: uint32(hint.ratio), Edges: uint64(hint.edges)}
}

// NextKeyRange is a range of keys still to fetch, `[StartKey, EndKey]`, both
// inclusive. The start key is the smallest key above the last one already
// synchronized, so passing it as the next request's start key resumes without
// fetching that key again. If there is no end key, the range is unbounded
// above.
type NextKeyRange struct {
	startKey *ownedBytes
	endKey   Maybe[*ownedBytes]
}

// codeIterator wraps a Rust CodeIteratorHandle<'p>, a Box<dyn Iterator + 'p>
// over the key-values of the proof it was created from. That proof must stay
// reachable and must not be freed until [codeIterator.free] returns; each
// proof type's CodeHashes method guarantees this for the iterator it creates.
type codeIterator struct {
	handle *C.CodeIteratorHandle
}

// RangeProof returns a proof that the values in the range [startKey, endKey] are
// included in the tree with the current root. The proof may be truncated to at
// most [maxLength] entries, if non-zero. If either [startKey] or [endKey] is
// Nothing, the range is unbounded in that direction. If [rootHash] is Nothing, the
// current root of the database is used.
func (db *Database) RangeProof(
	rootHash Hash,
	startKey, endKey Maybe[[]byte],
	maxLength uint32,
) (*RangeProof, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	args := C.CreateRangeProofArgs{
		root:       newCHashKey(rootHash),
		start_key:  newMaybeBorrowedBytes(startKey, &pinner),
		end_key:    newMaybeBorrowedBytes(endKey, &pinner),
		max_length: C.uint32_t(maxLength),
	}

	return getRangeProofFromRangeProofResult(C.fwd_db_range_proof(db.handle, args))
}

// RangeProofSized returns a range proof of the revision [rootHash] from the
// inclusive [startKey] whose serialized size approaches [budget] bytes and
// never exceeds it. If Complete is false, request the next chunk from the
// start key of the verified proof's [RangeProof.FindNextKey], passing Hint as
// [hint]; the zero SizingHint means no hint. Returns [ErrRevisionNotFound],
// [ErrEmptyTrie], or an [OverBudgetError] if even one entry does not fit.
func (db *Database) RangeProofSized(
	rootHash Hash,
	startKey Maybe[[]byte],
	budget uint64,
	hint SizingHint,
) (*SizedRangeProof, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	start := time.Now()
	defer func() {
		sizedProofDuration.WithLabelValues("range").Observe(time.Since(start).Seconds())
	}()

	var pinner runtime.Pinner
	defer pinner.Unpin()

	args := C.CreateSizedRangeProofArgs{
		root:      newCHashKey(rootHash),
		start_key: newMaybeBorrowedBytes(startKey, &pinner),
		budget:    C.uint64_t(budget),
		hint:      hint.toC(),
	}

	return getSizedRangeProofFromResult(C.fwd_db_range_proof_sized(db.handle, args))
}

// Verify verifies the provided range [proof] proves the values in the range
// [startKey, endKey] are included in the tree with the given [rootHash]. If the
// proof is valid, nil is returned; otherwise an error describing why the proof is
// invalid is returned.
func (p *RangeProof) Verify(
	rootHash Hash,
	startKey, endKey Maybe[[]byte],
	maxLength uint32,
) error {
	// Write lock: fwd_range_proof_verify takes the proof mutably.
	p.lease.mu.Lock()
	defer p.lease.mu.Unlock()
	if p.dropped {
		return errDroppedRangeProof
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	args := C.VerifyRangeProofArgs{
		proof:      p.ptr,
		root:       newCHashKey(rootHash),
		start_key:  newMaybeBorrowedBytes(startKey, &pinner),
		end_key:    newMaybeBorrowedBytes(endKey, &pinner),
		max_length: C.uint32_t(maxLength),
	}

	return getErrorFromVoidResult(C.fwd_range_proof_verify(args))
}

// VerifyRangeProof verifies the provided range [proof] proves the changes
// between [startRoot] and [endRoot] for keys in the range [startKey, endKey]. If
// the proof is valid, a proposal containing the changes is prepared. The
// call to [*Database.VerifyAndCommitRangeProof] will skip verification and commit the
// prepared proposal.
//
// The proposal replaces state across the range the proof proves: any existing
// key in that range that the proof does not carry is deleted. The range runs
// from [startKey] to the proof's right edge, which is [endKey] when the
// responder covered the whole request and a smaller key when it truncated.
// Keys past that edge are left as they are; [*RangeProof.FindNextKey] reports
// where to resume.
//
// The prepared proposal borrows the database, so a successful call binds the
// proof to db: db cannot be closed gracefully until the proof is dropped, and
// other databases reject the proof with an error. A successful
// [*Database.VerifyAndCommitRangeProof] binds the proof the same way.
func (db *Database) VerifyRangeProof(
	proof *RangeProof,
	startKey, endKey Maybe[[]byte],
	rootHash Hash,
	maxLength uint32,
) error {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return errDBClosed
	}

	// Write lock: fwd_db_verify_range_proof takes the proof mutably.
	proof.lease.mu.Lock()
	defer proof.lease.mu.Unlock()
	if proof.dropped {
		return errDroppedRangeProof
	}
	if proof.lease.registry != nil && proof.lease.registry != db.keepAlives {
		return errBoundToOtherDatabase
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	args := C.VerifyRangeProofArgs{
		proof:      proof.ptr,
		root:       newCHashKey(rootHash),
		start_key:  newMaybeBorrowedBytes(startKey, &pinner),
		end_key:    newMaybeBorrowedBytes(endKey, &pinner),
		max_length: C.uint32_t(maxLength),
	}

	if err := getErrorFromVoidResult(C.fwd_db_verify_range_proof(db.handle, args)); err != nil {
		return err
	}
	return proof.lease.ensureAttached(db.keepAlives, proof.Drop)
}

// VerifyAndCommitRangeProof verifies the provided range [proof] proves the values
// in the range [startKey, endKey] are included in the tree with the given
// [rootHash]. If the proof is valid, it is committed to the database and the
// new root hash is returned. The resulting root hash may not equal the
// provided root hash if the proof was truncated due to [maxLength].
//
// The commit replaces state across the proven range on the same terms as
// [*Database.VerifyRangeProof].
func (db *Database) VerifyAndCommitRangeProof(
	proof *RangeProof,
	startKey, endKey Maybe[[]byte],
	rootHash Hash,
	maxLength uint32,
) (Hash, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return EmptyRoot, errDBClosed
	}

	// Write lock: fwd_db_verify_and_commit_range_proof takes the proof mutably.
	proof.lease.mu.Lock()
	defer proof.lease.mu.Unlock()
	if proof.dropped {
		return EmptyRoot, errDroppedRangeProof
	}
	if proof.lease.registry != nil && proof.lease.registry != db.keepAlives {
		return EmptyRoot, errBoundToOtherDatabase
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	args := C.VerifyRangeProofArgs{
		proof:      proof.ptr,
		root:       newCHashKey(rootHash),
		start_key:  newMaybeBorrowedBytes(startKey, &pinner),
		end_key:    newMaybeBorrowedBytes(endKey, &pinner),
		max_length: C.uint32_t(maxLength),
	}

	db.commitLock.Lock()
	defer db.commitLock.Unlock()
	hash, err := getHashKeyFromHashResult(C.fwd_db_verify_and_commit_range_proof(db.handle, args))
	if err != nil {
		return EmptyRoot, err
	}
	if err := proof.lease.ensureAttached(db.keepAlives, proof.Drop); err != nil {
		return EmptyRoot, err
	}
	return hash, nil
}

// FindNextKey returns the next key range to fetch for this proof, if any. If the
// proof has been fully processed, nil is returned. If an error occurs while
// determining the next key range, that error is returned.
//
// FindNextKey can only be called after a successful call to [*Database.VerifyRangeProof] or
// [*Database.VerifyAndCommitRangeProof].
//
// The next key range indicates the next `[StartKey, EndKey]` range of keys
// that should be synchronized to complete the requested range; see
// [NextKeyRange]. ava-labs/firewood#352 tracks tightening the start key beyond
// the last proven key.
func (p *RangeProof) FindNextKey() (*NextKeyRange, error) {
	// Write lock: fwd_range_proof_find_next_key takes the proof mutably.
	p.lease.mu.Lock()
	defer p.lease.mu.Unlock()
	if p.dropped {
		return nil, errDroppedRangeProof
	}
	return getNextKeyRangeFromNextKeyRangeResult(C.fwd_range_proof_find_next_key(p.ptr))
}

// CodeHashes returns an iterator for the code hashes contained in the account nodes
// of this proof. This list may contain duplicates and is not guaranteed to be in any particular order.
//
// Note: this method is only relevant for Ethereum tries.
// This method can be called anytime after the proof is created.
//
// The iteration holds the proof's read lock until the loop ends, so any other
// method on the range proof MUST NOT be called during iteration.
func (p *RangeProof) CodeHashes() iter.Seq2[Hash, error] {
	return func(yield func(Hash, error) bool) {
		// The proof handle MUST be held for the lifetime of the iterator.
		p.lease.mu.RLock()
		defer p.lease.mu.RUnlock()
		if p.dropped {
			yield(EmptyRoot, errDroppedRangeProof)
			return
		}

		codeHashIter(C.fwd_range_proof_code_hash_iter(p.ptr), yield)
	}
}

func codeHashIter(ptr C.CodeIteratorResult, yield func(Hash, error) bool) {
	it, err := newCodeIterator(ptr)
	if err != nil {
		yield(EmptyRoot, err)
		return
	}
	defer func() {
		if err := it.free(); err != nil {
			panic(err)
		}
	}()
	for hash, err := it.next(); ; hash, err = it.next() {
		if err != nil {
			yield(EmptyRoot, err)
			return
		}
		if hash == EmptyRoot {
			return
		}
		if !yield(hash, err) {
			return
		}
	}
}

func (it *codeIterator) next() (Hash, error) {
	return getHashKeyFromHashResult(C.fwd_code_hash_iter_next(it.handle))
}

// free releases the Rust iterator, ending its borrow of the proof.
func (it *codeIterator) free() error {
	return getErrorFromVoidResult(C.fwd_code_hash_iter_free(it.handle))
}

// Marshal returns a serialized representation of this RangeProof.
//
// The format is unspecified and opaque to firewood.
func (p *RangeProof) Marshal() ([]byte, error) {
	p.lease.mu.RLock()
	defer p.lease.mu.RUnlock()
	if p.dropped {
		return nil, errDroppedRangeProof
	}

	start := time.Now()
	defer func() {
		proofMarshalDuration.WithLabelValues("range").Observe(time.Since(start).Seconds())
	}()

	return getValueFromValueResult(C.fwd_range_proof_to_bytes(p.ptr))
}

// UnmarshalRangeProof deserializes a RangeProof from [data], which must have
// been produced by [*RangeProof.Marshal]. The returned proof is bound to no
// database until it is verified.
func UnmarshalRangeProof(data []byte) (*RangeProof, error) {
	start := time.Now()
	defer func() {
		proofUnmarshalDuration.WithLabelValues("range").Observe(time.Since(start).Seconds())
	}()

	var pinner runtime.Pinner
	defer pinner.Unpin()
	return getRangeProofFromRangeProofResult(C.fwd_range_proof_from_bytes(newBorrowedBytes(data, &pinner)))
}

// ChangeProof returns a proof that the changes between [startRoot] and
// [endRoot] for keys in the range [startKey, endKey]. The proof may be
// truncated to at most [maxLength] entries, if non-zero. If either [startKey] or
// [endKey] is Nothing, the range is unbounded in that direction.
func (db *Database) ChangeProof(
	startRoot, endRoot Hash,
	startKey, endKey Maybe[[]byte],
	maxLength uint32,
) (*ChangeProof, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	args := C.CreateChangeProofArgs{
		start_root: newCHashKey(startRoot),
		end_root:   newCHashKey(endRoot),
		start_key:  newMaybeBorrowedBytes(startKey, &pinner),
		end_key:    newMaybeBorrowedBytes(endKey, &pinner),
		max_length: C.uint32_t(maxLength),
	}

	return getChangeProofFromChangeProofResult(C.fwd_db_change_proof(db.handle, args))
}

// ChangeProofSized returns a change proof between [startRoot] and [endRoot]
// from the inclusive [startKey] whose serialized size approaches [budget]
// bytes and never exceeds it. Both roots must be exact roots of retained
// revisions.
//
// Send the result's Wire. Complete is the only completion signal: if it is
// false, request the next chunk starting strictly after the last operation
// in the proof, passing the result's Hint as [hint]; a hint-less request (the
// zero SizingHint) needs extra sizing probes, so thread Hint through even
// across retries. [ChangeProof.FindNextKey] yields that next start key
// directly, but it also returns a non-nil range after the final chunk, so it
// must not be used to detect completion.
//
// Errors: [ErrStartRevisionNotFound] if [startRoot] is not found;
// [ErrEndRevisionNotFound] if [endRoot] is not found, which takes precedence
// when both are missing; [ErrProofOverBudget], as an [OverBudgetError], if
// one operation, or the edge proofs alone, do not fit in [budget]. Anything
// else is a generic error, including an end revision whose trie is empty and
// a single operation whose proof body exceeds the decoder's limit.
func (db *Database) ChangeProofSized(
	startRoot, endRoot Hash,
	startKey Maybe[[]byte],
	budget uint64,
	hint SizingHint,
) (*SizedChangeProof, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	start := time.Now()
	defer func() {
		sizedProofDuration.WithLabelValues("change").Observe(time.Since(start).Seconds())
	}()

	var pinner runtime.Pinner
	defer pinner.Unpin()

	args := C.CreateSizedChangeProofArgs{
		start_root: newCHashKey(startRoot),
		end_root:   newCHashKey(endRoot),
		start_key:  newMaybeBorrowedBytes(startKey, &pinner),
		budget:     C.uint64_t(budget),
		hint:       hint.toC(),
	}

	return getSizedChangeProofFromResult(C.fwd_db_change_proof_sized(db.handle, args))
}

// VerifyChangeProof verifies the change proof and creates a standard Proposal.
// The proof is not consumed — it can still be used for [ChangeProof.FindNextKey] or serialization.
func (db *Database) VerifyChangeProof(
	proof *ChangeProof,
	endRoot Hash,
	startKey, endKey Maybe[[]byte],
	maxLength uint32,
) (*Proposal, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	proof.lease.mu.RLock()
	defer proof.lease.mu.RUnlock()
	if proof.dropped {
		return nil, errDroppedChangeProof
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	args := C.CreateChangeProofArgs{
		end_root:   newCHashKey(endRoot),
		start_key:  newMaybeBorrowedBytes(startKey, &pinner),
		end_key:    newMaybeBorrowedBytes(endKey, &pinner),
		max_length: C.uint32_t(maxLength),
	}

	return getProposalFromProposalResult(
		C.fwd_db_verify_change_proof(db.handle, proof.ptr, args),
		db.keepAlives,
		&db.commitLock,
	)
}

// VerifyAndCommitChangeProof verifies the change proof and commits it in a
// single call. The proof is not consumed — it remains available for
// [ChangeProof.FindNextKey] or serialization afterward.
func (db *Database) VerifyAndCommitChangeProof(
	proof *ChangeProof,
	endRoot Hash,
	startKey, endKey Maybe[[]byte],
	maxLength uint32,
) (Hash, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return EmptyRoot, errDBClosed
	}

	proof.lease.mu.RLock()
	defer proof.lease.mu.RUnlock()
	if proof.dropped {
		return EmptyRoot, errDroppedChangeProof
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	args := C.CreateChangeProofArgs{
		end_root:   newCHashKey(endRoot),
		start_key:  newMaybeBorrowedBytes(startKey, &pinner),
		end_key:    newMaybeBorrowedBytes(endKey, &pinner),
		max_length: C.uint32_t(maxLength),
	}

	db.commitLock.Lock()
	defer db.commitLock.Unlock()
	return getHashKeyFromHashResult(C.fwd_db_verify_and_commit_change_proof(db.handle, proof.ptr, args))
}

// FindNextKey returns the next key range to fetch for a change proof,
// or nil if there are no more keys to fetch. The proof is not consumed.
func (proof *ChangeProof) FindNextKey(endKey Maybe[[]byte]) (*NextKeyRange, error) {
	proof.lease.mu.RLock()
	defer proof.lease.mu.RUnlock()
	if proof.dropped {
		return nil, errDroppedChangeProof
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	return getNextKeyRangeFromNextKeyRangeResult(
		C.fwd_change_proof_find_next_key(proof.ptr, newMaybeBorrowedBytes(endKey, &pinner)),
	)
}

// CodeHashes returns an iterator for the code hashes contained in the account nodes
// of this proof. This list may contain duplicates and is not guaranteed to be in any particular order.
//
// Note: this method is only relevant for Ethereum tries.
// This method can be called any time after the proof is created — verification
// is not required, since extraction is purely RLP parsing of the values
// already present in the proof. Only code hashes referenced by Put entries
// (the post-state of accounts touched by the proof) are yielded; Delete and
// DeleteRange entries are skipped.
//
// The iteration holds the proof's read lock until the loop ends, so any other
// method on the change proof MUST NOT be called during iteration.
func (p *ChangeProof) CodeHashes() iter.Seq2[Hash, error] {
	return func(yield func(Hash, error) bool) {
		// See [RangeProof.CodeHashes] for why the read lock spans the loop.
		p.lease.mu.RLock()
		defer p.lease.mu.RUnlock()
		if p.dropped {
			yield(EmptyRoot, errDroppedChangeProof)
			return
		}
		codeHashIter(C.fwd_change_proof_code_hash_iter(p.ptr), yield)
	}
}

// Marshal returns a serialized representation of this ChangeProof.
//
// The format is unspecified and opaque to firewood.
func (p *ChangeProof) Marshal() ([]byte, error) {
	p.lease.mu.RLock()
	defer p.lease.mu.RUnlock()
	if p.dropped {
		return nil, errDroppedChangeProof
	}

	start := time.Now()
	defer func() {
		proofMarshalDuration.WithLabelValues("change").Observe(time.Since(start).Seconds())
	}()

	return getValueFromValueResult(C.fwd_change_proof_to_bytes(p.ptr))
}

// UnmarshalChangeProof deserializes a ChangeProof from [data], which must have
// been produced by [*ChangeProof.Marshal].
func UnmarshalChangeProof(data []byte) (*ChangeProof, error) {
	start := time.Now()
	defer func() {
		proofUnmarshalDuration.WithLabelValues("change").Observe(time.Since(start).Seconds())
	}()

	var pinner runtime.Pinner
	defer pinner.Unpin()
	return getChangeProofFromChangeProofResult(C.fwd_change_proof_from_bytes(newBorrowedBytes(data, &pinner)))
}

// StartKey returns the inclusive start key of this key range: the smallest key
// above the last one already synchronized.
func (r *NextKeyRange) StartKey() []byte {
	return r.startKey.CopiedBytes()
}

// HasEndKey returns true if this key range has an inclusive end key.
func (r *NextKeyRange) HasEndKey() bool {
	return r.endKey != nil && r.endKey.HasValue()
}

// EndKey returns the inclusive end key of this key range if it exists or nil if
// it does not.
func (r *NextKeyRange) EndKey() []byte {
	if r.HasEndKey() {
		return r.endKey.Value().CopiedBytes()
	}
	return nil
}

// Free releases the resources associated with this NextKeyRange.
//
// It is safe to call Free more than once; subsequent calls after the first
// will be no-ops.
func (r *NextKeyRange) Free() error {
	var err1, err2 error

	err1 = r.startKey.Free()
	if r.HasEndKey() {
		err2 = r.endKey.Value().Free()
	}

	return errors.Join(err1, err2)
}

func newNextKeyRange(cRange C.NextKeyRange) *NextKeyRange {
	var nextKeyRange NextKeyRange

	nextKeyRange.startKey = newOwnedBytes(cRange.start_key)

	if cRange.end_key.tag == C.Maybe_OwnedBytes_Some_OwnedBytes {
		nextKeyRange.endKey = newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&cRange.end_key.anon0)))
	}

	return &nextKeyRange
}

func getNextKeyRangeFromNextKeyRangeResult(result C.NextKeyRangeResult) (*NextKeyRange, error) {
	switch result.tag {
	case C.NextKeyRangeResult_NullHandlePointer:
		return nil, errDBClosed
	case C.NextKeyRangeResult_NotPrepared:
		return nil, errNotPrepared
	case C.NextKeyRangeResult_None:
		return nil, nil
	case C.NextKeyRangeResult_Some:
		nkr := newNextKeyRange(*(*C.NextKeyRange)(unsafe.Pointer(&result.anon0)))
		runtime.SetFinalizer(nkr, (*NextKeyRange).Free)
		return nkr, nil
	case C.NextKeyRangeResult_Err:
		return nil, newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
	default:
		return nil, fmt.Errorf("unknown C.NextKeyRangeResult tag: %d", result.tag)
	}
}

func newCodeIterator(result C.CodeIteratorResult) (*codeIterator, error) {
	switch result.tag {
	case C.CodeIteratorResult_NullHandlePointer:
		return nil, errDBClosed
	case C.CodeIteratorResult_Ok:
		ptr := *(**C.CodeIteratorHandle)(unsafe.Pointer(&result.anon0))
		return &codeIterator{handle: ptr}, nil
	case C.CodeIteratorResult_Err:
		err := newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
		return nil, err
	default:
		return nil, fmt.Errorf("unknown C.CodeIteratorResult tag: %d", result.tag)
	}
}

func getRangeProofFromRangeProofResult(result C.RangeProofResult) (*RangeProof, error) {
	switch result.tag {
	case C.RangeProofResult_NullHandlePointer:
		return nil, errDBClosed
	case C.RangeProofResult_RevisionNotFound:
		// NOTE: the result value contains the provided root hash, we could use
		// it in the error message if needed.
		return nil, ErrRevisionNotFound
	case C.RangeProofResult_EmptyTrie:
		return nil, ErrEmptyTrie
	case C.RangeProofResult_Ok:
		ptr := *(**C.RangeProofContext)(unsafe.Pointer(&result.anon0))
		return newRangeProofFromPtr(ptr), nil
	case C.RangeProofResult_Err:
		err := newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
		return nil, err
	default:
		return nil, fmt.Errorf("unknown C.RangeProofResult tag: %d", result.tag)
	}
}

// newRangeProofFromPtr takes ownership of a Rust RangeProofContext, freeing it
// on Drop or, failing that, when the proof is garbage collected. The caller
// must not free or otherwise use ptr afterwards.
func newRangeProofFromPtr(ptr *C.RangeProofContext) *RangeProof {
	proof := &RangeProof{
		handle: newHandle(ptr, func(p *C.RangeProofContext) C.VoidResult { return C.fwd_free_range_proof(p) }),
	}
	runtime.AddCleanup(proof, drop[*C.RangeProofContext], proof.handle)
	return proof
}

// newChangeProofFromPtr takes ownership of a Rust ChangeProofContext under
// the same rules as [newRangeProofFromPtr].
func newChangeProofFromPtr(ptr *C.ChangeProofContext) *ChangeProof {
	proof := &ChangeProof{
		handle: newHandle(ptr, func(p *C.ChangeProofContext) C.VoidResult { return C.fwd_free_change_proof(p) }),
	}
	runtime.AddCleanup(proof, drop[*C.ChangeProofContext], proof.handle)
	return proof
}

// newSizedProof assembles a sized result from its C parts. It takes
// ownership of wire, copying it into Go memory and freeing the Rust
// allocation; if that free fails, proof is dropped so the Rust proof does not
// leak behind the error.
func newSizedProof[P interface{ Drop() error }](
	proof P,
	wire C.OwnedBytes,
	naturalEnd C.bool,
	hint C.SizingHint,
) (*SizedProof[P], error) {
	wireOwned := newOwnedBytes(wire)
	copied := wireOwned.CopiedBytes()
	if err := wireOwned.Free(); err != nil {
		return nil, errors.Join(err, proof.Drop())
	}
	return &SizedProof[P]{
		Proof:    proof,
		Wire:     copied,
		Complete: bool(naturalEnd),
		Hint:     newSizingHint(hint),
	}, nil
}

func getSizedRangeProofFromResult(result C.SizedRangeProofResult) (*SizedRangeProof, error) {
	switch result.tag {
	case C.SizedRangeProofResult_NullHandlePointer:
		return nil, errDBClosed
	case C.SizedRangeProofResult_RevisionNotFound:
		return nil, ErrRevisionNotFound
	case C.SizedRangeProofResult_EmptyTrie:
		return nil, ErrEmptyTrie
	case C.SizedRangeProofResult_OverBudget:
		// SAFETY: the tag says the union holds the OverBudget payload, a
		// uint64_t laid out at the start of the union by cbindgen.
		return nil, &OverBudgetError{Wire: uint64(*(*C.uint64_t)(unsafe.Pointer(&result.anon0)))}
	case C.SizedRangeProofResult_Ok:
		// SAFETY: the tag says the union holds a C.SizedRangeProof, which
		// cbindgen lays out at the start of the union. Reading it by value
		// moves ownership of its proof and wire allocations to Go: the proof
		// to the handle created first, the wire to newSizedProof.
		ok := *(*C.SizedRangeProof)(unsafe.Pointer(&result.anon0))
		return newSizedProof(newRangeProofFromPtr(ok.proof), ok.wire, ok.natural_end, ok.hint)
	case C.SizedRangeProofResult_Err:
		return nil, newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
	default:
		return nil, fmt.Errorf("unknown C.SizedRangeProofResult tag: %d", result.tag)
	}
}

func getSizedChangeProofFromResult(result C.SizedChangeProofResult) (*SizedChangeProof, error) {
	switch result.tag {
	case C.SizedChangeProofResult_NullHandlePointer:
		return nil, errDBClosed
	case C.SizedChangeProofResult_StartRevisionNotFound:
		return nil, ErrStartRevisionNotFound
	case C.SizedChangeProofResult_EndRevisionNotFound:
		return nil, ErrEndRevisionNotFound
	case C.SizedChangeProofResult_OverBudget:
		// SAFETY: as in getSizedRangeProofFromResult, the tag selects the
		// uint64_t payload at the start of the union.
		return nil, &OverBudgetError{Wire: uint64(*(*C.uint64_t)(unsafe.Pointer(&result.anon0)))}
	case C.SizedChangeProofResult_Ok:
		// SAFETY: as in getSizedRangeProofFromResult, with a
		// C.SizedChangeProof payload.
		ok := *(*C.SizedChangeProof)(unsafe.Pointer(&result.anon0))
		return newSizedProof(newChangeProofFromPtr(ok.proof), ok.wire, ok.natural_end, ok.hint)
	case C.SizedChangeProofResult_Err:
		return nil, newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
	default:
		return nil, fmt.Errorf("unknown C.SizedChangeProofResult tag: %d", result.tag)
	}
}

func getChangeProofFromChangeProofResult(result C.ChangeProofResult) (*ChangeProof, error) {
	switch result.tag {
	case C.ChangeProofResult_NullHandlePointer:
		return nil, errDBClosed
	case C.ChangeProofResult_StartRevisionNotFound:
		return nil, ErrStartRevisionNotFound
	case C.ChangeProofResult_EndRevisionNotFound:
		return nil, ErrEndRevisionNotFound
	case C.ChangeProofResult_Ok:
		ptr := *(**C.ChangeProofContext)(unsafe.Pointer(&result.anon0))
		return newChangeProofFromPtr(ptr), nil
	case C.ChangeProofResult_Err:
		err := newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
		return nil, err
	default:
		return nil, fmt.Errorf("unknown C.ChangeProofResult tag: %d", result.tag)
	}
}
