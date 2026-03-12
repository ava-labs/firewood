# Multi-Head Firewood: Design Document

## 1. Motivation

### Problem

In Avalanche-based blockchain networks, each validator on a given chain
independently executes transactions and maintains a Merkle trie representing
the current world state. Today, each validator runs its own Firewood database
instance. Since honest validators process the same blocks in the same order,
they produce **identical state** at each block height -- the same trie
structure, the same root hash, the same on-disk nodes.

When N validators run on shared infrastructure (e.g., a single machine or
cluster), this means N identical copies of the same database. For chains
with large state (e.g., the C-Chain with hundreds of GB), this duplication
is the dominant cost:

- **Disk usage** scales linearly with N (N x hundreds of GB)
- **Write amplification** scales linearly (each validator writes the same
  nodes independently)
- **Memory pressure** from N separate page caches and revision pools
- **Operational complexity** of managing N database lifecycles

For infrastructure operators running many validators, this cost is
prohibitive and wasteful given the data is identical.

### Solution

Allow N validators to share a single Firewood deployment. When two validators
produce the same revision (same root hash), store it once and point both
validators to the shared copy. Each validator maintains its own "head" (latest
revision pointer) but the underlying storage is shared.

Key insight: Firewood already deduplicates trie nodes by address within a
single database. Multi-head extends this deduplication across validators
by recognizing that identical root hashes imply identical trie content
(guaranteed by the Merkle property).

### Economic Model

The cost of operating Firewood is a **base cost** equivalent to one
validator's storage requirements.

- **Honest validators** all produce the same chain. Their data is
  deduplicated, so the entire group shares a single chain's worth of
  storage. The base cost is split evenly: each honest validator pays
  `base_cost / N_honest`.
- **A malicious validator** produces a divergent chain. Its unique nodes
  cannot be deduplicated. Firewood **logs** that this validator is on its
  own chain (not sharing with the majority). This log information is consumed
  by an external billing/incentive mechanism (outside the scope of this
  feature) which charges the malicious validator the **full base cost** --
  the same cost as operating an entire independent Firewood instance.
- **Result:** Honest validators benefit from cost sharing. A malicious
  validator pays the same as running its own database, gaining no economic
  advantage from sharing infrastructure.

### Divergence Logging

When Firewood detects that a validator's committed revision has a root hash
that differs from the majority at the same parent height, it logs:

- The validator ID
- The divergent root hash
- The expected root hash (the one shared by the most validators)
- A timestamp

This information is exposed via metrics and/or a queryable API so that the
external billing system can identify divergent validators and charge them
accordingly.

### Design Philosophy

No consensus logic in Firewood. Every validator's chain is persisted
independently. Firewood does not determine which validator is "correct" or
take punitive action. It stores everything efficiently through deduplication,
logs divergence events, and relies entirely on external economic incentives
and the Avalanche consensus protocol to handle Byzantine behavior.

---

## 2. Architecture

### 2.1 Multiple Heads

The `RevisionManager` maintains one "head" per registered validator. A head
is a reference (`Arc<NodeStore<Committed, FileBacked>>`) to that validator's
latest committed revision.

```text
    Single firewood.db
    +-----------------------------------------------------------+
    |  Header (2048 bytes):                                      |
    |    root[0] = Block 100 (Validator A)                       |
    |    root[1] = Block 100 (Validator B) -- same as root[0]    |
    |    root[2] = Block 99  (Validator C)                       |
    |    root[3] = Block 100' (Validator D) -- divergent         |
    |                                                            |
    |  Nodes on disk:                                            |
    |    Block 95, 96, 97, 98, 99 nodes (shared)                 |
    |    Block 100 nodes (shared by A, B)                        |
    |    Block 100' nodes (unique to D -- billed at full cost)   |
    +-----------------------------------------------------------+
```

### 2.2 Deduplication

When a validator commits a revision, the system checks whether a revision
with the same root hash already exists (via the existing `by_hash` map).
If it does, the new proposal is discarded and the validator's head is
pointed to the existing revision. Since `CommittedRevision` is `Arc`-wrapped,
this is a ref count increment -- zero storage duplication.

