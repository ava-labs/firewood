// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include "firewood.h"
import "C"

import "fmt"

// BatchOp represents a batch operation to be applied to the database.
// Use Put, Delete, or DeleteRange to create a BatchOp.
type BatchOp struct {
	key   []byte
	value []byte
	op    C.enum_BatchOpTag
}

// Put creates a BatchOp that inserts or updates a key with a value.
// The value may be empty (zero-length) to store an empty value.
func Put(key, value []byte) BatchOp {
	return BatchOp{
		key:   key,
		value: value,
		op:    C.BatchOpTag_Put,
	}
}

// Delete creates a BatchOp that deletes a specific key.
func Delete(key []byte) BatchOp {
	return BatchOp{
		key:   key,
		value: nil,
		op:    C.BatchOpTag_Delete,
	}
}

// DeleteRange creates a BatchOp that deletes all keys with the given prefix.
func DeleteRange(prefix []byte) BatchOp {
	return BatchOp{
		key:   prefix,
		value: nil,
		op:    C.BatchOpTag_DeleteRange,
	}
}

// batchOpsFromKeyValues converts parallel slices of keys and values into BatchOps.
// This maintains backward compatibility with the old API:
//   - nil value: DeleteRange operation using the key as a prefix
//   - non-nil value (including empty): Put operation
func batchOpsFromKeyValues(keys, vals [][]byte) ([]BatchOp, error) {
	if len(keys) != len(vals) {
		return nil, fmt.Errorf("%w: %d != %d", errKeysAndValues, len(keys), len(vals))
	}

	ops := make([]BatchOp, len(keys))
	for i := range keys {
		if vals[i] == nil {
			ops[i] = DeleteRange(keys[i])
		} else {
			ops[i] = Put(keys[i], vals[i])
		}
	}
	return ops, nil
}
