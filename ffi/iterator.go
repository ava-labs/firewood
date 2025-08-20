// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

package ffi

// #include <stdlib.h>
// #include "firewood.h"
import "C"

import (
	"encoding/binary"
	"runtime"
	"sync"
	"unsafe"
)

// IteratorBufferPool manages pre-allocated buffers for zero-copy iteration
type IteratorBufferPool struct {
	pool sync.Pool
}

// NewIteratorBufferPool creates a new buffer pool with default size buffers
func NewIteratorBufferPool(defaultSize int) *IteratorBufferPool {
	return &IteratorBufferPool{
		pool: sync.Pool{
			New: func() interface{} {
				// Create a C-allocated buffer that can be passed to Rust
				// We use C.malloc to ensure proper alignment and compatibility
				buf := C.malloc(C.size_t(defaultSize))
				if buf == nil {
					panic("failed to allocate buffer")
				}
				// Track buffer with Go runtime to prevent GC during use
				return &poolBuffer{
					ptr:  buf,
					size: defaultSize,
				}
			},
		},
	}
}

// poolBuffer wraps a C-allocated buffer
type poolBuffer struct {
	ptr  unsafe.Pointer
	size int
}

// Get retrieves a buffer from the pool
func (p *IteratorBufferPool) Get() *poolBuffer {
	return p.pool.Get().(*poolBuffer)
}

// Put returns a buffer to the pool
func (p *IteratorBufferPool) Put(buf *poolBuffer) {
	p.pool.Put(buf)
}

// Free releases all resources (should be called when pool is no longer needed)
func (p *IteratorBufferPool) Free() {
	// Note: In production, you might want to track all allocated buffers
	// and free them explicitly. For now, they'll be freed when process exits.
}

// Default buffer pool with 24MB buffers
var defaultBufferPool = NewIteratorBufferPool(1024 * 1024 * 24)

type DbIterator struct {
	dbHandle   *C.DatabaseHandle
	id         uint32
	currentKey []byte
	currentVal []byte
	err        error
}

// newIterator creates a new DbIterator from the given DatabaseHandle and Value.
// The Value must be returned from a Firewood FFI function.
// An error can only occur from parsing the Value.
func newIterator(handle *C.DatabaseHandle, val *C.struct_Value) (*DbIterator, error) {
	id, err := u32FromValue(val)
	if err != nil {
		return nil, err
	}

	return &DbIterator{
		dbHandle:   handle,
		id:         id,
		currentKey: nil,
		currentVal: nil,
		err:        nil,
	}, nil
}

func (it *DbIterator) Next() bool {
	v := C.fwd_iter_next(it.dbHandle, C.uint32_t(it.id))
	key, value, e := keyValueFromValue(&v)
	it.currentKey = key
	it.currentVal = value
	it.err = e
	if (key == nil && value == nil) || e != nil {
		return false
	}
	return true
}

func (it *DbIterator) Key() []byte {
	if (it.currentKey == nil && it.currentVal == nil) || it.err != nil {
		return nil
	}
	return it.currentKey
}

func (it *DbIterator) Value() []byte {
	if (it.currentKey == nil && it.currentVal == nil) || it.err != nil {
		return nil
	}
	return it.currentVal
}

func (it *DbIterator) Err() error {
	return it.err
}

