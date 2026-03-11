// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
import "C"

import (
	"fmt"
	"iter"
	"runtime"
	"unsafe"
)

// MultiRangeProof represents a range proof created from a multi-head database.
// Unlike [RangeProof], it can be verified and committed for a specific validator.
//
// Standalone operations (Verify, MarshalBinary, etc.) are available on
// [RangeProof] and can be used by first deserializing into a [RangeProof].
type MultiRangeProof struct {
	handle *C.MultiRangeProofContext

	// keepAliveHandle keeps the database alive while this proof owns an
	// embedded proposal.
	keepAliveHandle databaseKeepAliveHandle
}

// RangeProof returns a range proof from a multi-head database at the given root.
//
// This is a read-only operation -- no validator ID is needed.
func (db *MultiDatabase) RangeProof(
	rootHash Hash,
	startKey, endKey Maybe[[]byte],
	maxLength uint32,
) (*MultiRangeProof, error) {
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

	return getMultiRangeProofFromResult(C.fwd_multi_db_range_proof(db.handle, args))
}

// VerifyRangeProof verifies the range proof and prepares a proposal for
// the given validator without committing it.
func (db *MultiDatabase) VerifyRangeProof(
	proof *MultiRangeProof,
	startKey, endKey Maybe[[]byte],
	rootHash Hash,
	maxLength uint32,
	id ValidatorID,
) error {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return errDBClosed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	args := C.MultiVerifyRangeProofArgs{
		proof:        proof.handle,
		root:         newCHashKey(rootHash),
		start_key:    newMaybeBorrowedBytes(startKey, &pinner),
		end_key:      newMaybeBorrowedBytes(endKey, &pinner),
		max_length:   C.uint32_t(maxLength),
		validator_id: C.uint64_t(id),
	}

	if err := getErrorFromVoidResult(C.fwd_multi_db_verify_range_proof(db.handle, args)); err != nil {
		return err
	}

	// keep the database alive while the proof owns the embedded proposal
	proof.keepAliveHandle.init(&db.outstandingHandles)
	runtime.SetFinalizer(proof, (*MultiRangeProof).Free)
	return nil
}

// VerifyAndCommitRangeProof verifies the range proof and commits it for the
// given validator. The resulting root hash is returned.
func (db *MultiDatabase) VerifyAndCommitRangeProof(
	proof *MultiRangeProof,
	startKey, endKey Maybe[[]byte],
	rootHash Hash,
	maxLength uint32,
	id ValidatorID,
) (Hash, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return EmptyRoot, errDBClosed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	args := C.MultiVerifyRangeProofArgs{
		proof:        proof.handle,
		root:         newCHashKey(rootHash),
		start_key:    newMaybeBorrowedBytes(startKey, &pinner),
		end_key:      newMaybeBorrowedBytes(endKey, &pinner),
		max_length:   C.uint32_t(maxLength),
		validator_id: C.uint64_t(id),
	}

	var hash Hash
	err := proof.keepAliveHandle.disown(true /* evenOnError */, func() error {
		var err error
		hash, err = getHashKeyFromHashResult(C.fwd_multi_db_verify_and_commit_range_proof(db.handle, args))
		return err
	})
	return hash, err
}

// FindNextKey returns the next key range to fetch for this proof, if any.
func (p *MultiRangeProof) FindNextKey() (*NextKeyRange, error) {
	return getNextKeyRangeFromNextKeyRangeResult(C.fwd_multi_range_proof_find_next_key(p.handle))
}

