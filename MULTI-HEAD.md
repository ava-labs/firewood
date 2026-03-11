# Multi-Head Revision Manager: Detailed Implementation Plan

## Overview

Add support for N simultaneous validator heads in the `RevisionManager`,
enabling multiple Avalanche validators on the same chain to share a single
Firewood deployment. Identical revisions (same root hash) are deduplicated
-- stored once, referenced by all validators that produced them.

No consensus logic in Firewood. Every validator's chain is persisted.
Economic incentives outside Firewood discourage malicious behavior.

See [MULTI-HEAD-DESIGN.md](./MULTI-HEAD-DESIGN.md) for the full design
document including motivation, architecture, and API reference.

## Implementation Status

| Phase   | Description                                                       | Status |
|---------|-------------------------------------------------------------------|--------|
| Phase 1 | Error types (`RevisionManagerError`, `api::Error`, `ValidatorId`) | Done   |
| Phase 2 | Header format extension (`ValidatorRoot`, validation)             | Done   |
| Phase 3 | Multi-head `RevisionManager` (state, commit, reap, chains)        | Done   |
| Phase 4 | `MultiDb` public API (`db.rs`)                                    | Done   |
| Phase 5 | FFI bindings (Rust `extern "C"` + Go wrappers)                    | Done   |
| Phase 6 | Tests (Rust unit/integration + Go FFI tests)                      | Done   |
| Phase 7 | On-disk fork ID per node (cross-chain reaping safety)             | Done   |

## Implementation Constraints

- No `unwrap()`/`expect()`/`panic!()` outside of tests
- All cases handled explicitly (no silent failures)
- Correct error types for every failure mode (new errors when no existing match)
- Exhaustive test battery

---

## Phase 1: New Error Types

### File: `firewood/src/manager.rs`

Add new variants to `RevisionManagerError`:

```rust
#[derive(Debug, thiserror::Error)]
pub(crate) enum RevisionManagerError {
    // ... existing variants ...

    #[error("Validator {id:?} is not registered")]
    ValidatorNotFound { id: ValidatorId },

    #[error("Validator {id:?} is already registered")]
    ValidatorAlreadyRegistered { id: ValidatorId },

    #[error(
        "Maximum number of validators ({max}) reached; cannot register validator {id:?}"
    )]
    MaxValidatorsReached { id: ValidatorId, max: usize },

    #[error(
        "Proposal parent {provided:?} does not match validator {validator:?} head {expected:?}"
    )]
    NotValidatorHead {
        validator: ValidatorId,
        provided: Option<HashKey>,
        expected: Option<HashKey>,
    },

    #[error("Validator {id:?} has already been deregistered")]
    ValidatorDeregistered { id: ValidatorId },
}
```

### File: `firewood/src/v2/api.rs`

Add new variants to `api::Error`:

```rust
pub enum Error {
    // ... existing variants ...

    #[error("Validator {id:?} not found")]
    ValidatorNotFound { id: ValidatorId },

    #[error("Validator {id:?} already registered")]
    ValidatorAlreadyRegistered { id: ValidatorId },

    #[error("Maximum validators ({max}) reached")]
    MaxValidatorsReached { id: ValidatorId, max: usize },

    #[error("Parent does not match validator {validator:?} head")]
    NotValidatorHead {
        validator: ValidatorId,
        provided: Option<HashKey>,
        expected: Option<HashKey>,
    },

    #[error("Validator {id:?} has been deregistered")]
    ValidatorDeregistered { id: ValidatorId },
}
```

Add `From<RevisionManagerError>` mappings for the new variants.

### File: `firewood/src/v2/api.rs` (new type)

```rust
/// Identifies a validator within a multi-head Firewood deployment.
/// Corresponds to a root slot in the header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ValidatorId(u8);

impl ValidatorId {
    pub fn new(id: u8) -> Self {
        Self(id)
    }

    pub fn as_u8(self) -> u8 {
        self.0
    }
}
```

---

## Phase 2: Header Format Extension

### File: `storage/src/nodestore/header.rs`

#### New Types

```rust
/// Maximum number of validator roots supported in the header.
/// 16 validators * 40 bytes/root = 640 bytes, fits within header padding.
pub const MAX_VALIDATORS: usize = 16;

/// A per-validator root entry in the header.
#[derive(Debug, Clone, Copy, Zeroable, Pod)]
#[repr(C)]
pub struct ValidatorRoot {
    /// Disk address of this validator's trie root, or 0 for empty/unused.
    root_address: u64,  // 0 = None (cannot use Option with Pod)
    /// Merkle root hash of this validator's trie.
    root_hash: [u8; 32],
}
```

#### Header Struct Extension

Add fields to `NodeStoreHeader` (within the existing 2048-byte budget):

```rust
pub struct NodeStoreHeader {
    // ... existing fields (392 bytes) ...

    /// Number of active validator roots. 0 = legacy single-root mode.
    validator_count: u64,
    /// Per-validator root slots.
    validator_roots: [ValidatorRoot; MAX_VALIDATORS],
}
```

Compile-time assertion: `size_of::<NodeStoreHeader>() <= 2048`.

#### New Methods on `NodeStoreHeader`

