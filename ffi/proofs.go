// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
// #cgo noescape fwd_sizing_hint_new
// #cgo nocallback fwd_sizing_hint_new
// #cgo noescape fwd_free_sizing_hint
// #cgo nocallback fwd_free_sizing_hint
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
// #cgo noescape fwd_db_range_proof_from_bytes
// #cgo nocallback fwd_db_range_proof_from_bytes
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
// #cgo noescape fwd_db_change_proof_from_bytes
// #cgo nocallback fwd_db_change_proof_from_bytes
// #cgo noescape fwd_free_range_proof
// #cgo nocallback fwd_free_range_proof
// #cgo noescape fwd_free_change_proof
// #cgo nocallback fwd_free_change_proof
import "C"

import (
	"errors"
	"fmt"
	"runtime"
	"time"
	"unsafe"
)

var (
	errNotPrepared          = errors.New("proof not prepared into a proposal or committed")
	errDroppedRangeProof    = fmt.Errorf("range proof %w", ErrDropped)
	errDroppedChangeProof   = fmt.Errorf("change proof %w", ErrDropped)
	errDroppedSizingHint    = fmt.Errorf("sizing hint %w", ErrDropped)
	errBoundToOtherDatabase = errors.New("range proof is bound to a different database")
)

var (
	// ErrRangeProofOnEmptyTrie is returned for a range proof of an empty trie
	// with no start key.
	ErrRangeProofOnEmptyTrie = errors.New("cannot create a range proof for an empty trie")
	// ErrProofOverBudget is wrapped by [OverBudgetError] when even the smallest
	// sized proof chunk exceeds the budget.
	ErrProofOverBudget = errors.New("the smallest proof chunk exceeds the byte budget")
)

// OverBudgetError wraps [ErrProofOverBudget] with the smallest chunk's wire
// size in bytes; a budget of at least WireSize admits that chunk.
type OverBudgetError struct {
	WireSize uint64
}

// Error implements error.
func (e *OverBudgetError) Error() string {
	return fmt.Sprintf("%v: smallest chunk is %d bytes", ErrProofOverBudget, e.WireSize)
}

