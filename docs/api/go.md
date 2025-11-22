# Firewood Go API Reference (FFI)

This document provides a comprehensive reference for the Firewood Go API, which uses CGO to interface with the Firewood Rust library. The API is exposed through C-compatible FFI functions.

## Table of Contents

- [Installation and Setup](#installation-and-setup)
- [Memory Management](#memory-management)
- [Error Handling](#error-handling)
- [Database Operations](#database-operations)
  - [Opening and Closing](#opening-and-closing)
  - [Configuration](#configuration)
- [Key-Value Operations](#key-value-operations)
  - [Reading Data](#reading-data)
  - [Writing Data (Batch Operations)](#writing-data-batch-operations)
- [Proposals](#proposals)
- [Revisions](#revisions)
- [Iterators](#iterators)
- [Proof Operations](#proof-operations)
  - [Range Proofs](#range-proofs)
  - [Change Proofs](#change-proofs)
- [Metrics and Logging](#metrics-and-logging)
- [Type Reference](#type-reference)
- [Examples](#examples)

## Installation and Setup

### Local Development

To use Firewood in your Go project during development:

```bash
# Build Firewood first
cargo build --profile maxperf

# In your Go project
go mod edit -replace github.com/ava-labs/firewood-go-ethhash/ffi=/path/to/firewood/ffi
go mod tidy
```

### Production Use

For production, use the published FFI package:

```bash
go get github.com/ava-labs/firewood-go-ethhash/ffi
```

The package includes pre-built static libraries for supported architectures:
- x86_64-unknown-linux-gnu
- aarch64-unknown-linux-gnu
- aarch64-apple-darwin
- x86_64-apple-darwin

## Memory Management

The Firewood Go API requires careful memory management because it crosses the Go-Rust boundary via CGO.

### Rules

1. **Owned Memory**: Data returned from FFI functions (e.g., `OwnedBytes`) must be explicitly freed
2. **Borrowed Memory**: Data passed to FFI functions is borrowed and should not be freed by the caller during the call
3. **Handle Lifecycle**: Handles (database, proposal, revision, iterator) must be freed when no longer needed

### Free Functions

Each type that allocates memory has a corresponding `fwd_free_*` function:

- `fwd_free_owned_bytes(bytes OwnedBytes)` - Free byte array
- `fwd_free_owned_kv_pair(kv OwnedKeyValuePair)` - Free key-value pair
- `fwd_free_owned_key_value_batch(batch OwnedKeyValueBatch)` - Free batch of key-value pairs
- `fwd_close_db(db *DatabaseHandle)` - Close database handle
- `fwd_free_proposal(proposal *ProposalHandle)` - Free proposal handle
- `fwd_free_revision(revision *RevisionHandle)` - Free revision handle
- `fwd_free_iterator(iterator *IteratorHandle)` - Free iterator handle
- `fwd_free_range_proof(proof *RangeProofContext)` - Free range proof
- `fwd_free_change_proof(proof *ChangeProofContext)` - Free change proof

### Example

```go
// Open database
args := ffi.DatabaseHandleArgs{
    Path: ffi.NewBorrowedBytes([]byte("/path/to/db")),
    CacheSize: 10000,
    FreeListCacheSize: 1024,
    Revisions: 128,
    Strategy: 2, // Cache all reads
    Truncate: false,
}
result := ffi.FwdOpenDb(args)
if result.Tag != ffi.HandleResult_Some {
    // Handle error
}
db := result.Db

// Use database...

// Clean up
ffi.FwdCloseDb(db)
```

## Error Handling

Most FFI functions return result types that indicate success or various error conditions.

### Common Result Types

- `VoidResult` - Operation with no return value
- `HandleResult` - Returns a database handle
- `HashResult` - Returns a hash key
- `ValueResult` - Returns byte data
- `ProposalResult` - Returns a proposal handle
- `RevisionResult` - Returns a revision handle
- `IteratorResult` - Returns an iterator handle
- `KeyValueResult` - Returns a key-value pair
- `KeyValueBatchResult` - Returns multiple key-value pairs
- `RangeProofResult` - Returns a range proof
- `ChangeProofResult` - Returns a change proof

### Result Tags

Each result type has a `Tag` field that indicates the outcome:

```go
// Example: HandleResult tags
const (
    HandleResult_NullHandlePointer // Invalid input pointer
    HandleResult_Some              // Success
    HandleResult_Err               // Error occurred
)
```

### Error Pattern

```go
result := ffi.SomeOperation(...)
switch result.Tag {
case ffi.ResultType_Some:
    // Success - use result.Data
case ffi.ResultType_None:
    // Not found or empty result
case ffi.ResultType_NullHandlePointer:
    // Invalid handle provided
case ffi.ResultType_Err:
    // Error occurred
    errorMsg := string(result.Err.ToSlice())
    log.Printf("Error: %s", errorMsg)
    ffi.FwdFreeOwnedBytes(result.Err)
default:
    // Unexpected result
}
```

## Database Operations

### Opening and Closing

**Open Database:**

```c
struct HandleResult fwd_open_db(struct DatabaseHandleArgs args);
```

```go
args := ffi.DatabaseHandleArgs{
    Path: ffi.NewBorrowedBytes([]byte("/path/to/db")),
    CacheSize: 10000,
    FreeListCacheSize: 1024,
    Revisions: 128,
    Strategy: 2,
    Truncate: false,
}
result := ffi.FwdOpenDb(args)
if result.Tag != ffi.HandleResult_Some {
    // Handle error
}
db := result.Db
```

**Close Database:**

```c
struct VoidResult fwd_close_db(struct DatabaseHandle *db);
```

```go
result := ffi.FwdCloseDb(db)
if result.Tag != ffi.VoidResult_Ok {
    // Handle error
}
```

### Configuration

**`DatabaseHandleArgs`** - Configuration for opening a database:

- `Path: BorrowedBytes` - Path to the database file (must be valid UTF-8)
- `CacheSize: usize` - Node cache size (must be non-zero)
- `FreeListCacheSize: usize` - Free list cache size (must be non-zero)
- `Revisions: usize` - Maximum number of revisions to keep
- `Strategy: u8` - Cache read strategy:
  - `0` - No cache (WritesOnly)
  - `1` - Cache branch reads only
  - `2` - Cache all reads
- `Truncate: bool` - Whether to reset the database on open

## Key-Value Operations

### Reading Data

**Get Latest Value:**

```c
struct ValueResult fwd_get_latest(const struct DatabaseHandle *db, BorrowedBytes key);
```

```go
key := []byte("my_key")
result := ffi.FwdGetLatest(db, ffi.NewBorrowedBytes(key))
switch result.Tag {
case ffi.ValueResult_Some:
    value := result.Value.ToSlice()
    // Use value
    ffi.FwdFreeOwnedBytes(result.Value)
case ffi.ValueResult_None:
    // Key not found
case ffi.ValueResult_Err:
    errorMsg := string(result.Err.ToSlice())
    log.Printf("Error: %s", errorMsg)
    ffi.FwdFreeOwnedBytes(result.Err)
}
```

**Get Value from Specific Root:**

```c
struct ValueResult fwd_get_from_root(const struct DatabaseHandle *db,
                                      BorrowedBytes root,
                                      BorrowedBytes key);
```

```go
result := ffi.FwdGetFromRoot(db, ffi.NewBorrowedBytes(rootHash[:]), ffi.NewBorrowedBytes(key))
// Handle result as above
```

**Get Root Hash:**

```c
struct HashResult fwd_root_hash(const struct DatabaseHandle *db);
```

```go
result := ffi.FwdRootHash(db)
switch result.Tag {
case ffi.HashResult_Some:
    rootHash := result.Hash
    // Use rootHash
case ffi.HashResult_None:
    // Database is empty
case ffi.HashResult_Err:
    // Handle error
}
```

### Writing Data (Batch Operations)

Writes are performed via batch operations. A batch is a collection of Put, Delete, or DeleteRange operations.

**Batch Operation:**

```c
struct HashResult fwd_batch(const struct DatabaseHandle *db, BorrowedKeyValuePairs values);
```

```go
import "C"

// Prepare key-value pairs
kvPairs := []ffi.KeyValuePair{
    {
        Key: ffi.NewBorrowedBytes([]byte("key1")),
        Value: ffi.NewBorrowedBytes([]byte("value1")),
        Op: ffi.OpType_Put,
    },
    {
        Key: ffi.NewBorrowedBytes([]byte("key2")),
        Value: ffi.NewBorrowedBytes([]byte("value2")),
        Op: ffi.OpType_Put,
    },
    {
        Key: ffi.NewBorrowedBytes([]byte("old_key")),
        Op: ffi.OpType_Delete,
    },
}

// Create borrowed slice
batchData := ffi.BorrowedKeyValuePairs{
    Ptr: (*ffi.KeyValuePair)(unsafe.Pointer(&kvPairs[0])),
    Len: C.size_t(len(kvPairs)),
}

// Execute batch
result := ffi.FwdBatch(db, batchData)
switch result.Tag {
case ffi.HashResult_Some:
    newRootHash := result.Hash
    // Batch committed successfully
case ffi.HashResult_None:
    // Database is empty after batch
case ffi.HashResult_Err:
    errorMsg := string(result.Err.ToSlice())
    log.Printf("Error: %s", errorMsg)
    ffi.FwdFreeOwnedBytes(result.Err)
}
```

**Operation Types:**

- `OpType_Put` - Insert or update a key-value pair
- `OpType_Delete` - Delete a key
- `OpType_DeleteRange` - Delete all keys with a given prefix

## Proposals

Proposals allow you to stage changes before committing them. You can read from a proposal to verify changes before committing.

**Create Proposal:**

```c
struct ProposalResult fwd_propose_on_db(const struct DatabaseHandle *db,
                                         BorrowedKeyValuePairs values);
```

```go
result := ffi.FwdProposeOnDb(db, batchData)
if result.Tag != ffi.ProposalResult_Some {
    // Handle error
}
proposal := result.Proposal
```

**Read from Proposal:**

```c
struct ValueResult fwd_get_from_proposal(const struct ProposalHandle *handle, 
                                          BorrowedBytes key);
```

```go
result := ffi.FwdGetFromProposal(proposal, ffi.NewBorrowedBytes(key))
// Handle as in regular get operations
```

**Commit Proposal:**

```c
struct HashResult fwd_commit_proposal(struct ProposalHandle *proposal);
```

```go
result := ffi.FwdCommitProposal(proposal)
// Note: proposal is consumed and should not be freed after commit
switch result.Tag {
case ffi.HashResult_Some:
    newRootHash := result.Hash
    // Success
case ffi.HashResult_Err:
    // Handle error
}
```

**Nested Proposals:**

```c
struct ProposalResult fwd_propose_on_proposal(const struct ProposalHandle *handle,
                                               BorrowedKeyValuePairs values);
```

You can create proposals on top of other proposals to stage multiple levels of changes.

**Free Proposal (if not committed):**

```c
struct VoidResult fwd_free_proposal(struct ProposalHandle *proposal);
```

```go
ffi.FwdFreeProposal(proposal)
```

## Revisions

Revisions represent historical states of the database.

**Get Revision:**

```c
struct RevisionResult fwd_get_revision(const struct DatabaseHandle *db, 
                                        BorrowedBytes root);
```

```go
rootHash := // ... some hash
result := ffi.FwdGetRevision(db, ffi.NewBorrowedBytes(rootHash[:]))
if result.Tag != ffi.RevisionResult_Some {
    // Handle error
}
revision := result.Revision
```

**Read from Revision:**

```c
struct ValueResult fwd_get_from_revision(const struct RevisionHandle *revision, 
                                          BorrowedBytes key);
```

```go
result := ffi.FwdGetFromRevision(revision, ffi.NewBorrowedBytes(key))
// Handle as in regular get operations
```

**Free Revision:**

```c
struct VoidResult fwd_free_revision(struct RevisionHandle *revision);
```

```go
ffi.FwdFreeRevision(revision)
```

## Iterators

Iterators provide efficient traversal of key-value pairs.

**Create Iterator on Revision:**

```c
struct IteratorResult fwd_iter_on_revision(const struct RevisionHandle *revision,
                                            BorrowedBytes key);
```

```go
// Start from beginning (empty key)
result := ffi.FwdIterOnRevision(revision, ffi.NewBorrowedBytes([]byte{}))
if result.Tag != ffi.IteratorResult_Some {
    // Handle error
}
iter := result.Iterator

// Or start from specific key
result := ffi.FwdIterOnRevision(revision, ffi.NewBorrowedBytes([]byte("start_key")))
```

**Create Iterator on Proposal:**

```c
struct IteratorResult fwd_iter_on_proposal(const struct ProposalHandle *handle, 
                                            BorrowedBytes key);
```

**Get Next Item:**

```c
struct KeyValueResult fwd_iter_next(struct IteratorHandle *handle);
```

```go
for {
    result := ffi.FwdIterNext(iter)
    switch result.Tag {
    case ffi.KeyValueResult_Some:
        key := result.Key.ToSlice()
        value := result.Value.ToSlice()
        // Process key-value pair
        ffi.FwdFreeOwnedBytes(result.Key)
        ffi.FwdFreeOwnedBytes(result.Value)
    case ffi.KeyValueResult_None:
        // End of iteration
        return
    case ffi.KeyValueResult_Err:
        // Handle error
        errorMsg := string(result.Err.ToSlice())
        log.Printf("Error: %s", errorMsg)
        ffi.FwdFreeOwnedBytes(result.Err)
        return
    }
}
```

**Get Multiple Items:**

```c
struct KeyValueBatchResult fwd_iter_next_n(struct IteratorHandle *handle, size_t n);
```

```go
result := ffi.FwdIterNextN(iter, 100) // Get up to 100 items
switch result.Tag {
case ffi.KeyValueBatchResult_Some:
    batch := result.Batch.ToSlice()
    for _, kv := range batch {
        key := kv.Key.ToSlice()
        value := kv.Value.ToSlice()
        // Process key-value pair
    }
    ffi.FwdFreeOwnedKeyValueBatch(result.Batch)
case ffi.KeyValueBatchResult_None:
    // No more items
case ffi.KeyValueBatchResult_Err:
    // Handle error
}
```

**Free Iterator:**

```c
struct VoidResult fwd_free_iterator(struct IteratorHandle *iterator);
```

```go
ffi.FwdFreeIterator(iter)
```

## Proof Operations

Firewood supports cryptographic proofs for data authenticity and integrity.

### Range Proofs

Range proofs demonstrate that a set of key-value pairs exists within a revision.

**Generate Range Proof:**

```c
struct RangeProofResult fwd_db_range_proof(const struct DatabaseHandle *db,
                                            BorrowedBytes root,
                                            BorrowedBytes start,
                                            BorrowedBytes end,
                                            size_t limit);
```

```go
result := ffi.FwdDbRangeProof(
    db,
    ffi.NewBorrowedBytes(rootHash[:]),
    ffi.NewBorrowedBytes([]byte("start_key")),
    ffi.NewBorrowedBytes([]byte("end_key")),
    1000, // limit
)
if result.Tag != ffi.RangeProofResult_Some {
    // Handle error
}
proof := result.Proof
defer ffi.FwdFreeRangeProof(proof)
```

**Serialize Range Proof:**

```c
struct ValueResult fwd_range_proof_to_bytes(const struct RangeProofContext *proof);
```

```go
result := ffi.FwdRangeProofToBytes(proof)
if result.Tag == ffi.ValueResult_Some {
    proofBytes := result.Value.ToSlice()
    // Store or transmit proofBytes
    ffi.FwdFreeOwnedBytes(result.Value)
}
```

**Deserialize Range Proof:**

```c
struct RangeProofResult fwd_range_proof_from_bytes(BorrowedBytes bytes);
```

```go
result := ffi.FwdRangeProofFromBytes(ffi.NewBorrowedBytes(proofBytes))
if result.Tag == ffi.RangeProofResult_Some {
    proof := result.Proof
    defer ffi.FwdFreeRangeProof(proof)
}
```

**Verify Range Proof:**

```c
struct VoidResult fwd_db_verify_range_proof(const struct DatabaseHandle *db,
                                             BorrowedBytes root,
                                             const struct RangeProofContext *proof);
```

```go
result := ffi.FwdDbVerifyRangeProof(db, ffi.NewBorrowedBytes(rootHash[:]), proof)
if result.Tag == ffi.VoidResult_Ok {
    // Proof is valid
} else {
    // Proof is invalid or error occurred
}
```

**Verify and Commit Range Proof:**

```c
struct HashResult fwd_db_verify_and_commit_range_proof(const struct DatabaseHandle *db,
                                                        BorrowedBytes start,
                                                        const struct RangeProofContext *proof);
```

This verifies a range proof and commits the data to the database in a single operation.

**Iterate Range Proof Keys:**

```c
struct NextKeyRangeResult fwd_range_proof_find_next_key(struct RangeProofContext *proof);
```

### Change Proofs

Change proofs demonstrate differences between two revisions.

**Generate Change Proof:**

```c
struct ChangeProofResult fwd_db_change_proof(const struct DatabaseHandle *db,
                                              BorrowedBytes start_root,
                                              BorrowedBytes end_root,
                                              BorrowedBytes start,
                                              BorrowedBytes end,
                                              size_t limit);
```

```go
result := ffi.FwdDbChangeProof(
    db,
    ffi.NewBorrowedBytes(startRootHash[:]),
    ffi.NewBorrowedBytes(endRootHash[:]),
    ffi.NewBorrowedBytes([]byte("start_key")),
    ffi.NewBorrowedBytes([]byte("end_key")),
    1000,
)
if result.Tag != ffi.ChangeProofResult_Some {
    // Handle error
}
changeProof := result.Proof
defer ffi.FwdFreeChangeProof(changeProof)
```

**Verify Change Proof:**

```c
struct VoidResult fwd_db_verify_change_proof(const struct DatabaseHandle *db,
                                              BorrowedBytes start_root,
                                              BorrowedBytes end_root,
                                              const struct ChangeProofContext *proof);
```

**Verify and Commit Change Proof:**

```c
struct HashResult fwd_db_verify_and_commit_change_proof(const struct DatabaseHandle *db,
                                                         BorrowedBytes expected_end_root,
                                                         const struct ChangeProofContext *proof);
```

## Metrics and Logging

### Metrics

**Start Metrics (Default Port):**

```c
struct VoidResult fwd_start_metrics(void);
```

```go
result := ffi.FwdStartMetrics()
if result.Tag == ffi.VoidResult_Ok {
    // Metrics started successfully
}
```

**Start Metrics (Custom Port):**

```c
struct VoidResult fwd_start_metrics_with_exporter(uint16_t metrics_port);
```

```go
result := ffi.FwdStartMetricsWithExporter(9090)
```

**Gather Metrics:**

```c
struct ValueResult fwd_gather(void);
```

```go
result := ffi.FwdGather()
if result.Tag == ffi.ValueResult_Some {
    metricsData := string(result.Value.ToSlice())
    // metricsData contains Prometheus-format metrics
    ffi.FwdFreeOwnedBytes(result.Value)
}
```

### Logging

**Start Logging:**

```c
struct VoidResult fwd_start_logs(struct LogArgs args);
```

```go
args := ffi.LogArgs{
    LogLevel: ffi.NewBorrowedBytes([]byte("info")),
    LogDirectory: ffi.NewBorrowedBytes([]byte("/path/to/logs")),
    LogWriterMode: ffi.LogWriterMode_Both, // Or Async, Blocking
    LogCompress: true,
    LogFileSize: 1024 * 1024 * 100, // 100MB
    LogFileCount: 10,
}
result := ffi.FwdStartLogs(args)
if result.Tag == ffi.VoidResult_Ok {
    // Logging started
}
```

**Log Levels:**
- `trace`
- `debug`
- `info`
- `warn`
- `error`

**Log Writer Modes:**
- `LogWriterMode_Async` - Asynchronous logging (non-blocking)
- `LogWriterMode_Blocking` - Synchronous logging (blocking)
- `LogWriterMode_Both` - Both async and blocking

## Type Reference

### Core Types

- `DatabaseHandle` - Opaque database handle
- `ProposalHandle` - Opaque proposal handle
- `RevisionHandle` - Opaque revision handle
- `IteratorHandle` - Opaque iterator handle
- `RangeProofContext` - Range proof context
- `ChangeProofContext` - Change proof context

### Data Types

- `HashKey` - 32-byte hash key (array)
- `BorrowedBytes` - Borrowed byte slice (pointer + length)
- `OwnedBytes` - Owned byte slice that must be freed
- `KeyValuePair` - Key-value pair with operation type
- `BorrowedKeyValuePairs` - Borrowed array of key-value pairs
- `OwnedKeyValuePair` - Owned key-value pair
- `OwnedKeyValueBatch` - Owned batch of key-value pairs

### Operation Types

```go
const (
    OpType_Put
    OpType_Delete
    OpType_DeleteRange
)
```

## Examples

### Complete Example: Basic Operations

```go
package main

import (
    "log"
    ffi "github.com/ava-labs/firewood-go-ethhash/ffi"
)

func main() {
    // Open database
    args := ffi.DatabaseHandleArgs{
        Path: ffi.NewBorrowedBytes([]byte("test_db")),
        CacheSize: 10000,
        FreeListCacheSize: 1024,
        Revisions: 128,
        Strategy: 2,
        Truncate: true,
    }
    openResult := ffi.FwdOpenDb(args)
    if openResult.Tag != ffi.HandleResult_Some {
        log.Fatal("Failed to open database")
    }
    db := openResult.Db
    defer ffi.FwdCloseDb(db)

    // Write data
    kvPairs := []ffi.KeyValuePair{
        {
            Key: ffi.NewBorrowedBytes([]byte("key1")),
            Value: ffi.NewBorrowedBytes([]byte("value1")),
            Op: ffi.OpType_Put,
        },
        {
            Key: ffi.NewBorrowedBytes([]byte("key2")),
            Value: ffi.NewBorrowedBytes([]byte("value2")),
            Op: ffi.OpType_Put,
        },
    }
    
    batchData := ffi.BorrowedKeyValuePairs{
        Ptr: &kvPairs[0],
        Len: len(kvPairs),
    }
    
    batchResult := ffi.FwdBatch(db, batchData)
    if batchResult.Tag != ffi.HashResult_Some {
        log.Fatal("Failed to write batch")
    }

    // Read data
    getResult := ffi.FwdGetLatest(db, ffi.NewBorrowedBytes([]byte("key1")))
    if getResult.Tag == ffi.ValueResult_Some {
        value := getResult.Value.ToSlice()
        log.Printf("key1 = %s", string(value))
        ffi.FwdFreeOwnedBytes(getResult.Value)
    }
}
```

### Example: Working with Proposals

```go
// Create a proposal
kvPairs := []ffi.KeyValuePair{
    {
        Key: ffi.NewBorrowedBytes([]byte("test_key")),
        Value: ffi.NewBorrowedBytes([]byte("test_value")),
        Op: ffi.OpType_Put,
    },
}

batchData := ffi.BorrowedKeyValuePairs{
    Ptr: &kvPairs[0],
    Len: len(kvPairs),
}

proposeResult := ffi.FwdProposeOnDb(db, batchData)
if proposeResult.Tag != ffi.ProposalResult_Some {
    log.Fatal("Failed to create proposal")
}
proposal := proposeResult.Proposal

// Read from proposal to verify
getResult := ffi.FwdGetFromProposal(proposal, ffi.NewBorrowedBytes([]byte("test_key")))
if getResult.Tag == ffi.ValueResult_Some {
    value := getResult.Value.ToSlice()
    log.Printf("Value in proposal: %s", string(value))
    ffi.FwdFreeOwnedBytes(getResult.Value)
}

// Commit proposal
commitResult := ffi.FwdCommitProposal(proposal)
// Note: proposal is consumed, don't free it
if commitResult.Tag == ffi.HashResult_Some {
    log.Printf("Committed with root hash: %x", commitResult.Hash)
}
```

### Example: Iterating Keys

```go
// Get current root
rootResult := ffi.FwdRootHash(db)
if rootResult.Tag != ffi.HashResult_Some {
    log.Fatal("Failed to get root hash")
}

// Get revision
revResult := ffi.FwdGetRevision(db, ffi.NewBorrowedBytes(rootResult.Hash[:]))
if revResult.Tag != ffi.RevisionResult_Some {
    log.Fatal("Failed to get revision")
}
revision := revResult.Revision
defer ffi.FwdFreeRevision(revision)

// Create iterator
iterResult := ffi.FwdIterOnRevision(revision, ffi.NewBorrowedBytes([]byte{}))
if iterResult.Tag != ffi.IteratorResult_Some {
    log.Fatal("Failed to create iterator")
}
iter := iterResult.Iterator
defer ffi.FwdFreeIterator(iter)

// Iterate
for {
    kvResult := ffi.FwdIterNext(iter)
    switch kvResult.Tag {
    case ffi.KeyValueResult_Some:
        key := kvResult.Key.ToSlice()
        value := kvResult.Value.ToSlice()
        log.Printf("Key: %s, Value: %s", string(key), string(value))
        ffi.FwdFreeOwnedBytes(kvResult.Key)
        ffi.FwdFreeOwnedBytes(kvResult.Value)
    case ffi.KeyValueResult_None:
        return
    case ffi.KeyValueResult_Err:
        errorMsg := string(kvResult.Err.ToSlice())
        log.Printf("Error: %s", errorMsg)
        ffi.FwdFreeOwnedBytes(kvResult.Err)
        return
    }
}
```

## See Also

- [Firewood Repository](https://github.com/ava-labs/firewood)
- [Main README](../../README.md)
- [FFI README](../../ffi/README.md)
- [Rust API Reference](rust.md)
- [Go Tests](../../ffi/firewood_test.go)