```rust
impl NodeStoreHeader {
    /// Returns the number of active validator roots.
    pub fn validator_count(&self) -> usize {
        self.validator_count as usize
    }

    /// Returns the root info for a validator slot, or None if the slot is
    /// unused or out of range.
    pub fn validator_root(&self, slot: u8) -> Option<RootNodeInfo> {
        let slot = slot as usize;
        if slot >= self.validator_count as usize {
            return None;
        }
        let vr = &self.validator_roots[slot];
        let addr = LinearAddress::new(vr.root_address)?;
        Some((addr, TrieHash::from(vr.root_hash)))
    }

    /// Sets the root for a validator slot.
    ///
    /// # Errors
    ///
    /// Returns an error if the slot index is out of range.
    pub fn set_validator_root(
        &mut self,
        slot: u8,
        root: Option<RootNodeInfo>,
    ) -> Result<(), Error> {
        let slot_idx = slot as usize;
        if slot_idx >= MAX_VALIDATORS {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("validator slot {slot} exceeds maximum {MAX_VALIDATORS}"),
            ));
        }
        match root {
            Some((addr, hash)) => {
                self.validator_roots[slot_idx] = ValidatorRoot {
                    root_address: addr.get(),
                    root_hash: hash.into(),
                };
            }
            None => {
                self.validator_roots[slot_idx] = ValidatorRoot::zeroed();
            }
        }
        Ok(())
    }

    /// Sets the validator count.
    pub fn set_validator_count(&mut self, count: usize) {
        self.validator_count = count as u64;
    }
}
```

#### Validation Extension

Add to `validate()`:

```rust
fn validate(&self, expected_algo: NodeHashAlgorithm) -> Result<(), Error> {
    self.version.validate()?;
    self.validate_endian_test()?;
    self.validate_area_size_hash()?;
    self.validate_node_hash_algorithm(expected_algo)?;
    self.validate_validator_count()?;  // NEW
    Ok(())
}

fn validate_validator_count(&self) -> Result<(), Error> {
    if self.validator_count as usize > MAX_VALIDATORS {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "validator_count {} exceeds maximum {}",
                self.validator_count, MAX_VALIDATORS
            ),
        ));
    }
    Ok(())
}
```

#### Backward Compatibility

When `validator_count == 0`:

- The existing `root_address` and `root_hash` fields are the sole root (legacy mode)
- `validator_roots` array contains zeros (Pod default), which is ignored
- All existing databases open without modification

When opening a legacy database and registering validators:

- Set `validator_count` to the number of validators
- Copy `root_address`/`root_hash` into `validator_roots[0]` for the first validator

---

## Phase 3: Multi-Head Revision Manager

### File: `firewood/src/manager.rs` (Multi-Head State)

#### New Data Structures

```rust
/// Identifies a chain (lineage of revisions) within the multi-head manager.
type ChainId = u64;

/// Per-chain revision tracking.
#[derive(Debug)]
struct ChainState {
    /// Revisions on this chain, ordered oldest-to-newest.
    revisions: VecDeque<CommittedRevision>,
}

/// Per-validator state within the multi-head revision manager.
#[derive(Debug)]
struct ValidatorState {
    /// This validator's latest committed revision.
    head: CommittedRevision,
    /// Slot index in the header (0..MAX_VALIDATORS).
    slot: u8,
    /// Which chain this validator is currently on.
    chain: ChainId,
}

/// Tracks all validator heads and per-chain revision pools.
#[derive(Debug)]
struct MultiHeadState {
    /// Per-validator state, keyed by ValidatorId.
    validators: HashMap<ValidatorId, ValidatorState>,
    /// Per-chain revision deques (replaces the single global pool).
    chains: HashMap<ChainId, ChainState>,
    /// O(1) hash-based lookup into any chain's revision pool.
    by_hash: HashMap<TrieHash, CommittedRevision>,
    /// Reverse index: hash → chain that owns this revision.
    hash_to_chain: HashMap<TrieHash, ChainId>,
    /// Maximum number of validators allowed.
    max_validators: usize,
    /// Monotonic counter for allocating new chain IDs.
    next_chain_id: ChainId,
}
```

#### Integration with `RevisionManager`

Add `multi_head: Option<RwLock<MultiHeadState>>` to `RevisionManager`.
When `multi_head` is `None`, behavior is identical to today (single-head mode).

The existing `in_memory_revisions`, `by_hash`, and commit logic remain for
single-head mode. Multi-head mode uses `MultiHeadState` instead.

#### Key Methods

##### `register_validator`

```rust
pub fn register_validator(
    &self,
    id: ValidatorId,
) -> Result<(), RevisionManagerError> {
    let mut state = self.multi_head
        .as_ref()
        .ok_or(RevisionManagerError::ValidatorNotFound { id })?
        .write();

    if state.validators.contains_key(&id) {
        return Err(RevisionManagerError::ValidatorAlreadyRegistered { id });
    }

    let slot = state.next_available_slot()
        .ok_or(RevisionManagerError::MaxValidatorsReached {
            id,
            max: MAX_VALIDATORS,
        })?;

    // New validator starts at the tip of the most-populated chain.
    // Find the chain with the most validators (or chain 0 as fallback).
    let (tip_chain_id, base) = {
        let mut chain_counts: HashMap<ChainId, usize> = HashMap::new();
        for v in state.validators.values() {
            *chain_counts.entry(v.chain).or_insert(0) += 1;
        }
        if let Some((&cid, _)) = chain_counts.iter().max_by_key(|(_, &c)| c) {
            let rev = state.chains[&cid].revisions.back().cloned()
                .unwrap_or_else(|| self.current_revision());
            (cid, rev)
        } else {
            // No existing validators; start from current revision on chain 0
            let rev = state.chains.values()
                .next()
                .and_then(|c| c.revisions.back().cloned())
                .unwrap_or_else(|| self.current_revision());
            (0, rev)
        }
    };

    state.validators.insert(id, ValidatorState { head: base, slot, chain: tip_chain_id });
    Ok(())
}
```

##### `deregister_validator`

