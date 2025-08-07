// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// Package ffi provides a Go wrapper around the [Firewood] database.
//
// [Firewood]: https://github.com/ava-labs/firewood
package ffi

//go:generate go run generate_cgo.go

// // Note that -lm is required on Linux but not on Mac.
// // FIREWOOD_CGO_BEGIN_STATIC_LIBS
// // #cgo linux,amd64 LDFLAGS: -L${SRCDIR}/libs/x86_64-unknown-linux-gnu
// // #cgo linux,arm64 LDFLAGS: -L${SRCDIR}/libs/aarch64-unknown-linux-gnu
// // #cgo darwin,amd64 LDFLAGS: -L${SRCDIR}/libs/x86_64-apple-darwin
// // #cgo darwin,arm64 LDFLAGS: -L${SRCDIR}/libs/aarch64-apple-darwin
// // FIREWOOD_CGO_END_STATIC_LIBS
// // FIREWOOD_CGO_BEGIN_LOCAL_LIBS
// #cgo LDFLAGS: -L${SRCDIR}/../target/debug
// #cgo LDFLAGS: -L${SRCDIR}/../target/release
// #cgo LDFLAGS: -L${SRCDIR}/../target/maxperf
// // FIREWOOD_CGO_END_LOCAL_LIBS
// #cgo LDFLAGS: -lfirewood_ffi -lm
// #include <stdlib.h>
// #include "firewood.h"
import "C"

import (
	"bytes"
	"errors"
	"fmt"
	"runtime"
)

// These constants are used to identify errors returned by the Firewood Rust FFI.
// These must be changed if the Rust FFI changes - should be reported by tests.
const (
	RootLength = 32
)

var (
	errDBClosed = errors.New("firewood database already closed")
	EmptyRoot   = make([]byte, RootLength)
)

// A Database is a handle to a Firewood database.
// It is not safe to call these methods with a nil handle.
type Database struct {
	// handle is returned and accepted by cgo functions. It MUST be treated as
	// an opaque value without special meaning.
	// https://en.wikipedia.org/wiki/Blinkenlights
	handle *C.DatabaseHandle
}

// Config configures the opening of a [Database].
type Config struct {
	Truncate             bool
	NodeCacheEntries     uint
	FreeListCacheEntries uint
	Revisions            uint
	ReadCacheStrategy    CacheStrategy
}

// DefaultConfig returns a sensible default Config.
func DefaultConfig() *Config {
	return &Config{
		NodeCacheEntries:     1_000_000,
		FreeListCacheEntries: 40_000,
		Revisions:            100,
		ReadCacheStrategy:    OnlyCacheWrites,
	}
}

// A CacheStrategy represents the caching strategy used by a [Database].
type CacheStrategy C.CacheReadStrategy

const (
	OnlyCacheWrites  CacheStrategy = C.CacheReadStrategy_WritesOnly
	CacheBranchReads CacheStrategy = C.CacheReadStrategy_BranchReads
	CacheAllReads    CacheStrategy = C.CacheReadStrategy_All

	invalidCacheStrategy = CacheAllReads + 1
)

