// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
import "C"

import (
	"fmt"
	"runtime"
	"unsafe"
)

// TruncatedTrie is a Go wrapper around a truncated trie handle returned from
// Firewood. A truncated trie holds the top K nibble levels of a Merkle trie
// with children below that depth replaced by hash-only proxy nodes.
//
// Instances are created via [Database.CreateTruncatedTrie] and must be freed
// with [TruncatedTrie.Free] when no longer needed. A finalizer safety net
// will eventually free the handle if forgotten, but explicit Free is preferred
// for prompt resource release.
type TruncatedTrie struct {
	handle  *C.TruncatedTrieHandle
	root    Hash
	cleanup runtime.Cleanup
}

// Root returns the root hash of the truncated trie.
func (t *TruncatedTrie) Root() Hash {
	return t.root
}

// RootHash returns the root hash of the truncated trie by querying the
// underlying handle. Returns [EmptyRoot] if the trie is empty.
func (t *TruncatedTrie) RootHash() (Hash, error) {
	if t.handle == nil {
		return EmptyRoot, fmt.Errorf("truncated trie already freed")
	}
	return getHashKeyFromHashResult(C.fwd_truncated_trie_root_hash(t.handle))
}

// VerifyRootHash verifies that the truncated trie's root hash matches the
// expected value.
func (t *TruncatedTrie) VerifyRootHash(expected Hash) error {
	if t.handle == nil {
		return fmt.Errorf("truncated trie already freed")
	}
	return getErrorFromVoidResult(C.fwd_verify_truncated_trie_root_hash(
		t.handle,
		newCHashKey(expected),
	))
}

// Free releases the resources associated with this TruncatedTrie.
//
// It is safe to call Free more than once; subsequent calls after the first
// will be no-ops. Explicit Free is preferred for prompt resource release,
// though a finalizer safety net will eventually free the handle if forgotten.
func (t *TruncatedTrie) Free() error {
	if t.handle == nil {
		return nil
	}

	// Cancel the GC-based cleanup since we are freeing explicitly.
	t.cleanup.Stop()

	if err := getErrorFromVoidResult(C.fwd_free_truncated_trie(t.handle)); err != nil {
		return err
	}

	t.handle = nil
	return nil
}

// CreateTruncatedTrie creates a truncated trie from the database revision
// matching the given root hash. The trie holds the top [depth] nibble levels
// with children below that depth replaced by hash-only proxy nodes.
//
// The returned [TruncatedTrie] must be freed with [TruncatedTrie.Free] when
// no longer needed.
func (db *Database) CreateTruncatedTrie(root Hash, depth uint) (*TruncatedTrie, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	return getTruncatedTrieFromResult(C.fwd_create_truncated_trie(
		db.handle,
		newCHashKey(root),
		C.size_t(depth),
	))
}

// VerifyWitness verifies a witness proof against this truncated trie and
// returns a new [TruncatedTrie] reflecting the updated state. The old trie
// is NOT consumed; the caller must still free it.
//
// Re-executes the batch operations from the witness on top of the truncated
// trie, hashes the result, and verifies it matches the witness's new root
// hash.
func (t *TruncatedTrie) VerifyWitness(witness *WitnessProof) (*TruncatedTrie, error) {
	if t.handle == nil {
		return nil, fmt.Errorf("truncated trie already freed")
	}
	if witness == nil || witness.handle == nil {
		return nil, fmt.Errorf("witness proof is nil or already freed")
	}

	return getTruncatedTrieFromResult(C.fwd_verify_witness(
		t.handle,
		witness.handle,
	))
}

// getTruncatedTrieFromResult converts a C.TruncatedTrieResult to a
// TruncatedTrie or error.
func getTruncatedTrieFromResult(result C.TruncatedTrieResult) (*TruncatedTrie, error) {
	switch result.tag {
	case C.TruncatedTrieResult_NullHandlePointer:
		return nil, errDBClosed
	case C.TruncatedTrieResult_Ok:
		body := (*C.TruncatedTrieResult_Ok_Body)(unsafe.Pointer(&result.anon0))
		hashKey := *(*Hash)(unsafe.Pointer(&body.root_hash._0))
		trie := &TruncatedTrie{
			handle: body.handle,
			root:   hashKey,
		}
		trie.cleanup = runtime.AddCleanup(trie, func(h *C.TruncatedTrieHandle) {
			C.fwd_free_truncated_trie(h)
		}, body.handle)
		return trie, nil
	case C.TruncatedTrieResult_Err:
		return nil, newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
	default:
		return nil, fmt.Errorf("unknown C.TruncatedTrieResult tag: %d", result.tag)
	}
}

// MarshalBinary serializes the TruncatedTrie into a binary format suitable
// for wire transport (e.g., gRPC). Implements [encoding.BinaryMarshaler].
func (t *TruncatedTrie) MarshalBinary() ([]byte, error) {
	if t.handle == nil {
		return nil, fmt.Errorf("truncated trie already freed")
	}
	result := C.fwd_truncated_trie_to_bytes(t.handle)
	return getValueFromValueResult(result)
}

// UnmarshalBinary deserializes bytes produced by [TruncatedTrie.MarshalBinary]
// into this TruncatedTrie, replacing any existing state.
//
// If the receiver already holds a handle, it is freed first.
// Implements [encoding.BinaryUnmarshaler].
func (t *TruncatedTrie) UnmarshalBinary(data []byte) error {
	// Free any existing handle
	if t.handle != nil {
		if err := t.Free(); err != nil {
			return err
		}
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	result := C.fwd_truncated_trie_from_bytes(newBorrowedBytes(data, &pinner))
	newTrie, err := getTruncatedTrieFromResult(result)
	if err != nil {
		return err
	}

	// Stop the cleanup registered on newTrie since we are transferring
	// ownership of the handle to the receiver.
	newTrie.cleanup.Stop()

	t.handle = newTrie.handle
	t.root = newTrie.root

	// Register a new cleanup on the receiver.
	t.cleanup = runtime.AddCleanup(t, func(h *C.TruncatedTrieHandle) {
		C.fwd_free_truncated_trie(h)
	}, t.handle)

	return nil
}

// GenerateWitness generates a witness proof for a set of batch operations
// against the revision identified by [root]. The witness contains the minimal
// set of trie nodes needed for a client to independently re-execute the
// operations and verify the resulting root hash.
//
// [newRoot] is the root hash after applying the batch operations.
// [depth] is the client's truncation depth in nibble levels.
//
// The returned [WitnessProof] must be freed with [WitnessProof.Free] when
// no longer needed.
func (db *Database) GenerateWitness(root Hash, batch []BatchOp, newRoot Hash, depth uint) (*WitnessProof, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	kvp := newKeyValuePairsFromBatch(batch, &pinner)

	return getWitnessProofFromResult(C.fwd_generate_witness(
		db.handle,
		newCHashKey(root),
		kvp,
		newCHashKey(newRoot),
		C.size_t(depth),
	))
}
