// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![expect(
    clippy::default_trait_access,
    reason = "Found 3 occurrences after enabling the lint."
)]

use nonzero_ext::nonzero;
use parking_lot::{Mutex, MutexGuard, RwLock};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::num::{NonZero, NonZeroU64};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use firewood_storage::logger::{trace, warn};
use rayon::{ThreadPool, ThreadPoolBuilder};
use typed_builder::TypedBuilder;

use crate::merkle::Merkle;
use crate::persist_worker::{PersistError, PersistWorker};
use crate::root_store::RootStore;
use crate::v2::api::{ArcDynDbView, HashKey, OptionalHashKeyExt, ValidatorId};

use firewood_metrics::{firewood_increment, firewood_set};
pub use firewood_storage::CacheReadStrategy;
use firewood_storage::{
    BranchNode, Committed, FileBacked, FileIoError, HashedNodeReader, ImmutableProposal,
    IntoHashType, MAX_VALIDATORS, NodeHashAlgorithm, NodeStore, NodeStoreHeader, TrieHash,
};

pub(crate) const DB_FILE_NAME: &str = "firewood.db";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, TypedBuilder)]
/// Revision manager configuratoin
pub struct RevisionManagerConfig {
    /// The number of committed revisions to keep in memory.
    ///
    /// Must be > `deferred_persistence_commit_count`.
    #[builder(default = 128)]
    max_revisions: usize,

    /// The size of the node cache (number of entries).
    ///
    /// **Deprecated:** Use `node_cache_memory_limit` instead for memory-based sizing.
    /// If specified, this value is multiplied by 128 to estimate memory usage.
    /// Cannot be specified together with `node_cache_memory_limit`.
    #[deprecated(since = "0.2.0", note = "Use node_cache_memory_limit instead")]
    #[builder(default, setter(strip_option))]
    node_cache_size: Option<NonZero<usize>>,

    /// The memory limit for the node cache in bytes.
    ///
    /// If neither this nor `node_cache_size` is specified, defaults to 192MB (1,500,000 × 128).
    /// Cannot be specified together with `node_cache_size`.
    #[builder(default, setter(strip_option))]
    node_cache_memory_limit: Option<NonZero<usize>>,

    #[builder(default_code = "NonZero::new(1000000).expect(\"non-zero\")")]
    free_list_cache_size: NonZero<usize>,

    #[builder(default = CacheReadStrategy::WritesOnly)]
    cache_read_strategy: CacheReadStrategy,

    /// The maximum number of unpersisted revisions that can exist at a given time.
    ///
    /// Must be < `max_revisions`.
    #[builder(default = nonzero!(1u64))]
    deferred_persistence_commit_count: NonZeroU64,
}

impl RevisionManagerConfig {
    /// Compute the actual node cache memory limit from the configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if both `node_cache_size` and `node_cache_memory_limit` are specified.
    #[expect(deprecated)]
    pub(crate) fn compute_node_cache_memory_limit(
        &self,
    ) -> Result<NonZero<usize>, crate::v2::api::Error> {
        // Convert entry count to memory: size × 128 bytes per node (estimate)
        const BYTES_PER_NODE_ESTIMATE: usize = 128;
        // Default: 192MB (equivalent to 1,500,000 nodes × 128 bytes)
        const DEFAULT_MEMORY_LIMIT: usize = 192_000_000;

        match (self.node_cache_size, self.node_cache_memory_limit) {
            (Some(_), Some(_)) => Err(crate::v2::api::Error::ConflictingCacheConfig),
            (Some(size), None) => {
                warn!(
                    "node_cache_size is deprecated as of 0.2.0; use node_cache_memory_limit instead"
                );
                Ok(
                    NonZero::new(size.get().saturating_mul(BYTES_PER_NODE_ESTIMATE))
                        .expect("non-zero size produces non-zero memory"),
                )
            }
            (None, Some(limit)) => Ok(limit),
            (None, None) => Ok(NonZero::new(DEFAULT_MEMORY_LIMIT).expect("default is non-zero")),
        }
    }
}

#[derive(Clone, Debug, TypedBuilder)]
#[non_exhaustive]
/// Configuration manager that contains both truncate and revision manager config
pub struct ConfigManager {
    /// The directory where the database files will be stored (required).
    pub root_dir: PathBuf,
    /// The algorithm used for hashing nodes (required).
    pub node_hash_algorithm: NodeHashAlgorithm,

    /// Whether to create the DB if it doesn't exist.
    #[builder(default = true)]
    pub create: bool,
    /// Whether to truncate the DB when opening it. If set, the DB will be reset and all its
    /// existing contents will be lost.
    #[builder(default = false)]
    pub truncate: bool,
    /// Whether to enable `RootStore`.
    #[builder(default = false)]
    pub root_store: bool,
    /// Revision manager configuration.
    #[builder(default = RevisionManagerConfig::builder().build())]
    pub manager: RevisionManagerConfig,
}

pub type CommittedRevision = Arc<NodeStore<Committed, FileBacked>>;
type ProposedRevision = Arc<NodeStore<Arc<ImmutableProposal>, FileBacked>>;

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
    /// Slot index in the header (0..`MAX_VALIDATORS`).
    slot: u8,
    /// Which chain this validator is on.
    chain: ChainId,
}

/// Tracks all validator heads and per-chain revision pools.
#[derive(Debug)]
struct MultiHeadState {
    /// Per-validator state, keyed by [`ValidatorId`].
    validators: HashMap<ValidatorId, ValidatorState>,
    /// Per-chain revision deques (replaces the single global pool).
    chains: HashMap<ChainId, ChainState>,
    /// O(1) hash-based lookup into any chain's revision pool.
    by_hash: HashMap<TrieHash, CommittedRevision>,
    /// Reverse index: hash → chain that contains this revision.
    hash_to_chain: HashMap<TrieHash, ChainId>,
    /// Maximum number of validators allowed.
    max_validators: usize,
    /// Monotonic counter for allocating new chain IDs.
    next_chain_id: ChainId,
}

impl MultiHeadState {
    fn next_available_slot(&self) -> Option<u8> {
        let used_slots: std::collections::HashSet<u8> =
            self.validators.values().map(|v| v.slot).collect();
        (0..self.max_validators as u8).find(|slot| !used_slots.contains(slot))
    }
}

#[derive(Debug)]
pub(crate) struct RevisionManager {
    /// Maximum number of revisions to keep in memory.
    ///
    /// When this limit is exceeded, the oldest revision is removed from memory.
    /// If `root_store` is `None`, the oldest revision's nodes are added to the
    /// free list for space reuse. Otherwise, the oldest revision
    /// is preserved on disk for historical queries.
    max_revisions: usize,

    /// FIFO queue of committed revisions kept in memory. The queue always
    /// contains at least one revision.
    in_memory_revisions: RwLock<VecDeque<CommittedRevision>>,

    /// Hash-based index of committed revisions kept in memory.
    ///
    /// Maps root hash to the corresponding committed revision for O(1) lookup
    /// performance. This allows efficient retrieval of revisions without
    /// scanning through the `in_memory_revisions` queue.
    by_hash: RwLock<HashMap<TrieHash, CommittedRevision>>,

