// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// Package ffi provides a Go wrapper around the [Firewood] database.
//
// [Firewood]: https://github.com/ava-labs/firewood
package ffi

// #include <stdlib.h>
// #include "firewood.h"
import "C"

import (
	"errors"
	"fmt"
	"log"
	"runtime"
	"unsafe"
)

var errUnknown = errors.New("unknown error")

type Pinner interface {
	Pin(ptr any)
	Unpin()
}

type ownedBytesAsError RustOwnedBytes

func (e *ownedBytesAsError) Error() string {
	if e == nil {
		return errUnknown.Error()
	}

	return (*RustOwnedBytes)(e).string()
}

type HashKey [32]byte

func NewHashKey(b []byte) (HashKey, error) {
	if len(b) != 32 {
		return HashKey{}, fmt.Errorf("hash key must be 32 bytes, got %d", len(b))
	}
	var key HashKey
	copy(key[:], b)
	return key, nil
}

func (h HashKey) toC() C.HashKey {
	return *(*C.HashKey)(unsafe.Pointer(&h))
}

// newBorrowedBytes creates a new BorrowedBytes from a Go byte slice.
//
// Provide a Pinner to ensure the memory is pinned while the BorrowedBytes is in use.
func newBorrowedBytes(slice []byte, pinner Pinner) C.BorrowedBytes {
	sliceLen := len(slice)
	if sliceLen == 0 {
		return C.BorrowedBytes{ptr: nil, len: 0}
	}

	ptr := unsafe.SliceData(slice)
	if ptr == nil {
		return C.BorrowedBytes{ptr: nil, len: 0}
	}

	pinner.Pin(ptr)

	return C.BorrowedBytes{
		ptr: (*C.uint8_t)(ptr),
		len: C.size_t(sliceLen),
	}
}

// newKeyValuePair creates a new KeyValuePair from Go byte slices for key and value.
//
// Provide a Pinner to ensure the memory is pinned while the KeyValuePair is in use.
func newKeyValuePair(key, value []byte, pinner Pinner) C.KeyValuePair {
	return C.KeyValuePair{
		key:   newBorrowedBytes(key, pinner),
		value: newBorrowedBytes(value, pinner),
	}
}

// newBorrowedKeyValuePairs creates a new BorrowedKeyValuePairs from a slice of KeyValuePair.
//
// Provide a Pinner to ensure the memory is pinned while the BorrowedKeyValuePairs is
// in use.
func newBorrowedKeyValuePairs(pairs []C.KeyValuePair, pinner Pinner) C.BorrowedKeyValuePairs {
	sliceLen := len(pairs)
	if sliceLen == 0 {
		return C.BorrowedKeyValuePairs{ptr: nil, len: 0}
	}

	ptr := unsafe.SliceData(pairs)
	if ptr == nil {
		return C.BorrowedKeyValuePairs{ptr: nil, len: 0}
	}

	pinner.Pin(ptr)

	return C.BorrowedKeyValuePairs{
		ptr: ptr,
		len: C.size_t(sliceLen),
	}
}

// newKeyValuePairs creates a new BorrowedKeyValuePairs from slices of keys and values.
//
// The keys and values must have the same length.
//
// Provide a Pinner to ensure the memory is pinned while the BorrowedKeyValuePairs is
// in use.
func newKeyValuePairs(keys, vals [][]byte, pinner Pinner) (C.BorrowedKeyValuePairs, error) {
	if len(keys) != len(vals) {
		return C.BorrowedKeyValuePairs{}, fmt.Errorf("keys and values must have the same length: %d != %d", len(keys), len(vals))
	}

	pairs := make([]C.KeyValuePair, len(keys))
	for i := range keys {
		pairs[i] = newKeyValuePair(keys[i], vals[i], pinner)
	}

	return newBorrowedKeyValuePairs(pairs, pinner), nil
}

// free releases the memory associated with the Database.
//
// This is not safe to call while there are any outstanding Proposals. All proposals
// must be freed or committed before calling this.
//
// This is safe to call if the pointer is nil, in which case it does nothing. The
// pointer will be set to nil after freeing to prevent double free.
//
// clearFinalizer indicates whether to clear the finalizer to prevent double finalization.
//
// This should be false when called by the finalizer itself but true when called
// from other code that explicitly frees the memory.
func (db *Database) free(clearFinalizer bool) error {
	if db == nil {
		return nil
	}

	if clearFinalizer {
		runtime.SetFinalizer(db, nil) // Prevent double finalization
	}

	ptr := db.handle
	if ptr == nil {
		return nil
	}

	// Set before freeing in case we panic or race with the finalizer
	db.handle = nil

	if err := fromVoidResult(C.fwd_close_db(ptr)); err != nil {
		return fmt.Errorf("unexpected error when closing database: %w", err)
	}

	return nil
}

