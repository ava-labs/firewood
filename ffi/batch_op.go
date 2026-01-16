// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include "firewood.h"
import "C"

import "fmt"

// batchOp represents a single batch operation to be applied to the database.
type batchOp struct {
	key   []byte
	value []byte
	op    C.enum_BatchOpTag
}

// Batch accumulates database operations to be applied atomically.
// Create a new Batch with [NewBatch], add operations with [Batch.Put],
// [Batch.Delete], and [Batch.PrefixDelete], then apply with
// [Database.UpdateBatch] or [Database.ProposeBatch].
type Batch struct {
	ops []batchOp
}

// NewBatch creates a new empty Batch.
func NewBatch() *Batch {
	return &Batch{}
}

// Put adds an operation to insert or update a key with a value.
// The value may be empty (zero-length) to store an empty value.
func (b *Batch) Put(key, value []byte) {
	b.ops = append(b.ops, batchOp{
		key:   key,
		value: value,
		op:    C.BatchOpTag_Put,
	})
}

// Delete adds an operation to delete a specific key.
func (b *Batch) Delete(key []byte) {
	b.ops = append(b.ops, batchOp{
		key:   key,
		value: nil,
		op:    C.BatchOpTag_Delete,
	})
}

// PrefixDelete adds an operation to delete all keys with the given prefix.
func (b *Batch) PrefixDelete(prefix []byte) {
	b.ops = append(b.ops, batchOp{
		key:   prefix,
		value: nil,
		op:    C.BatchOpTag_DeleteRange,
	})
}

// Len returns the number of operations in the batch.
func (b *Batch) Len() int {
	return len(b.ops)
}

// batchFromKeyValues converts parallel slices of keys and values into a Batch.
// This maintains backward compatibility with the old API:
//   - nil value: PrefixDelete operation using the key as a prefix
//   - non-nil value (including empty): Put operation
func batchFromKeyValues(keys, vals [][]byte) (*Batch, error) {
	if len(keys) != len(vals) {
		return nil, fmt.Errorf("%w: %d != %d", errKeysAndValues, len(keys), len(vals))
	}

	batch := &Batch{ops: make([]batchOp, len(keys))}
	for i := range keys {
		if vals[i] == nil {
			batch.ops[i] = batchOp{
				key:   keys[i],
				value: nil,
				op:    C.BatchOpTag_DeleteRange,
			}
		} else {
			batch.ops[i] = batchOp{
				key:   keys[i],
				value: vals[i],
				op:    C.BatchOpTag_Put,
			}
		}
	}
	return batch, nil
}