// NextN returns up to n key-value pairs in a single FFI call.
// It returns (nil, nil, nil) when the iterator is exhausted.
func (it *DbIterator) NextN(n int) ([][]byte, [][]byte, error) {
	if n <= 0 {
		return nil, nil, nil
	}
	v := C.fwd_iter_next_n(it.dbHandle, C.uint32_t(it.id), C.size_t(n))
	buf, err := bytesFromValue(&v)
	if err != nil {
		return nil, nil, err
	}
	if len(buf) == 0 {
		// Exhausted
		return nil, nil, nil
	}

	if len(buf) < 8 {
		return nil, nil, errBadValue
	}
	count := int(binary.LittleEndian.Uint64(buf[:8]))
	pos := 8
	keys := make([][]byte, count)
	vals := make([][]byte, count)
	for i := 0; i < count; i++ {
		if pos+8 > len(buf) {
			return nil, nil, errBadValue
		}
		kvLen := int(binary.LittleEndian.Uint64(buf[pos : pos+8]))
		pos += 8
		if pos+kvLen > len(buf) {
			return nil, nil, errBadValue
		}
		kv := buf[pos : pos+kvLen]
		pos += kvLen

		// Parse inner packed KV: [key_len: u64][key][value]
		if len(kv) < 8 {
			return nil, nil, errBadValue
		}
		keyLen := int(binary.LittleEndian.Uint64(kv[:8]))
		if 8+keyLen > len(kv) {
			return nil, nil, errBadValue
		}
		keys[i] = kv[8 : 8+keyLen]
		vals[i] = kv[8+keyLen:]
	}

	return keys, vals, nil
}

// NextNFast returns up to n key-value pairs using a pre-allocated buffer pool for zero-copy performance.
// It returns (nil, nil, nil) when the iterator is exhausted.
// This method is more efficient than NextN for large batches as it avoids memory allocations.
func (it *DbIterator) NextNFast(n int) ([][]byte, [][]byte, error) {
	return it.NextNFastWithPool(n, defaultBufferPool)
}

// NextNFastWithPool returns up to n key-value pairs using a custom buffer pool.
// The pool parameter allows you to provide your own buffer pool with custom sizes.
func (it *DbIterator) NextNFastWithPool(n int, pool *IteratorBufferPool) ([][]byte, [][]byte, error) {
	if n <= 0 {
		return nil, nil, nil
	}

	// Get a buffer from the pool
	poolBuf := pool.Get()
	//defer pool.Put(poolBuf)

	// Pin the buffer memory to prevent GC from moving it
	runtime.KeepAlive(poolBuf)

	// Create BorrowedBytes struct for C
	//borrowedBytes := C.struct_BorrowedSlice_u8{
	//	ptr: (*C.uint8_t)(poolBuf.ptr),
	//	len: C.size_t(poolBuf.size),
	//}

	// Call the fast version that writes directly to our buffer
	v := C.fwd_iter_next_n_fast(it.dbHandle, C.uint32_t(it.id), C.size_t(n), (*C.uint8_t)(poolBuf.ptr), C.size_t(poolBuf.size))

	// Check for errors
	if v.data != nil {
		defer C.fwd_free_value(&v)
		return nil, nil, errorFromValue(&v)
	}

	// v.len contains the number of bytes written
	bytesWritten := int(v.len)
	if bytesWritten == 0 {
		// Iterator exhausted
		return nil, nil, nil
	}

	// Parse the data directly from the buffer
	buf := (*[1 << 30]byte)(poolBuf.ptr)[:bytesWritten:bytesWritten]

	if len(buf) < 8 {
		return nil, nil, errBadValue
	}

	count := int(binary.LittleEndian.Uint64(buf[:8]))
	pos := 8

	// Pre-allocate slices
	keys := make([][]byte, 0, count)
	vals := make([][]byte, 0, count)

	for i := 0; i < count; i++ {
		if pos+8 > len(buf) {
			return nil, nil, errBadValue
		}
		kvLen := int(binary.LittleEndian.Uint64(buf[pos : pos+8]))
		pos += 8
		if pos+kvLen > len(buf) {
			return nil, nil, errBadValue
		}
		kv := buf[pos : pos+kvLen]
		pos += kvLen

		// Parse inner packed KV: [key_len: u64][key][value]
		if len(kv) < 8 {
			return nil, nil, errBadValue
		}
		keyLen := int(binary.LittleEndian.Uint64(kv[:8]))
		if 8+keyLen > len(kv) {
			return nil, nil, errBadValue
		}

		// Copy the data since we're returning it to the user
		// and the buffer will be reused
		//keyCopy := make([]byte, keyLen)
		//copy(keyCopy, kv[8:8+keyLen])
		//
		//valCopy := make([]byte, len(kv)-8-keyLen)
		//copy(valCopy, kv[8+keyLen:])
		//
		//keys = append(keys, keyCopy)
		//vals = append(vals, valCopy)
		keys = append(keys, kv[8:8+keyLen])
		vals = append(vals, kv[8+keyLen:])
	}

	return keys, vals, nil
}

