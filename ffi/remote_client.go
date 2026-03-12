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

// EvictionPolicy selects the eviction strategy for the remote client's
// read cache. Mirrors the Rust [ReadCache] eviction policies.
type EvictionPolicy int

const (
	// EvictionLRU evicts the least recently used entry.
	EvictionLRU EvictionPolicy = 0
	// EvictionRandom evicts a uniformly random entry.
	EvictionRandom EvictionPolicy = 1
	// EvictionClock uses the clock (second-chance) algorithm.
	EvictionClock EvictionPolicy = 2
	// EvictionSampleK samples K random entries and evicts the LRU of the sample.
	EvictionSampleK EvictionPolicy = 3
)

// RemoteClient is a Go wrapper around a Rust RemoteClientHandle that owns
// a committed [TruncatedTrie] and an optional read cache. The cache is
// managed entirely in Rust.
//
// Instances are created via [NewRemoteClient] and must be freed with
// [RemoteClient.Free] when no longer needed.
type RemoteClient struct {
	handle  *C.RemoteClientHandle
	cleanup runtime.Cleanup
}

// NewRemoteClient creates a new remote client with an optional read cache.
// If maxCacheBytes is 0, no cache is created. The trie starts uninitialized;
// call [RemoteClient.Bootstrap] to set it.
//
// sampleK is used only when policy is [EvictionSampleK]; ignored otherwise.
func NewRemoteClient(maxCacheBytes int, policy EvictionPolicy, sampleK int) (*RemoteClient, error) {
	if maxCacheBytes < 0 {
		maxCacheBytes = 0
	}
	if sampleK <= 0 {
		sampleK = 5 // default sample size
	}

	result := C.fwd_create_remote_client(
		C.size_t(maxCacheBytes),
		C.enum_CEvictionPolicy(policy),
		C.size_t(sampleK),
	)
	return getRemoteClientFromResult(result)
}

// getRemoteClientFromResult converts a C.RemoteClientResult to a
// RemoteClient or error.
func getRemoteClientFromResult(result C.RemoteClientResult) (*RemoteClient, error) {
	switch result.tag {
	case C.RemoteClientResult_NullHandlePointer:
		return nil, errDBClosed
	case C.RemoteClientResult_Ok:
		ptr := *(**C.RemoteClientHandle)(unsafe.Pointer(&result.anon0))
		rc := &RemoteClient{handle: ptr}
		rc.cleanup = runtime.AddCleanup(rc, func(h *C.RemoteClientHandle) {
			C.fwd_free_remote_client(h)
		}, ptr)
		return rc, nil
	case C.RemoteClientResult_Err:
		return nil, newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
	default:
		return nil, fmt.Errorf("unknown C.RemoteClientResult tag: %d", result.tag)
	}
}

// Bootstrap deserializes trieBytes into a truncated trie, verifies that
// its root hash matches expectedHash, replaces the internal trie, and
// clears the cache. Returns the root hash on success.
func (rc *RemoteClient) Bootstrap(trieBytes []byte, expectedHash Hash) (Hash, error) {
	if rc.handle == nil {
		return EmptyRoot, fmt.Errorf("remote client already freed")
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	return getHashKeyFromHashResult(C.fwd_remote_client_bootstrap(
		rc.handle,
		newBorrowedBytes(trieBytes, &pinner),
		newCHashKey(expectedHash),
	))
}

// CacheLookupResult represents the outcome of a cache lookup.
type CacheLookupResult struct {
	// Value holds the cached value bytes. Nil if the key was absent or missed.
	Value []byte
	// Found is true if the cache held a positive result (key exists with value).
	Found bool
	// Cached is true if the lookup was a cache hit (either present or absent).
	Cached bool
}

// CacheLookup looks up a key in the read cache.
//
// Returns a [CacheLookupResult] where:
//   - Cached=false means cache miss (or no cache configured).
//   - Cached=true, Found=true means the key was cached with a value.
//   - Cached=true, Found=false means the key was cached as absent.
func (rc *RemoteClient) CacheLookup(key []byte) CacheLookupResult {
	if rc.handle == nil {
		return CacheLookupResult{}
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	result := C.fwd_remote_client_cache_lookup(
		rc.handle,
		newBorrowedBytes(key, &pinner),
	)
	return getCacheLookupFromResult(result)
}

// getCacheLookupFromResult converts a C.CacheLookupResult to a Go
// CacheLookupResult.
func getCacheLookupFromResult(result C.CacheLookupResult) CacheLookupResult {
	switch result.tag {
	case C.CacheLookupResult_NullHandlePointer, C.CacheLookupResult_Miss:
		return CacheLookupResult{}
	case C.CacheLookupResult_HitPresent:
		owned := newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0)))
		value := owned.CopiedBytes()
		_ = owned.Free()
		return CacheLookupResult{Value: value, Found: true, Cached: true}
	case C.CacheLookupResult_HitAbsent:
		return CacheLookupResult{Cached: true}
	case C.CacheLookupResult_Err:
		// Errors during lookup are treated as misses.
		_ = newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).Free()
		return CacheLookupResult{}
	default:
		return CacheLookupResult{}
	}
}