```text
Validator A commits Block 100 -> root hash H
  Result: New revision created, persisted, added to by_hash[H]

Validator B commits Block 100 -> root hash H
  Check by_hash[H] -> exists!
  Result: Proposal discarded. B's head = by_hash[H] (same Arc as A)
  Log: (nothing -- B is on the shared chain)

Storage impact: 1 revision stored, 2 heads pointing to it.
```

### 2.3 Divergence Detection and Logging

When a validator commits a revision whose root hash does NOT match any
existing revision committed from the same parent, Firewood logs the
divergence:

```text
Validator D commits Block 100' -> root hash H' (parent = Block 99)
  Check by_hash[H'] -> not found
  Check: other validators also committed from parent Block 99 with hash H
  Result: D is divergent.
  Log: "validator D diverged at parent Block 99: expected H, got H'"
  Metric: firewood_validator_divergence_count incremented for D
  Persist: D's revision is stored normally (D pays full base cost)
```

### 2.4 Independent Chains

Each validator is assigned to a **chain** (a lineage of revisions) identified
by a `ChainId`. Chains are created on divergence and merged on deduplication.

```text
MultiHeadState:
  validators: HashMap<ValidatorId, ValidatorState>
  chains:     HashMap<ChainId, ChainState>  // per-chain revision deques
  by_hash:    HashMap<TrieHash, CommittedRevision>
  hash_to_chain: HashMap<TrieHash, ChainId> // reverse index
  next_chain_id: ChainId                    // monotonic counter
```

`ChainState` holds a `VecDeque<CommittedRevision>` ordered oldest-to-newest.
Each `ValidatorState` carries a `chain: ChainId` field indicating which chain
the validator is currently on.

**Fork on divergence:** When a validator commits a root hash that differs from
every existing revision at that parent, a new `ChainId` is allocated and a
`ChainState` is inserted for it.

**Merge on deduplication:** When a validator's commit matches an existing
revision (same root hash), the validator is moved to the chain that owns that
revision. If the validator's old chain now has no remaining validators, it is
cleaned up immediately (see Section 5).

**Merge on advance:** `advance_validator_to_hash` similarly moves the validator
to the chain that owns the target hash, cleaning up the vacated chain.

Each validator proposes from its own head and commits against its own head.
There is no global "latest" that all validators share. Validators do not
block each other.

### 2.5 Skip-Propose Optimization

If a validator knows the expected root hash for a block (e.g., from the block
header received over the network), it can advance its head in O(1) without
computing a proposal:

```text
Validator C knows Block 100's hash is H (from block header).
C calls advance_to_hash(H).
by_hash[H] exists -> C's head = by_hash[H].

Result: C advanced from Block 99 to Block 100 with zero trie computation.
```

---

## 3. Storage Model

### 3.1 Header Format

The current `NodeStoreHeader` is 2048 bytes with ~392 bytes used, leaving
1656 bytes of padding. The multi-head extension adds per-validator root
slots within this existing space.

```text
Existing fields (392 bytes):
  version:              16 bytes
  endian_test:           8 bytes
  size:                  8 bytes
  free_lists:          184 bytes  (23 area sizes x 8 bytes)
  root_address:          8 bytes
  area_size_hash:       32 bytes
  node_hash_algorithm:   8 bytes
  root_hash:            32 bytes
  cargo_version:        32 bytes
  git_describe:         64 bytes

New fields:
  validator_count:       8 bytes
  validator_roots:     640 bytes  (16 slots x 40 bytes each)

Total: 1040 bytes (within 2048 limit)
```

Each `ValidatorRoot` slot contains:

| Field            | Size     | Description                                        |
|------------------|----------|----------------------------------------------------|
| `root_address`   | 8 bytes  | Disk offset of validator's trie root (0 = unused)  |
| `root_hash`      | 32 bytes | Merkle root hash of validator's trie               |