// NextNBuf returns up to n key-value pairs using a custom buffer pool.
// The pool parameter allows you to provide your own buffer pool with custom sizes.
func (it *DbIterator) NextNBuf(n int, par bool) ([][]byte, [][]byte, error) {
	return it.NextNBufWithPool(n, par, defaultBufferPool)
}

func (it *DbIterator) NextNBufWithPool(n int, par bool, pool *IteratorBufferPool) ([][]byte, [][]byte, error) {
	if n <= 0 {
		return nil, nil, nil
	}

	// Get a buffer from the pool
	poolBuf := pool.Get()
	//defer pool.Put(poolBuf) // Return to pool when done

	// Call the C function
	v := C.fwd_iter_next_n_buf(
		it.dbHandle,
		C.uint32_t(it.id),
		C.size_t(n),
		(*C.uint8_t)(poolBuf.ptr),
		C.size_t(poolBuf.size),
	)

	// Check for errors
	if v.data != nil {
		defer C.fwd_free_value(&v)
		return nil, nil, errorFromValue(&v)
	}

	bytesWritten := int(v.len)
	if bytesWritten == 0 {
		return nil, nil, nil // Iterator exhausted
	}

	// Create a single view of the buffer (avoid bounds checks)
	buf := unsafe.Slice((*byte)(poolBuf.ptr), bytesWritten)

	if len(buf) < 8 {
		return nil, nil, errBadValue
	}

	// Read count once
	count := int(binary.LittleEndian.Uint64(buf[:8]))

	// Pre-allocate with exact size
	keys := make([][]byte, count)
	vals := make([][]byte, count)

	// For small counts, use sequential processing
	if !par {
		parseSequential(buf, keys, vals, count)
	} else {
		// For large counts, use parallel processing
		parseParallel(buf, keys, vals, count)
	}

	return keys, vals, nil
}

// Sequential parsing - optimized for cache locality
func parseSequential(buf []byte, keys, vals [][]byte, count int) {
	// Process metadata in a single pass with better cache usage
	// Metadata starts at offset 8, each entry is 32 bytes (4 * 8)
	metadataPtr := unsafe.Pointer(&buf[8])

	for i := 0; i < count; i++ {
		// Direct memory access without bounds checks
		metadata := (*[4]uint64)(unsafe.Add(metadataPtr, i*32))

		keyOffset := metadata[0]
		keyLen := metadata[1]
		valueOffset := metadata[2]
		valueLen := metadata[3]

		// Create slices without copying
		keys[i] = buf[keyOffset : keyOffset+keyLen : keyOffset+keyLen]
		vals[i] = buf[valueOffset : valueOffset+valueLen : valueOffset+valueLen]
	}
}

// Parallel parsing for large datasets
func parseParallel(buf []byte, keys, vals [][]byte, count int) {
	// Determine optimal worker count
	numWorkers := runtime.NumCPU()
	//if count < numWorkers*16 {
	//	// Not worth parallelizing for small counts
	//	parseSequential(buf, keys, vals, count)
	//	return
	//}

	// Divide work among workers
	chunkSize := (count + numWorkers - 1) / numWorkers

	var wg sync.WaitGroup
	wg.Add(numWorkers)

	for w := 0; w < numWorkers; w++ {
		start := w * chunkSize
		end := start + chunkSize
		if end > count {
			end = count
		}

		go func(start, end int) {
			defer wg.Done()

			// Each worker processes its chunk
			metadataPtr := unsafe.Pointer(&buf[8+start*32])

			for i := start; i < end; i++ {
				idx := i - start
				metadata := (*[4]uint64)(unsafe.Add(metadataPtr, idx*32))

				keyOffset := metadata[0]
				keyLen := metadata[1]
				valueOffset := metadata[2]
				valueLen := metadata[3]

				keys[i] = buf[keyOffset : keyOffset+keyLen : keyOffset+keyLen]
				vals[i] = buf[valueOffset : valueOffset+valueLen : valueOffset+valueLen]
			}
		}(start, end)
	}

	wg.Wait()
}

