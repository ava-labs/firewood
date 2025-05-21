// Package firewood provides a Go wrapper around the [Firewood] database.
//
// [Firewood]: https://github.com/ava-labs/firewood
package firewood

// // Note that -lm is required on Linux but not on Mac.
// #cgo LDFLAGS: -L${SRCDIR}/../target/release -L/usr/local/lib -lfirewood_ffi -lm
// #include <stdlib.h>
// #include "firewood.h"
import "C"

import (
	"errors"
	"runtime"
	"unsafe"
)

var errCommittedProposal = errors.New("proposal already committed")

type Proposal struct {
	// handle is returned and accepted by cgo functions. It MUST be treated as
	// an opaque value without special meaning.
	// https://en.wikipedia.org/wiki/Blinkenlights
	handle *C.DatabaseHandle

	// The proposal ID.
	// id = 0 is reserved for a dropped proposal.
	id uint32
}

func newProposal(handle *C.DatabaseHandle, id uint32) *Proposal {
	if handle == nil {
		return nil
	}

	p := &Proposal{
		handle: handle,
		id:     id,
	}

	// To avoid require the consumer to explicitly deallocate the proposal,
	// we will use a cleanup function to free the proposal when it is no longer needed.
	// Note that runtime.AddCleanup isn't available in this version of Go, but should be replaced eventually.
	runtime.SetFinalizer(p, func(p *Proposal) {
		if p.handle != nil {
			C.fwd_drop_proposal(p.handle, C.uint32_t(p.id))
		}
	})
	return p
}

// Get retrieves the value for the given key.
// If the key does not exist, it returns (nil, nil).
func (p *Proposal) Get(key []byte) ([]byte, error) {
	if p.handle == nil {
		return nil, errDBClosed
	}

	if p.id == 0 {
		return nil, errCommittedProposal
	}
	values, cleanup := newValueFactory()
	defer cleanup()

	// Get the value for the given key.
	val := C.fwd_get_from_proposal(p.handle, C.uint32_t(p.id), values.from(key))
	return extractBytesThenFree(&val)
}

// Propose creates a new proposal with the given keys and values.
// The proposal is not committed until Commit is called.
func (p *Proposal) Propose(keys, vals [][]byte) (*Proposal, error) {
	if p.handle == nil {
		return nil, errDBClosed
	}

	if p.id == 0 {
		return nil, errCommittedProposal
	}

	ops := make([]KeyValue, len(keys))
	for i := range keys {
		ops[i] = KeyValue{keys[i], vals[i]}
	}

	values, cleanup := newValueFactory()
	defer cleanup()

	ffiOps := make([]C.struct_KeyValue, len(ops))
	for i, op := range ops {
		ffiOps[i] = C.struct_KeyValue{
			key:   values.from(op.Key),
			value: values.from(op.Value),
		}
	}

	// Propose the keys and values.
	val := C.fwd_propose_on_proposal(p.handle, C.uint32_t(p.id),
		C.size_t(len(ffiOps)),
		unsafe.SliceData(ffiOps),
	)
	id, err := extractUintThenFree(&val)
	if err != nil {
		return nil, err
	}

	return newProposal(p.handle, id), nil
}

// Commit commits the proposal and returns any errors.
// If an error occurs, the proposal is dropped and no longer valid.
func (p *Proposal) Commit() error {
	if p.handle == nil {
		return errDBClosed
	}

	if p.id == 0 {
		return errCommittedProposal
	}

	// Commit the proposal and return the hash.
	errVal := C.fwd_commit(p.handle, C.uint32_t(p.id))
	// Even if there was an error, it is now impossible to attempt to recommit the proposal.
	p.id = 0
	return extractErrorThenFree(&errVal)
}