// VerifyGet verifies a single-key proof against the committed root hash
// and, on success, stores the result in the cache.
//
// Set value to nil for exclusion proofs (proving a key does not exist).
func (rc *RemoteClient) VerifyGet(key, value, proof []byte) error {
	if rc.handle == nil {
		return fmt.Errorf("remote client already freed")
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	valueIsPresent := value != nil

	return getErrorFromVoidResult(C.fwd_remote_client_verify_get(
		rc.handle,
		newBorrowedBytes(key, &pinner),
		newBorrowedBytes(value, &pinner),
		C.bool(valueIsPresent),
		newBorrowedBytes(proof, &pinner),
	))
}

// VerifyWitness verifies a witness proof against the committed trie and
// returns a new [TruncatedTrie] for the proposal.
//
// The witness is NOT consumed — the caller keeps it alive for
// [RemoteClient.CommitTrie].
func (rc *RemoteClient) VerifyWitness(witness *WitnessProof, expectedOps []BatchOp) (*TruncatedTrie, error) {
	if rc.handle == nil {
		return nil, fmt.Errorf("remote client already freed")
	}
	if witness == nil || witness.handle == nil {
		return nil, fmt.Errorf("witness proof is nil or already freed")
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	cOps := newKeyValuePairsFromBatch(expectedOps, &pinner)

	return getTruncatedTrieFromResult(C.fwd_remote_client_verify_witness(
		rc.handle,
		witness.handle,
		cOps,
	))
}

// CommitTrie takes ownership of newTrie, writes it into the remote client's
// internal committed trie, and invalidates cache entries affected by the
// witness's batch operations. Returns the new root hash.
//
// After this call, newTrie is consumed and must not be used.
func (rc *RemoteClient) CommitTrie(newTrie *TruncatedTrie, witness *WitnessProof) (Hash, error) {
	if rc.handle == nil {
		return EmptyRoot, fmt.Errorf("remote client already freed")
	}
	if newTrie == nil || newTrie.handle == nil {
		return EmptyRoot, fmt.Errorf("new trie is nil or already freed")
	}
	if witness == nil || witness.handle == nil {
		return EmptyRoot, fmt.Errorf("witness proof is nil or already freed")
	}

	// Cancel the cleanup on newTrie since ownership transfers to Rust.
	newTrie.cleanup.Stop()
	trieHandle := newTrie.handle
	newTrie.handle = nil

	result := C.fwd_remote_client_commit_trie(
		rc.handle,
		trieHandle,
		witness.handle,
	)
	return getHashKeyFromHashResult(result)
}

// Free releases the resources associated with this RemoteClient.
//
// It is safe to call Free more than once; subsequent calls after the first
// will be no-ops.
func (rc *RemoteClient) Free() error {
	if rc.handle == nil {
		return nil
	}

	rc.cleanup.Stop()

	if err := getErrorFromVoidResult(C.fwd_free_remote_client(rc.handle)); err != nil {
		return err
	}

	rc.handle = nil
	return nil
}
