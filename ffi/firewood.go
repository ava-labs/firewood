// Package firewood provides a Go wrapper around the [Firewood] database.
//
// [Firewood]: https://github.com/ava-labs/firewood
package firewood

// #cgo LDFLAGS: -L${SRCDIR}/../target/release -lfirewood_ffi
// #include "firewood.h"
// #include <stdlib.h>
import "C"

import (
	"fmt"
	"runtime"
	"unsafe"
)

// A Database is a handle to a Firewood database.
type Database struct {
	// handle is returned and accepted by cgo functions. It MUST be treated as
	// an opaque value without special meaning.
	// https://en.wikipedia.org/wiki/Blinkenlights
	handle unsafe.Pointer
}

// Config configures the opening of a [Database].
type Config struct {
	Path              string
	Create            bool
	NodeCacheEntries  uint
	Revisions         uint
	ReadCacheStrategy CacheStrategy
	MetricsPort       uint16
}

// DefaultConfig returns a sensible default Config.
func DefaultConfig() *Config {
	return &Config{
		NodeCacheEntries:  1_000_000,
		Revisions:         100,
		ReadCacheStrategy: OnlyCacheWrites,
		Path:              "firewood.db",
		MetricsPort:       3000,
	}
}

// A CacheStrategy represents the caching strategy used by a [Database].
type CacheStrategy uint8

const (
	OnlyCacheWrites CacheStrategy = iota
	CacheBranchReads
	CacheAllReads

	// invalidCacheStrategy MUST be the final value in the iota block to make it
	// the smallest value greater than all valid values.
	invalidCacheStrategy
)

// New opens or creates a new Firewood database with the given configuration.
func New(opts *Config) (*Database, error) {
	if opts.ReadCacheStrategy >= invalidCacheStrategy {
		return nil, fmt.Errorf("invalid %T (%[1]d)", opts.ReadCacheStrategy)
	}
	if opts.Revisions < 2 {
		return nil, fmt.Errorf("%T.Revisions must be >= 2", opts)
	}
	if opts.NodeCacheEntries < 1 {
		return nil, fmt.Errorf("%T.NodeCacheEntries must be >= 1", opts)
	}

	var db unsafe.Pointer
	if opts.Create {
		db = C.fwd_create_db(C.CString(opts.Path), C.size_t(opts.NodeCacheEntries), C.size_t(opts.Revisions), C.uint8_t(opts.ReadCacheStrategy), C.uint16_t(opts.MetricsPort))
	} else {
		db = C.fwd_open_db(C.CString(opts.Path), C.size_t(opts.NodeCacheEntries), C.size_t(opts.Revisions), C.uint8_t(opts.ReadCacheStrategy), C.uint16_t(opts.MetricsPort))
	}
	return &Database{handle: db}, nil
}

// KeyValue is a key-value pair.
type KeyValue struct {
	Key   []byte
	Value []byte
}

// Batch applies a batch of updates to the database, returning the hash of the
// root node after the batch is applied.
//
// NOTE that if the `Value` is empty, the respective `Key` will be deleted as a
// prefix deletion; i.e. all children will be deleted.
//
// WARNING: a consequence of prefix deletion is that calling Batch with an empty
// key and value will delete the entire database.
func (db *Database) Batch(ops []KeyValue) []byte {
	// TODO(arr4n) refactor this to require explicit signalling from the caller
	// that they want prefix deletion, similar to `rm --no-preserve-root`.

	values, cleanup := newValueFactory()
	defer cleanup()

	ffiOps := make([]C.struct_KeyValue, len(ops))
	for i, op := range ops {
		ffiOps[i] = C.struct_KeyValue{
			key:   values.from(op.Key),
			value: values.from(op.Value),
		}
	}
	ptr := (*C.struct_KeyValue)(unsafe.SliceData(ffiOps))
	hash := C.fwd_batch(db.handle, C.size_t(len(ops)), ptr)
	hashBytes := C.GoBytes(unsafe.Pointer(hash.data), C.int(hash.len))
	C.fwd_free_value(&hash)
	return hashBytes
}

// Get retrieves the value for the given key. It always returns a nil error.
func (db *Database) Get(key []byte) ([]byte, error) {
	values, cleanup := newValueFactory()
	defer cleanup()
	ffiKey := values.from(key)

	value := C.fwd_get(db.handle, ffiKey)
	ffiBytes := C.GoBytes(unsafe.Pointer(value.data), C.int(value.len))
	C.fwd_free_value(&value)
	if len(ffiBytes) == 0 {
		return nil, nil
	}
	return ffiBytes, nil
}

// Root returns the current root hash of the trie.
func (db *Database) Root() []byte {
	hash := C.fwd_root_hash(db.handle)
	hashBytes := C.GoBytes(unsafe.Pointer(hash.data), C.int(hash.len))
	C.fwd_free_value(&hash)
	return hashBytes
}

// Close closes the database and releases all held resources. It always returns
// nil.
func (db *Database) Close() error {
	C.fwd_close_db(db.handle)
	return nil
}

// newValueFactory returns a factory for converting byte slices into cgo Value
// structs that can be passed as arguments to cgo functions. The returned
// cleanup function MUST be called when the constructed values are no longer
// required, after which they can no longer be used as cgo arguments.
func newValueFactory() (*valueFactory, func()) {
	f := new(valueFactory)
	return f, func() { f.pin.Unpin() }
}

type valueFactory struct {
	pin runtime.Pinner
}

func (f *valueFactory) from(data []byte) C.struct_Value {
	if len(data) == 0 {
		return C.struct_Value{0, nil}
	}
	ptr := (*C.uchar)(unsafe.SliceData(data))
	f.pin.Pin(ptr)
	return C.struct_Value{C.size_t(len(data)), ptr}
}