    /// Active proposals that have not yet been committed.
    proposals: Mutex<Vec<ProposedRevision>>,

    /// Lazily initialized thread pool for parallel operations.
    threadpool: OnceLock<ThreadPool>,

    /// Optional persistent store for historical root addresses.
    ///
    /// When present, enables retrieval of revisions beyond `max_revisions` by
    /// persisting root hash to disk address mappings. This allows historical
    /// queries of arbitrarily old revisions without keeping them in memory.
    root_store: Option<Arc<RootStore>>,

    /// Worker responsible for persisting revisions to disk.
    persist_worker: PersistWorker,

    /// Multi-head state. When `Some`, the manager supports multiple validator
    /// heads. When `None`, it operates in legacy single-head mode.
    multi_head: Option<RwLock<MultiHeadState>>,

    /// Shared reference to the underlying storage backing all nodestores.
    storage: Arc<FileBacked>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum RevisionManagerError {
    #[error("Revision for {provided:?} not found")]
    RevisionNotFound { provided: HashKey },
    #[error("Revision for {provided:?} has no address")]
    RevisionWithoutAddress { provided: HashKey },
    #[error(
        "The proposal cannot be committed since it is not a direct child of the most recent commit. Proposal parent: {provided:?}, current root: {expected:?}"
    )]
    NotLatest {
        provided: Option<HashKey>,
        expected: Option<HashKey>,
    },
    #[error("A FileIO error occurred during the commit: {0}")]
    FileIoError(#[from] FileIoError),
    #[error("An IO error occurred while creating the database directory: {0}")]
    IOError(#[from] io::Error),
    #[error("A RootStore error occurred: {0}")]
    RootStoreError(#[source] Box<dyn std::error::Error + Send + Sync>),
    #[error("A deferred persistence error occurred: {0}")]
    PersistError(#[source] PersistError),
    #[error(
        "max_revisions ({max_revisions}) must be > deferred_persistence_commit_count ({commit_count})"
    )]
    InsufficientRevisions {
        max_revisions: usize,
        commit_count: u64,
    },
    #[error("Validator {id:?} is not registered")]
    ValidatorNotFound { id: ValidatorId },
    #[error("Validator {id:?} is already registered")]
    ValidatorAlreadyRegistered { id: ValidatorId },
    #[error("Maximum number of validators ({max}) reached; cannot register validator {id:?}")]
    MaxValidatorsReached { id: ValidatorId, max: usize },
    #[error(
        "Proposal parent {provided:?} does not match validator {validator:?} head {expected:?}"
    )]
    NotValidatorHead {
        validator: ValidatorId,
        provided: Option<HashKey>,
        expected: Option<HashKey>,
    },
    #[expect(dead_code, reason = "Reserved for future deregistration workflow")]
    #[error("Validator {id:?} has already been deregistered")]
    ValidatorDeregistered { id: ValidatorId },
}

impl RevisionManager {
    pub fn new(config: ConfigManager) -> Result<Self, RevisionManagerError> {
        let commit_count = config.manager.deferred_persistence_commit_count.get();
        if (config.manager.max_revisions as u64) <= commit_count {
            return Err(RevisionManagerError::InsufficientRevisions {
                max_revisions: config.manager.max_revisions,
                commit_count,
            });
        }

        if config.create {
            std::fs::create_dir_all(&config.root_dir).map_err(RevisionManagerError::IOError)?;
        }

        let file = config.root_dir.join(DB_FILE_NAME);
        let node_cache_memory_limit =
            config
                .manager
                .compute_node_cache_memory_limit()
                .map_err(|e| {
                    RevisionManagerError::IOError(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        e,
                    ))
                })?;
        let fb = FileBacked::new(
            file,
            node_cache_memory_limit,
            config.manager.free_list_cache_size,
            config.truncate,
            config.create,
            config.manager.cache_read_strategy,
            config.node_hash_algorithm,
        )?;

        // Acquire an advisory lock on the database file to prevent multiple processes
        // from opening the same database simultaneously
        fb.lock()?;

        let storage = Arc::new(fb);
        let header = match NodeStoreHeader::read_from_storage(storage.as_ref()) {
            Ok(header) => header,
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => {
                // Empty file - create a new header for a fresh database
                NodeStoreHeader::new(config.node_hash_algorithm)
            }
            Err(err) => return Err(err.into()),
        };
        let nodestore = Arc::new(NodeStore::open(&header, storage.clone())?);
        let root_store = config
            .root_store
            .then(|| {
                let root_store_dir = config.root_dir.join("root_store");
                RootStore::new(root_store_dir, storage.clone(), config.truncate)
                    .map_err(RevisionManagerError::RootStoreError)
            })
            .transpose()?
            .map(Arc::new);

        if config.truncate {
            header.flush_to(storage.as_ref())?;
            storage.set_len(NodeStoreHeader::SIZE)?;
        }

        let mut by_hash = HashMap::new();
        if let Some(hash) = nodestore.root_hash().or_default_root_hash() {
            by_hash.insert(hash, nodestore.clone());
        }

        let persist_worker = PersistWorker::new(
            config.manager.deferred_persistence_commit_count,
            header,
            root_store.clone(),
        );

        let manager = Self {
            max_revisions: config.manager.max_revisions,
            in_memory_revisions: RwLock::new(VecDeque::from([nodestore.clone()])),
            by_hash: RwLock::new(by_hash),
            proposals: Mutex::new(Default::default()),
            threadpool: OnceLock::new(),
            root_store,
            persist_worker,
            multi_head: None,
            storage: storage.clone(),
        };

        // On startup, we always write the latest revision to RootStore
        if let Some(root_hash) = manager.current_revision().root_hash() {
            let root_address = manager.current_revision().root_address().ok_or(
                RevisionManagerError::RevisionWithoutAddress {
                    provided: root_hash.clone(),
                },
            )?;

            if let Some(store) = &manager.root_store {
                store
                    .add_root(&root_hash, &root_address)
                    .map_err(RevisionManagerError::RootStoreError)?;
            }
        }

