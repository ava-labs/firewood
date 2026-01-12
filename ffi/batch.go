// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

type Batch struct {
	keys   [][]byte
	values [][]byte
}

func NewBatch() *Batch {
	return &Batch{
		keys:   make([][]byte, 0),
		values: make([][]byte, 0),
	}
}

func (b *Batch) Put(key, value []byte) {
	b.keys = append(b.keys, key)
	if len(value) == 0 {
		value = []byte{}
	}
	b.values = append(b.values, value)
}

func (b *Batch) PrefixDelete(prefix []byte) {
	b.keys = append(b.keys, prefix)
	b.values = append(b.values, nil)
}