// NextNBufWithPoolX returns up to n key-value pairs using a custom buffer pool.
// The pool parameter allows you to provide your own buffer pool with custom sizes.
func (it *DbIterator) NextNBufWithPoolX(n int, pool *IteratorBufferPool) ([][]byte, [][]byte, error) {
	if n <= 0 {
		return nil, nil, nil
	}

	// Get a buffer from the pool
	poolBuf := pool.Get()
	//defer pool.Put(poolBuf)

	// Pin the buffer memory to prevent GC from moving it
	runtime.KeepAlive(poolBuf)

	// Create BorrowedBytes struct for C
	//borrowedBytes := C.struct_BorrowedSlice_u8{
	//	ptr: (*C.uint8_t)(poolBuf.ptr),
	//	len: C.size_t(poolBuf.size),
	//}

	// Call the fast version that writes directly to our buffer
	v := C.fwd_iter_next_n_buf(it.dbHandle, C.uint32_t(it.id), C.size_t(n), (*C.uint8_t)(poolBuf.ptr), C.size_t(poolBuf.size))

	// Check for errors
	if v.data != nil {
		defer C.fwd_free_value(&v)
		return nil, nil, errorFromValue(&v)
	}

	// v.len contains the number of bytes written
	bytesWritten := int(v.len)
	if bytesWritten == 0 {
		// Iterator exhausted
		return nil, nil, nil
	}

	// Parse the data directly from the buffer
	buf := (*[1 << 30]byte)(poolBuf.ptr)[:bytesWritten:bytesWritten]

	if len(buf) < 8 {
		return nil, nil, errBadValue
	}

	count := int(binary.LittleEndian.Uint64(buf[:8]))

	// Pre-allocate slices
	keys := make([][]byte, 0, count)
	vals := make([][]byte, 0, count)

	for i := 0; i < count; i++ {
		pos := 8 + 4*8*i
		keyOffset := int(binary.LittleEndian.Uint64(buf[pos : pos+8]))
		pos += 8
		keyLen := int(binary.LittleEndian.Uint64(buf[pos : pos+8]))
		pos += 8
		valueOffset := int(binary.LittleEndian.Uint64(buf[pos : pos+8]))
		pos += 8
		valueLen := int(binary.LittleEndian.Uint64(buf[pos : pos+8]))
		//if pos+8 > len(buf) {
		//	return nil, nil, errBadValue
		//}
		keys = append(keys, buf[keyOffset:keyOffset+keyLen])
		vals = append(vals, buf[valueOffset:valueOffset+valueLen])
	}

	return keys, vals, nil
}

func (it *DbIterator) NextNZero(n int) ([][]byte, [][]byte, error) {
	if n <= 0 {
		return nil, nil, nil
	}

	// Call the fast version that writes directly to our buffer
	v := C.fwd_iter_next_n_zero(it.dbHandle, C.uint32_t(it.id), C.size_t(n))

	return kvPairsFromC(&v)
}

func (it *DbIterator) NextNZero2(n int) ([][]byte, [][]byte, error) {
	if n <= 0 {
		return nil, nil, nil
	}

	// Call the fast version that writes directly to our buffer
	v := C.fwd_iter_next_n_zero(it.dbHandle, C.uint32_t(it.id), C.size_t(n))

	return kvPairsFromC2(&v)
}

func (it *DbIterator) NextNNoRet(n int) (uint32, error) {
	if n <= 0 {
		return 0, nil
	}

	// Call the fast version that writes directly to our buffer
	v := C.fwd_iter_next_n_no_ret(it.dbHandle, C.uint32_t(it.id), C.size_t(n))
	x, e := u32FromValue(&v)
	if e != nil {
		return 0, e
	}
	return x, nil
}