Uses the same split pattern: write lock for in-memory update, then header
lock released separately.

```rust
pub fn deregister_validator(
    &self,
    id: ValidatorId,
) -> Result<(), RevisionManagerError> {
    let (slot, validator_count, revisions_to_reap) = {
        let mut state = self.multi_head
            .as_ref()
            .ok_or(RevisionManagerError::ValidatorNotFound { id })?
            .write();

        let removed = state.validators.remove(&id)
            .ok_or(RevisionManagerError::ValidatorNotFound { id })?;

        // Clean up the chain if now empty
        let revisions_to_reap =
            if !state.validators.values().any(|v| v.chain == removed.chain) {
                self.collect_all_chain_revisions(removed.chain, &mut state)
            } else {
                Vec::new()
            };

        let validator_count = state.validators.len();
        (removed.slot, validator_count, revisions_to_reap)
    }; // write lock dropped

    // Clear this validator's root in the header (no multi_head lock held)
    let mut header = self.persist_worker.locked_header();
    header.set_validator_root(slot, None)?;
    header.set_validator_count(validator_count);
    drop(header);

    // Reap orphaned revisions (no locks held)
    for rev in revisions_to_reap {
        if let Ok(owned) = Arc::try_unwrap(rev) {
            self.persist_worker.reap(owned)
                .map_err(RevisionManagerError::PersistError)?;
        }
    }

    Ok(())
}
```

##### `commit_for_validator`

Uses a **3-phase split critical section** to minimize lock contention while
maintaining correctness. No two locks are held simultaneously.

```rust
pub fn commit_for_validator(
    &self,
    id: ValidatorId,
    proposal: ProposedRevision,
) -> Result<(), RevisionManagerError> {
    // Pre-check persist worker health (no lock)
    self.persist_worker
        .check_error()
        .map_err(RevisionManagerError::PersistError)?;

    // ---------------------------------------------------------------
    // Phase 1: Under multi_head write lock (fast, in-memory only)
    // ---------------------------------------------------------------
    let (committed, slot, root_info, revisions_to_reap) = {
        let mut state = self.multi_head
            .as_ref()
            .ok_or(RevisionManagerError::ValidatorNotFound { id })?
            .write();

        let validator = state.validators.get(&id)
            .ok_or(RevisionManagerError::ValidatorNotFound { id })?;

        // 1. Parent check
        if !proposal.parent_hash_is(validator.head.root_hash()) {
            return Err(RevisionManagerError::NotValidatorHead {
                validator: id,
                provided: proposal.root_hash(),
                expected: validator.head.root_hash(),
            });
        }

        let new_hash = proposal.root_hash().or_default_root_hash();
        let validator_chain = validator.chain;

        // 2. Deduplication: does a revision with this hash already exist?
        if let Some(ref hash) = new_hash {
            if let Some(existing) = state.by_hash.get(hash).cloned() {
                // Move validator to the chain that owns the existing revision.
                // Clean up old chain if now empty.
                let target_chain = *state.hash_to_chain.get(hash).unwrap();
                let validator = state.validators.get_mut(&id).unwrap();
                validator.head = existing;
                validator.chain = target_chain;
                let root_info = self.root_info_for_validator(validator);
                let slot = validator.slot;
                let revisions_to_reap =
                    if validator_chain != target_chain
                        && !state.validators.values().any(|v| v.chain == validator_chain)
                    {
                        self.collect_all_chain_revisions(validator_chain, &mut state)
                    } else {
                        Vec::new()
                    };
                drop(state); // release write lock

                // Phase 2 (dedup path): update header, reap orphaned chain
                let mut header = self.persist_worker.locked_header();
                header.set_validator_root(slot, root_info)?;
                drop(header);
                for rev in revisions_to_reap {
                    if let Ok(owned) = Arc::try_unwrap(rev) {
                        self.persist_worker.reap(owned)?;
                    }
                }
                // Phase 3 (proposals lock): cleanup
                self.cleanup_proposals_inner(&proposal);
                for p in &*self.proposals.lock() { proposal.commit_reparent(p); }
                return Ok(());
            }
        }

        // 3. First validator to produce this hash.
        let committed: CommittedRevision = proposal.as_committed().into();

        // 4. Fork if the new revision is divergent (different hash from
        //    what other validators on this chain have already committed).
        //    Append to existing chain otherwise.
        let target_chain_id = /* fork or reuse validator_chain */ {
            let is_divergent = /* check if chain tip differs */ false; // simplified
            if is_divergent {
                let new_id = state.next_chain_id;
                state.next_chain_id = state.next_chain_id.wrapping_add(1);
                state.chains.insert(new_id, ChainState {
                    revisions: VecDeque::from([committed.clone()]),
                });
                new_id
            } else {
                state.chains.get_mut(&validator_chain).unwrap()
                    .revisions.push_back(committed.clone());
                validator_chain
            }
        };

        if let Some(ref hash) = new_hash {
            state.by_hash.insert(hash.clone(), committed.clone());
            state.hash_to_chain.insert(hash.clone(), target_chain_id);
        }

        let validator = state.validators.get_mut(&id).unwrap();
        validator.head = committed.clone();
        validator.chain = target_chain_id;
        let root_info = self.root_info_for_validator(validator);
        let slot = validator.slot;

        // Collect revisions eligible for reaping on this chain
        let revisions_to_reap =
            self.collect_reapable_revisions(target_chain_id, &mut state);

        (committed, slot, root_info, revisions_to_reap)
    }; // write lock dropped

    // ---------------------------------------------------------------
    // Phase 2: No lock held (disk I/O may block)
    // ---------------------------------------------------------------
    self.persist_worker
        .persist(committed.clone())
        .map_err(RevisionManagerError::PersistError)?;

    let mut header = self.persist_worker.locked_header();
    header.set_validator_root(slot, root_info.clone())?;
    header.set_root_location(root_info); // legacy compat
    drop(header);

    for rev in revisions_to_reap {
        if let Ok(owned) = Arc::try_unwrap(rev) {
            self.persist_worker
                .reap(owned)
                .map_err(RevisionManagerError::PersistError)?;
        }
    }

    // ---------------------------------------------------------------
    // Phase 3: Under proposals lock (short)
    // ---------------------------------------------------------------
    self.cleanup_proposals_inner(&proposal);
    for p in &*self.proposals.lock() {
        proposal.commit_reparent(p);
    }

    Ok(())
}
```