// CodeHashes returns an iterator for the code hashes contained in the account
// nodes of this proof.
func (p *MultiRangeProof) CodeHashes() iter.Seq2[Hash, error] {
	return func(yield func(Hash, error) bool) {
		iter, err := getCodeHashIteratorFromCodeHashIteratorResult(C.fwd_multi_range_proof_code_hash_iter(p.handle))
		if err != nil {
			yield(EmptyRoot, err)
			return
		}
		defer func() {
			if err := iter.Free(); err != nil {
				panic(err)
			}
		}()
		for hash, err := iter.Next(); ; hash, err = iter.Next() {
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
}

// Free releases the resources associated with this MultiRangeProof.
// It is safe to call Free more than once.
func (p *MultiRangeProof) Free() error {
	return p.keepAliveHandle.disown(false /* evenOnError */, func() error {
		if p.handle == nil {
			return nil
		}

		if err := getErrorFromVoidResult(C.fwd_free_multi_range_proof(p.handle)); err != nil {
			return err
		}

		p.handle = nil
		return nil
	})
}

// ChangeProof returns a change proof between two revisions from a multi-head
// database.
//
// This is a read-only operation -- no validator ID is needed.
func (db *MultiDatabase) ChangeProof(
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

	return getChangeProofFromChangeProofResult(C.fwd_multi_db_change_proof(db.handle, args))
}

// MultiProposedChangeProof contains a proposed change proof for a multi-head
// database with a specific validator.
type MultiProposedChangeProof struct {
	handle *C.MultiProposedChangeProofContext
	db     *MultiDatabase
}

// ProposeChangeProof creates a proposal from a verified change proof for a
// validator.
func (db *MultiDatabase) ProposeChangeProof(
	proof *VerifiedChangeProof,
	id ValidatorID,
) (*MultiProposedChangeProof, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	args := C.MultiProposedChangeProofArgs{
		proof:        proof.handle,
		validator_id: C.uint64_t(id),
	}

	return getMultiProposedChangeProofFromResult(db, C.fwd_multi_db_propose_change_proof(db.handle, args))
}

// CommitChangeProof commits the proposed change proof.
func (proof *MultiProposedChangeProof) CommitChangeProof() (Hash, error) {
	proof.db.handleLock.RLock()
	defer proof.db.handleLock.RUnlock()
	if proof.db.handle == nil {
		return EmptyRoot, errDBClosed
	}

	args := C.MultiCommittedChangeProofArgs{
		proof: proof.handle,
	}

	return getHashKeyFromHashResult(C.fwd_multi_db_commit_change_proof(args))
}

// FindNextKey returns the next key range to fetch for this proposed change
// proof, if any.
func (proof *MultiProposedChangeProof) FindNextKey() (*NextKeyRange, error) {
	return getNextKeyRangeFromNextKeyRangeResult(C.fwd_multi_change_proof_find_next_key_proposed(proof.handle))
}

// Free releases the resources associated with this MultiProposedChangeProof.
func (proof *MultiProposedChangeProof) Free() error {
	if proof.handle == nil {
		return nil
	}

	if err := getErrorFromVoidResult(C.fwd_free_multi_proposed_change_proof(proof.handle)); err != nil {
		return err
	}

	proof.handle = nil
	return nil
}

func getMultiRangeProofFromResult(result C.MultiRangeProofResult) (*MultiRangeProof, error) {
	switch result.tag {
	case C.MultiRangeProofResult_NullHandlePointer:
		return nil, errDBClosed
	case C.MultiRangeProofResult_RevisionNotFound:
		return nil, errRevisionNotFound
	case C.MultiRangeProofResult_EmptyTrie:
		return nil, errEmptyTrie
	case C.MultiRangeProofResult_Ok:
		ptr := *(**C.MultiRangeProofContext)(unsafe.Pointer(&result.anon0))
		return &MultiRangeProof{handle: ptr}, nil
	case C.MultiRangeProofResult_Err:
		err := newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
		return nil, err
	default:
		return nil, fmt.Errorf("unknown C.MultiRangeProofResult tag: %d", result.tag)
	}
}

func getMultiProposedChangeProofFromResult(db *MultiDatabase, result C.MultiProposedChangeProofResult) (*MultiProposedChangeProof, error) {
	switch result.tag {
	case C.MultiProposedChangeProofResult_NullHandlePointer:
		return nil, errDBClosed
	case C.MultiProposedChangeProofResult_Ok:
		ptr := *(**C.MultiProposedChangeProofContext)(unsafe.Pointer(&result.anon0))
		return &MultiProposedChangeProof{handle: ptr, db: db}, nil
	case C.MultiProposedChangeProofResult_Err:
		err := newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
		return nil, err
	default:
		return nil, fmt.Errorf("unknown C.MultiProposedChangeProofResult tag: %d", result.tag)
	}
}