#### Backward Compatibility

When `validator_count == 0`, the database operates in legacy single-root
mode. The existing `root_address` and `root_hash` fields are the sole root.

### 3.2 Node Storage

All validators' nodes are stored in the same linear address space within
the same `firewood.db` file. Addresses are disk offsets (`LinearAddress`).

Each node carries a **fork ID** that identifies which fork of the chain
allocated it. This is used during reaping to prevent cross-chain corruption
(see Section 5.5).

#### On-Disk Format

The AreaIndex byte (first byte of every stored area) uses bit 7 as a
"has fork_id" flag:

```text
V0 (legacy):  [AreaIndex:1][FirstByte:1][NodeData...]          — AreaIndex 0-22
V1 (new):     [AreaIndex:1][ForkId:8 LE][FirstByte:1][NodeData...] — AreaIndex 0x80|0-22
Free area:    [AreaIndex:1][0xFF:1][varint next_addr]          — unchanged
```

- **Writing new nodes**: When `fork_id != 0`, set `AreaIndex = real_index | 0x80`
  and write the fork_id as 8 little-endian bytes after the AreaIndex byte.
  The AreaIndex is recomputed from the new total size (original + 8 bytes).
  When `fork_id == 0` (single-head or pre-fork), the legacy V0 format is used.
- **Reading nodes**: Check bit 7 of AreaIndex. If set, read 8 bytes of fork_id,
  then node data. If clear, no fork_id (legacy node, fork_id defaults to 0).
- **Free areas**: Unchanged. The fork_id flag is stripped when writing FreeArea.
- **Backward compatibility**: Old databases have all AreaIndex values < 23.
  New nodes use AreaIndex values 128-150. Readers handle both transparently.
- **Limitation**: Bit 7 is borrowed from the AreaIndex byte, so the index
  portion is limited to values 0-127. Currently there are only 23 area sizes
  (indices 0-22), so this is not a practical concern. If the number of area
  sizes ever exceeds 127, the encoding would need to change (e.g., a dedicated
  format-version byte before the AreaIndex).

#### Space Impact

Every new multi-head node allocation adds 8 bytes, which may bump small
nodes to the next area size class (e.g., 16-byte bucket to 32-byte bucket).
For larger nodes (>128 bytes), the overhead is negligible.

### 3.3 Free Lists

No changes to the free list structure or allocation logic.

---

## 4. Commit Protocol

### 4.1 Normal Commit (First Validator to Commit a Block)

1. Validate: proposal's parent == this validator's head
2. Check `by_hash`: does a revision with this root hash exist?
   No -- this is the first validator to commit this block.
3. Check: did other validators also commit from the same parent with a
   different hash? If yes, fork a new chain and log divergence event.
4. Convert proposal to Committed revision
5. Persist: write nodes to disk, update header root for this validator
6. Add to `by_hash` and `hash_to_chain` for future dedup lookups
7. Append to the validator's chain deque (`ChainState.revisions`)
8. Update this validator's head and `chain` field
9. Run `collect_reapable_revisions` on the chain; reap outside lock
10. Cleanup proposals

### 4.2 Deduplicated Commit (Subsequent Validators)

1. Validate: proposal's parent == this validator's head
2. Check `by_hash`: does a revision with this root hash exist?
   Yes -- another validator already committed this block.
3. Discard proposal (no nodes written, no persistence)
4. Move validator to the chain that owns the existing revision (via `hash_to_chain`)
5. If the validator's old chain is now empty of validators, clean it up
6. Update this validator's head = existing revision from `by_hash`
7. Update header root for this validator

No divergence logging -- this validator produced the same hash.

### 4.3 Divergent Commit (Malicious Validator)

Same as 4.1, except step 3 detects that other validators committed from the
same parent with a different hash. The divergence is logged.

### 4.4 Parent Validation

Each validator's commit is validated against its own head:

```text
if proposal.parent_hash != validator.head.root_hash:
    return Error::NotValidatorHead
```

---

## 5. Reaping and Space Management