##### `advance_validator_to_hash`

Skip-propose optimization: advance a validator's head to an existing revision
without computing a proposal. Uses `hash_to_chain` for O(1) chain lookup.
If the validator moves to a different chain, the vacated chain is cleaned up
if it becomes empty.

```rust
pub fn advance_validator_to_hash(
    &self,
    id: ValidatorId,
    hash: HashKey,
) -> Result<(), RevisionManagerError> {
    let (slot, root_info, revisions_to_reap) = {
        let mut state = self.multi_head
            .as_ref()
            .ok_or(RevisionManagerError::ValidatorNotFound { id })?
            .write();

        if !state.validators.contains_key(&id) {
            return Err(RevisionManagerError::ValidatorNotFound { id });
        }

        let existing = state.by_hash.get(&hash)
            .ok_or(RevisionManagerError::RevisionNotFound { provided: hash.clone() })?
            .clone();

        // O(1) lookup of the chain that owns this revision
        let target_chain = state.hash_to_chain.get(&hash).copied()
            .ok_or_else(|| RevisionManagerError::IOError(
                io::Error::other("hash_to_chain inconsistent in advance_validator_to_hash")
            ))?;

        let source_chain = state.validators.get(&id)
            .ok_or(RevisionManagerError::ValidatorNotFound { id })?
            .chain;

        let validator = state.validators.get_mut(&id)
            .ok_or(RevisionManagerError::ValidatorNotFound { id })?;
        validator.head = existing;
        validator.chain = target_chain;
        let root_info = self.root_info_for_validator(validator);
        let slot = validator.slot;

        // Clean up source chain if now empty of validators
        let revisions_to_reap = if source_chain != target_chain
            && !state.validators.values().any(|v| v.chain == source_chain)
        {
            self.collect_all_chain_revisions(source_chain, &mut state)
        } else {
            Vec::new()
        };

        (slot, root_info, revisions_to_reap)
    }; // write lock dropped

    // Update header (no multi_head lock held)
    let mut header = self.persist_worker.locked_header();
    header.set_validator_root(slot, root_info)?;
    drop(header);

    // Reap collected revisions (no locks held)
    for rev in revisions_to_reap {
        if let Ok(owned) = Arc::try_unwrap(rev) {
            self.persist_worker.reap(owned)
                .map_err(RevisionManagerError::PersistError)?;
        }
    }

    Ok(())
}
```

##### `validator_view`

Return a view at a specific validator's current head.

```rust
pub fn validator_view(
    &self,
    id: ValidatorId,
) -> Result<CommittedRevision, RevisionManagerError> {
    let state = self.multi_head
        .as_ref()
        .ok_or(RevisionManagerError::ValidatorNotFound { id })?
        .read();

    state.validators.get(&id)
        .map(|v| v.head.clone())
        .ok_or(RevisionManagerError::ValidatorNotFound { id })
}
```

##### `collect_reapable_revisions` (per-chain budget enforcement)

Pops revisions from the front of a chain's deque when the chain exceeds
`max_revisions` and the revision is older than the minimum head on that chain.
Returns the collected revisions for reaping **outside** the lock.

```rust
fn collect_reapable_revisions(
    &self,
    chain_id: ChainId,
    state: &mut MultiHeadState,
) -> Vec<CommittedRevision> {
    let Some(chain) = state.chains.get_mut(&chain_id) else {
        return Vec::new(); // Chain already cleaned up
    };

    // Minimum head position among validators on THIS chain
    let min_pos = state.validators.values()
        .filter(|v| v.chain == chain_id)
        .filter_map(|v| chain.revisions.iter().position(|r| Arc::ptr_eq(r, &v.head)))
        .min()
        .unwrap_or(0);

    let mut collected = Vec::new();
    let mut reaped = 0;
    while chain.revisions.len() > self.max_revisions && reaped < min_pos {
        let Some(oldest) = chain.revisions.pop_front() else { break };

        if let Some(hash) = oldest.root_hash().or_default_root_hash() {
            let still_referenced = state.validators.values()
                .any(|v| v.head.root_hash().or_default_root_hash().as_ref() == Some(&hash));
            if !still_referenced {
                state.by_hash.remove(&hash);
                state.hash_to_chain.remove(&hash);
            }
        }
        collected.push(oldest);
        reaped = reaped.wrapping_add(1);
    }
    collected
}
```

##### `collect_all_chain_revisions` (empty-chain cleanup)

Removes a chain entirely and returns all its revisions for reaping outside
the lock. Called when the last validator leaves a chain.