// Unwrap returns [ErrProofOverBudget] so [errors.Is] matches it.
func (*OverBudgetError) Unwrap() error {
	return ErrProofOverBudget
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

// SizingHint is a sized request's budget and, after a chunk, its calibration
// for the next one. Create one with [NewSizingHint]; a sized request consumes
// the hint it is given, even on error, and returns a new one in its result.
// Drop a hint that will not be passed on.
type SizingHint struct {
	*handle[*C.SizingHint]
}

// NewSizingHint returns a hint for chunks whose serialized size never
// exceeds [budget] bytes.
func NewSizingHint(budget uint64) *SizingHint {
	return newSizingHintFromPtr(C.fwd_sizing_hint_new(C.size_t(budget)))
}

// newSizingHintFromPtr takes ownership of a Rust SizingHint, freeing it on
// Drop or, failing that, when the hint is garbage collected.
func newSizingHintFromPtr(ptr *C.SizingHint) *SizingHint {
	hint := &SizingHint{
		handle: newHandle(ptr, func(p *C.SizingHint) C.VoidResult { return C.fwd_free_sizing_hint(p) }),
	}
	runtime.AddCleanup(hint, drop[*C.SizingHint], hint.handle)
	return hint
}

// take hands the Rust hint to a request that consumes it. A nil hint yields
// a nil pointer, which Rust reads as a default-budget hint.
func (h *SizingHint) take() (*C.SizingHint, error) {
	if h == nil {
		return nil, nil
	}
	var ptr *C.SizingHint
	err := h.lease.release(func() error {
		if h.dropped {
			return errDroppedSizingHint
		}
		ptr = h.ptr
		h.ptr = nil
		h.dropped = true
		return nil
	})
	return ptr, err
}

// SizedProof is a size-targeted proof chunk with its serialized wire bytes.
type SizedProof[P any] struct {
	// Proof is the proof handle, which the caller should Drop when done.
	Proof P
	// Wire is Proof's serialization, ready to send without calling Marshal.
	// It is a Go copy that outlives Proof.
	Wire []byte
	// Complete is true if no further request is needed. It is out-of-band
	// metadata: verify the proof before trusting it.
	Complete bool
	// Hint is the budget and calibration for the next request; pass it on
	// or Drop it.
	Hint *SizingHint
}

// SizedRangeProof is a size-targeted range proof; Complete means it reached
// the end of the keyspace.
type SizedRangeProof = SizedProof[*RangeProof]

// SizedChangeProof is a size-targeted change proof; Complete means it reached
// the end of the diff and is the only completion signal, since
// [ChangeProof.FindNextKey] may still return a range after the final chunk.
type SizedChangeProof = SizedProof[*ChangeProof]

// NextKeyRange is a range of keys still to fetch, `[StartKey, EndKey]`, both
// inclusive. StartKey is the smallest key above the last one already
// synchronized, so passing it as the next request's start key resumes without
// fetching that key again. If EndKey has no value, the range is unbounded
// above.
//
// The keys are Go-owned copies; a NextKeyRange needs no release.
type NextKeyRange struct {
	StartKey []byte
	EndKey   Maybe[[]byte]
}

// codeIterator wraps a Rust CodeIteratorHandle<'p>, a Box<dyn Iterator + 'p>
// over the entries of the proof it was created from. The proof's read lock is
// held for the iterator's whole life by the CodeHashes method that drains it.
type codeIterator struct {
	handle *C.CodeIteratorHandle
}

// RangeProof returns a proof that the values in the range [startKey, endKey] are
// included in the tree with the current root. The proof may be truncated to at
// most [maxLength] entries, if non-zero. If either [startKey] or [endKey] is
// Nothing, the range is unbounded in that direction. If [rootHash] is Nothing, the
// current root of the database is used. Returns [ErrRangeProofOnEmptyTrie] if the
// trie is empty and [startKey] is Nothing.
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

// RangeProofSized returns a range proof of the revision [rootHash] over the
// inclusive range [startKey, endKey] (Nothing means unbounded on that side)
// whose serialized size approaches the budget of [hint] and never exceeds it.
// The call consumes [hint]; a nil [hint] means a default budget. If the
// result's Complete is false, request the next chunk from the start key of
// [RangeProof.FindNextKey] after [Database.VerifyRangeProof] or
// [Database.VerifyAndCommitRangeProof], passing the result's Hint as [hint].
// Returns [ErrRevisionNotFound], [ErrRangeProofOnEmptyTrie] if the trie is
// empty and both keys are Nothing, or an [OverBudgetError] if even one entry,
// or the edge proofs alone, do not fit.
func (db *Database) RangeProofSized(
	rootHash Hash,
	startKey, endKey Maybe[[]byte],
	hint *SizingHint,
) (sized *SizedRangeProof, err error) {
	start := time.Now()
	defer func() {
		if err == nil {
			sizedProofDuration.WithLabelValues("range", db.metricsTag).Observe(time.Since(start).Seconds())
		}
	}()

	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	hintPtr, err := hint.take()
	if err != nil {
		return nil, err
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	args := C.CreateSizedRangeProofArgs{
		root:      newCHashKey(rootHash),
		start_key: newMaybeBorrowedBytes(startKey, &pinner),
		end_key:   newMaybeBorrowedBytes(endKey, &pinner),
		hint:      hintPtr,
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

// CodeHashes returns the contract code hashes referenced by the account values
// in this proof, in no particular order and possibly with duplicates.
//
// Only Ethereum-mode proofs have account values; for a merkledb-mode proof
// this returns an error. This method can be called any time after the proof
// is created.
func (p *RangeProof) CodeHashes() ([]Hash, error) {
	p.lease.mu.RLock()
	defer p.lease.mu.RUnlock()
	if p.dropped {
		return nil, errDroppedRangeProof
	}
	return collectCodeHashes(C.fwd_range_proof_code_hash_iter(p.ptr))
}

// collectCodeHashes drains and frees a Rust code-hash iterator. The caller
// holds the owning proof's read lock for the duration.
func collectCodeHashes(result C.CodeIteratorResult) (_ []Hash, retErr error) {
	it, err := newCodeIterator(result)
	if err != nil {
		return nil, err
	}

	defer func() {
		retErr = errors.Join(retErr, it.free())
	}()

	var hashes []Hash
	for {
		hash, ok, err := it.next()
		if err != nil {
			return nil, err
		}
		if !ok {
			break
		}
		hashes = append(hashes, hash)
	}

	return hashes, nil
}

// next returns the next code hash, or ok == false once the iterator is
// exhausted.
func (it *codeIterator) next() (hash Hash, ok bool, err error) {
	result := C.fwd_code_hash_iter_next(it.handle)
	if result.tag == C.HashResult_None {
		return EmptyRoot, false, nil
	}
	hash, err = getHashKeyFromHashResult(result)
	if err != nil {
		return EmptyRoot, false, err
	}
	return hash, true, nil
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

// UnmarshalRangeProof parses a proof produced by [RangeProof.Marshal] for use
// with db. A proof encoded for the other hash mode is rejected here, before
// any verification. The returned proof holds no lease on db until it is
// verified.
func (db *Database) UnmarshalRangeProof(data []byte) (*RangeProof, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	start := time.Now()
	defer func() {
		proofUnmarshalDuration.WithLabelValues("range", db.metricsTag).Observe(time.Since(start).Seconds())
	}()

	var pinner runtime.Pinner
	defer pinner.Unpin()
	return getRangeProofFromRangeProofResult(C.fwd_db_range_proof_from_bytes(db.handle, newBorrowedBytes(data, &pinner)))
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

// ChangeProofSized returns a proof of the changes that turn the committed
// revision [startRoot] into [endRoot] over the inclusive range
// [startKey, endKey] (Nothing means unbounded on that side), whose serialized
// size approaches the budget of [hint] and never exceeds it. The call
// consumes [hint]; a nil [hint] means a default budget. If the result's
// Complete is false, request the next chunk from the start key of
// [ChangeProof.FindNextKey] called with [endKey], passing the result's Hint
// as [hint]. Returns [ErrEndRevisionNotFound] (also when both roots are
// missing), [ErrStartRevisionNotFound], or an [OverBudgetError] if even one
// operation, or the edge proofs alone, do not fit.
func (db *Database) ChangeProofSized(
	startRoot, endRoot Hash,
	startKey, endKey Maybe[[]byte],
	hint *SizingHint,
) (sized *SizedChangeProof, err error) {
	start := time.Now()
	defer func() {
		if err == nil {
			sizedProofDuration.WithLabelValues("change").Observe(time.Since(start).Seconds())
		}
	}()

	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	hintPtr, err := hint.take()
	if err != nil {
		return nil, err
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	args := C.CreateSizedChangeProofArgs{
		start_root: newCHashKey(startRoot),
		end_root:   newCHashKey(endRoot),
		start_key:  newMaybeBorrowedBytes(startKey, &pinner),
		end_key:    newMaybeBorrowedBytes(endKey, &pinner),
		hint:       hintPtr,
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

// CodeHashes returns the contract code hashes referenced by the account values
// this proof puts, on the same terms as [RangeProof.CodeHashes]. Delete and
// DeleteRange entries are skipped.
//
// Only Ethereum-mode proofs have account values; for a merkledb-mode proof
// this returns an error.
func (p *ChangeProof) CodeHashes() ([]Hash, error) {
	p.lease.mu.RLock()
	defer p.lease.mu.RUnlock()
	if p.dropped {
		return nil, errDroppedChangeProof
	}
	return collectCodeHashes(C.fwd_change_proof_code_hash_iter(p.ptr))
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

// UnmarshalChangeProof parses a proof produced by [ChangeProof.Marshal] for use
// with db. A proof encoded for the other hash mode is rejected here, before
// any verification.
func (db *Database) UnmarshalChangeProof(data []byte) (*ChangeProof, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	start := time.Now()
	defer func() {
		proofUnmarshalDuration.WithLabelValues("change", db.metricsTag).Observe(time.Since(start).Seconds())
	}()

	var pinner runtime.Pinner
	defer pinner.Unpin()
	return getChangeProofFromChangeProofResult(C.fwd_db_change_proof_from_bytes(db.handle, newBorrowedBytes(data, &pinner)))
}

// getNextKeyRangeFromNextKeyRangeResult copies the range into Go memory and
// frees the Rust keys, so the returned value outlives every C allocation.
func getNextKeyRangeFromNextKeyRangeResult(result C.NextKeyRangeResult) (*NextKeyRange, error) {
	switch result.tag {
	case C.NextKeyRangeResult_NullHandlePointer:
		return nil, errDBClosed
	case C.NextKeyRangeResult_NotPrepared:
		return nil, errNotPrepared
	case C.NextKeyRangeResult_None:
		return nil, nil
	case C.NextKeyRangeResult_Some:
		c := (*C.NextKeyRange)(unsafe.Pointer(&result.anon0))
		startKey := newOwnedBytes(c.start_key)
		nkr := &NextKeyRange{StartKey: startKey.CopiedBytes(), EndKey: Nothing[[]byte]()}
		var freeErr error
		if err := startKey.Free(); err != nil {
			freeErr = fmt.Errorf("startKey: %w", err)
		}
		if c.end_key.tag == C.Maybe_OwnedBytes_Some_OwnedBytes {
			endKey := newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&c.end_key.anon0)))
			nkr.EndKey = Some(endKey.CopiedBytes())
			if err := endKey.Free(); err != nil {
				freeErr = errors.Join(freeErr, fmt.Errorf("endKey: %w", err))
			}
		}
		if freeErr != nil {
			return nil, fmt.Errorf("%w: %w", errFreeingValue, freeErr)
		}
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
		return nil, newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
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
		return nil, ErrRangeProofOnEmptyTrie
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

// newSizedProof assembles a sized result from its C parts. It owns both
// proof and wire: the wire is copied into Go memory and freed, and if that
// free fails the proof is dropped too.
func newSizedProof[P interface{ Drop() error }](
	proof P,
	wire C.OwnedBytes,
	naturalEnd C.bool,
	hint *C.SizingHint,
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
		Hint:     newSizingHintFromPtr(hint),
	}, nil
}

func getSizedRangeProofFromResult(result C.SizedRangeProofResult) (*SizedRangeProof, error) {
	switch result.tag {
	case C.SizedRangeProofResult_NullHandlePointer:
		return nil, errDBClosed
	case C.SizedRangeProofResult_RevisionNotFound:
		return nil, ErrRevisionNotFound
	case C.SizedRangeProofResult_EmptyTrie:
		return nil, ErrRangeProofOnEmptyTrie
	case C.SizedRangeProofResult_OverBudget:
		// SAFETY: the tag says the union holds the OverBudget payload, a
		// uint64_t laid out at the start of the union by cbindgen.
		return nil, &OverBudgetError{WireSize: uint64(*(*C.uint64_t)(unsafe.Pointer(&result.anon0)))}
	case C.SizedRangeProofResult_Ok:
		// SAFETY: the tag says the union holds a C.SizedRangeProof, which
		// cbindgen lays out at the start of the union. Its proof and wire
		// allocations are each released exactly once: the proof by the
		// handle created here, the wire by newSizedProof.
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
		// SAFETY: the tag says the union holds the OverBudget payload, a
		// uint64_t laid out at the start of the union by cbindgen.
		return nil, &OverBudgetError{WireSize: uint64(*(*C.uint64_t)(unsafe.Pointer(&result.anon0)))}
	case C.SizedChangeProofResult_Ok:
		// SAFETY: the tag says the union holds a C.SizedChangeProof, which
		// cbindgen lays out at the start of the union. Its proof and wire
		// allocations are each released exactly once: the proof by the
		// handle created here, the wire by newSizedProof.
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