// New opens or creates a new Firewood database with the given configuration. If
// a nil `Config` is provided [DefaultConfig] will be used instead.
func New(filePath string, conf *Config) (*Database, error) {
	if conf == nil {
		conf = DefaultConfig()
	}
	if conf.ReadCacheStrategy >= invalidCacheStrategy {
		return nil, fmt.Errorf("invalid %T (%[1]d)", conf.ReadCacheStrategy)
	}
	if conf.Revisions < 2 {
		return nil, fmt.Errorf("%T.Revisions must be >= 2", conf)
	}
	if conf.NodeCacheEntries < 1 {
		return nil, fmt.Errorf("%T.NodeCacheEntries must be >= 1", conf)
	}
	if conf.FreeListCacheEntries < 1 {
		return nil, fmt.Errorf("%T.FreeListCacheEntries must be >= 1", conf)
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	args := C.struct_DatabaseHandleArgs{
		path:                 newBorrowedBytes([]byte(filePath), &pinner),
		cache_size:           C.size_t(conf.NodeCacheEntries),
		free_list_cache_size: C.size_t(conf.FreeListCacheEntries),
		revisions:            C.size_t(conf.Revisions),
		strategy:             C.CacheReadStrategy(conf.ReadCacheStrategy),
		truncate:             C.bool(conf.Truncate),
	}

	return fromHandleResult(C.fwd_open_db(args))
}

// Update applies a batch of updates to the database, returning the hash of the
// root node after the batch is applied.
//
// WARNING: a consequence of prefix deletion is that calling Update with an empty
// key and value will delete the entire database.
func (db *Database) Update(keys, vals [][]byte) ([]byte, error) {
	if db.handle == nil {
		return nil, errDBClosed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	kvp, err := newKeyValuePairs(keys, vals, &pinner)
	if err != nil {
		return nil, err
	}

	return fromHashResult(C.fwd_batch(db.handle, kvp))
}

func (db *Database) Propose(keys, vals [][]byte) (*Proposal, error) {
	if db.handle == nil {
		return nil, errDBClosed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	kvp, err := newKeyValuePairs(keys, vals, &pinner)
	if err != nil {
		return nil, err
	}

	return fromProposalResult(C.fwd_propose_on_db(db.handle, kvp), db)
}

// Get retrieves the value for the given key. It always returns a nil error.
// If the key is not found, the return value will be (nil, nil).
func (db *Database) Get(key []byte) ([]byte, error) {
	if db.handle == nil {
		return nil, errDBClosed
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	val, err := fromValueResult(C.fwd_get_latest(db.handle, newBorrowedBytes(key, &pinner)))
	if errors.Is(err, errRevisionNotFound) {
		return nil, nil
	}

	return val, err
}

// GetFromRoot retrieves the value for the given key from a specific root hash.
// If the root is not found, it returnas an error.
// If key is not found, it returns (nil, nil).
func (db *Database) GetFromRoot(root, key []byte) ([]byte, error) {
	if db.handle == nil {
		return nil, errDBClosed
	}

	if len(root) == 0 || bytes.Equal(root, EmptyRoot) {
		return nil, nil // Empty root is treated as no data
	}

	if len(root) != RootLength {
		return nil, errInvalidRootLength
	}

	rootKey, err := NewHashKey(root)
	if err != nil {
		return nil, fmt.Errorf("invalid root hash: %w", err)
	}

	var pinner runtime.Pinner
	defer pinner.Unpin()

	return fromValueResult(C.fwd_get_from_root(
		db.handle,
		rootKey.toC(),
		newBorrowedBytes(key, &pinner),
	))
}

// Root returns the current root hash of the trie.
// Empty trie must return common.Hash{}.
func (db *Database) Root() ([]byte, error) {
	if db.handle == nil {
		return nil, errDBClosed
	}

	hash, err := fromHashResult(C.fwd_root_hash(db.handle))
	if err != nil {
		return nil, err
	}

	// If the root hash is not found, return a zeroed slice.
	if len(hash) == 0 {
		return EmptyRoot, nil
	}

	return hash, nil
}

// Revision returns a historical revision of the database.
func (db *Database) Revision(root []byte) (*Revision, error) {
	if root == nil || len(root) != RootLength {
		return nil, errInvalidRootLength
	}

	// Attempt to get any value from the root.
	// This will verify that the root is valid and accessible.
	// If the root is not valid, this will return an error.
	_, err := db.GetFromRoot(root, []byte{})
	if err != nil {
		return nil, err
	}

	return &Revision{database: db, root: root}, nil
}

// Close closes the database and releases all held resources.
// Returns an error if already closed.
func (db *Database) Close() error {
	return db.free(true)
}