```rust
fn collect_all_chain_revisions(
    &self,
    chain_id: ChainId,
    state: &mut MultiHeadState,
) -> Vec<CommittedRevision> {
    let Some(mut chain) = state.chains.remove(&chain_id) else {
        return Vec::new(); // Already removed
    };

    let mut collected = Vec::new();
    while let Some(rev) = chain.revisions.pop_front() {
        if let Some(hash) = rev.root_hash().or_default_root_hash() {
            let still_referenced = state.validators.values()
                .any(|v| v.head.root_hash().or_default_root_hash().as_ref() == Some(&hash));
            if !still_referenced {
                state.by_hash.remove(&hash);
                state.hash_to_chain.remove(&hash);
            }
        }
        collected.push(rev);
    }
    collected
}
```

Both helpers run under the `multi_head` write lock and only collect revisions
into a `Vec`; actual disk I/O (`persist_worker.reap`) happens after the lock
is released, using `Arc::try_unwrap` as a safety gate.

#### Header Root Updates

```rust
fn update_header_validator_root(
    &self,
    id: ValidatorId,
    validator: &ValidatorState,
) -> Result<(), RevisionManagerError> {
    let root_info = validator.head.root_hash()
        .and_then(|hash| {
            validator.head.root_address()
                .map(|addr| (addr, hash))
        });

    self.locked_header_mut()
        .set_validator_root(validator.slot, root_info)
        .map_err(|e| RevisionManagerError::IOError(e))?;

    Ok(())
}
```

#### Crash Recovery

In `RevisionManager::new()`, after reading the header, per-chain structures
are reconstructed. Validators sharing the same root hash are placed on the
same chain; each unique hash gets its own chain.

```rust
if header.validator_count() > 0 {
    let mut multi_head = MultiHeadState {
        validators: HashMap::new(),
        chains: HashMap::new(),
        by_hash: HashMap::new(),
        hash_to_chain: HashMap::new(),
        max_validators,
        next_chain_id: 0,
    };

    for slot in 0..header.validator_count() {
        let slot_u8 = u8::try_from(slot)
            .map_err(|_| RevisionManagerError::IOError(
                io::Error::new(io::ErrorKind::InvalidData, "slot overflow")
            ))?;

        if let Some((addr, hash)) = header.validator_root(slot_u8) {
            let id = ValidatorId::new(slot_u8);

            // Dedup on recovery: reuse existing Arc if same hash seen before
            let (head, chain_id) = if let Some(existing) = multi_head.by_hash.get(&hash).cloned() {
                let chain_id = multi_head.hash_to_chain[&hash];
                (existing, chain_id)
            } else {
                let committed: CommittedRevision = Arc::new(
                    NodeStore::open_from_address(addr, &storage)?
                );
                let chain_id = multi_head.next_chain_id;
                multi_head.next_chain_id = multi_head.next_chain_id.wrapping_add(1);
                multi_head.chains.insert(chain_id, ChainState {
                    revisions: VecDeque::from([committed.clone()]),
                });
                multi_head.by_hash.insert(hash.clone(), committed.clone());
                multi_head.hash_to_chain.insert(hash, chain_id);
                (committed, chain_id)
            };

            multi_head.validators.insert(id, ValidatorState {
                head,
                slot: slot_u8,
                chain: chain_id,
            });
        }
    }

    // Store multi_head state in self.multi_head = Some(RwLock::new(multi_head));
}
```

---

## Phase 4: MultiDb API

### File: `firewood/src/db.rs`

```rust
/// Configuration for multi-validator mode.
#[derive(Clone, Debug, TypedBuilder)]
pub struct MultiDbConfig {
    /// Base database configuration.
    pub db: DbConfig,
    /// Maximum number of validators (1..=MAX_VALIDATORS).
    #[builder(default = MAX_VALIDATORS)]
    pub max_validators: usize,
}

/// A multi-validator Firewood database.
///
/// Wraps a single `Db` instance and provides per-validator
/// propose/commit/view operations with automatic deduplication.
#[derive(Debug)]
pub struct MultiDb {
    db: Db,
}

impl MultiDb {
    /// Create a new multi-validator database.
    pub fn new<P: AsRef<Path>>(
        db_dir: P,
        cfg: MultiDbConfig,
    ) -> Result<Self, api::Error> {
        let db = Db::new(db_dir, cfg.db)?;
        // Enable multi-head mode in the revision manager
        db.manager.enable_multi_head(cfg.max_validators)?;
        Ok(Self { db })
    }

    /// Register a new validator. Returns the assigned ValidatorId.
    ///
    /// The validator starts at the latest persisted revision.
    pub fn register_validator(
        &self,
        id: ValidatorId,
    ) -> Result<(), api::Error> {
        self.db.manager
            .register_validator(id)
            .map_err(Into::into)
    }

    /// Deregister a validator, freeing its header slot.
    ///
    /// Any revisions unique to this validator become candidates for cleanup.
    pub fn deregister_validator(
        &self,
        id: ValidatorId,
    ) -> Result<(), api::Error> {
        self.db.manager
            .deregister_validator(id)
            .map_err(Into::into)
    }

    /// Create a proposal for a validator from its current head.
    pub fn propose(
        &self,
        id: ValidatorId,
        batch: impl IntoBatchIter,
    ) -> Result<Proposal<'_>, api::Error> {
        let head = self.db.manager.validator_view(id)?;
        self.db.propose_with_parent(batch, &head)
    }

    /// Commit a proposal for a validator.
    ///
    /// If a revision with the same root hash already exists (committed by
    /// another validator), the proposal is discarded and the validator's
    /// head advances to the existing revision (deduplication).
    pub fn commit(
        &self,
        id: ValidatorId,
        proposal: Proposal<'_>,
    ) -> Result<(), api::Error> {
        self.db.manager
            .commit_for_validator(id, proposal.nodestore)
            .map_err(Into::into)
    }

    /// Advance a validator's head to an existing revision by hash.
    ///
    /// This is the skip-propose optimization: if a validator knows the
    /// expected root hash (e.g., from a block header), it can advance
    /// without computing a proposal.
    pub fn advance_to_hash(
        &self,
        id: ValidatorId,
        hash: HashKey,
    ) -> Result<(), api::Error> {
        self.db.manager
            .advance_validator_to_hash(id, hash)
            .map_err(Into::into)
    }

    /// Get a read-only view at a validator's current head.
    pub fn validator_view(
        &self,
        id: ValidatorId,
    ) -> Result<CommittedRevision, api::Error> {
        self.db.manager
            .validator_view(id)
            .map_err(Into::into)
    }

    /// Get a read-only view at any committed revision by hash.
    pub fn view(&self, hash: HashKey) -> Result<ArcDynDbView, api::Error> {
        self.db.view(hash)
    }

    /// Close the database gracefully.
    pub fn close(self) -> Result<(), api::Error> {
        self.db.close()
    }
}
```