### 5.1 Per-Chain Revision Budget

Each chain independently enforces `max_revisions`. The global revision pool
is replaced by per-chain `VecDeque<CommittedRevision>` deques. Reaping is
triggered after each commit or advance and operates chain-by-chain.

```text
For each chain C:
  while C.revisions.len() > max_revisions:
    if ANY validator's head == C.revisions.front():
      break  -- cannot reap past a validator head
    pop oldest revision from C.revisions
    remove from by_hash / hash_to_chain (if no validator head still holds it)
    send to persist_worker.reap()
```

This means a slow chain (one with a lagging validator) only blocks reaping on
its own chain, not on other chains. Honest chains with synchronized validators
reap freely regardless of divergent or lagging chains.

### 5.2 Chain Cleanup

When the last validator leaves a chain (by committing a deduplicated revision,
advancing to a hash on another chain, or being deregistered), the chain is
**cleaned up immediately**:

1. Call `collect_all_chain_revisions(chain_id)` under the write lock.
2. Remove the `ChainState` from `chains`.
3. Remove all stale entries from `by_hash` / `hash_to_chain`.
4. Outside the lock, attempt `Arc::try_unwrap` on each revision and reap it.

`collect_reapable_revisions` handles normal per-chain budget enforcement.
`collect_all_chain_revisions` handles the empty-chain cleanup case.

### 5.3 Safe Deletion Rules

**Shared (deduplicated) revisions:** All validators on a chain produced the
same revision. When the chain is reaped, the `deleted` list in each revision
is sent to the persist worker to return space to the free lists.

**Divergent revisions:** Live on their own chain. When the chain is cleaned up
(all validators have moved away), the revisions' deleted nodes may reference
nodes still alive in sibling chains. `Arc::try_unwrap` is used as a safety
gate: if any external consumer (e.g., an open view) still holds the Arc, the
revision is not reaped and the space is not freed.

### 5.4 Fork-Aware Reaping

When validators diverge, chains share disk addresses for ancestor nodes.
Reaping one chain's deleted nodes could free addresses still live in another
chain's trie. The **fork tree** and per-node **fork_id** prevent this.

#### Fork Tree

`ForkTree` tracks parent-child relationships between fork IDs. Each chain
has a fork_id. When a chain diverges, a new fork_id is allocated as a child
of the parent chain's fork_id. When chains converge (deduplication), the
interior fork_id is absorbed into the surviving node.

#### `can_free` Predicate

During reaping, each deleted node is checked via `can_free(node_fork_id)`:

1. **Own allocation** (`node_fork_id == chain_fork_id`): always safe to free.
2. **Ancestor allocation**: safe only if no other active chain descends
   from the same ancestor fork_id.
3. **Pre-fork nodes** (`fork_id == 0`): not freed while multiple chains
   are active. Freed after convergence to a single chain.

The `can_free` closure is constructed from a snapshot of the fork tree
and active fork IDs at commit time, **before** any fork tree mutations
(removals) that would invalidate `is_ancestor` lookups. The fork tree
is wrapped in `Arc<ForkTree>`, so snapshots are cheap reference-count
increments rather than deep clones. Mutations use `Arc::make_mut()` for
copy-on-write semantics.

#### Safe Leak Strategy

Nodes that cannot be safely freed are skipped during reaping. This is a
conservative approach: space may be temporarily leaked, but data integrity
is preserved. After chains converge, all leaked space becomes reclaimable.

### 5.5 Space Leakage

| Scenario                             | Leaked space                              |
|--------------------------------------|-------------------------------------------|
| All honest                           | Zero                                      |
| 1 divergent, D blocks                | ~D blocks of trie modifications (KB-MB)   |
| After validator eviction + cleanup   | Full recovery                             |

---

## 6. Divergence Logging

### 6.1 Log Events

Firewood emits structured log events when divergence is detected:

```rust
log::warn!(
    "validator {id:?} diverged: parent={parent_hash:?}, \
     expected={expected_hash:?}, got={actual_hash:?}"
);
```

