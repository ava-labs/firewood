# Firewood Rust API Reference

This document provides a comprehensive reference for the Firewood Rust API. For full API documentation with all implementation details, see the [hosted Cargo docs](https://docs.rs/firewood).

## Table of Contents

- [Core Concepts](#core-concepts)
- [Database Operations](#database-operations)
  - [Database Initialization](#database-initialization)
  - [Configuration](#configuration)
- [Key-Value Operations](#key-value-operations)
  - [Reading Data](#reading-data)
  - [Writing Data](#writing-data)
- [Batch Operations and Proposals](#batch-operations-and-proposals)
- [Revision Management](#revision-management)
- [Proof Generation and Verification](#proof-generation-and-verification)
  - [Single Key Proofs](#single-key-proofs)
  - [Range Proofs](#range-proofs)
- [Iterator Usage](#iterator-usage)
- [Error Types and Handling](#error-types-and-handling)
- [Examples](#examples)

## Core Concepts

Firewood organizes its API around several key concepts:

- **Database (`Db`)**: The main database instance that manages revisions and provides access to data
- **Revision**: A historical point-in-time state of the trie, identified by a root hash
- **Proposal**: A proposed change to the database that can be committed
- **View (`DbView`)**: A read-only interface to a revision or proposal
- **BatchOp**: An operation (Put, Delete, or DeleteRange) to be applied to the database

## Database Operations

### Database Initialization

To create or open a database, use the `Db::new` function:

```rust
use firewood::db::{Db, DbConfig};
use firewood::manager::RevisionManagerConfig;
use std::num::NonZeroUsize;

// Create a database with default configuration
let db = Db::new("path/to/db", DbConfig::builder().build())?;

// Create with custom configuration
let rev_config = RevisionManagerConfig::builder()
    .node_cache_size(NonZeroUsize::new(10000).unwrap())
    .max_revisions(128)
    .build();

let config = DbConfig::builder()
    .truncate(false)
    .create_if_missing(true)
    .manager(rev_config)
    .build();

let db = Db::new("path/to/db", config)?;
```

**Key Methods:**
- `Db::new<P: AsRef<Path>>(db_path: P, cfg: DbConfig) -> Result<Self, api::Error>`: Creates or opens a database at the specified path

### Configuration

**`DbConfig`** - Database configuration options:
- `create_if_missing: bool` - Create the database if it doesn't exist (default: `true`)
- `truncate: bool` - Reset the database on open, losing all existing data (default: `false`)
- `manager: RevisionManagerConfig` - Revision manager configuration

**`RevisionManagerConfig`** - Revision management configuration:
- `node_cache_size: NonZeroUsize` - Size of the node cache (default: 20480)
- `max_revisions: usize` - Maximum number of revisions to keep (default: 128)
- `cache_read_strategy: CacheReadStrategy` - Cache strategy (WritesOnly, BranchReads, or All)
- `free_list_cache_size: NonZeroUsize` - Free list cache size (default: 1024)

## Key-Value Operations

### Reading Data

Reading data requires obtaining a view of the database, either the latest revision or a specific historical revision:

```rust
use firewood::v2::api::{Db as _, DbView};

// Get the root hash of the latest revision
let root_hash = db.root_hash()?.expect("database is not empty");

// Get a view of the latest revision
let revision = db.revision(root_hash)?;

// Read a value
let value = revision.val(b"my_key")?;
match value {
    Some(bytes) => println!("Value: {:?}", bytes),
    None => println!("Key not found"),
}
```

**Key Methods:**
- `Db::root_hash(&self) -> Result<Option<HashKey>, Error>`: Get the root hash of the latest revision
- `Db::revision(&self, hash: TrieHash) -> Result<Arc<Self::Historical>, Error>`: Get a view of a specific revision
- `DbView::val<K: KeyType>(&self, key: K) -> Result<Option<Value>, Error>`: Get the value for a key
- `DbView::root_hash(&self) -> Result<Option<HashKey>, Error>`: Get the root hash of this view

### Writing Data

Writing data in Firewood is done through proposals that are then committed:

```rust
use firewood::db::BatchOp;
use firewood::v2::api::{Db as _, Proposal as _};

// Create a batch of operations
let batch = vec![
    BatchOp::Put { 
        key: b"key1".to_vec(), 
        value: b"value1".to_vec() 
    },
    BatchOp::Put { 
        key: b"key2".to_vec(), 
        value: b"value2".to_vec() 
    },
    BatchOp::Delete { 
        key: b"old_key".to_vec() 
    },
];

// Create a proposal
let proposal = db.propose(batch)?;

// Commit the proposal
proposal.commit()?;
```

## Batch Operations and Proposals

Firewood uses a proposal-based system for mutations. A proposal is a set of changes that can be committed atomically.

**`BatchOp<K, V>`** - Represents a single operation:
- `BatchOp::Put { key: K, value: V }` - Insert or update a key-value pair
- `BatchOp::Delete { key: K }` - Delete a key
- `BatchOp::DeleteRange { prefix: K }` - Delete all keys with the given prefix

**Proposal Workflow:**

1. Create a proposal with `Db::propose()`
2. Optionally read from the proposal to verify changes
3. Commit with `Proposal::commit()` or discard by dropping

```rust
use firewood::v2::api::{Db as _, DbView, Proposal as _};

// Create a proposal
let proposal = db.propose(vec![
    BatchOp::Put { key: b"key".to_vec(), value: b"value".to_vec() }
])?;

// Read from the proposal before committing
let value = proposal.val(b"key")?;
assert_eq!(value.as_deref(), Some(b"value".as_ref()));

// Commit the proposal
let new_root = proposal.commit()?;
println!("New root hash: {:?}", new_root);
```

**Key Methods:**
- `Db::propose(&self, data: impl IntoIterator) -> Result<Self::Proposal<'_>, Error>`: Create a new proposal
- `Proposal::commit(self) -> Result<Option<HashKey>, Error>`: Commit the proposal and return the new root hash

## Revision Management

Firewood maintains a configurable number of recent revisions. Each commit creates a new revision identified by its root hash.

```rust
use firewood::v2::api::Db as _;

// Get all available revision hashes
let hashes = db.all_hashes()?;
println!("Available revisions: {}", hashes.len());

// Access a specific historical revision
for hash in hashes {
    let revision = db.revision(hash)?;
    println!("Root hash: {:?}", revision.root_hash()?);
}
```

**Key Methods:**
- `Db::all_hashes(&self) -> Result<Vec<TrieHash>, Error>`: Get all available revision hashes
- `Db::revision(&self, hash: TrieHash) -> Result<Arc<Self::Historical>, Error>`: Get a specific revision

## Proof Generation and Verification

Firewood supports cryptographic proofs for data authenticity.

### Single Key Proofs

A single key proof demonstrates that a key exists (or doesn't exist) in a specific revision:

```rust
use firewood::v2::api::DbView;

// Get a proof for a single key
let proof = revision.single_key_proof(b"my_key")?;

// The proof contains the merkle path and can be verified independently
```

**Key Methods:**
- `DbView::single_key_proof<K: KeyType>(&self, key: K) -> Result<FrozenProof, Error>`: Generate a proof for a single key

### Range Proofs

A range proof demonstrates all key-value pairs in a range:

```rust
use firewood::v2::api::DbView;
use std::num::NonZeroUsize;

// Get a range proof for all keys between first_key and last_key
let range_proof = revision.range_proof(
    Some(b"first_key"),
    Some(b"last_key"),
    NonZeroUsize::new(1000), // Maximum 1000 keys
)?;

// Get a range proof for all keys (no bounds)
let all_keys_proof = revision.range_proof(
    None::<&[u8]>,
    None::<&[u8]>,
    NonZeroUsize::new(10000),
)?;
```

**Key Methods:**
- `DbView::range_proof<K: KeyType>(&self, first_key: Option<K>, last_key: Option<K>, limit: Option<NonZeroUsize>) -> Result<FrozenRangeProof, Error>`: Generate a range proof

## Iterator Usage

Firewood provides efficient iteration over key-value pairs:

```rust
use firewood::v2::api::DbView;

// Iterate over all keys starting from the beginning
let mut iter = revision.iter()?;
while let Some(result) = iter.next() {
    let (key, value) = result?;
    println!("Key: {:?}, Value: {:?}", key, value);
}

// Iterate starting from a specific key
let mut iter = revision.iter_from(b"start_key")?;
for result in iter {
    let (key, value) = result?;
    println!("Key: {:?}, Value: {:?}", key, value);
}

// Use iter_option for optional starting key
let start_key = Some(b"maybe_key");
let mut iter = revision.iter_option(start_key)?;
```

**Key Methods:**
- `DbView::iter(&self) -> Result<Self::Iter<'_>, Error>`: Create an iterator from the beginning
- `DbView::iter_from<K: KeyType>(&self, first_key: K) -> Result<Self::Iter<'_>, Error>`: Create an iterator from a specific key
- `DbView::iter_option<K: KeyType>(&self, first_key: Option<K>) -> Result<Self::Iter<'_>, Error>`: Create an iterator with optional starting key

## Error Types and Handling

Firewood uses the `api::Error` enum for error handling:

```rust
use firewood::v2::api::Error;

match db.root_hash() {
    Ok(Some(hash)) => println!("Root hash: {:?}", hash),
    Ok(None) => println!("Database is empty"),
    Err(Error::IO(e)) => println!("I/O error: {}", e),
    Err(Error::FileIO(e)) => println!("File I/O error: {}", e),
    Err(Error::RevisionNotFound { provided }) => {
        println!("Revision not found: {:?}", provided)
    }
    Err(e) => println!("Other error: {}", e),
}
```

**Common Error Variants:**
- `Error::RevisionNotFound { provided: Option<HashKey> }` - Requested revision doesn't exist
- `Error::ParentNotLatest { provided, expected }` - Cannot commit proposal; parent is not the latest revision
- `Error::InvalidRange { start_key, end_key }` - Invalid range specification
- `Error::IO(std::io::Error)` - I/O error
- `Error::FileIO(FileIoError)` - File-specific I/O error
- `Error::AlreadyCommitted` - Attempted to commit an already-committed proposal
- `Error::LatestIsEmpty` - The database has no root hash

## Examples

### Basic Put and Get

```rust
use firewood::db::{Db, DbConfig, BatchOp};
use firewood::v2::api::{Db as _, DbView, Proposal as _};

// Open database
let db = Db::new("my_db", DbConfig::builder().build())?;

// Write data
let proposal = db.propose(vec![
    BatchOp::Put { key: b"hello".to_vec(), value: b"world".to_vec() }
])?;
proposal.commit()?;

// Read data
let root = db.root_hash()?.expect("db not empty");
let revision = db.revision(root)?;
let value = revision.val(b"hello")?;
assert_eq!(value.as_deref(), Some(b"world".as_ref()));

Ok::<(), firewood::v2::api::Error>(())
```

### Batch Operations

```rust
use firewood::db::{Db, DbConfig, BatchOp};
use firewood::v2::api::{Db as _, Proposal as _};

let db = Db::new("my_db", DbConfig::builder().build())?;

// Create a batch of operations
let batch = vec![
    BatchOp::Put { key: b"key1".to_vec(), value: b"value1".to_vec() },
    BatchOp::Put { key: b"key2".to_vec(), value: b"value2".to_vec() },
    BatchOp::Put { key: b"key3".to_vec(), value: b"value3".to_vec() },
];

// Propose and commit
let proposal = db.propose(batch)?;
proposal.commit()?;

Ok::<(), firewood::v2::api::Error>(())
```

### Iterating Over Keys

```rust
use firewood::db::{Db, DbConfig};
use firewood::v2::api::{Db as _, DbView};

let db = Db::new("my_db", DbConfig::builder().build())?;
let root = db.root_hash()?.expect("db not empty");
let revision = db.revision(root)?;

// Iterate over all keys
for result in revision.iter()? {
    let (key, value) = result?;
    println!("Key: {:?}, Value: {:?}", key, value);
}

Ok::<(), firewood::v2::api::Error>(())
```

### Working with Historical Revisions

```rust
use firewood::db::{Db, DbConfig, BatchOp};
use firewood::v2::api::{Db as _, DbView, Proposal as _};

let db = Db::new("my_db", DbConfig::builder().build())?;

// First commit
let proposal = db.propose(vec![
    BatchOp::Put { key: b"key".to_vec(), value: b"value1".to_vec() }
])?;
proposal.commit()?;

// Second commit
let proposal = db.propose(vec![
    BatchOp::Put { key: b"key".to_vec(), value: b"value2".to_vec() }
])?;
proposal.commit()?;

// Access all revisions
let hashes = db.all_hashes()?;
for hash in hashes {
    let revision = db.revision(hash)?;
    let value = revision.val(b"key")?;
    println!("Revision {:?}: {:?}", hash, value);
}

Ok::<(), firewood::v2::api::Error>(())
```

## See Also

- [Firewood Repository](https://github.com/ava-labs/firewood)
- [Main README](../../README.md)
- [Go API Reference](go.md)
- [Example Code](../../firewood/examples/)