---

## Phase 5: FFI Bindings

### File: `ffi/src/`

Add FFI functions for multi-validator operations:

- `firewood_multi_db_new(path, config) -> HandleResult`
- `firewood_register_validator(handle, validator_id) -> VoidResult`
- `firewood_deregister_validator(handle, validator_id) -> VoidResult`
- `firewood_validator_propose(handle, validator_id, batch) -> ProposalResult`
- `firewood_validator_commit(handle, validator_id, proposal) -> VoidResult`
- `firewood_validator_advance(handle, validator_id, hash) -> VoidResult`
- `firewood_validator_view(handle, validator_id) -> RevisionResult`

Each follows the existing `invoke_with_handle` pattern with null-handle checks
and panic catching. New FFI result variants if needed.

---

## Phase 6: Test Battery

### File: `firewood/src/manager.rs` (tests module)

#### Unit Tests for Multi-Head State

```rust
#[cfg(test)]
mod multi_head_tests {
    use super::*;

    // === Registration ===

    #[test]
    fn test_register_single_validator() { ... }

    #[test]
    fn test_register_multiple_validators() { ... }

    #[test]
    fn test_register_duplicate_validator_returns_error() { ... }

    #[test]
    fn test_register_exceeds_max_validators_returns_error() { ... }

    #[test]
    fn test_deregister_validator() { ... }

    #[test]
    fn test_deregister_nonexistent_validator_returns_error() { ... }

    #[test]
    fn test_deregister_then_reregister() { ... }

    // === Commit ===

    #[test]
    fn test_commit_single_validator() { ... }

    #[test]
    fn test_commit_two_validators_same_block_deduplicates() {
        // Both validators propose the same batch from the same parent.
        // First commits normally. Second should dedup (reuse existing revision).
        // Both heads should point to the same Arc (Arc::ptr_eq).
    }

    #[test]
    fn test_commit_dedup_does_not_write_duplicate_nodes() {
        // After dedup, verify that no additional nodes were written to storage.
        // The storage size should not increase after the second commit.
    }

    #[test]
    fn test_commit_wrong_parent_returns_not_validator_head() {
        // Validator A is at block 5. Proposal's parent is block 3.
        // Should return NotValidatorHead error.
    }

    #[test]
    fn test_commit_for_nonexistent_validator_returns_error() { ... }

    #[test]
    fn test_commit_divergent_produces_separate_revision() {
        // Validator D produces a different root hash at the same block height.
        // Should create a new revision, not dedup.
        // Both the honest and divergent revisions should exist in by_hash.
    }

    #[test]
    fn test_commit_after_persist_worker_failure_returns_error() { ... }

    // === Deduplication ===

    #[test]
    fn test_dedup_revisions_share_arc() {
        // Commit same block from two validators.
        // Verify Arc::ptr_eq on both heads.
    }

    #[test]
    fn test_dedup_does_not_affect_other_validators() {
        // Validator A is at block 10. Validator B commits block 10 (same hash).
        // Validator C is still at block 8. Verify C's head unchanged.
    }

    #[test]
    fn test_three_validators_all_dedup_same_block() {
        // A, B, C all commit block N with same hash. Only one revision created.
    }

    // === Skip-Propose (advance_to_hash) ===

    #[test]
    fn test_advance_to_existing_hash() {
        // Validator A commits block 5. Validator B advances to block 5's hash.
        // B's head should match A's.
    }

    #[test]
    fn test_advance_to_nonexistent_hash_returns_error() { ... }

    #[test]
    fn test_advance_does_not_require_propose() {
        // Validator B advances without calling propose().
        // Should succeed with zero trie computation.
    }

    // === Reaping ===

    #[test]
    fn test_reaping_waits_for_all_heads() {
        // A is at block 10, B is at block 5.
        // Blocks before 5 can be reaped. Blocks 5-10 cannot.
    }

    #[test]
    fn test_reaping_shared_revision_frees_deleted_nodes() {
        // All validators advance past block N. Block N's deleted nodes
        // should be added to free lists.
    }

    #[test]
    fn test_reaping_divergent_revision_does_not_free_deleted_nodes() {
        // Divergent revision's deleted list may reference nodes still
        // alive in the honest chain. Verify those nodes are NOT freed.
    }

    #[test]
    fn test_reaping_after_validator_deregistration() {
        // Validator B at block 5 is deregistered.
        // Blocks that only B held should become reapable.
    }

    #[test]
    fn test_reaping_respects_max_revisions() {
        // With max_revisions=10, verify that at most 10 revisions are
        // kept even with multiple validators.
    }

    // === Divergence ===

    #[test]
    fn test_divergent_validator_chain_persisted() {
        // Validator D commits a block with wrong hash.
        // Verify the divergent revision is persisted to disk.
    }

    #[test]
    fn test_divergent_validator_does_not_affect_honest_chains() {
        // D diverges. A and B continue committing normally.
        // A and B's heads should be unaffected.
    }

    #[test]
    fn test_divergent_then_evict_cleans_up() {
        // D diverges, then is deregistered.
        // D's unique revisions should be removable.
    }

    // === Views ===

    #[test]
    fn test_validator_view_returns_correct_head() {
        // A at block 5, B at block 3. Verify A sees block 5 data, B sees block 3 data.
    }

    #[test]
    fn test_view_by_hash_works_across_validators() {
        // Any committed hash (from any validator) should be viewable.
    }

    // === Concurrency ===

    #[test]
    fn test_concurrent_commits_different_validators() {
        // Multiple threads commit for different validators simultaneously.
        // All should succeed without deadlock.
    }

    #[test]
    fn test_concurrent_commit_and_advance() {
        // One thread commits for A, another advances B to the same hash.
        // Both should succeed.
    }

    #[test]
    fn test_concurrent_commit_same_block_dedup_race() {
        // Two validators race to commit the same block.
        // One should commit normally, the other should dedup.
        // No double-write or corruption.
    }

    #[test]
    fn test_concurrent_commits_different_chains() {
        // Validators on separate chains commit simultaneously.
        // Per-chain reaping should not interfere across chains.
    }

    #[test]
    fn test_concurrent_reap_does_not_double_free() {
        // Reaping on two chains triggered concurrently.
        // Verify no revision is freed twice.
    }

    #[test]
    fn test_concurrent_register_deregister_under_commit_load() {
        // One goroutine registers/deregisters validators while another
        // continuously commits. No deadlock, no panic.
    }

    #[test]
    fn test_concurrent_advance_merges_chains_safely() {
        // Many validators advance to the same hash simultaneously.
        // All vacated chains should be cleaned up exactly once.
    }

    #[test]
    fn test_concurrent_fork_and_merge() {
        // Some validators produce divergent hashes (fork) while others
        // dedup back to the shared hash (merge). Concurrent execution.
        // Verify chain map is consistent after all operations.
    }

    #[test]
    fn test_no_deadlock_commit_then_view() {
        // Commit on one thread, validator_view on another.
        // No deadlock between write lock and read lock.
    }

    #[test]
    fn test_no_deadlock_header_and_multi_head_lock_ordering() {
        // Verify that header lock is never acquired while holding multi_head
        // lock. Instrument with a try_lock check.
    }
}
```

