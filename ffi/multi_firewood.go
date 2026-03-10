// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
import "C"

import (
	"context"
	"fmt"
	"runtime"
	"sync"
	"unsafe"
)

// ValidatorID is the type used for validator identifiers in multi-head mode.
type ValidatorID uint64

// MultiDatabase is an FFI wrapper for the Rust multi-validator Firewood database.
// All functions rely on CGO to call into the underlying Rust implementation.
// Instances are created via [NewMulti] and must be closed with [MultiDatabase.Close]
// when no longer needed.
//
// Unlike [Database], MultiDatabase does not have a single "latest" revision.
// Each registered validator has its own head, and all operations are validator-scoped.
// MultiDatabase operations are internally synchronized, so no external commitLock is needed.
type MultiDatabase struct {
	handle             *C.MultiDatabaseHandle
	handleLock         sync.RWMutex
	outstandingHandles sync.WaitGroup
}

// NewMulti opens or creates a new multi-validator Firewood database.
// The database directory will be created at the provided path if it does not already exist.
//
// [maxValidators] sets the maximum number of validators that can be registered.
//
// It is the caller's responsibility to call [MultiDatabase.Close] when the database
// is no longer needed.
func NewMulti(dbDir string, nodeHashAlgorithm NodeHashAlgorithm, maxValidators uint, opts ...Option) (*MultiDatabase, error) {
	conf := defaultConfig()
	for _, opt := range opts {
		opt(conf)
	}

	if conf.readCacheStrategy >= invalidCacheStrategy {
		return nil, fmt.Errorf("invalid cache strategy (%d)", conf.readCacheStrategy)
	}
	if conf.revisions < 2 {
		return nil, fmt.Errorf("revisions must be >= 2, got %d", conf.revisions)
	}
	if conf.nodeCacheSizeInBytes < 1 {
		return nil, fmt.Errorf("node cache size in bytes must be >= 1, got %d", conf.nodeCacheSizeInBytes)
	}
	if conf.freeListCacheEntries < 1 {
		return nil, fmt.Errorf("free list cache entries must be >= 1, got %d", conf.freeListCacheEntries)
	}
	if maxValidators < 1 {
		return nil, fmt.Errorf("max validators must be >= 1, got %d", maxValidators)
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	dbArgs := C.struct_DatabaseHandleArgs{
		dir:                               newBorrowedBytes([]byte(dbDir), &pinner),
		node_cache_memory_limit:           C.size_t(conf.nodeCacheSizeInBytes),
		free_list_cache_size:              C.size_t(conf.freeListCacheEntries),
		revisions:                         C.size_t(conf.revisions),
		strategy:                          C.uint8_t(conf.readCacheStrategy),
		truncate:                          C.bool(conf.truncate),
		root_store:                        C.bool(conf.rootStore),
		expensive_metrics:                 C.bool(conf.expensiveMetricsEnabled),
		node_hash_algorithm:               C.enum_NodeHashAlgorithm(nodeHashAlgorithm),
		deferred_persistence_commit_count: C.uint64_t(conf.deferredPersistenceCommitCount),
	}

	args := C.struct_MultiDatabaseHandleArgs{
		db_args:        dbArgs,
		max_validators: C.size_t(maxValidators),
	}

	return getMultiDatabaseFromHandleResult(C.fwd_multi_open_db(args))
}

// RegisterValidator registers a new validator. The validator starts at the
// latest persisted revision.
func (db *MultiDatabase) RegisterValidator(id ValidatorID) error {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return errDBClosed
	}

	return getErrorFromVoidResult(C.fwd_multi_register_validator(db.handle, C.uint64_t(id)))
}

// DeregisterValidator deregisters a validator, freeing its header slot.
func (db *MultiDatabase) DeregisterValidator(id ValidatorID) error {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return errDBClosed
	}

	return getErrorFromVoidResult(C.fwd_multi_deregister_validator(db.handle, C.uint64_t(id)))
}

// Get retrieves the value for the given key from a validator's current head.
// If the key is not found, the return value will be nil.
func (db *MultiDatabase) Get(id ValidatorID, key []byte) ([]byte, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	return getValueFromValueResult(C.fwd_multi_get(db.handle, C.uint64_t(id), newBorrowedBytes(key, &pinner)))
}

// Root returns the root hash of a validator's current head.
func (db *MultiDatabase) Root(id ValidatorID) (Hash, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return EmptyRoot, errDBClosed
	}

	return getHashKeyFromHashResult(C.fwd_multi_root_hash(db.handle, C.uint64_t(id)))
}

// Update applies a batch of operations for a validator, committing immediately.
// This is equivalent to calling Propose followed by Commit.
//
// Use [Put], [Delete], and [PrefixDelete] to create batch operations.
func (db *MultiDatabase) Update(id ValidatorID, batch []BatchOp) (Hash, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return EmptyRoot, errDBClosed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	kvp := newKeyValuePairsFromBatch(batch, &pinner)
	return getHashKeyFromHashResult(C.fwd_multi_update(db.handle, C.uint64_t(id), kvp))
}