### 6.2 Metrics

New Prometheus-style metrics (using the existing `firewood_metrics` macros):

| Metric                                  | Type    | Description                                            |
|-----------------------------------------|---------|--------------------------------------------------------|
| `firewood_validator_divergence_total`   | Counter | Total divergence events per validator                  |
| `firewood_validator_chain_shared`       | Gauge   | 1 if validator is on the shared chain, 0 if diverged   |
| `firewood_validator_head_block`         | Gauge   | Current head position per validator                    |

### 6.3 Query API

```rust
impl MultiDb {
    /// Returns whether a validator is currently on the shared (majority)
    /// chain or has diverged.
    pub fn is_validator_diverged(&self, id: ValidatorId) -> Result<bool, Error>;

    /// Returns divergence information for all validators.
    pub fn validator_statuses(&self) -> Result<Vec<(ValidatorId, ValidatorChainStatus)>, Error>;
}

pub enum ValidatorChainStatus {
    /// Validator is on the shared chain (same root hash as majority).
    Shared,
    /// Validator has diverged. Contains the block height and hash where
    /// divergence was first detected.
    Diverged {
        diverged_at_parent: HashKey,
        expected_hash: HashKey,
        actual_hash: HashKey,
    },
}
```

This information is consumed by the external billing system to charge
divergent validators the full base cost.

---

## 7. Persistence and Crash Recovery

### 7.1 Persistence

Each validator's root is stored in the header. Updated on every commit or
advance operation.

When deferred persistence is enabled (`commit_count > 1`), the persist
worker accumulates committed revisions from all chains in a `Vec` between
persist cycles. Each cycle persists every accumulated revision, ensuring
that revisions from different chains are not lost between cycles.

### 7.2 Crash Recovery

On startup, read N validator roots from header. Reconstruct each head.
Dedup across validators with matching hashes. Each validator recovers to
its last persisted root.

### 7.3 Flush Ordering

1. Nodes written to disk first
2. Header (with validator roots) written last
3. Crash between 1 and 2: header has previous valid state

---

## 8. Error Handling

### 8.1 New Error Variants

| Error                                                  | When                                   |
|--------------------------------------------------------|----------------------------------------|
| `ValidatorNotFound { id }`                             | Operation on unregistered validator    |
| `ValidatorAlreadyRegistered { id }`                    | Re-registering active validator        |
| `MaxValidatorsReached { id, max }`                     | All header slots full                  |
| `NotValidatorHead { validator, provided, expected }`   | Parent mismatch                        |
| `ValidatorDeregistered { id }`                         | Operation on removed validator         |

### 8.2 Constraints

- No `unwrap()` / `expect()` / `panic!()` outside of tests
- Every failure mode has its own error variant
- All `Result` types propagated correctly

---

## 9. API

### 9.1 Rust API

```rust
pub struct MultiDb { ... }

impl MultiDb {
    pub fn new(path: impl AsRef<Path>, cfg: MultiDbConfig) -> Result<Self, Error>;
    pub fn register_validator(&self, id: ValidatorId) -> Result<(), Error>;
    pub fn deregister_validator(&self, id: ValidatorId) -> Result<(), Error>;
    pub fn propose(&self, id: ValidatorId, batch: impl IntoBatchIter) -> Result<Proposal, Error>;
    pub fn commit(&self, id: ValidatorId, proposal: Proposal) -> Result<(), Error>;
    pub fn advance_to_hash(&self, id: ValidatorId, hash: HashKey) -> Result<(), Error>;
    pub fn validator_view(&self, id: ValidatorId) -> Result<impl DbView, Error>;
    pub fn view(&self, hash: HashKey) -> Result<impl DbView, Error>;
    pub fn is_validator_diverged(&self, id: ValidatorId) -> Result<bool, Error>;
    pub fn validator_statuses(&self) -> Result<Vec<(ValidatorId, ValidatorChainStatus)>, Error>;
    pub fn close(self) -> Result<(), Error>;
}
```

