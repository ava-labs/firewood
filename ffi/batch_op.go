// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include "firewood.h"
import "C"

// BatchOp represents a single batch operation to be applied to the database.
// Create BatchOp values using the [Put], [Delete], and [PrefixDelete] functions.
type BatchOp struct {
	tag   C.BatchOp_Tag
	key   []byte // key for Put/Delete, prefix for DeleteRange
	value []byte // only used for Put
}

// Put creates a BatchOp that inserts or updates a key with a value.
// The value may be empty (zero-length) to store an empty value.
func Put(key, value []byte) BatchOp {
	return BatchOp{
		tag:   C.BatchOp_Put,
		key:   key,
		value: value,
	}
}

// Delete creates a BatchOp that deletes a specific key.
func Delete(key []byte) BatchOp {
	return BatchOp{
		tag: C.BatchOp_Delete,
		key: key,
	}
}

// PrefixDelete creates a BatchOp that deletes all keys with the given prefix.
func PrefixDelete(prefix []byte) BatchOp {
	return BatchOp{
		tag: C.BatchOp_DeleteRange,
		key: prefix, // stored in key field; C union has prefix at same offset
	}
}

// IsPut returns true if this is a Put operation.
func (op BatchOp) IsPut() bool {
	return op.tag == C.BatchOp_Put
}

// IsDelete returns true if this is a Delete operation.
func (op BatchOp) IsDelete() bool {
	return op.tag == C.BatchOp_Delete
}

// IsPrefixDelete returns true if this is a PrefixDelete operation.
func (op BatchOp) IsPrefixDelete() bool {
	return op.tag == C.BatchOp_DeleteRange
}

// Key returns the key for Put/Delete operations, or the prefix for
// PrefixDelete operations.
func (op BatchOp) Key() []byte {
	return op.key
}

// Value returns the value for Put operations. Returns nil for Delete and
// PrefixDelete operations.
func (op BatchOp) Value() []byte {
	if op.tag != C.BatchOp_Put {
		return nil
	}
	return op.value
}