### File: `firewood/src/db.rs` (tests module)

#### Integration Tests for MultiDb

```rust
#[cfg(test)]
mod multi_db_tests {
    use super::*;

    // Helper: create MultiDb with N validators registered
    fn create_multi_db(n: usize) -> (MultiDb, Vec<ValidatorId>, tempfile::TempDir) { ... }

    // === Basic Operations ===

    #[test]
    fn test_multi_db_create_and_close() { ... }

    #[test]
    fn test_multi_db_propose_commit_single_validator() { ... }

    #[test]
    fn test_multi_db_two_validators_same_batch() {
        // Both propose and commit the same batch.
        // Verify dedup: second commit is a no-op for storage.
    }

    #[test]
    fn test_multi_db_validator_view_reads_correct_state() {
        // Validator A commits key=X, value=1.
        // Validator B hasn't committed yet.
        // Verify A sees (X, 1), B sees empty.
    }

    #[test]
    fn test_multi_db_advance_skips_propose() {
        // A commits block. B advances to A's hash.
        // Verify B sees the same data as A without proposing.
    }

    // === Persistence and Recovery ===

    #[test]
    fn test_multi_db_persists_all_validator_roots() {
        // Commit for all validators. Close. Reopen.
        // Verify all validator heads restored from header.
    }

    #[test]
    fn test_multi_db_crash_recovery_restores_heads() {
        // Simulate crash (drop without close).
        // Reopen and verify heads match last persisted state.
    }

    #[test]
    fn test_multi_db_legacy_to_multi_head_migration() {
        // Create a standard Db, commit some data.
        // Reopen as MultiDb. Verify first validator sees existing data.
    }

    // === N Validators ===

    #[test]
    fn test_multi_db_max_validators() {
        // Register MAX_VALIDATORS validators. All should succeed.
        // Registering one more should fail with MaxValidatorsReached.
    }

    #[test]
    fn test_multi_db_n_validators_same_chain() {
        // N validators each commit the same sequence of blocks.
        // Verify storage growth is ~1x (not Nx) due to dedup.
    }

    // === Error Cases ===

    #[test]
    fn test_propose_for_deregistered_validator_returns_error() { ... }

    #[test]
    fn test_commit_for_deregistered_validator_returns_error() { ... }

    #[test]
    fn test_advance_for_deregistered_validator_returns_error() { ... }

    #[test]
    fn test_view_for_deregistered_validator_returns_error() { ... }
}
```

### File: `storage/src/nodestore/header.rs` (tests)