### 9.2 FFI API (Rust `extern "C"` Functions)

**Database lifecycle:**

| Function             | Returns             | Description                          |
|----------------------|---------------------|--------------------------------------|
| `fwd_multi_open_db`  | `MultiHandleResult` | Open/create multi-validator database |
| `fwd_multi_close_db` | `VoidResult`        | Close database, free handle          |

**Validator management:**

| Function                          | Returns      | Description              |
|-----------------------------------|--------------|--------------------------|
| `fwd_multi_register_validator`    | `VoidResult` | Register a new validator |
| `fwd_multi_deregister_validator`  | `VoidResult` | Deregister a validator   |

**Database operations (validator-scoped):**

| Function                      | Returns               | Description                          |
|-------------------------------|-----------------------|--------------------------------------|
| `fwd_multi_get`               | `ValueResult`         | Get value from validator's head      |
| `fwd_multi_root_hash`         | `HashResult`          | Get root hash of validator's head    |
| `fwd_multi_latest_revision`   | `RevisionResult`      | Get read-only view at validator head |
| `fwd_multi_revision`          | `RevisionResult`      | Get historical revision by hash      |
| `fwd_multi_propose`           | `MultiProposalResult` | Create proposal for validator        |
| `fwd_multi_update`            | `HashResult`          | Propose + commit in one call         |
| `fwd_multi_advance_to_hash`   | `VoidResult`          | Skip-propose: advance to known hash  |
| `fwd_multi_db_dump`           | `ValueResult`         | Dump validator's trie (Graphviz DOT) |

**Proposal operations:**

| Function                         | Returns               | Description                          |
|----------------------------------|-----------------------|--------------------------------------|
| `fwd_multi_commit_proposal`      | `HashResult`          | Commit and consume proposal          |
| `fwd_multi_free_proposal`        | `VoidResult`          | Drop proposal without committing     |
| `fwd_multi_propose_on_proposal`  | `MultiProposalResult` | Create child proposal                |
| `fwd_multi_get_from_proposal`    | `ValueResult`         | Read from proposal                   |
| `fwd_multi_iter_on_proposal`     | `IteratorResult`      | Create iterator on proposal          |
| `fwd_multi_proposal_dump`        | `ValueResult`         | Dump proposal's trie (Graphviz DOT)  |

### 9.3 Go API

The Go wrapper provides two primary types in the `ffi` package:

**`MultiDatabase`** -- wraps `MultiDatabaseHandle`:

```go
func NewMulti(dbDir string, algo NodeHashAlgorithm, maxValidators uint, opts ...Option) (*MultiDatabase, error)
func (db *MultiDatabase) Close(ctx context.Context) error
func (db *MultiDatabase) RegisterValidator(id ValidatorID) error
func (db *MultiDatabase) DeregisterValidator(id ValidatorID) error
func (db *MultiDatabase) Get(id ValidatorID, key []byte) ([]byte, error)
func (db *MultiDatabase) Root(id ValidatorID) (Hash, error)
func (db *MultiDatabase) LatestRevision(id ValidatorID) (*Revision, error)
func (db *MultiDatabase) Revision(hash Hash) (*Revision, error)
func (db *MultiDatabase) Propose(id ValidatorID, batch []BatchOp) (*MultiProposal, error)
func (db *MultiDatabase) Update(id ValidatorID, batch []BatchOp) (Hash, error)
func (db *MultiDatabase) AdvanceToHash(id ValidatorID, hash Hash) error
func (db *MultiDatabase) Dump(id ValidatorID) (string, error)
```

**`MultiProposal`** -- wraps `MultiProposalHandle`:

```go
func (p *MultiProposal) Root() Hash
func (p *MultiProposal) Get(key []byte) ([]byte, error)
func (p *MultiProposal) Iter(key []byte) (*Iterator, error)
func (p *MultiProposal) Propose(batch []BatchOp) (*MultiProposal, error)
func (p *MultiProposal) Commit() error
func (p *MultiProposal) Drop() error
func (p *MultiProposal) Dump() (string, error)
```