// free releases the memory associated with the Proposal.
//
// This is safe to call if the pointer is nil, in which case it does nothing.
//
// The pointer will be set to nil after freeing to prevent double free.
func (p *Proposal) free(clearFinalizer bool) error {
	if p == nil {
		return nil
	}

	if clearFinalizer {
		runtime.SetFinalizer(p, nil) // Prevent double finalization
	}

	ptr := p.handle
	if ptr == nil {
		return nil
	}

	// Set before freeing in case we panic or race with the finalizer
	p.handle = nil

	if err := fromVoidResult(C.fwd_free_proposal(ptr)); err != nil {
		return fmt.Errorf("unexpected error when freeing proposal: %w", err)
	}

	return nil
}

// RustOwnedBytes is a wrapper around C.OwnedBytes that provides a Go interface
// for Rust-owned byte slices.
type RustOwnedBytes struct {
	owned C.OwnedBytes
}

// Free releases the memory associated with the RustOwnedBytes.
//
// It is safe to call this method multiple times, and it will do nothing if the
// RustOwnedBytes is nil or has already been freed.
func (b *RustOwnedBytes) Free() error {
	return b.free(true)
}

func (b *RustOwnedBytes) free(clearFinalizer bool) error {
	if b == nil {
		return nil
	}

	if clearFinalizer {
		runtime.SetFinalizer(b, nil) // Prevent double finalization
	}

	this := *b               // copy so we can reset before freeing,
	b.owned = C.OwnedBytes{} // reset to clear the pointer

	if this.owned.ptr == nil {
		return nil
	}

	if err := fromVoidResult(C.fwd_free_owned_bytes(this.owned)); err != nil {
		return fmt.Errorf("unexpected error when freeing owned bytes: %w", err)
	}

	return nil
}

// BorrowedBytes returns the underlying byte slice. It may return nil if the
// data has already been freed was never set.
//
// The returned slice is valid only as long as the RustOwnedBytes is valid.
//
// It does not copy the data; however, the slice is valid only as long as the
// RustOwnedBytes is valid. If the RustOwnedBytes is freed, the slice will
// become invalid.
func (b *RustOwnedBytes) BorrowedBytes() []byte {
	if b == nil {
		return nil
	}

	if b.owned.ptr == nil {
		return nil
	}

	return unsafe.Slice((*byte)(b.owned.ptr), b.owned.len)
}

func (b *RustOwnedBytes) CopiedBytes() []byte {
	if b == nil {
		return nil
	}

	if b.owned.ptr == nil {
		return nil
	}

	return C.GoBytes(unsafe.Pointer(b.owned.ptr), C.int(b.owned.len))
}

// string returns the string representation of the RustOwnedBytes.
//
// It is not exported because it should not be used except by types that are certain
// that the RustOwnedBytes is of a utf-8 encoded string.
func (b *RustOwnedBytes) string() string {
	if b == nil {
		return ""
	}

	if b.owned.ptr == nil {
		return ""
	}

	return unsafe.String((*byte)(b.owned.ptr), b.owned.len)
}

// fromOwnedBytes creates a RustOwnedBytes from a C.OwnedBytes.
//
// It sets a finalizer to free the memory when the RustOwnedBytes is no longer
// referenced. If the C.OwnedBytes is nil, it returns nil.
func fromOwnedBytes(owned C.OwnedBytes) *RustOwnedBytes {
	if owned.ptr == nil {
		return nil
	}

	rustBytes := &RustOwnedBytes{owned: owned}
	runtime.SetFinalizer(rustBytes, func(b *RustOwnedBytes) {
		if err := b.free(false); err != nil {
			log.Panicf("failed to free RustOwnedBytes: %v", err)
		}
	})

	return rustBytes
}

// fromHashResult creates a byte slice or error from a C.HashResult.
//
// It returns nil, nil if the result is None.
// It returns nil, err if the result is an error.
// It returns a byte slice, nil if the result is Some.
func fromHashResult(result C.HashResult) ([]byte, error) {
	switch result.tag {
	case C.HashResult_NullHandlePointer:
		return nil, errDBClosed
	case C.HashResult_None:
		return nil, nil
	case C.HashResult_Some:
		// copy the bytes from the C.HashResult to a fixed-size array
		hashKey := fromHashKey((*C.HashKey)(unsafe.Pointer(&result.anon0)))
		return hashKey, nil
	case C.HashResult_Err:
		ownedBytes := fromOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0)))
		// the finalizer will free the memory when the pointer goes out of scope
		return nil, (*ownedBytesAsError)(ownedBytes)
	default:
		return nil, fmt.Errorf("unknown C.HashResult tag: %d", result.tag)
	}
}