        Ok(manager)
    }

    /// Commit a proposal
    /// To commit a proposal involves a few steps:
    /// 1. Check if the persist worker has failed.
    ///    If so, return the error as this means we won't be able to make any
    ///    further progress.
    /// 2. Commit check.
    ///    The proposal's parent must be the last committed revision, otherwise the commit fails.
    ///    It only contains the address of the nodes that are deleted, which should be very small.
    /// 3. Revision reaping.
    ///    If more than the maximum number of revisions are kept in memory, the
    ///    oldest revision is removed from memory and sent to the `PersistWorker`
    ///    for reaping.
    /// 4. Signal to the `PersistWorker` to persist this revision.
    /// 5. Set last committed revision.
    ///    Set last committed revision in memory.
    /// 6. Proposal Cleanup.
    ///    Any other proposals that have this proposal as a parent should be reparented to the committed version.
    ///
    /// Steps 1 through 5 are executed behind a lock to maintain the invariant
    /// that only one revision can commit at a time.
    #[fastrace::trace(short_name = true)]
    #[crate::metrics("proposal.commit", "proposal commit to storage")]
    pub fn commit(&self, proposal: ProposedRevision) -> Result<(), RevisionManagerError> {
        // Hold a write lock on `in_memory_revisions` for the duration of the
        // critical section (steps 1-5). This is necessary because:
        // 1. Without the lock, two proposals with the same parent could pass the
        //    commit check simultaneously, allowing both to commit.
        // 2. New proposals rely on the latest committed revision via
        //    `current_revision()`, which takes a read lock on `in_memory_revisions`.
        //    The write lock here prevents proposals from being created against
        //    an older revision while a newer revision is mid-commit.
        let mut in_memory_revisions = self.in_memory_revisions.write();

        // 1. Check if the persist worker has failed.
        self.persist_worker
            .check_error()
            .map_err(RevisionManagerError::PersistError)?;

        // 2. Commit check
        let current_revision = in_memory_revisions
            .back()
            .expect("there is always one revision");
        if !proposal.parent_hash_is(current_revision.root_hash()) {
            return Err(RevisionManagerError::NotLatest {
                provided: proposal.root_hash(),
                expected: current_revision.root_hash(),
            });
        }

        let committed = proposal.as_committed();

        // 3. Revision reaping
        // When we exceed max_revisions, remove the oldest revision from memory
        // and send it to the `PersistWorker`.
        // If you crash after freeing some of these, then the free list will point to nodes that are not actually free.
        // TODO: Handle the case where we get something off the free list that is not free
        while in_memory_revisions.len() >= self.max_revisions {
            let oldest = in_memory_revisions.pop_front().expect("must be present");
            let oldest_hash = oldest.root_hash().or_default_root_hash();
            if let Some(ref hash) = oldest_hash {
                self.by_hash.write().remove(hash);
            }

            // The warning in the docs for `Arc::try_unwrap` does not apply here
            // because `original` is retained and not immediately dropped.
            match Arc::try_unwrap(oldest) {
                Ok(oldest) => self
                    .persist_worker
                    .reap(oldest)
                    .map_err(RevisionManagerError::PersistError)?,
                Err(original) => {
                    warn!("Oldest revision could not be reaped; still referenced");
                    in_memory_revisions.push_front(original);
                    break;
                }
            }
            firewood_set!(crate::registry::ACTIVE_REVISIONS, in_memory_revisions.len());
            firewood_set!(crate::registry::MAX_REVISIONS, self.max_revisions);
        }

        // 4. Signal to the `PersistWorker` to persist this revision.
        let committed: CommittedRevision = committed.into();
        self.persist_worker
            .persist(committed.clone())
            .map_err(RevisionManagerError::PersistError)?;

        // 5. Set last committed revision
        // The revision is added to `by_hash` here while it still exists in `proposals`.
        // The `view()` method relies on this ordering - it checks `proposals` first,
        // then `by_hash`, ensuring the revision is always findable during the transition.
        in_memory_revisions.push_back(committed.clone());
        if let Some(hash) = committed.root_hash().or_default_root_hash() {
            self.by_hash.write().insert(hash, committed.clone());
        }

        // At this point, we can release the lock on the queue of in-memory
        // revisions as we've now set the new latest committed revision.
        drop(in_memory_revisions);

        // 6. Proposal Cleanup
        // Free proposal that is being committed as well as any proposals no longer
        // referenced by anyone else. Track how many were discarded (dropped without commit).
        {
            let mut lock = self.proposals.lock();
            let mut discarded = 0u64;
            lock.retain(|p| {
                let should_retain = !Arc::ptr_eq(&proposal, p) && Arc::strong_count(p) > 1;
                if !should_retain {
                    discarded = discarded.wrapping_add(1);
                }
                should_retain
            });

            if discarded > 0 {
                firewood_increment!(crate::registry::PROPOSALS_DISCARDED, discarded);
            }

            // Update uncommitted proposals gauge after cleanup
            firewood_set!(crate::registry::PROPOSALS_UNCOMMITTED, lock.len());
        }

        // then reparent any proposals that have this proposal as a parent
        for p in &*self.proposals.lock() {
            proposal.commit_reparent(p);
        }

        if crate::logger::trace_enabled() {
            let merkle = Merkle::from(committed);
            if let Ok(s) = merkle.dump_to_string() {
                trace!("{s}");
            }
        }

        Ok(())
    }

    /// View the database at a specific hash.
    /// To view the database at a specific hash involves a few steps:
    /// 1. Try to find it in proposals.
    /// 2. Try to find it in committed revisions.
    pub fn view(&self, root_hash: HashKey) -> Result<ArcDynDbView, RevisionManagerError> {
        // 1. Try to find it in proposals.
        let proposal = self
            .proposals
            .lock()
            .iter()
            .find(|p| p.root_hash().as_ref() == Some(&root_hash))
            .cloned();

        if let Some(proposal) = proposal {
            return Ok(proposal);
        }

        // 2. Try to find it in committed revisions.
        self.revision(root_hash).map(|r| r as ArcDynDbView)
    }

    pub fn add_proposal(&self, proposal: ProposedRevision) {
        let len = {
            let mut lock = self.proposals.lock();
            lock.push(proposal);
            lock.len()
        };
        // Update uncommitted proposals gauge after adding
        firewood_set!(crate::registry::PROPOSALS_UNCOMMITTED, len);
    }

    /// Retrieve a committed revision by its root hash.
    ///
    /// Checks in-memory revisions (single-head and multi-head), then `RootStore`.
    pub fn revision(&self, root_hash: HashKey) -> Result<CommittedRevision, RevisionManagerError> {
        // Check the in-memory revision manager (single-head).
        if let Some(revision) = self.by_hash.read().get(&root_hash).cloned() {
            return Ok(revision);
        }

        // Check the multi-head state's `by_hash`.
        if let Some(multi_head) = &self.multi_head
            && let Some(revision) = multi_head.read().by_hash.get(&root_hash).cloned()
        {
            return Ok(revision);
        }

        // 2. Check `RootStore` (if it exists).
        let root_store =
            self.root_store
                .as_ref()
                .ok_or(RevisionManagerError::RevisionNotFound {
                    provided: root_hash.clone(),
                })?;
        let revision = root_store
            .get(&root_hash)
            .map_err(RevisionManagerError::RootStoreError)?
            .ok_or(RevisionManagerError::RevisionNotFound {
                provided: root_hash.clone(),
            })?;

        Ok(revision)
    }

    pub fn root_hash(&self) -> Option<HashKey> {
        self.current_revision().root_hash()
    }

    pub fn current_revision(&self) -> CommittedRevision {
        self.in_memory_revisions
            .read()
            .back()
            .expect("there is always one revision")
            .clone()
    }

    /// Acquires a lock on the header and returns a guard.
    pub(crate) fn locked_header(&self) -> MutexGuard<'_, NodeStoreHeader> {
        self.persist_worker.locked_header()
    }

    /// Gets or creates a threadpool associated with the revision manager.
    ///
    /// # Panics
    ///
    /// Panics if the it cannot create a thread pool.
    pub fn threadpool(&self) -> &ThreadPool {
        // Note that OnceLock currently doesn't support get_or_try_init (it is available in a
        // nightly release). The get_or_init should be replaced with get_or_try_init once it
        // is available to allow the error to be passed back to the caller.
        self.threadpool.get_or_init(|| {
            ThreadPoolBuilder::new()
                .num_threads(BranchNode::MAX_CHILDREN)
                .build()
                .expect("Error in creating threadpool")
        })
    }

    /// Checks if the `PersistWorker` has errored.
    pub fn check_persist_error(&self) -> Result<(), RevisionManagerError> {
        self.persist_worker
            .check_error()
            .map_err(RevisionManagerError::PersistError)
    }

    /// Enable multi-head mode, allowing multiple validator heads.
    ///
    /// This transitions the revision manager from single-head to multi-head
    /// mode. The current revision becomes the base for all future validators.
    pub fn enable_multi_head(&mut self, max_validators: usize) -> Result<(), RevisionManagerError> {
        let max_validators = max_validators.min(MAX_VALIDATORS);
        let current = self.current_revision();

        let mut by_hash = HashMap::new();
        let mut hash_to_chain = HashMap::new();

        // Create chain 0 with the current revision as its sole entry
        let chain_revisions = VecDeque::from([current.clone()]);
        if let Some(hash) = current.root_hash().or_default_root_hash() {
            by_hash.insert(hash.clone(), current.clone());
            hash_to_chain.insert(hash, 0);
        }

        let mut chains = HashMap::new();
        chains.insert(
            0,
            ChainState {
                revisions: chain_revisions,
            },
        );

        self.multi_head = Some(RwLock::new(MultiHeadState {
            validators: HashMap::new(),
            chains,
            by_hash,
            hash_to_chain,
            max_validators,
            next_chain_id: 1,
        }));

        // Update header with validator count from on-disk state
        let header = self.persist_worker.locked_header();
        let count = header.validator_count();
        drop(header);

        // If the database already had validators on disk, restore them
        if count > 0 {
            self.restore_validators_from_header()?;
        }

        Ok(())
    }

    /// Restore validator state from the header after opening an existing
    /// multi-head database.
    ///
    /// Validators with the same root hash are assigned to the same chain;
    /// validators with different root hashes are assigned to different chains.
    fn restore_validators_from_header(&self) -> Result<(), RevisionManagerError> {
        // Phase 1: Read all validator data from the header (header lock only)
        let header = self.persist_worker.locked_header();
        let count = header.validator_count();

        let mut validators_data = Vec::new();
        for slot in 0..count {
            let slot_u8 = u8::try_from(slot).map_err(|_| {
                RevisionManagerError::IOError(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "slot overflow",
                ))
            })?;

            if let Some((validator_id, (addr, hash))) = header.validator_root(slot_u8) {
                validators_data.push((slot_u8, validator_id, addr, hash));
            }
        }
        drop(header);

        // Phase 2: Update multi-head state (write lock only, no header lock)
        let mut state = self
            .multi_head
            .as_ref()
            .ok_or(RevisionManagerError::IOError(io::Error::other(
                "multi-head mode not enabled",
            )))?
            .write();

        for (slot_u8, validator_id, addr, hash) in validators_data {
            let id = ValidatorId::new(validator_id);

            // Check if we already have a revision with this hash (dedup on recovery)
            let (head, chain_id) = if let Some(existing) = state.by_hash.get(&hash).cloned() {
                let chain_id = state.hash_to_chain.get(&hash).copied().ok_or_else(|| {
                    RevisionManagerError::IOError(io::Error::other(
                        "hash_to_chain inconsistent during recovery",
                    ))
                })?;
                (existing, chain_id)
            } else {
                // Reconstruct from the shared storage
                let committed = Arc::new(NodeStore::with_root(
                    hash.clone().into_hash_type(),
                    addr,
                    self.storage.clone(),
                ));

                let chain_id = state.next_chain_id;
                state.next_chain_id = state.next_chain_id.wrapping_add(1);

                state.chains.insert(
                    chain_id,
                    ChainState {
                        revisions: VecDeque::from([committed.clone()]),
                    },
                );
                state.by_hash.insert(hash.clone(), committed.clone());
                state.hash_to_chain.insert(hash, chain_id);

                (committed, chain_id)
            };

            state.validators.insert(
                id,
                ValidatorState {
                    head,
                    slot: slot_u8,
                    chain: chain_id,
                },
            );
        }

        Ok(())
    }

    /// Register a new validator. The validator starts at the current latest revision.
    ///
    /// The validator is assigned to the chain that the tip revision belongs to.
    pub fn register_validator(&self, id: ValidatorId) -> Result<(), RevisionManagerError> {
        let (slot, root_info, validator_count) = {
            let mut state = self
                .multi_head
                .as_ref()
                .ok_or(RevisionManagerError::ValidatorNotFound { id })?
                .write();

            if state.validators.contains_key(&id) {
                return Err(RevisionManagerError::ValidatorAlreadyRegistered { id });
            }

            let slot =
                state
                    .next_available_slot()
                    .ok_or(RevisionManagerError::MaxValidatorsReached {
                        id,
                        max: state.max_validators,
                    })?;

            // Find the tip chain: the chain with the most validators, or any existing chain.
            // If all chains have been cleaned up, create a fresh chain from current_revision.
            let (tip_chain_id, base) = {
                let mut chain_counts: HashMap<ChainId, usize> = HashMap::new();
                for v in state.validators.values() {
                    chain_counts
                        .entry(v.chain)
                        .and_modify(|c| *c = c.wrapping_add(1))
                        .or_insert(1);
                }
                let best_chain = chain_counts
                    .into_iter()
                    .max_by_key(|&(_, count)| count)
                    .map(|(cid, _)| cid)
                    .or_else(|| state.chains.keys().copied().next());

                if let Some(chain_id) = best_chain {
                    let base = state
                        .chains
                        .get(&chain_id)
                        .and_then(|c| c.revisions.back().cloned())
                        .unwrap_or_else(|| self.current_revision());
                    (chain_id, base)
                } else {
                    // All chains cleaned up; create a fresh one from the current revision
                    let base = self.current_revision();
                    let chain_id = state.next_chain_id;
                    state.next_chain_id = state.next_chain_id.wrapping_add(1);

                    let mut chain_revisions = VecDeque::new();
                    chain_revisions.push_back(base.clone());
                    state.chains.insert(
                        chain_id,
                        ChainState {
                            revisions: chain_revisions,
                        },
                    );
                    if let Some(hash) = base.root_hash().or_default_root_hash() {
                        state.hash_to_chain.insert(hash.clone(), chain_id);
                        state.by_hash.insert(hash, base.clone());
                    }
                    (chain_id, base)
                }
            };

            state.validators.insert(
                id,
                ValidatorState {
                    head: base,
                    slot,
                    chain: tip_chain_id,
                },
            );

            let validator_count = state.validators.len();
            let validator = state
                .validators
                .get(&id)
                .ok_or(RevisionManagerError::ValidatorNotFound { id })?;
            let root_info = self.root_info_for_validator(id, validator);

            (slot, root_info, validator_count)
        }; // write lock dropped

        // Update header (no multi_head lock held)
        let mut header = self.persist_worker.locked_header();
        header.set_validator_count(validator_count);
        if let Err(e) = header.set_validator_root(slot, root_info) {
            return Err(RevisionManagerError::IOError(e));
        }

        Ok(())
    }

    /// Deregister a validator, freeing its header slot.
    ///
    /// If this was the last validator on its chain, the chain is cleaned up
    /// and its revisions are reaped.
    pub fn deregister_validator(&self, id: ValidatorId) -> Result<(), RevisionManagerError> {
        let (removed_slot, validator_count, revisions_to_reap) = {
            let mut state = self
                .multi_head
                .as_ref()
                .ok_or(RevisionManagerError::ValidatorNotFound { id })?
                .write();

            let removed = state
                .validators
                .remove(&id)
                .ok_or(RevisionManagerError::ValidatorNotFound { id })?;

            let source_chain = removed.chain;
            let validator_count = state.validators.len();

            // If source chain has no remaining validators, collect its revisions for reaping
            let source_has_validators = state.validators.values().any(|v| v.chain == source_chain);
            let revisions_to_reap = if source_has_validators {
                Vec::new()
            } else {
                self.collect_all_chain_revisions(source_chain, &mut state)
            };

            (removed.slot, validator_count, revisions_to_reap)
        }; // write lock dropped

        // Clear this validator's root in the header (no multi_head lock held)
        let mut header = self.persist_worker.locked_header();
        if let Err(e) = header.set_validator_root(removed_slot, None) {
            return Err(RevisionManagerError::IOError(e));
        }
        header.set_validator_count(validator_count);
        drop(header);

        // Reap collected revisions (no locks held, may block)
        for rev in revisions_to_reap {
            match Arc::try_unwrap(rev) {
                Ok(owned) => {
                    self.persist_worker
                        .reap(owned)
                        .map_err(RevisionManagerError::PersistError)?;
                }
                Err(_still_held) => {
                    // External reference (e.g., a view) holds this revision.
                }
            }
        }

        Ok(())
    }

    /// Commit a proposal for a specific validator.
    ///
    /// If a revision with the same root hash already exists (committed by
    /// another validator), the proposal is discarded and the validator's
    /// head advances to the existing revision (deduplication).
    ///
    /// Uses a 3-phase approach to minimize lock contention:
    /// - Phase 1 (write lock): validate, dedup, update in-memory state
    /// - Phase 2 (no lock): persist, update header, reap
    /// - Phase 3 (proposals lock): cleanup
    #[fastrace::trace(short_name = true)]
    pub fn commit_for_validator(
        &self,
        id: ValidatorId,
        proposal: ProposedRevision,
    ) -> Result<(), RevisionManagerError> {
        // Check persist worker health (no lock needed)
        self.persist_worker
            .check_error()
            .map_err(RevisionManagerError::PersistError)?;

        // Phase 1: Under write lock (fast, in-memory only)
        let (committed, slot, root_info, revisions_to_reap) = {
            let mut state = self
                .multi_head
                .as_ref()
                .ok_or(RevisionManagerError::ValidatorNotFound { id })?
                .write();

            let validator = state
                .validators
                .get(&id)
                .ok_or(RevisionManagerError::ValidatorNotFound { id })?;

            // 1. Parent check: proposal's parent must be this validator's head
            if !proposal.parent_hash_is(validator.head.root_hash()) {
                return Err(RevisionManagerError::NotValidatorHead {
                    validator: id,
                    provided: proposal.root_hash(),
                    expected: validator.head.root_hash(),
                });
            }

            // 2. Dedup check: does a revision with this hash already exist?
            let new_hash = proposal.root_hash().or_default_root_hash();
            if let Some(ref hash) = new_hash
                && let Some(existing) = state.by_hash.get(hash)
            {
                // Another validator already committed this. Reuse it.
                let existing = existing.clone();
                let source_chain = state
                    .validators
                    .get(&id)
                    .ok_or(RevisionManagerError::ValidatorNotFound { id })?
                    .chain;

                // Look up target chain via hash_to_chain
                let target_chain = state.hash_to_chain.get(hash).copied().ok_or_else(|| {
                    RevisionManagerError::IOError(io::Error::other(
                        "hash_to_chain inconsistent: hash in by_hash but not hash_to_chain",
                    ))
                })?;

                // Update validator head and chain assignment
                {
                    let validator = state
                        .validators
                        .get_mut(&id)
                        .ok_or(RevisionManagerError::ValidatorNotFound { id })?;
                    validator.head = existing;
                    if source_chain != target_chain {
                        validator.chain = target_chain;
                    }
                } // mutable borrow of validator dropped

                // Clean up source chain if now empty of validators
                let revisions_to_reap = if source_chain != target_chain
                    && !state.validators.values().any(|v| v.chain == source_chain)
                {
                    self.collect_all_chain_revisions(source_chain, &mut state)
                } else {
                    Vec::new()
                };

                let validator = state
                    .validators
                    .get(&id)
                    .ok_or(RevisionManagerError::ValidatorNotFound { id })?;
                let root_info = self.root_info_for_validator(id, validator);
                let slot = validator.slot;
                drop(state); // release write lock

                // Phase 2 (dedup path): update header, reap orphaned chain
                let mut header = self.persist_worker.locked_header();
                if let Err(e) = header.set_validator_root(slot, root_info) {
                    return Err(RevisionManagerError::IOError(e));
                }
                drop(header);

                for rev in revisions_to_reap {
                    match Arc::try_unwrap(rev) {
                        Ok(owned) => {
                            self.persist_worker
                                .reap(owned)
                                .map_err(RevisionManagerError::PersistError)?;
                        }
                        Err(_still_held) => {}
                    }
                }

                // Phase 3 (dedup path): cleanup proposals
                self.cleanup_proposals_inner(&proposal);

                return Ok(());
            }

            // 3. Check for divergence: other validators on same chain have
            //    a different head than this validator's parent
            let validator_chain = state
                .validators
                .get(&id)
                .ok_or(RevisionManagerError::ValidatorNotFound { id })?
                .chain;
            let parent_hash = state
                .validators
                .get(&id)
                .ok_or(RevisionManagerError::ValidatorNotFound { id })?
                .head
                .root_hash();

            let diverged = state.validators.iter().any(|(other_id, other_v)| {
                *other_id != id
                    && other_v.chain == validator_chain
                    && other_v.head.root_hash() != parent_hash
            });

            // 4. Convert proposal to committed
            let committed: CommittedRevision = proposal.as_committed().into();

            if diverged {
                // Fork: create new chain for this validator
                if let Some(ref new_h) = new_hash {
                    warn!("validator {id:?} diverged: parent={parent_hash:?}, got={new_h:?}");
                    firewood_increment!(crate::registry::VALIDATOR_DIVERGENCE_TOTAL, 1);
                }

                let new_chain_id = state.next_chain_id;
                state.next_chain_id = state.next_chain_id.wrapping_add(1);

                state.chains.insert(
                    new_chain_id,
                    ChainState {
                        revisions: VecDeque::from([committed.clone()]),
                    },
                );

                if let Some(ref hash) = new_hash {
                    state.by_hash.insert(hash.clone(), committed.clone());
                    state.hash_to_chain.insert(hash.clone(), new_chain_id);
                }

                let validator = state
                    .validators
                    .get_mut(&id)
                    .ok_or(RevisionManagerError::ValidatorNotFound { id })?;
                validator.head = committed.clone();
                validator.chain = new_chain_id;
                let root_info = self.root_info_for_validator(id, validator);
                let slot = validator.slot;

                // No reaping needed for brand-new chain (only 1 revision)
                (committed, slot, root_info, Vec::new())
            } else {
                // Normal case: append new revision to the validator's current chain
                if let Some(ref hash) = new_hash {
                    state.by_hash.insert(hash.clone(), committed.clone());
                    state.hash_to_chain.insert(hash.clone(), validator_chain);
                }

                let chain = state.chains.get_mut(&validator_chain).ok_or_else(|| {
                    RevisionManagerError::IOError(io::Error::other(
                        "validator's chain not found in chains map",
                    ))
                })?;
                chain.revisions.push_back(committed.clone());

                let validator = state
                    .validators
                    .get_mut(&id)
                    .ok_or(RevisionManagerError::ValidatorNotFound { id })?;
                validator.head = committed.clone();
                let root_info = self.root_info_for_validator(id, validator);
                let slot = validator.slot;

                // 5. Collect revisions to reap from this chain (per-chain budget)
                let revisions_to_reap =
                    self.collect_reapable_revisions(validator_chain, &mut state);

                (committed, slot, root_info, revisions_to_reap)
            }
        }; // write lock dropped

        // Phase 2: No lock held (may block)
        // Persist the new revision
        self.persist_worker
            .persist(committed.clone())
            .map_err(RevisionManagerError::PersistError)?;

        // Update header
        let mut header = self.persist_worker.locked_header();
        if let Err(e) = header.set_validator_root(slot, root_info.clone()) {
            return Err(RevisionManagerError::IOError(e));
        }
        // Also update legacy root for backward compat
        header.set_root_location(root_info.map(|(_, ri)| ri));
        drop(header);

        // Reap collected revisions
        for rev in revisions_to_reap {
            match Arc::try_unwrap(rev) {
                Ok(owned) => {
                    self.persist_worker
                        .reap(owned)
                        .map_err(RevisionManagerError::PersistError)?;
                }
                Err(_still_held) => {
                    // External reference (e.g., a view) holds this revision.
                }
            }
        }

        // Phase 3: Under proposals lock (short)
        self.cleanup_proposals_inner(&proposal);

        // Reparent any proposals that have this proposal as a parent
        for p in &*self.proposals.lock() {
            proposal.commit_reparent(p);
        }

        if crate::logger::trace_enabled() {
            let merkle = Merkle::from(committed);
            if let Ok(s) = merkle.dump_to_string() {
                trace!("{s}");
            }
        }

        Ok(())
    }

    /// Advance a validator's head to an existing revision by hash.
    ///
    /// This is the skip-propose optimization: if a validator knows the
    /// expected root hash (e.g., from a block header), it can advance
    /// without computing a proposal.
    ///
    /// If the target revision is on a different chain, the validator moves
    /// to that chain. The source chain is cleaned up if it becomes empty.
    pub fn advance_validator_to_hash(
        &self,
        id: ValidatorId,
        hash: HashKey,
    ) -> Result<(), RevisionManagerError> {
        let (slot, root_info, revisions_to_reap) = {
            let mut state = self
                .multi_head
                .as_ref()
                .ok_or(RevisionManagerError::ValidatorNotFound { id })?
                .write();

            if !state.validators.contains_key(&id) {
                return Err(RevisionManagerError::ValidatorNotFound { id });
            }

            let existing = state
                .by_hash
                .get(&hash)
                .ok_or(RevisionManagerError::RevisionNotFound {
                    provided: hash.clone(),
                })?
                .clone();

            // Look up target chain
            let target_chain = state.hash_to_chain.get(&hash).copied().ok_or_else(|| {
                RevisionManagerError::IOError(io::Error::other(
                    "hash_to_chain inconsistent in advance_validator_to_hash",
                ))
            })?;

            let source_chain = state
                .validators
                .get(&id)
                .ok_or(RevisionManagerError::ValidatorNotFound { id })?
                .chain;

            let validator = state
                .validators
                .get_mut(&id)
                .ok_or(RevisionManagerError::ValidatorNotFound { id })?;
            validator.head = existing;
            validator.chain = target_chain;
            let root_info = self.root_info_for_validator(id, validator);
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
        if let Err(e) = header.set_validator_root(slot, root_info) {
            return Err(RevisionManagerError::IOError(e));
        }
        drop(header);

        // Reap collected revisions (no locks held, may block)
        for rev in revisions_to_reap {
            match Arc::try_unwrap(rev) {
                Ok(owned) => {
                    self.persist_worker
                        .reap(owned)
                        .map_err(RevisionManagerError::PersistError)?;
                }
                Err(_still_held) => {}
            }
        }

        Ok(())
    }

    /// Return a view at a specific validator's current head.
    pub fn validator_view(
        &self,
        id: ValidatorId,
    ) -> Result<CommittedRevision, RevisionManagerError> {
        let state = self
            .multi_head
            .as_ref()
            .ok_or(RevisionManagerError::ValidatorNotFound { id })?
            .read();

        state
            .validators
            .get(&id)
            .map(|v| v.head.clone())
            .ok_or(RevisionManagerError::ValidatorNotFound { id })
    }

    /// Collect revisions to reap from a specific chain (per-chain budget enforcement).
    ///
    /// Pops revisions from the front of the chain's deque when:
    /// - The chain exceeds `max_revisions`
    /// - The revision is older than all validators' heads on this chain
    ///
    /// Returns the collected revisions for reaping outside the lock.
    fn collect_reapable_revisions(
        &self,
        chain_id: ChainId,
        state: &mut MultiHeadState,
    ) -> Vec<CommittedRevision> {
        let mut collected = Vec::new();

        let Some(chain) = state.chains.get_mut(&chain_id) else {
            return collected; // Chain already cleaned up
        };

        // Find the min-head position of validators on THIS chain
        let min_pos = state
            .validators
            .values()
            .filter(|v| v.chain == chain_id)
            .filter_map(|v| chain.revisions.iter().position(|r| Arc::ptr_eq(r, &v.head)))
            .min()
            .unwrap_or(0);

        let mut reaped = 0;
        while chain.revisions.len() > self.max_revisions && reaped < min_pos {
            let Some(oldest) = chain.revisions.pop_front() else {
                break;
            };

            // Remove from hash indexes if no validator head references this hash
            if let Some(hash) = oldest.root_hash().or_default_root_hash() {
                let still_referenced = state
                    .validators
                    .values()
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

    /// Collect all revisions from an empty chain for cleanup.
    ///
    /// Removes the chain from the `chains` map and returns all its revisions
    /// for reaping outside the lock. Hash indexes are cleaned up for revisions
    /// that are no longer referenced by any validator head.
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
                let still_referenced = state
                    .validators
                    .values()
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

    /// Extract root info (`validator_id`, address, hash) from a validator's head,
    /// in the format expected by `set_validator_root`.
    fn root_info_for_validator(
        &self,
        id: ValidatorId,
        validator: &ValidatorState,
    ) -> Option<(u64, (firewood_storage::LinearAddress, TrieHash))> {
        let hash = validator.head.root_hash()?;
        let addr = validator.head.root_address()?;
        Some((id.id(), (addr, hash)))
    }

    /// Clean up proposals after a commit (shared helper).
    fn cleanup_proposals_inner(&self, proposal: &ProposedRevision) {
        let mut lock = self.proposals.lock();
        let mut discarded = 0u64;
        lock.retain(|p| {
            let should_retain = !Arc::ptr_eq(proposal, p) && Arc::strong_count(p) > 1;
            if !should_retain {
                discarded = discarded.wrapping_add(1);
            }
            should_retain
        });

        if discarded > 0 {
            firewood_increment!(crate::registry::PROPOSALS_DISCARDED, discarded);
        }

        firewood_set!(crate::registry::PROPOSALS_UNCOMMITTED, lock.len());
    }

    /// Closes the revision manager gracefully.
    ///
    /// This method shuts down the background persistence worker and persists
    /// the latest committed revision.
    pub fn close(self) -> Result<(), RevisionManagerError> {
        let current_revision = self.current_revision();
        self.persist_worker
            .close(current_revision)
            .map_err(RevisionManagerError::PersistError)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use firewood_storage::RootReader;

    use super::*;

    impl RevisionManager {
        /// Get all proposal hashes available.
        pub fn proposal_hashes(&self) -> Vec<TrieHash> {
            self.proposals
                .lock()
                .iter()
                .filter_map(|p| p.root_hash().or_default_root_hash())
                .collect()
        }

        /// Wait until all pending commits have been persisted.
        pub(crate) fn wait_persisted(&self) {
            self.persist_worker.wait_persisted();
        }

        /// Returns true if the root node (if it exists) of this revision is
        /// persisted. Otherwise, returns false.
        ///
        /// ## Errors
        ///
        /// Returns an error if the revision does not exist.
        pub(crate) fn revision_persist_status(
            &self,
            root_hash: TrieHash,
        ) -> Result<bool, RevisionManagerError> {
            let revision = self.revision(root_hash)?;
            Ok(revision
                .root_as_maybe_persisted_node()
                .is_some_and(|node| node.unpersisted().is_none()))
        }
    }

    #[test]
    fn test_file_advisory_lock() {
        // Create a temporary file for testing
        let db_dir = tempfile::tempdir().unwrap();

        let config = ConfigManager::builder()
            .root_dir(db_dir.as_ref().to_path_buf())
            .node_hash_algorithm(NodeHashAlgorithm::compile_option())
            .create(true)
            .truncate(false)
            .build();

        // First database instance should open successfully
        let first_manager = RevisionManager::new(config.clone());
        assert!(
            first_manager.is_ok(),
            "First database should open successfully"
        );

        // Second database instance should fail to open due to file locking
        let second_manager = RevisionManager::new(config.clone());
        assert!(
            second_manager.is_err(),
            "Second database should fail to open"
        );

        // Verify the error message contains the expected information
        let error = second_manager.unwrap_err();
        let error_string = error.to_string();

        assert!(
            error_string.contains("database may be opened by another instance"),
            "Error is missing 'database may be opened by another instance', got: {error_string}"
        );

        // The file lock is held by the FileBacked instance. When we drop the first_manager,
        // the Arc<FileBacked> should be dropped, releasing the file lock.
        drop(first_manager.unwrap());

        // Now the second database should open successfully
        let third_manager = RevisionManager::new(config);
        assert!(
            third_manager.is_ok(),
            "Database should open after first instance is dropped"
        );
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn test_slow_concurrent_view_during_commit() {
        use firewood_storage::{
            ImmutableProposal, LeafNode, NibblesIterator, Node, NodeStore, Path,
        };
        use std::sync::Barrier;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::thread;

        const NUM_ITERATIONS: usize = 1000;
        const NUM_VIEWER_THREADS: usize = 10;
        const NUM_COMMITTER_THREADS: usize = 5;

        // Create a temporary database
        let db_dir = tempfile::tempdir().unwrap();

        let config = ConfigManager::builder()
            .root_dir(db_dir.as_ref().to_path_buf())
            .node_hash_algorithm(NodeHashAlgorithm::compile_option())
            .create(true)
            .manager(
                RevisionManagerConfig::builder()
                    .max_revisions(100_000) // Set very high to prevent reaping during test
                    .build(),
            )
            .build();

        let manager = Arc::new(RevisionManager::new(config).unwrap());

        // Create an initial proposal and commit it to have a non-empty base
        let base_revision = manager.current_revision();
        let mut proposal = NodeStore::new(&*base_revision).unwrap();
        {
            let root = proposal.root_mut();
            *root = Some(Node::Leaf(LeafNode {
                partial_path: Path::from_nibbles_iterator(NibblesIterator::new(b"initial")),
                value: b"value".to_vec().into_boxed_slice(),
            }));
        }
        let proposal: Arc<NodeStore<Arc<ImmutableProposal>, _>> =
            Arc::new(proposal.try_into().unwrap());
        manager.add_proposal(proposal.clone());
        manager.commit(proposal).unwrap();

        let error_count = Arc::new(AtomicUsize::new(0));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let barrier = Arc::new(Barrier::new(NUM_VIEWER_THREADS + NUM_COMMITTER_THREADS));

        let mut handles = vec![];

        // Spawn viewer threads
        for thread_id in 0..NUM_VIEWER_THREADS {
            let manager = Arc::clone(&manager);
            let error_count = Arc::clone(&error_count);
            let stop_flag = Arc::clone(&stop_flag);
            let barrier = Arc::clone(&barrier);

            let handle = thread::spawn(move || {
                barrier.wait(); // Synchronize thread start

                let mut local_errors = 0;
                for _ in 0..NUM_ITERATIONS {
                    if stop_flag.load(Ordering::Relaxed) {
                        break;
                    }

                    // Try to view the current revision
                    let current_hash = manager.current_revision().root_hash();
                    if let Some(hash) = current_hash {
                        match manager.view(hash.clone()) {
                            Ok(_) => {}
                            Err(RevisionManagerError::RevisionNotFound { .. }) => {
                                local_errors += 1;
                                eprintln!("Thread {thread_id}: RevisionNotFound for hash {hash:?}");
                            }
                            Err(e) => {
                                eprintln!("Thread {thread_id}: Unexpected error: {e:?}");
                            }
                        }
                    }

                    // Small yield to increase contention
                    thread::yield_now();
                }

                error_count.fetch_add(local_errors, Ordering::Relaxed);
            });

            handles.push(handle);
        }

        // Spawn committer threads
        for thread_id in 0..NUM_COMMITTER_THREADS {
            let manager = Arc::clone(&manager);
            let stop_flag = Arc::clone(&stop_flag);
            let barrier = Arc::clone(&barrier);

            let handle = thread::spawn(move || {
                barrier.wait(); // Synchronize thread start

                for i in 0..NUM_ITERATIONS {
                    if stop_flag.load(Ordering::Relaxed) {
                        break;
                    }

                    // Create and commit a proposal
                    let current = manager.current_revision();
                    let mut new_proposal = match NodeStore::new(&*current) {
                        Ok(p) => p,
                        Err(e) => {
                            eprintln!("Thread {thread_id}: Failed to create proposal: {e:?}");
                            continue;
                        }
                    };

                    // Modify the proposal
                    {
                        let root = new_proposal.root_mut();
                        let key = format!("key_{thread_id}_{i}");
                        *root = Some(Node::Leaf(LeafNode {
                            partial_path: Path::from_nibbles_iterator(NibblesIterator::new(
                                key.as_bytes(),
                            )),
                            value: format!("value_{i}").as_bytes().to_vec().into_boxed_slice(),
                        }));
                    }

                    let immutable: Arc<NodeStore<Arc<ImmutableProposal>, _>> =
                        Arc::new(match new_proposal.try_into() {
                            Ok(p) => p,
                            Err(e) => {
                                eprintln!(
                                    "Thread {thread_id}: Failed to convert to immutable: {e:?}"
                                );
                                continue;
                            }
                        });

                    manager.add_proposal(immutable.clone());

                    match manager.commit(immutable) {
                        Ok(()) | Err(RevisionManagerError::NotLatest { .. }) => {
                            // Expected when multiple threads try to commit
                        }
                        Err(e) => {
                            eprintln!("Thread {thread_id}: Unexpected commit error: {e:?}");
                            stop_flag.store(true, Ordering::Relaxed);
                            break;
                        }
                    }

                    thread::yield_now();
                }
            });

            handles.push(handle);
        }

        // Wait for all threads to complete
        for handle in handles {
            handle.join().unwrap();
        }

        let total_errors = error_count.load(Ordering::Relaxed);

        if total_errors > 0 {
            eprintln!(
                "\nRace condition detected! {total_errors} RevisionNotFound errors occurred."
            );
            eprintln!("This confirms the race condition exists between commit and view.");
        } else {
            eprintln!(
                "\nNo race condition detected in this run. Try running the test multiple times or increasing iterations."
            );
        }

        // For now, we expect the race to occur, so we fail if we see errors
        assert_eq!(
            total_errors, 0,
            "Race condition detected: {total_errors} threads failed to find revisions that should exist"
        );
    }

    #[test]
    fn test_no_fjall_directory_when_root_store_disabled() {
        // Create a temporary directory for the database
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.as_ref().to_path_buf();

        // Create a database with root_store disabled (default)
        let config = ConfigManager::builder()
            .root_dir(db_path.clone())
            .node_hash_algorithm(NodeHashAlgorithm::compile_option())
            .create(true)
            .root_store(false)
            .build();

        let _manager = RevisionManager::new(config).unwrap();

        // Verify that the root_store directory does NOT exist
        let root_store_dir = db_path.join("root_store");
        assert!(
            !root_store_dir.exists(),
            "root_store directory should not be created when root_store is disabled"
        );
    }

    #[test]
    fn test_fjall_directory_when_root_store_enabled() {
        // Create a temporary directory for the database
        let db_dir = tempfile::tempdir().unwrap();
        let db_path = db_dir.as_ref().to_path_buf();

        // Create a database with root_store enabled
        let config = ConfigManager::builder()
            .root_dir(db_path.clone())
            .node_hash_algorithm(NodeHashAlgorithm::compile_option())
            .create(true)
            .root_store(true)
            .build();

        let _manager = RevisionManager::new(config).unwrap();

        // Verify that the root_store directory DOES exist
        let root_store_dir = db_path.join("root_store");
        assert!(
            root_store_dir.exists(),
            "root_store directory should be created when root_store is enabled"
        );
    }

    #[test]
    fn test_cache_config_both_specified_error() {
        // Test that specifying both node_cache_size and node_cache_memory_limit returns an error
        #[expect(deprecated)]
        let result = RevisionManagerConfig::builder()
            .node_cache_size(NonZero::new(1000).unwrap())
            .node_cache_memory_limit(NonZero::new(128_000).unwrap())
            .build()
            .compute_node_cache_memory_limit();

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::v2::api::Error::ConflictingCacheConfig
        ));
    }

    #[test]
    fn test_cache_config_default_memory_limit() {
        // Test that when neither field is specified, we get the default memory limit
        let config = RevisionManagerConfig::builder().build();
        let memory_limit = config.compute_node_cache_memory_limit().unwrap();

        // Default should be 192MB (1,500,000 × 128 bytes)
        assert_eq!(memory_limit.get(), 192_000_000);
    }

    #[test]
    fn test_cache_config_size_to_memory_conversion() {
        // Test that node_cache_size is correctly converted to memory (× 128)
        #[expect(deprecated)]
        let config = RevisionManagerConfig::builder()
            .node_cache_size(NonZero::new(1000).unwrap())
            .build();
        let memory_limit = config.compute_node_cache_memory_limit().unwrap();

        assert_eq!(memory_limit.get(), 1000 * 128);
    }

    #[test]
    fn test_cache_config_memory_limit_used_directly() {
        // Test that node_cache_memory_limit is used directly when specified
        let config = RevisionManagerConfig::builder()
            .node_cache_memory_limit(NonZero::new(256_000_000).unwrap())
            .build();
        let memory_limit = config.compute_node_cache_memory_limit().unwrap();

        assert_eq!(memory_limit.get(), 256_000_000);
    }

    #[test]
    fn test_revision_count() {
        let db_dir = tempfile::tempdir().unwrap();
        let commit_count = nonzero!(10u64);

        // `max_revisions` < `commit_count`
        let config = ConfigManager::builder()
            .root_dir(db_dir.as_ref().to_path_buf())
            .node_hash_algorithm(NodeHashAlgorithm::compile_option())
            .create(true)
            .manager(
                RevisionManagerConfig::builder()
                    .max_revisions(5)
                    .deferred_persistence_commit_count(commit_count)
                    .build(),
            )
            .build();

        let result = RevisionManager::new(config);
        assert!(result.is_err());

        // `max_revisions` == `commit_count`
        let config = ConfigManager::builder()
            .root_dir(db_dir.as_ref().to_path_buf())
            .node_hash_algorithm(NodeHashAlgorithm::compile_option())
            .manager(
                RevisionManagerConfig::builder()
                    .max_revisions(commit_count.get() as usize)
                    .deferred_persistence_commit_count(commit_count)
                    .build(),
            )
            .build();

        let result = RevisionManager::new(config);
        assert!(result.is_err());

        // `max_revisions` > `commit_count`
        let max_revisions = commit_count.get().wrapping_add(1) as usize;
        let config = ConfigManager::builder()
            .root_dir(db_dir.as_ref().to_path_buf())
            .node_hash_algorithm(NodeHashAlgorithm::compile_option())
            .manager(
                RevisionManagerConfig::builder()
                    .max_revisions(max_revisions)
                    .deferred_persistence_commit_count(commit_count)
                    .build(),
            )
            .build();

        let result = RevisionManager::new(config);
        assert!(result.is_ok());
    }
}