```rust
#[cfg(test)]
mod multi_head_header_tests {
    use super::*;

    #[test]
    fn test_header_size_with_validator_roots_fits_in_2048() {
        assert!(size_of::<NodeStoreHeader>() <= NodeStoreHeader::SIZE as usize);
    }

    #[test]
    fn test_validator_root_roundtrip() {
        // Set a validator root, read it back, verify equality.
    }

    #[test]
    fn test_validator_root_none_when_slot_unused() { ... }

    #[test]
    fn test_validator_root_out_of_range_returns_none() { ... }

    #[test]
    fn test_set_validator_root_out_of_range_returns_error() { ... }

    #[test]
    fn test_validator_count_zero_is_legacy_mode() { ... }

    #[test]
    fn test_validate_rejects_invalid_validator_count() {
        // validator_count > MAX_VALIDATORS should fail validation.
    }

    #[test]
    fn test_header_backward_compatible_with_zero_validators() {
        // Write header with validator_count=0.
        // Read with legacy code path. Should work identically.
    }

    #[test]
    fn test_header_persists_multiple_validator_roots() {
        // Write header with 4 validator roots.
        // Read back and verify all 4 are correct.
    }
}
```

### File: `ffi/` (tests)

```rust
// In existing FFI test suite, add:

#[test]
fn test_ffi_multi_db_lifecycle() { ... }

#[test]
fn test_ffi_register_and_deregister_validator() { ... }

#[test]
fn test_ffi_validator_propose_commit() { ... }

#[test]
fn test_ffi_validator_advance() { ... }

#[test]
fn test_ffi_null_handle_returns_error() { ... }
```

---

## Files Modified

| File                                 | Changes                                                                                                                                                                                                             |
|--------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `storage/src/nodestore/header.rs`    | `ValidatorRoot` type, N root slots in header, validation, getters/setters                                                                                                                                           |
| `firewood/src/manager.rs`            | `MultiHeadState`, `ChainState`, `ValidatorState`, `commit_for_validator`, `advance_validator_to_hash`, `register/deregister_validator`, `collect_reapable_revisions`, `collect_all_chain_revisions`, crash recovery |
| `firewood/src/db.rs`                 | `MultiDb`, `MultiDbConfig`, per-validator propose/commit/view API                                                                                                                                                   |
| `firewood/src/v2/api.rs`             | `ValidatorId` type, new error variants, `From` conversions                                                                                                                                                          |
| `firewood/src/persist_worker.rs`     | Per-validator header root updates                                                                                                                                                                                   |
| `ffi/src/`                           | FFI bindings for multi-validator operations                                                                                                                                                                         |

## Files NOT Modified

| File                                   | Reason                                                       |
|----------------------------------------|--------------------------------------------------------------|
| `storage/src/nodestore/alloc.rs`       | Allocator unchanged; uses updated `area_index_and_size`      |
| `storage/src/linear/filebacked.rs`     | Storage layer unchanged                                      |
| `storage/src/checker/mod.rs`           | Delegates to updated read/size functions; no direct changes  |

---

## Phase 7: On-Disk Fork ID Per Node

### Problem

When validators diverge, chains share disk addresses for ancestor nodes.
Reaping one chain's deleted nodes can free addresses still live in another
chain's trie, causing data corruption.

### Solution

Each node carries a **fork ID** (`u64`) identifying which fork allocated it.
A **fork tree** tracks parent-child relationships between forks. During
reaping, the `can_free` predicate consults the fork tree to determine
whether a deleted node can be safely returned to the free list.

### On-Disk Format

The AreaIndex byte (first byte of every stored area) uses bit 7 as a
"has fork\_id" flag:

- **V0 (legacy):** `[AreaIndex:1][FirstByte:1][NodeData...]` — AreaIndex 0-22
- **V1 (new):** `[AreaIndex:1][ForkId:8 LE][FirstByte:1][NodeData...]` —
  AreaIndex `0x80|0-22`
- **Free area:** unchanged

When `fork_id == 0` (single-head or pre-fork), V0 format is used for full
backward compatibility. Old databases are read transparently.

### Reaping Rules

1. **Own allocation** (`node_fork_id == chain_fork_id`): always freed
2. **Ancestor allocation**: freed only if no other active chain descends
   from the same ancestor
3. **Pre-fork nodes** (`fork_id == 0`): not freed while multiple chains
   are active; freed after convergence

### Divergence Metric Label

When Firewood detects a divergent commit, the divergence counter is
incremented with a `"source"` label identifying the validator. This
allows external billing systems to attribute storage costs.

### Cross-Chain Safety Guarantees

- Reaping on one chain never corrupts another chain's trie
- Shared ancestor nodes are safely leaked until convergence
- Fork tree is persisted in the header and survives restart
- `can_free` closure is constructed from a fork tree snapshot before
  any tree mutations, ensuring consistent `is_ancestor` lookups

### Space Reclamation

| Scenario                           | Behavior                                     |
|------------------------------------|----------------------------------------------|
| Single chain (all honest)          | Full reaping, zero leaked space              |
| Multiple chains (diverged)         | Own allocations freed; shared ancestors leak |
| After convergence                  | Full reaping resumes                         |
| After deregistration + cleanup     | Full recovery                                |

### Modified Files (Phase 7)

| File                                  | Changes                                                                          |
|---------------------------------------|----------------------------------------------------------------------------------|
| `storage/src/nodestore/primitives.rs` | `from_raw_byte`, `with_fork_id_flag` on AreaIndex                                |
| `storage/src/nodestore/mod.rs`        | Fork-aware read/write, `read_fork_id_from_disk`, `reap_deleted` with `can_free`  |
| `storage/src/nodestore/persist.rs`    | `serialize_node_to_bump` fork\_id insertion                                      |
| `firewood/src/manager.rs`             | `validator_fork_id`, `build_can_free`, fork tree ops                             |
| `firewood/src/persist_worker.rs`      | `CanFreeFn`, `ReapItem`, pass `can_free` through                                 |
| `firewood/src/db.rs`                  | Fork\_id propagation in `propose_with_parent`                                    |
| `firewood/src/merkle/parallel.rs`     | Pass fork\_id through parallel proposal creation                                 |