// Propose creates a new proposal for a validator. The proposal is not committed
// until [MultiProposal.Commit] is called.
//
// Use [Put], [Delete], and [PrefixDelete] to create batch operations.
func (db *MultiDatabase) Propose(id ValidatorID, batch []BatchOp) (*MultiProposal, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	kvp := newKeyValuePairsFromBatch(batch, &pinner)
	return getMultiProposalFromResult(C.fwd_multi_propose(db.handle, C.uint64_t(id), kvp), &db.outstandingHandles)
}

// AdvanceToHash advances a validator's head to an existing revision by hash.
// This is the skip-propose optimization.
func (db *MultiDatabase) AdvanceToHash(id ValidatorID, hash Hash) error {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return errDBClosed
	}

	return getErrorFromVoidResult(C.fwd_multi_advance_to_hash(db.handle, C.uint64_t(id), newCHashKey(hash)))
}

// LatestRevision returns a [Revision] representing the current head of a validator.
// The [Revision] must be dropped prior to closing the database.
func (db *MultiDatabase) LatestRevision(id ValidatorID) (*Revision, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	return getRevisionFromResult(C.fwd_multi_latest_revision(db.handle, C.uint64_t(id)), &db.outstandingHandles)
}

// Revision returns a historical revision by hash.
// The [Revision] must be dropped prior to closing the database.
func (db *MultiDatabase) Revision(hash Hash) (*Revision, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	return getRevisionFromResult(C.fwd_multi_revision(db.handle, newCHashKey(hash)), &db.outstandingHandles)
}

