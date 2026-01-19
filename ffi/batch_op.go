// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include "firewood.h"
import "C"

import "fmt"

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

// batchFromKeyValues converts parallel slices of keys and values into a []BatchOp.
// This maintains backward compatibility with the old API:
//   - nil value: PrefixDelete operation using the key as a prefix
//   - non-nil value (including empty): Put operation
func batchFromKeyValues(keys, vals [][]byte) ([]BatchOp, error) {
	if len(keys) != len(vals) {
		return nil, fmt.Errorf("%w: %d != %d", errKeysAndValues, len(keys), len(vals))
	}

	batch := make([]BatchOp, len(keys))
	for i := range keys {
		if vals[i] == nil {
			batch[i] = PrefixDelete(keys[i])
		} else {
			batch[i] = Put(keys[i], vals[i])
		}
	}
	return batch, nil
}