// fromVoidResult converts a C.VoidResult to an error.
//
// It return nil if the result is Ok, otherwise it return an error.
func fromVoidResult(result C.VoidResult) error {
	switch result.tag {
	case C.VoidResult_NullHandlePointer:
		return errDBClosed
	case C.VoidResult_Ok:
		return nil
	case C.VoidResult_Err:
		ownedBytes := fromOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0)))
		// the finalizer will free the memory when the pointer goes out of scope
		return (*ownedBytesAsError)(ownedBytes)
	default:
		return fmt.Errorf("unknown C.VoidResult tag: %d", result.tag)
	}
}

// fromValueResult converts a C.ValueResult to a byte slice or error.
//
// It returns nil, nil if the result is None.
// It returns nil, errRevisionNotFound if the result is RevisionNotFound.
// It returns a byte slice, nil if the result is Some.
// It returns an error if the result is an error.
func fromValueResult(result C.ValueResult) ([]byte, error) {
	switch result.tag {
	case C.ValueResult_NullHandlePointer:
		return nil, errDBClosed
	case C.ValueResult_RevisionNotFound:
		// NOTE: the result value contains the provided root hash, we could use
		// it in the error message if needed.
		return nil, errRevisionNotFound
	case C.ValueResult_None:
		return nil, nil
	case C.ValueResult_Some:
		ownedBytes := fromOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0)))
		bytes := ownedBytes.CopiedBytes()
		if err := ownedBytes.Free(); err != nil {
			return nil, fmt.Errorf("failed to free owned bytes: %w", err)
		}
		return bytes, nil
	case C.ValueResult_Err:
		ownedBytes := fromOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0)))
		// the finalizer will free the memory when the pointer goes out of scope
		return nil, (*ownedBytesAsError)(ownedBytes)
	default:
		return nil, fmt.Errorf("unknown C.ValueResult tag: %d", result.tag)
	}
}

// fromHandleResult converts a C.HandleResult to a Database or error.
//
// It sets a finalizer to free the memory when the Database is no longer
// referenced.
//
// If the C.HandleResult is an error, it returns an error instead of a Database.
func fromHandleResult(result C.HandleResult) (*Database, error) {
	switch result.tag {
	case C.HandleResult_Ok:
		ptr := *(**C.DatabaseHandle)(unsafe.Pointer(&result.anon0))
		db := &Database{
			handle: ptr,
		}
		runtime.SetFinalizer(db, func(db *Database) {
			if err := db.free(false); err != nil {
				log.Panicf("failed to free Database: %v", err)
			}
		})
		return db, nil
	case C.HandleResult_Err:
		ownedBytes := fromOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0)))
		// the finalizer will free the memory when the pointer goes out of scope
		return nil, (*ownedBytesAsError)(ownedBytes)
	default:
		return nil, fmt.Errorf("unknown C.HandleResult tag: %d", result.tag)
	}
}

// fromProposalResult converts a C.ProposalResult to a Proposal or error.
//
// It sets a finalizer to free the memory when the Proposal is no longer
// referenced. However, there is no guarantee that the Proposal will be free
// before the Database is closed, so it is recommended to call Commit() or Free()
// on the Proposal before closing the Database.
func fromProposalResult(result C.ProposalResult, db *Database) (*Proposal, error) {
	switch result.tag {
	case C.ProposalResult_NullHandlePointer:
		return nil, errDBClosed
	case C.ProposalResult_Ok:
		body := (*C.ProposalResult_Ok_Body)(unsafe.Pointer(&result.anon0))
		proposal := &Proposal{
			db:     db,
			handle: body.handle,
			root:   fromHashKey(&body.root_hash),
		}
		runtime.SetFinalizer(proposal, func(p *Proposal) {
			if err := p.free(false); err != nil {
				log.Panicf("failed to free Proposal: %v", err)
			}
		})
		return proposal, nil
	case C.ProposalResult_Err:
		ownedBytes := fromOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0)))
		// the finalizer will free the memory when the pointer goes out of scope
		return nil, (*ownedBytesAsError)(ownedBytes)
	default:
		return nil, fmt.Errorf("unknown C.ProposalResult tag: %d", result.tag)
	}
}

func fromHashKey(key *C.HashKey) []byte {
	if key == nil {
		return nil
	}
	hashKey := make([]byte, 32)
	copy(hashKey, unsafe.Slice((*byte)(unsafe.Pointer(key)), 32))
	return hashKey
}