### 9.4 FFI Memory Safety

All Rust-allocated objects crossing the FFI boundary are freed exactly once:

| Rust Object                | Go Receives Via                    | Freed By                                              | Finalizer |
|----------------------------|------------------------------------|-------------------------------------------------------|-----------|
| `Box<MultiDatabaseHandle>` | `getMultiDatabaseFromHandleResult` | `Close()` via `fwd_multi_close_db`                    | No        |
| `Box<MultiProposalHandle>` | `getMultiProposalFromResult`       | `Commit()` or finalizer via `fwd_multi_free_proposal` | Yes       |
| `Box<RevisionHandle>`      | `getRevisionFromResult`            | `Drop()` via `fwd_free_revision` or finalizer         | Yes       |
| `OwnedBytes` (errors)      | `newOwnedBytes().intoError()`      | Copied to Go heap, then `fwd_free_owned_bytes`        | N/A       |
| `OwnedBytes` (values)      | `getValueFromValueResult`          | `CopiedBytes()` then `fwd_free_owned_bytes`           | N/A       |
| `HashKey`                  | `getHashKeyFromHashResult`         | Stack-allocated 32-byte array, no freeing needed      | N/A       |

The `handle[T]` generic wrapper (used by `MultiProposal` and `Revision`)
registers `runtime.AddCleanup` finalizers to prevent leaks if `Drop()` or
`Commit()` is not called explicitly. The finalizer checks `dropped` to
avoid double-frees.

---

## 10. Concurrency

### 10.1 Lock Inventory

| Lock          | Type                              | Guards                                                              |
|---------------|-----------------------------------|---------------------------------------------------------------------|
| `multi_head`  | `RwLock`                          | `MultiHeadState` (validators, chains, by\_hash, hash\_to\_chain)    |
| `header`      | `Mutex` (inside `persist_worker`) | On-disk header (validator roots)                                    |
| `proposals`   | `Mutex`                           | Live proposal list                                                  |

### 10.2 Lock Ordering

Locks are **never nested**. The protocol is:

1. Acquire `multi_head` write lock.
2. Perform in-memory state update. Collect any revisions to reap.
3. **Release** `multi_head` lock.
4. Acquire `header` lock. Update validator root slots. Release.
5. Send collected revisions to the persist worker (no lock held).
6. Acquire `proposals` lock. Clean up stale proposals. Release.

This ordering eliminates deadlock: no code path holds two locks simultaneously.

### 10.3 Split Critical Section in `commit_for_validator`

`commit_for_validator` uses a **3-phase** approach:

- **Phase 1 (write lock):** Validate parent, dedup check, update in-memory
  validator head and chain state, collect reapable revisions. Release lock.
- **Phase 2 (no lock):** Persist revision to disk, update header, reap
  collected revisions via `Arc::try_unwrap`.
- **Phase 3 (proposals lock):** Remove committed proposal and discard any
  proposals that are no longer referenced.

The write lock in Phase 1 is short (in-memory only). The potentially blocking
disk I/O in Phase 2 runs lock-free.

### 10.4 Read Paths

`validator_view`, `revision()`, and divergence queries take a **read lock**
on `multi_head`. Multiple readers proceed concurrently. `revision()` now also
checks `multi_head.by_hash` after checking `in_memory_revisions`, so views
created from any chain are accessible.

### 10.5 Register and Deregister

`register_validator` and `deregister_validator` take the `multi_head` write
lock, update in-memory state, release the lock, then update the header under
the header lock. The same lock ordering (multi\_head → header) applies.

---

## 11. Configuration

| Parameter          | Default | Description                                      |
|--------------------|---------|--------------------------------------------------|
| `max_validators`   | 16      | Max validators (header size limit)               |
| `max_revisions`    | 128     | Max revisions in pool (>= max validator spread)  |

---

## 12. Invariants

