# Delete Log Design

## Motivation

Firewood currently has two space-leakage problems:

1. **Crash recovery**: If the process crashes before the background thread
   persists in-memory revisions, the deleted node lists carried by those
   revisions are lost. The nodes become unreachable but their disk space is
   never reclaimed.

2. **Shutdown leakage**: On clean shutdown, the `PersistLoop` persists the
   latest revision, but older revisions still in the in-memory queue may have
   their delete lists discarded without being reaped, permanently leaking space.

The delete log solves both problems by recording every committed revision's
metadata and delete list to an append-only file (`firewood.dlg`). On startup,
the log is read and any un-reaped entries are processed to reclaim space.

## Overview

- **Append-only during runtime**: Every call to `RevisionManager::commit()`
  appends an entry to the delete log. No cleanup or compaction occurs while the
  process is running.
- **Read-and-wipe on startup**: When the database is opened, the delete log is
  read, recoverable entries are reaped, the file is wiped clean, and a seed
  entry is written for the initial revision.
- **Coexists with RootStore**: The delete log tracks in-memory (committed but
  not necessarily persisted) revisions. `RootStore` tracks persisted revisions.
  They serve different purposes and both remain.

## File Format

The delete log file (`firewood.dlg`) is located alongside the main database
file. It contains a sequence of entries, each representing one committed
revision. All multi-byte integers are little-endian.

### Entry Layout

```text
+-------------------+------------------------------------------+
| entry_length: u64 | total bytes after this field (inc. csum) |
+-------------------+------------------------------------------+
| checksum: u32     | CRC32C over all bytes after this field   |
+-------------------+------------------------------------------+
| root_address: u64 |                                          |
+-------------------+------------------------------------------+
| root_hash: [u8;32]|                                          |
+-------------------+------------------------------------------+
| num_deletes: u64  |                                          |
+-------------------+------------------------------------------+
| delete_entries    | num_deletes sequential DeleteEntry values |
+-------------------+------------------------------------------+
```

### DeleteEntry: Persisted Node

```text
+----------------+
| tag: u8 = 0x00 |
+----------------+
| address: u64   |
+----------------+
```

### DeleteEntry: Unpersisted Node

```text
+------------------------+
| tag: u8 = 0x01         |
+------------------------+
| content_length: u64    |
+------------------------+
| content: [u8; content_length] |
+------------------------+
```

### Checksum

CRC32C (hardware-accelerated on most platforms). Covers all bytes after the
checksum field through the end of the entry. If a checksum fails or the entry
is truncated, that entry and all subsequent entries are discarded (partial write
from crash).

## Write Path (Commit Time)

The delete log entry is written synchronously in `RevisionManager::commit()`,
after the new `Committed` revision is constructed but before returning.

For each `MaybePersistedNode` in the revision's delete list:

- If it has a `LinearAddress` (Persisted or Allocated): write as tag `0x00` +
  address.
- If it is Unpersisted: serialize the node content and write as tag `0x01` +
  length + content.

Serialization is performed in-memory to a `Vec<u8>` first (to compute the
checksum), then written via a single `write_all`. No `fsync` is called,
matching the existing durability model of the main database file.

## Read Path (Startup)

During `Db` initialization, before the `RevisionManager` is fully constructed:

1. **Open and read** `firewood.dlg` if it exists.
2. **Parse entries sequentially.** For each entry, validate the checksum. On the
   first invalid or truncated entry, stop and discard it and everything after.
3. **Determine reaping threshold.** Read the persisted `root_address` from the
   `NodeStoreHeader`. Any entry whose root address is less than or equal to the
   persisted root address represents an already-reaped revision. Skip it.
4. **Reap remaining entries:**
   - For persisted delete nodes (tag `0x00`): call
     `NodeAllocator::delete_node()` using the stored address.
   - For unpersisted delete nodes (tag `0x01`): perform a linear scan of the
     database file to find matching nodes by content, then free their space if
     found (see [Trie Walk for Unpersisted Nodes](#trie-walk-for-unpersisted-nodes)).
5. **Flush the updated header** (free lists have changed from reaping).
6. **Wipe the file.** Truncate `firewood.dlg` to 0 bytes.
7. **Re-seed.** Write an entry for the initial revision loaded from the header.

### Comparing Root Addresses

`LinearAddress` values are bump-allocated (monotonically increasing). A
revision's root address being less than or equal to the header's persisted root
address means it was already persisted and reaped. This provides a clean
threshold without extra markers.

## Trie Walk for Unpersisted Nodes

An unpersisted node at commit time may have been later persisted by the
background thread. To find it on disk and reclaim its space:

1. Collect all unpersisted node contents from the delete log into a set.
2. Perform a single linear scan of the database file (skipping the header and
   free areas identified by the `0xFF` marker).
3. For each allocated node, compare its serialized content against the set.
4. If a match is found, free that address via `NodeAllocator::delete_node()`.

### Performance Considerations

- Unpersisted deleted nodes should be rare: they only occur when a revision's
  parent was not yet persisted.
- The linear scan only happens on startup after a crash or unclean shutdown.
- Batching all unpersisted nodes into a single scan pass keeps the cost at
  O(database size) regardless of the number of unpersisted nodes.

## Struct Design

### New Module

`storage/src/delete_log.rs`

### Types

```rust
pub struct DeleteLog {
    file: File, // opened in append mode
}

pub struct DeleteLogEntry {
    pub root_address: LinearAddress,
    pub root_hash: [u8; 32],
    pub deleted: Vec<DeleteEntry>,
}

pub enum DeleteEntry {
    Persisted(LinearAddress),
    Unpersisted(Vec<u8>), // serialized node content
}
```

### Public API

- `DeleteLog::new(path: &Path) -> Result<Self>`: Create or open the file.
- `DeleteLog::append(&mut self, entry: &DeleteLogEntry) -> Result<()>`:
  Serialize and write an entry.
- `DeleteLog::read_all(path: &Path) -> Result<Vec<DeleteLogEntry>>`: Parse all
  valid entries from the file.
- `DeleteLog::wipe_and_seed(&mut self, entry: &DeleteLogEntry) -> Result<()>`:
  Truncate the file and write the initial entry.

### Integration

- `RevisionManager` holds a `DeleteLog` instance.
- `RevisionManager::commit()` calls `delete_log.append()` after constructing
  the new `Committed` revision.
- `Db::new()` calls `DeleteLog::read_all()`, performs recovery, then calls
  `wipe_and_seed()`.

## Error Handling and Edge Cases

| Scenario | Behavior |
| --- | --- |
| Crash during delete log write | Partially written entry detected by checksum on startup and discarded. One entry's space recovery is lost (acceptable). |
| Empty delete log on startup | Nothing to recover. Wipe and seed. |
| No delete log file on startup | First-time creation or migration. Create fresh file and seed. |
| All entries below reaping threshold | All revisions already reaped. Wipe and seed. |
| Unpersisted node not found in scan | Node was never persisted (crash before background thread ran). Skip silently; no disk space to reclaim. |
| Empty/new database file | No trie walk needed. Skip unpersisted node recovery. |
| Concurrent access | Not a concern. Writes happen under the commit write lock; reads happen during single-threaded init. |