// CreateTruncatedTrie creates a truncated trie from the revision matching
// the given root hash. The trie holds the top [depth] nibble levels with
// children below that depth replaced by hash-only proxy nodes.
//
// The returned [TruncatedTrie] must be freed with [TruncatedTrie.Free] when
// no longer needed.
func (db *MultiDatabase) CreateTruncatedTrie(root Hash, depth uint) (*TruncatedTrie, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	return getTruncatedTrieFromResult(C.fwd_multi_create_truncated_trie(
		db.handle,
		newCHashKey(root),
		C.size_t(depth),
	))
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
func (db *MultiDatabase) GenerateWitness(root Hash, batch []BatchOp, newRoot Hash, depth uint) (*WitnessProof, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	kvp := newKeyValuePairsFromBatch(batch, &pinner)

	return getWitnessProofFromResult(C.fwd_multi_generate_witness(
		db.handle,
		newCHashKey(root),
		kvp,
		newCHashKey(newRoot),
		C.size_t(depth),
	))
}

// GetWithProof retrieves a value and its single-key Merkle proof from the
// revision identified by root.
func (db *MultiDatabase) GetWithProof(root Hash, key []byte) (*GetWithProofResult, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return nil, errDBClosed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	result := C.fwd_multi_get_with_proof(
		db.handle,
		newCHashKey(root),
		newBorrowedBytes(key, &pinner),
	)

	return getGetWithProofFromResult(result)
}

// RangeProof returns a proof that the values in the range [startKey, endKey]
// are included in the tree at the given root hash.
func (db *MultiDatabase) RangeProof(
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

	return getRangeProofFromRangeProofResult(C.fwd_multi_range_proof(db.handle, args))
}

// Dump returns a DOT (Graphviz) format representation of the trie structure
// of a validator's current head for debugging purposes.
func (db *MultiDatabase) Dump(id ValidatorID) (string, error) {
	db.handleLock.RLock()
	defer db.handleLock.RUnlock()
	if db.handle == nil {
		return "", errDBClosed
	}

	bytes, err := getValueFromValueResult(C.fwd_multi_db_dump(db.handle, C.uint64_t(id)))
	if err != nil {
		return "", err
	}
	return string(bytes), nil
}

// Close releases the memory associated with the MultiDatabase and stops the
// background persistence thread.
//
// This blocks until all outstanding keep-alive handles are disowned or the
// context is cancelled.
func (db *MultiDatabase) Close(ctx context.Context) error {
	db.handleLock.Lock()
	defer db.handleLock.Unlock()
	if db.handle == nil {
		return nil
	}

	done := make(chan struct{})
	go func() {
		db.outstandingHandles.Wait()
		close(done)
	}()

	select {
	case <-done:
	case <-ctx.Done():
		return fmt.Errorf("%w: %w", ctx.Err(), ErrActiveKeepAliveHandles)
	}

	if err := getErrorFromVoidResult(C.fwd_multi_close_db(db.handle)); err != nil {
		return fmt.Errorf("unexpected error when closing multi database: %w", err)
	}

	db.handle = nil
	return nil
}

// MultiProposal represents a set of proposed changes for a multi-validator database.
// The validator ID is captured at creation time and used automatically for Commit.
type MultiProposal struct {
	*handle[*C.MultiProposalHandle]
	root Hash
}

// Root retrieves the root hash of the proposal.
func (p *MultiProposal) Root() Hash {
	return p.root
}

// Get retrieves the value for the given key from the proposal.
// If the key does not exist, it returns nil.
func (p *MultiProposal) Get(key []byte) ([]byte, error) {
	p.keepAliveHandle.mu.RLock()
	defer p.keepAliveHandle.mu.RUnlock()
	if p.ptr == nil {
		return nil, errDroppedProposal
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	return getValueFromValueResult(C.fwd_multi_get_from_proposal(p.ptr, newBorrowedBytes(key, &pinner)))
}

// Iter creates an [Iterator] over the key-value pairs in this proposal,
// starting at the first key greater than or equal to the provided key.
// Pass nil or an empty slice to iterate from the beginning.
func (p *MultiProposal) Iter(key []byte) (*Iterator, error) {
	p.keepAliveHandle.mu.RLock()
	defer p.keepAliveHandle.mu.RUnlock()
	if p.ptr == nil {
		return nil, errDroppedProposal
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	itResult := C.fwd_multi_iter_on_proposal(p.ptr, newBorrowedBytes(key, &pinner))
	return getIteratorFromIteratorResult(itResult)
}

// Propose creates a child proposal that inherits the validator ID.
// The returned proposal cannot be committed until the parent has been committed.
func (p *MultiProposal) Propose(batch []BatchOp) (*MultiProposal, error) {
	p.keepAliveHandle.mu.RLock()
	defer p.keepAliveHandle.mu.RUnlock()
	if p.ptr == nil {
		return nil, errDroppedProposal
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	kvp := newKeyValuePairsFromBatch(batch, &pinner)
	return getMultiProposalFromResult(C.fwd_multi_propose_on_proposal(p.ptr, kvp), p.keepAliveHandle.outstandingHandles)
}

// Commit commits the proposal using the stored validator ID.
// The underlying data is no longer available during/after this call.
func (p *MultiProposal) Commit() error {
	return p.keepAliveHandle.disown(true /* evenOnError */, func() error {
		if p.dropped {
			return errDroppedProposal
		}

		resp := C.fwd_multi_commit_proposal(p.ptr)
		_, err := getHashKeyFromHashResult(resp)

		p.ptr = nil
		p.dropped = true

		return err
	})
}

// Dump returns a DOT (Graphviz) format representation of the trie structure
// of this proposal for debugging purposes.
func (p *MultiProposal) Dump() (string, error) {
	p.keepAliveHandle.mu.RLock()
	defer p.keepAliveHandle.mu.RUnlock()
	if p.ptr == nil {
		return "", errDroppedProposal
	}

	bytes, err := getValueFromValueResult(C.fwd_multi_proposal_dump(p.ptr))
	if err != nil {
		return "", err
	}
	return string(bytes), nil
}

// getMultiDatabaseFromHandleResult converts a C.MultiHandleResult to a MultiDatabase or error.
func getMultiDatabaseFromHandleResult(result C.MultiHandleResult) (*MultiDatabase, error) {
	switch result.tag {
	case C.MultiHandleResult_Ok:
		ptr := *(**C.MultiDatabaseHandle)(unsafe.Pointer(&result.anon0))
		db := &MultiDatabase{handle: ptr}
		return db, nil
	case C.MultiHandleResult_Err:
		err := newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
		return nil, err
	default:
		return nil, fmt.Errorf("unknown C.MultiHandleResult tag: %d", result.tag)
	}
}

// getMultiProposalFromResult converts a C.MultiProposalResult to a MultiProposal or error.
func getMultiProposalFromResult(result C.MultiProposalResult, wg *sync.WaitGroup) (*MultiProposal, error) {
	switch result.tag {
	case C.MultiProposalResult_NullHandlePointer:
		return nil, errDBClosed
	case C.MultiProposalResult_Ok:
		body := (*C.MultiProposalResult_Ok_Body)(unsafe.Pointer(&result.anon0))
		hashKey := *(*Hash)(unsafe.Pointer(&body.root_hash._0))
		proposal := &MultiProposal{
			handle: createHandle(body.handle, wg, func(p *C.MultiProposalHandle) C.VoidResult { return C.fwd_multi_free_proposal(p) }),
			root:   hashKey,
		}
		runtime.AddCleanup(proposal, drop[*C.MultiProposalHandle], proposal.handle)
		return proposal, nil
	case C.MultiProposalResult_Err:
		err := newOwnedBytes(*(*C.OwnedBytes)(unsafe.Pointer(&result.anon0))).intoError()
		return nil, err
	default:
		return nil, fmt.Errorf("unknown C.MultiProposalResult tag: %d", result.tag)
	}
}