1. A validator's head always points to a valid, persisted revision.
2. `by_hash` contains every committed revision accessible by any validator.
3. Revisions with the same root hash have identical trie content
   (guaranteed by deterministic Merkle hashing).
4. Reaping never frees nodes still referenced by any validator's trie.
5. The header reflects the latest persisted state for each validator.
6. `validator_count == 0` means legacy single-root mode.
7. Divergence events are always logged and reflected in metrics.
8. Every hash in `by_hash` has a corresponding entry in `hash_to_chain`, and
   vice versa. These two maps are always mutually consistent.
9. Every `ValidatorState.chain` refers to a `ChainId` that exists in `chains`.
   No validator points to a chain that has been removed.
10. No lock is ever held when acquiring another lock. Locks are acquired and
    released in strict sequence: `multi_head` → `header` → `proposals`.

---

## 13. Testing Strategy

### Test Categories

| Category        | Tests    | Focus                                                                                                    |
|-----------------|----------|----------------------------------------------------------------------------------------------------------|
| Registration    | 7        | Lifecycle, limits, errors                                                                                |
| Commit          | 7        | Normal, dedup, wrong parent, nonexistent validator                                                       |
| Deduplication   | 4        | Arc sharing, N-way dedup, isolation                                                                      |
| Skip-Propose    | 3        | Advance existing/nonexistent, no-compute                                                                 |
| Reaping         | 5        | Min-head frontier, shared vs divergent, post-deregister                                                  |
| Divergence      | 5        | Detection, logging, metrics, does not affect honest chains                                               |
| Views           | 2        | Correct head, cross-validator lookup                                                                     |
| Concurrency     | 13       | Parallel commits, commit+advance race, dedup race, per-chain reap race, register/deregister under load   |
| Header          | 8        | Size, roundtrip, backward compat, validation                                                             |
| Integration     | 6        | Create/close, persist/recover, migration, max validators                                                 |
| FFI             | 5        | Lifecycle, register, propose/commit, advance, null handle                                                |
| **Total**       | **~55**  |                                                                                                          |

---

## 14. Scope

### Modified Files

| File                                 | Lines        | Changes                                                            |
|--------------------------------------|--------------|--------------------------------------------------------------------|
| `storage/src/nodestore/header.rs`    | ~150         | ValidatorRoot, N root slots, validation                            |
| `firewood/src/manager.rs`            | ~700         | MultiHeadState, commit/advance/register/reap, divergence detection |
| `firewood/src/db.rs`                 | ~400         | MultiDb, per-validator API, divergence query                       |
| `firewood/src/v2/api.rs`             | ~120         | ValidatorId, ValidatorChainStatus, error variants                  |
| `firewood/src/persist_worker.rs`     | ~100         | Per-validator header root updates                                  |
| `firewood/src/registry.rs`           | ~30          | New metrics for divergence                                         |
| `ffi/src/`                           | ~200         | FFI bindings                                                       |
| **Implementation total**             | **~1,700**   |                                                                    |
| Tests                                | ~1,300       | 55 tests                                                           |
| **Grand total**                      | **~3,000**   |                                                                    |

### Fork ID Files (On-Disk Fork ID Per Node)

| File                                  | Changes                                                                           |
|---------------------------------------|-----------------------------------------------------------------------------------|
| `storage/src/nodestore/primitives.rs` | `from_raw_byte`, `with_fork_id_flag` on AreaIndex                                 |
| `storage/src/nodestore/mod.rs`        | `area_index_and_size` update, `read_fork_id_from_disk`, fork-aware `reap_deleted` |
| `storage/src/nodestore/persist.rs`    | `serialize_node_to_bump` fork\_id insertion                                       |
| `firewood/src/merkle/parallel.rs`     | Pass fork\_id through parallel proposal creation                                  |

### Unmodified Files

- `storage/src/nodestore/alloc.rs` -- Allocator unchanged (uses updated `area_index_and_size`)
- `storage/src/linear/filebacked.rs` -- Storage layer unchanged
- `storage/src/checker/mod.rs` -- Checker unchanged (delegates to updated read/size functions)
