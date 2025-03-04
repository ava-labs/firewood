// Package firewood provides a Go wrapper around the [Firewood] database.
//
// [Firewood]: https://github.com/ava-labs/firewood
package firewood

// #cgo LDFLAGS: -L${SRCDIR}/../target/release -L/usr/local/lib -lfirewood_ffi -lm
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
	handle *C.void
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

	return &Database{handle: (*C.void)(db)}, nil
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

	pin := new(runtime.Pinner)
	defer pin.Unpin()

	ffiOps := make([]C.struct_KeyValue, len(ops))
	for i, op := range ops {
		ffiOps[i] = C.struct_KeyValue{
			key:   makeValue(pin, op.Key),
			value: makeValue(pin, op.Value),
		}
	}
	ptr := (*C.struct_KeyValue)(unsafe.Pointer(&ffiOps[0]))
	hash := C.fwd_batch(unsafe.Pointer(db.handle), C.size_t(len(ops)), ptr)
	hashBytes := C.GoBytes(unsafe.Pointer(hash.data), C.int(hash.len))
	C.fwd_free_value(&hash)
	return hashBytes
}

// Get retrieves the value for the given key. It always returns a nil error.
func (db *Database) Get(key []byte) ([]byte, error) {
	pin := new(runtime.Pinner)
	defer pin.Unpin()
	ffiKey := makeValue(pin, key)

	value := C.fwd_get(unsafe.Pointer(db.handle), ffiKey)
	ffiBytes := C.GoBytes(unsafe.Pointer(value.data), C.int(value.len))
	C.fwd_free_value(&value)
	if len(ffiBytes) == 0 {
		return nil, nil
	}
	return ffiBytes, nil
}

func makeValue(pin *runtime.Pinner, data []byte) C.struct_Value {
	if len(data) == 0 {
		return C.struct_Value{0, nil}
	}
	ptr := (*C.uchar)(unsafe.Pointer(&data[0]))
	pin.Pin(ptr)
	return C.struct_Value{C.size_t(len(data)), ptr}
}

// Root returns the current root hash of the trie.
func (db *Database) Root() []byte {
	hash := C.fwd_root_hash(unsafe.Pointer(db.handle))
	hashBytes := C.GoBytes(unsafe.Pointer(hash.data), C.int(hash.len))
	C.fwd_free_value(&hash)
	return hashBytes
}

// Close closes the database and releases all held resources. It always returns
// nil.
func (db *Database) Close() error {
	C.fwd_close_db(unsafe.Pointer(db.handle))
	return nil
}
