// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![expect(
    clippy::default_trait_access,
    reason = "Found 3 occurrences after enabling the lint."
)]

use nonzero_ext::nonzero;
use parking_lot::{Mutex, MutexGuard, RwLock};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io;
use std::num::{NonZero, NonZeroU64};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use firewood_storage::logger::{trace, warn};
use rayon::{ThreadPool, ThreadPoolBuilder};
use typed_builder::TypedBuilder;

use crate::merkle::Merkle;
use crate::persist_worker::{CanFreeFn, PersistError, PersistWorker};
use crate::root_store::RootStore;
use crate::v2::api::{ArcDynDbView, HashKey, OptionalHashKeyExt, ValidatorId};

use firewood_metrics::{firewood_increment, firewood_set};
pub use firewood_storage::CacheReadStrategy;
use firewood_storage::{
    BranchNode, Committed, FileBacked, FileIoError, ForkId, HashedNodeReader, ImmutableProposal,
    IntoHashType, MAX_FORK_NODES, MAX_VALIDATORS, NodeHashAlgorithm, NodeStore, NodeStoreHeader,
    PersistedForkNode, TrieHash,
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
    /// Fork ID assigned to this chain. Nodes allocated by this chain
    /// are tagged with this ID for safe cross-chain reaping.
    fork_id: ForkId,
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
    /// Fork tree tracking fork relationships for safe node reaping.
    fork_tree: Arc<ForkTree>,
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

    /// Monotonically increasing counter, bumped on each `fork_tree.fork()`.
    /// Used by `can_free` closures to detect concurrent forks that would
    /// invalidate ancestor-reaping decisions.
    fork_generation: Arc<AtomicU64>,

    /// The most recently committed revision that was sent to the persist worker.
    /// Used by `close()` to pass the correct revision to `persist_on_shutdown`
    /// in multi-head mode, where `current_revision()` returns the initial
    /// nodestore rather than the latest validator commit.
    last_committed: Mutex<Option<CommittedRevision>>,
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
    #[error("fork tree capacity exhausted (max {max} nodes)")]
    ForkTreeFull { max: usize },
    #[error("internal error: {0}")]
    InternalError(String),
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
            fork_generation: Arc::new(AtomicU64::new(0)),
            last_committed: Mutex::new(None),
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
                    .reap(oldest, Arc::new(|_| true))
                    .map_err(RevisionManagerError::PersistError)?,
                Err(original) => {
                    warn!("Oldest revision could not be reaped; still referenced");
                    // Re-insert hash that was removed above
                    if let Some(ref hash) = oldest_hash {
                        self.by_hash.write().insert(hash.clone(), original.clone());
                    }
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

    /// Returns both the validator's head revision and fork ID in a single lock acquisition.
    pub fn validator_view_and_fork_id(
        &self,
        id: ValidatorId,
    ) -> Result<(CommittedRevision, ForkId), RevisionManagerError> {
        let state = self
            .multi_head
            .as_ref()
            .ok_or(RevisionManagerError::ValidatorNotFound { id })?
            .read();

        let validator = state
            .validators
            .get(&id)
            .ok_or(RevisionManagerError::ValidatorNotFound { id })?;
        let fork_id = state
            .chains
            .get(&validator.chain)
            .ok_or_else(|| {
                RevisionManagerError::InternalError(format!(
                    "chain {} not found for validator {id:?}",
                    validator.chain
                ))
            })?
            .fork_id;
        Ok((validator.head.clone(), fork_id))
    }

    /// Construct a `can_free` closure from the current fork tree state.
    ///
    /// Captures a snapshot of the fork tree and active fork IDs so that
    /// the reaper can safely determine which nodes to free.
    fn build_can_free(&self, state: &MultiHeadState, chain_fork_id: ForkId) -> CanFreeFn {
        let active_fork_ids: Vec<ForkId> = state.chains.values().map(|c| c.fork_id).collect();
        let fork_tree_snapshot = Arc::clone(&state.fork_tree);
        let gen_at_snapshot = self.fork_generation.load(Ordering::Acquire);
        let fork_gen = Arc::clone(&self.fork_generation);
        Arc::new(move |node_fork_id: ForkId| -> bool {
            // Own allocation: always safe to free regardless of concurrent forks
            if node_fork_id == chain_fork_id {
                return true;
            }
            // Ancestor node: only safe if no new forks happened since snapshot
            if fork_gen.load(Ordering::Acquire) != gen_at_snapshot {
                return false;
            }
            fork_tree_snapshot.can_free(node_fork_id, chain_fork_id, &active_fork_ids)
        })
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

        let fork_tree = Arc::new(ForkTree::new());

        let mut chains = HashMap::new();
        chains.insert(
            0,
            ChainState {
                revisions: chain_revisions,
                fork_id: 0, // root fork
            },
        );

        self.multi_head = Some(RwLock::new(MultiHeadState {
            validators: HashMap::new(),
            chains,
            by_hash,
            hash_to_chain,
            max_validators,
            next_chain_id: 1,
            fork_tree,
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
        // Phase 1: Read all validator data and fork tree from the header (header lock only)
        let header = self.persist_worker.locked_header();
        let count = header.validator_count();

        let mut validators_data = Vec::new();
        let mut validator_fork_ids = Vec::new();
        for slot in 0..count {
            let slot_u8 = u8::try_from(slot).map_err(|_| {
                RevisionManagerError::IOError(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "slot overflow",
                ))
            })?;

            if let Some((validator_id, (addr, hash))) = header.validator_root(slot_u8) {
                let fork_id = header.validator_fork_id(slot_u8);
                validators_data.push((slot_u8, validator_id, addr, hash));
                validator_fork_ids.push((slot_u8, fork_id));
            }
        }

        // Restore fork tree from header
        let fork_tree_next_id = header.fork_tree_next_id();
        let fork_tree_entries = header.fork_tree_entries().to_vec();
        drop(header);

        // Phase 2: Update multi-head state (write lock only, no header lock)
        let mut state = self
            .multi_head
            .as_ref()
            .ok_or(RevisionManagerError::IOError(io::Error::other(
                "multi-head mode not enabled",
            )))?
            .write();

        // Restore the fork tree
        if !fork_tree_entries.is_empty() {
            state.fork_tree = Arc::new(ForkTree::from_persisted(fork_tree_next_id, &fork_tree_entries));
        }

        // Build a map from slot -> fork_id for chain assignment
        let slot_fork_map: HashMap<u8, ForkId> = validator_fork_ids.into_iter().collect();

        for (slot_u8, validator_id, addr, hash) in validators_data {
            let id = ValidatorId::new(validator_id);
            // fork_id=0 is valid for pre-fork validators and legacy databases
            let fork_id = slot_fork_map.get(&slot_u8).copied().unwrap_or(0);

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
                state.next_chain_id = state.next_chain_id.checked_add(1).ok_or(
                    RevisionManagerError::InternalError("chain ID overflow".into()),
                )?;

                state.chains.insert(
                    chain_id,
                    ChainState {
                        revisions: VecDeque::from([committed.clone()]),
                        fork_id,
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

        // Prune fork tree: remove leaf nodes not referenced by any chain.
        // This cleans up phantom entries from crashes between persist_fork_tree
        // and revision persist.
        let active_fork_ids: HashSet<ForkId> = state.chains.values().map(|c| c.fork_id).collect();
        Arc::make_mut(&mut state.fork_tree).prune_unreferenced(&active_fork_ids);

        drop(state);

        // Re-persist the pruned fork tree
        if let Some(mh) = &self.multi_head {
            let state = mh.read();
            self.persist_fork_tree(&state)?;
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
                    state.next_chain_id = state.next_chain_id.checked_add(1).ok_or(
                        RevisionManagerError::InternalError("chain ID overflow".into()),
                    )?;

                    let mut chain_revisions = VecDeque::new();
                    chain_revisions.push_back(base.clone());
                    state.chains.insert(
                        chain_id,
                        ChainState {
                            revisions: chain_revisions,
                            fork_id: 0, // fresh chain inherits root fork
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
        drop(header);

        // Flush header to disk so validator metadata survives restart
        self.flush_header_to_disk()?;

        Ok(())
    }

    /// Deregister a validator, freeing its header slot.
    ///
    /// If this was the last validator on its chain, the chain is cleaned up
    /// and its revisions are reaped.
    pub fn deregister_validator(&self, id: ValidatorId) -> Result<(), RevisionManagerError> {
        let (removed_slot, validator_count, revisions_to_reap, can_free, source_chain) = {
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

            let (revisions_to_reap, can_free) =
                self.cleanup_empty_chain(source_chain, &mut state)?;

            // Persist fork tree changes before releasing lock
            self.persist_fork_tree(&state)?;

            (
                removed.slot,
                validator_count,
                revisions_to_reap,
                can_free,
                source_chain,
            )
        }; // write lock dropped

        // Clear this validator's root in the header (no multi_head lock held)
        let mut header = self.persist_worker.locked_header();
        if let Err(e) = header.set_validator_root(removed_slot, None) {
            return Err(RevisionManagerError::IOError(e));
        }
        header.set_validator_count(validator_count);
        drop(header);

        // Flush header to disk so deregistration survives restart
        self.flush_header_to_disk()?;

        // Reap collected revisions (no locks held, may block)
        let failed = self.reap_revisions(revisions_to_reap, &can_free)?;
        self.reinsert_failed_revisions(source_chain, failed);

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
        source: &str,
    ) -> Result<(), RevisionManagerError> {
        // Check persist worker health (no lock needed)
        self.persist_worker
            .check_error()
            .map_err(RevisionManagerError::PersistError)?;

        // Phase 1: Under write lock (fast, in-memory only)
        let (committed, slot, root_info, revisions_to_reap, can_free, reap_chain_id) = {
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
                let (revisions_to_reap, dedup_can_free) = if source_chain != target_chain {
                    self.cleanup_empty_chain(source_chain, &mut state)?
                } else {
                    (Vec::new(), Arc::new(|_| true) as CanFreeFn)
                };

                let validator = state
                    .validators
                    .get(&id)
                    .ok_or(RevisionManagerError::ValidatorNotFound { id })?;
                let root_info = self.root_info_for_validator(id, validator);
                let slot = validator.slot;

                // Persist fork tree while we still hold the state lock
                self.persist_fork_tree(&state)?;

                drop(state); // release write lock

                // Phase 2 (dedup path): update header (next persist cycle writes it)
                let mut header = self.persist_worker.locked_header();
                if let Err(e) = header.set_validator_root(slot, root_info) {
                    return Err(RevisionManagerError::IOError(e));
                }
                drop(header);

                let failed = self.reap_revisions(revisions_to_reap, &dedup_can_free)?;
                self.reinsert_failed_revisions(source_chain, failed);

                // Phase 3 (dedup path): cleanup proposals
                self.cleanup_proposals_inner(&proposal);

                return Ok(());
            }

            // 3. Check for divergence: other validators on same chain have
            //    a different head than this validator's parent
            let validator = state
                .validators
                .get(&id)
                .ok_or(RevisionManagerError::ValidatorNotFound { id })?;
            let validator_chain = validator.chain;
            let parent_hash = validator.head.root_hash();

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
                    let other_validators: Vec<_> = state
                        .validators
                        .iter()
                        .filter(|(other_id, other_v)| {
                            **other_id != id && other_v.chain == validator_chain
                        })
                        .map(|(vid, _)| *vid)
                        .collect();
                    warn!(
                        "chain divergence detected: validator {id:?} forked to new chain. \
                         parent={parent_hash:?}, new_hash={new_h:?}. \
                         validators on original chain {validator_chain}: {other_validators:?}"
                    );
                    firewood_increment!(crate::registry::VALIDATOR_DIVERGENCE_TOTAL, 1, "source" => source.to_owned());
                }

                // Update fork tree: fork the parent chain's fork_id
                let parent_fork_id = state
                    .chains
                    .get(&validator_chain)
                    .ok_or_else(|| {
                        RevisionManagerError::InternalError(format!(
                            "chain {validator_chain} not found during fork"
                        ))
                    })?
                    .fork_id;
                let active_fork_ids: HashSet<ForkId> =
                    state.chains.values().map(|c| c.fork_id).collect();
                let (continuation_fork_id, new_fork_id) = Arc::make_mut(&mut state.fork_tree)
                    .fork(parent_fork_id, &active_fork_ids)
                    .map_err(|e| match e {
                        ForkError::CapacityExhausted { max } => {
                            RevisionManagerError::ForkTreeFull { max }
                        }
                        ForkError::ParentNotFound(id) => RevisionManagerError::InternalError(
                            format!("fork tree parent {id} not found"),
                        ),
                    })?;
                self.fork_generation.fetch_add(1, Ordering::Release);

                // Update the original chain's fork_id to the continuation
                if let Some(original_chain) = state.chains.get_mut(&validator_chain) {
                    original_chain.fork_id = continuation_fork_id;
                }

                let new_chain_id = state.next_chain_id;
                state.next_chain_id = state.next_chain_id.checked_add(1).ok_or(
                    RevisionManagerError::InternalError("chain ID overflow".into()),
                )?;

                state.chains.insert(
                    new_chain_id,
                    ChainState {
                        revisions: VecDeque::from([committed.clone()]),
                        fork_id: new_fork_id,
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

                // Persist fork tree while we still hold the write lock (matches dedup path)
                self.persist_fork_tree(&state)?;

                // No reaping needed for brand-new chain (only 1 revision)
                let cf = Arc::new(|_| true) as CanFreeFn;
                (committed, slot, root_info, Vec::new(), cf, new_chain_id)
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

                // Build can_free from fork tree for safe reaping
                let chain_fork_id = state
                    .chains
                    .get(&validator_chain)
                    .ok_or_else(|| {
                        RevisionManagerError::InternalError(format!(
                            "chain {validator_chain} not found during append"
                        ))
                    })?
                    .fork_id;
                let cf = self.build_can_free(&state, chain_fork_id);

                // 5. Collect revisions to reap from this chain (per-chain budget)
                let revisions_to_reap =
                    self.collect_reapable_revisions(validator_chain, &mut state);

                (
                    committed,
                    slot,
                    root_info,
                    revisions_to_reap,
                    cf,
                    validator_chain,
                )
            }
        };

        // Phase 2: No lock held (may block)
        // Update header BEFORE persist so the background thread's header write
        // includes the validator root (persist worker writes the full header).
        let mut header = self.persist_worker.locked_header();
        if let Err(e) = header.set_validator_root(slot, root_info.clone()) {
            return Err(RevisionManagerError::IOError(e));
        }
        // Also update legacy root for backward compat
        header.set_root_location(root_info.map(|(_, ri)| ri));
        drop(header);

        self.persist_worker
            .persist(committed.clone())
            .map_err(RevisionManagerError::PersistError)?;

        *self.last_committed.lock() = Some(committed.clone());

        // Reap collected revisions
        let failed = self.reap_revisions(revisions_to_reap, &can_free)?;
        self.reinsert_failed_revisions(reap_chain_id, failed);

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
        let (slot, root_info, revisions_to_reap, can_free, source_chain) = {
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
            let (revisions_to_reap, cf) = if source_chain != target_chain {
                self.cleanup_empty_chain(source_chain, &mut state)?
            } else {
                (Vec::new(), Arc::new(|_| true) as CanFreeFn)
            };

            // Persist fork tree if source chain was cleaned up
            if source_chain != target_chain {
                self.persist_fork_tree(&state)?;
            }

            (slot, root_info, revisions_to_reap, cf, source_chain)
        }; // write lock dropped

        // Update header (no multi_head lock held)
        let mut header = self.persist_worker.locked_header();
        if let Err(e) = header.set_validator_root(slot, root_info) {
            return Err(RevisionManagerError::IOError(e));
        }
        drop(header);

        // Reap collected revisions (no locks held, may block)
        let failed = self.reap_revisions(revisions_to_reap, &can_free)?;
        self.reinsert_failed_revisions(source_chain, failed);

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

    /// Remove a revision's hash from `by_hash` and `hash_to_chain` if no
    /// validator head still references it.
    ///
    /// Takes individual fields rather than `&mut MultiHeadState` to avoid
    /// borrow conflicts when the caller also holds a reference to `chains`.
    fn remove_hash_if_unreferenced(
        validators: &HashMap<ValidatorId, ValidatorState>,
        by_hash: &mut HashMap<TrieHash, CommittedRevision>,
        hash_to_chain: &mut HashMap<TrieHash, ChainId>,
        rev: &CommittedRevision,
    ) {
        if let Some(hash) = rev.root_hash().or_default_root_hash() {
            let still_referenced = validators
                .values()
                .any(|v| v.head.root_hash().or_default_root_hash().as_ref() == Some(&hash));
            if !still_referenced {
                by_hash.remove(&hash);
                hash_to_chain.remove(&hash);
            }
        }
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

        while chain.revisions.len() > self.max_revisions {
            // Stop if ANY validator's head is the oldest revision
            let Some(oldest) = chain.revisions.front() else {
                break;
            };
            let held_by_validator = state
                .validators
                .values()
                .any(|v| v.chain == chain_id && Arc::ptr_eq(&v.head, oldest));
            if held_by_validator {
                break;
            }
            let Some(oldest) = chain.revisions.pop_front() else {
                break;
            };
            Self::remove_hash_if_unreferenced(
                &state.validators,
                &mut state.by_hash,
                &mut state.hash_to_chain,
                &oldest,
            );
            collected.push(oldest);
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
            Self::remove_hash_if_unreferenced(
                &state.validators,
                &mut state.by_hash,
                &mut state.hash_to_chain,
                &rev,
            );
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

    /// Persist the fork tree state to the header.
    ///
    /// Called after fork tree mutations (fork, remove) to ensure the
    /// fork tree is recoverable on restart.
    fn persist_fork_tree(&self, state: &MultiHeadState) -> Result<(), RevisionManagerError> {
        let (next_id, entries) = state.fork_tree.to_persisted();
        let mut header = self.persist_worker.locked_header();
        header
            .set_fork_tree(next_id, &entries)
            .map_err(RevisionManagerError::IOError)?;

        // Also persist per-validator fork_ids
        for validator in state.validators.values() {
            let fork_id = state
                .chains
                .get(&validator.chain)
                .ok_or_else(|| {
                    RevisionManagerError::InternalError(format!(
                        "chain {} not found for validator slot {}",
                        validator.chain, validator.slot
                    ))
                })?
                .fork_id;
            header
                .set_validator_fork_id(validator.slot, fork_id)
                .map_err(RevisionManagerError::IOError)?;
        }
        Ok(())
    }

    /// Flush the in-memory header to disk so validator metadata survives restart.
    /// Caller must NOT already hold the header lock (this method acquires it).
    fn flush_header_to_disk(&self) -> Result<(), RevisionManagerError> {
        let header = self.persist_worker.locked_header();
        header.flush_to(self.storage.as_ref())?;
        Ok(())
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

    /// Reap a list of collected revisions, attempting to free each one.
    ///
    /// Revisions still held by external references (e.g., open views) cannot
    /// be unwrapped and are returned so the caller can re-insert them into
    /// tracking structures. This must be called without any locks held.
    fn reap_revisions(
        &self,
        revisions: Vec<CommittedRevision>,
        can_free: &CanFreeFn,
    ) -> Result<Vec<CommittedRevision>, RevisionManagerError> {
        let mut failed = Vec::new();
        for rev in revisions {
            match Arc::try_unwrap(rev) {
                Ok(owned) => {
                    self.persist_worker
                        .reap(owned, can_free.clone())
                        .map_err(RevisionManagerError::PersistError)?;
                }
                Err(still_held) => {
                    warn!("revision could not be reaped; still referenced");
                    failed.push(still_held);
                }
            }
        }
        Ok(failed)
    }

    /// Re-insert revisions that could not be reaped (still held by external references)
    /// back into the tracking structures.
    ///
    /// If the chain no longer exists, the revisions are dropped — they will be
    /// kept alive by the external `Arc` references and cleaned up when those drop.
    fn reinsert_failed_revisions(&self, chain_id: ChainId, failed: Vec<CommittedRevision>) {
        if failed.is_empty() {
            return;
        }
        let Some(mh) = &self.multi_head else { return };
        let mut state = mh.write();
        if !state.chains.contains_key(&chain_id) {
            // Chain no longer exists — revisions will be dropped when
            // the last external view is closed (acceptable leak).
            return;
        }
        // Re-insert hash tracking first (avoids borrow conflict with chains)
        for rev in failed.iter().rev() {
            if let Some(hash) = rev.root_hash().or_default_root_hash() {
                state
                    .by_hash
                    .entry(hash.clone())
                    .or_insert_with(|| rev.clone());
                state.hash_to_chain.entry(hash).or_insert(chain_id);
            }
        }
        // Then re-insert into chain deque
        if let Some(chain) = state.chains.get_mut(&chain_id) {
            for rev in failed.into_iter().rev() {
                chain.revisions.push_front(rev);
            }
        }
    }

    /// Collect and clean up an empty chain's revisions and fork tree entry.
    ///
    /// Must be called under the write lock. Returns collected revisions and
    /// a `can_free` closure. The fork_id is removed from the fork tree before
    /// returning, so `reap_revisions` can be called after releasing the lock.
    fn cleanup_empty_chain(
        &self,
        source_chain: ChainId,
        state: &mut MultiHeadState,
    ) -> Result<(Vec<CommittedRevision>, CanFreeFn), RevisionManagerError> {
        let source_has_validators = state.validators.values().any(|v| v.chain == source_chain);
        if source_has_validators {
            return Ok((Vec::new(), Arc::new(|_| true) as CanFreeFn));
        }

        // Build can_free BEFORE removing from fork tree so is_ancestor works
        let chain_fork_id = state
            .chains
            .get(&source_chain)
            .ok_or_else(|| {
                RevisionManagerError::InternalError(format!(
                    "chain {source_chain} not found during cleanup"
                ))
            })?
            .fork_id;
        let can_free = self.build_can_free(state, chain_fork_id);

        // Remove this chain's fork_id from the fork tree
        if chain_fork_id != 0 {
            Arc::make_mut(&mut state.fork_tree).remove(chain_fork_id);
        }
        let revisions = self.collect_all_chain_revisions(source_chain, state);
        Ok((revisions, can_free))
    }

    /// Closes the revision manager gracefully.
    ///
    /// This method shuts down the background persistence worker and persists
    /// the latest committed revision, then syncs validator root metadata to
    /// disk so it survives restart.
    pub fn close(self) -> Result<(), RevisionManagerError> {
        // Destructure to retain access to fields after persist_worker is consumed.
        let Self {
            persist_worker,
            in_memory_revisions,
            multi_head,
            storage,
            last_committed,
            ..
        } = self;

        let revision = last_committed
            .lock()
            .take()
            .unwrap_or_else(|| {
                in_memory_revisions
                    .read()
                    .back()
                    .expect("there is always one revision")
                    .clone()
            });

        // Close the persist worker — this joins the background thread,
        // ensuring all revisions are fully persisted and root addresses are
        // available on committed revisions.
        persist_worker
            .close(revision)
            .map_err(RevisionManagerError::PersistError)?;

        // After the persist worker has shut down, sync validator roots to the
        // header on disk. Read the header back from storage, update validator
        // root entries, and flush.
        if let Some(ref multi_head) = multi_head {
            let state = multi_head.read();
            if state.validators.is_empty() {
                return Ok(());
            }
            let mut header = NodeStoreHeader::read_from_storage(storage.as_ref())?;

            // Persist all unique validator head revisions so their nodes are on
            // disk and root_address() returns valid addresses. Dedup by Arc
            // pointer — validators sharing a revision (dedup path) only persist once.
            // Skip heads that the persist worker already flushed (root_address is Some).
            let mut seen = HashSet::new();
            for validator in state.validators.values() {
                let ptr = Arc::as_ptr(&validator.head) as usize;
                if seen.insert(ptr) && validator.head.root_address().is_none() {
                    validator.head.persist(&mut header)?;
                }
            }

            for (vid, validator) in &state.validators {
                let root_info = validator.head.root_hash().and_then(|hash| {
                    let addr = validator.head.root_address()?;
                    Some((vid.id(), (addr, hash)))
                });
                if let Err(e) = header.set_validator_root(validator.slot, root_info) {
                    return Err(RevisionManagerError::IOError(e));
                }
            }
            header.flush_to(storage.as_ref())?;
        }

        Ok(())
    }
}

/// A node in the fork tree representing a single fork lineage point.
#[derive(Debug, Clone)]
struct ForkNode {
    /// Parent fork ID, or `None` for the root.
    parent: Option<ForkId>,
    /// Direct children of this fork node.
    children: HashSet<ForkId>,
    /// Fork IDs absorbed during deferred compaction. NOT persisted to disk.
    /// After restart, nodes with these fork_ids will not be freed (safe space leak).
    ///
    /// When a fork is removed and its parent becomes single-child,
    /// the parent is merged into the child, and the parent's fork_id
    /// is added here for future `is_ancestor` lookups.
    absorbed_ids: HashSet<ForkId>,
}

/// Errors from fork tree operations.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ForkError {
    #[error("fork tree parent {0} not found")]
    ParentNotFound(ForkId),
    #[error("fork tree capacity exhausted (max {max} nodes)")]
    CapacityExhausted { max: usize },
}

/// Tracks fork relationships between chains for safe node reaping.
///
/// Each chain is assigned a fork ID. When chains diverge, both parent and
/// child get new IDs. The fork tree records these relationships so that
/// `can_free` can determine if a node allocated by one fork can be safely
/// freed by another.
///
/// Fork ID 0 is the root (pre-fork era). IDs are monotonically increasing
/// and never reused (u64 space is practically infinite).
#[derive(Debug, Clone)]
pub(crate) struct ForkTree {
    /// Map of fork_id to its node in the tree.
    nodes: HashMap<ForkId, ForkNode>,
    /// Next fork ID to allocate.
    next_id: ForkId,
}

impl ForkTree {
    /// Create a new fork tree with a root node at fork_id=0.
    pub fn new() -> Self {
        let mut nodes = HashMap::new();
        nodes.insert(
            0,
            ForkNode {
                parent: None,
                children: HashSet::new(),
                absorbed_ids: HashSet::new(),
            },
        );
        Self { nodes, next_id: 1 }
    }

    /// Create a fork: allocate two new child IDs under `parent_fork_id`.
    ///
    /// Returns `(new_parent_continuation_id, new_child_id)`.
    /// Both the continuing parent chain and the new child chain get fresh IDs.
    ///
    /// If the tree is at capacity, attempts deferred compaction before failing.
    /// `active_fork_ids` are the fork IDs of all currently active chains —
    /// these are excluded from compaction to prevent removing live nodes.
    ///
    /// # Errors
    ///
    /// Returns [`ForkError::ParentNotFound`] if the parent doesn't exist, or
    /// [`ForkError::CapacityExhausted`] if the tree is full even after compaction.
    pub fn fork(
        &mut self,
        parent_fork_id: ForkId,
        active_fork_ids: &HashSet<ForkId>,
    ) -> Result<(ForkId, ForkId), ForkError> {
        if !self.nodes.contains_key(&parent_fork_id) {
            return Err(ForkError::ParentNotFound(parent_fork_id));
        }
        // Need 2 new slots; check capacity
        if self.nodes.len().saturating_add(2) > MAX_FORK_NODES {
            // Attempt deferred compaction before failing
            self.compact_all(active_fork_ids);
            if self.nodes.len().saturating_add(2) > MAX_FORK_NODES {
                return Err(ForkError::CapacityExhausted {
                    max: MAX_FORK_NODES,
                });
            }
        }

        let continuation_id = self.next_id;
        let child_id = self.next_id.wrapping_add(1);
        self.next_id = self.next_id.wrapping_add(2);

        // Add both as children of the parent
        if let Some(parent) = self.nodes.get_mut(&parent_fork_id) {
            parent.children.insert(continuation_id);
            parent.children.insert(child_id);
        }

        self.nodes.insert(
            continuation_id,
            ForkNode {
                parent: Some(parent_fork_id),
                children: HashSet::new(),
                absorbed_ids: HashSet::new(),
            },
        );
        self.nodes.insert(
            child_id,
            ForkNode {
                parent: Some(parent_fork_id),
                children: HashSet::new(),
                absorbed_ids: HashSet::new(),
            },
        );

        Ok((continuation_id, child_id))
    }

    /// Remove a fork node (e.g., when a chain converges or is deregistered).
    ///
    /// If the removed node's parent becomes a single-child interior node
    /// (no active chain references it), it is compacted: the parent is
    /// absorbed into its remaining child.
    pub fn remove(&mut self, fork_id: ForkId) {
        // Don't remove root
        if fork_id == 0 {
            return;
        }

        let Some(node) = self.nodes.remove(&fork_id) else {
            return;
        };

        // Remove from parent's children
        if let Some(parent_id) = node.parent
            && let Some(parent) = self.nodes.get_mut(&parent_id)
        {
            parent.children.remove(&fork_id);
        }

        // Reparent children to the removed node's parent
        for child_id in &node.children {
            if let Some(child) = self.nodes.get_mut(child_id) {
                child.parent = node.parent;
            }
            if let Some(parent_id) = node.parent
                && let Some(parent) = self.nodes.get_mut(&parent_id)
            {
                parent.children.insert(*child_id);
            }
        }

        // Note: compaction is deferred until the tree is at capacity (compact_all).
        // This avoids creating absorbed_ids that are not persisted to disk.
    }

    /// Compact all inactive single-child interior nodes (except root).
    ///
    /// Called when the fork tree is at capacity to free up slots.
    /// Creates absorbed_ids which are NOT persisted — after restart,
    /// nodes with compacted fork_ids will leak (safe direction).
    fn compact_all(&mut self, active_fork_ids: &HashSet<ForkId>) {
        let compactable: Vec<ForkId> = self
            .nodes
            .iter()
            .filter(|&(&id, node)| {
                id != 0 && node.children.len() == 1 && !active_fork_ids.contains(&id)
            })
            .map(|(&id, _)| id)
            .collect();
        for id in compactable {
            self.try_compact_single(id);
        }
    }

    /// Try to compact a single node: if it has exactly one child,
    /// absorb it into its child.
    fn try_compact_single(&mut self, node_id: ForkId) {
        // Don't compact root
        if node_id == 0 {
            return;
        }

        let Some(node) = self.nodes.get(&node_id) else {
            return;
        };

        if node.children.len() != 1 {
            return;
        }

        let child_id = *node.children.iter().next().expect("checked len == 1");
        let parent_of_node = node.parent;
        let absorbed = node.absorbed_ids.clone();

        // Remove the interior node
        self.nodes.remove(&node_id);

        // Update child's parent
        if let Some(child) = self.nodes.get_mut(&child_id) {
            child.parent = parent_of_node;
            // The child absorbs the compacted node's ID and its previously absorbed IDs
            child.absorbed_ids.insert(node_id);
            child.absorbed_ids.extend(absorbed);
        }

        // Update grandparent's children
        if let Some(parent_id) = parent_of_node
            && let Some(parent) = self.nodes.get_mut(&parent_id)
        {
            parent.children.remove(&node_id);
            parent.children.insert(child_id);
        }
    }

    /// Check if `ancestor` is an ancestor of `descendant` in the fork tree.
    ///
    /// Also checks absorbed IDs: if an interior node was compacted into
    /// a descendant, its fork_id is still considered an ancestor.
    pub fn is_ancestor(&self, ancestor: ForkId, descendant: ForkId) -> bool {
        if ancestor == descendant {
            return true;
        }

        let mut current = descendant;
        loop {
            let Some(node) = self.nodes.get(&current) else {
                return false;
            };

            // Check if the ancestor was absorbed into this node
            if node.absorbed_ids.contains(&ancestor) {
                return true;
            }

            match node.parent {
                Some(parent_id) if parent_id == ancestor => return true,
                Some(parent_id) => current = parent_id,
                None => return false,
            }
        }
    }

    /// Determine if a node with `node_fork_id` can be safely freed by
    /// the chain at `chain_fork_id`, given the set of all active fork IDs.
    ///
    /// A node can be freed if:
    /// 1. It was allocated by this chain (`node_fork_id == chain_fork_id`), OR
    /// 2. It was allocated by an ancestor, AND no other active chain also
    ///    descends from that ancestor.
    pub fn can_free(
        &self,
        node_fork_id: ForkId,
        chain_fork_id: ForkId,
        active_fork_ids: &[ForkId],
    ) -> bool {
        // Own allocation
        if node_fork_id == chain_fork_id {
            return true;
        }

        // Must be an ancestor
        if !self.is_ancestor(node_fork_id, chain_fork_id) {
            return false;
        }

        // No other active chain should also descend from this ancestor
        !active_fork_ids
            .iter()
            .any(|&other| other != chain_fork_id && self.is_ancestor(node_fork_id, other))
    }

    /// Serialize the fork tree for persistence in the header.
    pub fn to_persisted(&self) -> (u64, Vec<PersistedForkNode>) {
        let entries: Vec<PersistedForkNode> = self
            .nodes
            .iter()
            .map(|(&fork_id, node)| PersistedForkNode {
                fork_id,
                parent_fork_id: node.parent.unwrap_or(u64::MAX),
            })
            .collect();
        (self.next_id, entries)
    }

    /// Deserialize the fork tree from persisted header data.
    pub fn from_persisted(next_id: u64, entries: &[PersistedForkNode]) -> Self {
        let mut nodes: HashMap<ForkId, ForkNode> = HashMap::new();

        // First pass: create all nodes
        for entry in entries {
            let parent = if entry.parent_fork_id == u64::MAX {
                None
            } else {
                Some(entry.parent_fork_id)
            };
            nodes.insert(
                entry.fork_id,
                ForkNode {
                    parent,
                    children: HashSet::new(),
                    absorbed_ids: HashSet::new(),
                },
            );
        }

        // Second pass: populate children
        let parent_map: Vec<(ForkId, ForkId)> = nodes
            .iter()
            .filter_map(|(&id, node)| node.parent.map(|p| (p, id)))
            .collect();
        for (parent_id, child_id) in parent_map {
            if let Some(parent) = nodes.get_mut(&parent_id) {
                parent.children.insert(child_id);
            }
        }

        // If entries are empty, create a default tree with root
        if nodes.is_empty() {
            nodes.insert(
                0,
                ForkNode {
                    parent: None,
                    children: HashSet::new(),
                    absorbed_ids: HashSet::new(),
                },
            );
        }

        Self {
            nodes,
            next_id: next_id.max(1),
        }
    }

    /// Remove leaf nodes that are not referenced by any active chain.
    /// Repeats until no more unreferenced leaves exist (handles chains of
    /// unreferenced nodes from leaf inward).
    pub fn prune_unreferenced(&mut self, active_fork_ids: &HashSet<ForkId>) {
        loop {
            let unreferenced_leaves: Vec<ForkId> = self
                .nodes
                .iter()
                .filter(|&(&id, node)| {
                    id != 0 && node.children.is_empty() && !active_fork_ids.contains(&id)
                })
                .map(|(&id, _)| id)
                .collect();
            if unreferenced_leaves.is_empty() {
                break;
            }
            for id in unreferenced_leaves {
                self.remove(id);
            }
        }
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

    // ── ForkTree unit tests ──

    #[test]
    fn test_fork_tree_new_has_root() {
        let tree = ForkTree::new();
        assert!(tree.nodes.contains_key(&0));
        assert_eq!(tree.next_id, 1);
        assert!(tree.nodes[&0].parent.is_none());
        assert!(tree.nodes[&0].children.is_empty());
    }

    #[test]
    fn test_fork_tree_fork_creates_two_children() {
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();
        let (cont, child) = tree.fork(0, &no_active).unwrap();
        assert_eq!(cont, 1);
        assert_eq!(child, 2);
        assert_eq!(tree.next_id, 3);

        // Root should have both as children
        assert!(tree.nodes[&0].children.contains(&1));
        assert!(tree.nodes[&0].children.contains(&2));

        // Both should have root as parent
        assert_eq!(tree.nodes[&1].parent, Some(0));
        assert_eq!(tree.nodes[&2].parent, Some(0));
    }

    #[test]
    fn test_fork_tree_nested_fork() {
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();
        let (cont1, _child1) = tree.fork(0, &no_active).unwrap();
        let (cont2, child2) = tree.fork(cont1, &no_active).unwrap();

        // Tree: 0 -> {1, 2}, 1 -> {3, 4}
        assert_eq!(cont2, 3);
        assert_eq!(child2, 4);
        assert!(tree.nodes[&cont1].children.contains(&3));
        assert!(tree.nodes[&cont1].children.contains(&4));
    }

    #[test]
    fn test_fork_tree_remove_leaf() {
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();
        let (_cont, child) = tree.fork(0, &no_active).unwrap();
        tree.remove(child);

        // Child should be gone
        assert!(!tree.nodes.contains_key(&child));
        // Root should no longer list it
        assert!(!tree.nodes[&0].children.contains(&child));
    }

    #[test]
    fn test_fork_tree_remove_does_not_compact() {
        // Compaction is deferred — remove() no longer auto-compacts.
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();
        // 0 -> {1, 2}
        let (cont, child) = tree.fork(0, &no_active).unwrap();
        assert_eq!(cont, 1);
        assert_eq!(child, 2);

        // Remove child=2. Root keeps only child=1 but is never compacted.
        tree.remove(2);
        assert!(tree.nodes.contains_key(&0));
        assert!(tree.nodes.contains_key(&1));

        // Now create: 1 -> {3, 4}
        let (cont2, child2) = tree.fork(1, &no_active).unwrap();
        assert_eq!(cont2, 3);
        assert_eq!(child2, 4);

        // Remove child2=4. Node 1 now has only child=3 but is NOT compacted.
        tree.remove(child2);
        assert!(
            tree.nodes.contains_key(&1),
            "interior node 1 should NOT be compacted on remove (deferred)"
        );
        assert_eq!(tree.nodes[&3].parent, Some(1));
        assert!(tree.nodes[&3].absorbed_ids.is_empty());
    }

    #[test]
    fn test_fork_tree_compact_all() {
        // compact_all merges single-child interior nodes.
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();
        // 0 -> {1, 2}
        let (cont, _child) = tree.fork(0, &no_active).unwrap();
        // 1 -> {3, 4}
        let (cont2, child2) = tree.fork(cont, &no_active).unwrap();

        // Remove child2(4): node 1 becomes single-child (only 3)
        tree.remove(child2);
        assert!(tree.nodes.contains_key(&cont)); // still present (deferred)

        // compact_all with no active IDs compacts node 1 into 3
        tree.compact_all(&no_active);
        assert!(
            !tree.nodes.contains_key(&cont),
            "interior node 1 should be compacted by compact_all"
        );
        assert_eq!(tree.nodes[&cont2].parent, Some(0));
        assert!(tree.nodes[&cont2].absorbed_ids.contains(&cont));
    }

    #[test]
    fn test_fork_tree_compact_all_skips_active() {
        // compact_all must not compact nodes that are active chain fork_ids.
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();
        // 0 -> {1, 2}
        let (cont, _child) = tree.fork(0, &no_active).unwrap();
        // 1 -> {3, 4}
        let (_cont2, child2) = tree.fork(cont, &no_active).unwrap();

        // Remove child2(4): node 1 becomes single-child
        tree.remove(child2);

        // Mark node 1 (cont) as active — compact_all should skip it
        let active = HashSet::from([cont]);
        tree.compact_all(&active);
        assert!(
            tree.nodes.contains_key(&cont),
            "active node 1 should NOT be compacted"
        );
    }

    #[test]
    fn test_fork_tree_can_free_own_fork_id() {
        let tree = ForkTree::new();
        assert!(tree.can_free(0, 0, &[0]));
    }

    #[test]
    fn test_fork_tree_can_free_ancestor_sole_descendant() {
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();
        let (cont, _child) = tree.fork(0, &no_active).unwrap();
        // Only cont is active
        assert!(tree.can_free(0, cont, &[cont]));
    }

    #[test]
    fn test_fork_tree_cannot_free_shared_ancestor() {
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();
        let (cont, child) = tree.fork(0, &no_active).unwrap();
        // Both active: neither can free ancestor 0
        assert!(!tree.can_free(0, cont, &[cont, child]));
        assert!(!tree.can_free(0, child, &[cont, child]));
    }

    #[test]
    fn test_fork_tree_cannot_free_non_ancestor() {
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();
        let (cont, child) = tree.fork(0, &no_active).unwrap();
        // cont(1) is not ancestor of child(2) — they are siblings
        assert!(!tree.can_free(cont, child, &[cont, child]));
        assert!(!tree.can_free(child, cont, &[cont, child]));
    }

    #[test]
    fn test_fork_tree_convergence_enables_cleanup() {
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();
        let (cont, child) = tree.fork(0, &no_active).unwrap();
        // Remove child (converged), now cont is sole descendant
        tree.remove(child);
        assert!(tree.can_free(0, cont, &[cont]));
    }

    #[test]
    fn test_fork_tree_three_way_fork() {
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();
        // 0 -> {1, 2}
        let (cont, child) = tree.fork(0, &no_active).unwrap();
        // 1 -> {3, 4}  (fork from continuation)
        let (cont2, child2) = tree.fork(cont, &no_active).unwrap();

        let active = [cont, child, cont2, child2];

        // 3 cannot free 0 because 2 also descends from 0
        assert!(!tree.can_free(0, cont2, &active));
        // 3 can free 1 only if no other active chain descends from 1 — but 4 does
        assert!(!tree.can_free(cont, cont2, &[cont2, child2]));
        // If only cont2 is active under 1:
        assert!(tree.can_free(cont, cont2, &[cont2]));
    }

    #[test]
    fn test_fork_tree_is_ancestor() {
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();
        // 0 -> {1, 2}
        let (cont, child) = tree.fork(0, &no_active).unwrap();
        // 1 -> {3, 4}
        let (cont2, _child2) = tree.fork(cont, &no_active).unwrap();

        assert!(tree.is_ancestor(0, cont2)); // 0 -> 1 -> 3
        assert!(tree.is_ancestor(cont, cont2)); // 1 -> 3
        assert!(!tree.is_ancestor(child, cont2)); // 2 is not ancestor of 3
        assert!(!tree.is_ancestor(cont2, 0)); // 3 is not ancestor of 0
        assert!(tree.is_ancestor(cont2, cont2)); // self
    }

    #[test]
    fn test_fork_tree_persistence_roundtrip() {
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();
        tree.fork(0, &no_active).unwrap();
        tree.fork(1, &no_active).unwrap();

        let (next_id, entries) = tree.to_persisted();
        let restored = ForkTree::from_persisted(next_id, &entries);

        assert_eq!(restored.next_id, tree.next_id);
        assert_eq!(restored.nodes.len(), tree.nodes.len());
        for (&id, node) in &tree.nodes {
            let restored_node = restored.nodes.get(&id).unwrap();
            assert_eq!(restored_node.parent, node.parent);
            assert_eq!(restored_node.children, node.children);
        }
    }

    #[test]
    fn test_fork_tree_absorbed_ids_ancestry() {
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();
        // 0 -> {1, 2}
        let (cont, child) = tree.fork(0, &no_active).unwrap();
        // 1 -> {3, 4}
        let (cont2, child2) = tree.fork(cont, &no_active).unwrap();

        // Remove child2(4): node 1 becomes single-child, but NOT compacted yet
        tree.remove(child2);
        assert!(tree.nodes.contains_key(&cont)); // deferred

        // Explicitly compact
        tree.compact_all(&no_active);
        // Now 1 should be absorbed into 3
        assert!(!tree.nodes.contains_key(&cont));
        assert!(tree.nodes[&cont2].absorbed_ids.contains(&cont));

        // is_ancestor should still work through absorbed IDs
        assert!(tree.is_ancestor(cont, cont2)); // 1 was absorbed into 3
        assert!(tree.is_ancestor(0, cont2));

        // can_free should work correctly
        assert!(!tree.can_free(0, cont2, &[cont2, child])); // child(2) still active
        assert!(tree.can_free(cont, cont2, &[cont2])); // 1 absorbed, sole descendant
    }

    #[test]
    fn test_fork_tree_fork_returns_error_when_full() {
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();
        // Fill up to near capacity
        // We start with 1 node (root). Each fork adds 2. MAX_FORK_NODES = 32.
        // So we can do (32-1)/2 = 15 forks from root, getting to 31 nodes.
        for _ in 0..15 {
            assert!(tree.fork(0, &no_active).is_ok());
        }
        // Now we have 1 + 30 = 31 nodes. One more fork needs 2 slots = 33 > 32.
        let err = tree.fork(0, &no_active).unwrap_err();
        assert!(
            matches!(err, ForkError::CapacityExhausted { .. }),
            "expected CapacityExhausted, got {err:?}"
        );
    }

    #[test]
    fn test_fork_tree_fork_parent_not_found() {
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();
        let err = tree.fork(999, &no_active).unwrap_err();
        assert!(
            matches!(err, ForkError::ParentNotFound(999)),
            "expected ParentNotFound(999), got {err:?}"
        );
    }

    #[test]
    fn test_fork_tree_deferred_compaction_frees_capacity() {
        // When the tree is full, fork() should attempt compact_all before failing.
        // We need 2 compactable nodes (each compaction frees 1 slot, fork needs 2).
        let mut tree = ForkTree::new();
        let no_active = HashSet::new();

        // Build chain: 0 -> {1,2}, 1 -> {3,4}, 3 -> {5,6}
        let (cont, _child) = tree.fork(0, &no_active).unwrap();
        let (cont2, child2) = tree.fork(cont, &no_active).unwrap();
        let (_cont3, child3) = tree.fork(cont2, &no_active).unwrap();
        // Remove child2(4) and child3(6): nodes 1 and 3 become single-child
        tree.remove(child2); // 1 -> {3}
        tree.remove(child3); // 3 -> {5}

        // Fill remaining capacity from root
        while tree.nodes.len() + 2 <= MAX_FORK_NODES {
            tree.fork(0, &no_active).unwrap();
        }
        // Tree is now full (31 nodes, need 33 for fork)
        assert!(tree.nodes.len() + 2 > MAX_FORK_NODES);

        // Next fork triggers deferred compaction which merges nodes 1 and 3,
        // freeing 2 slots so the fork succeeds
        assert!(tree.fork(0, &no_active).is_ok());
    }

    #[test]
    fn test_fork_tree_from_persisted_empty() {
        let tree = ForkTree::from_persisted(0, &[]);
        assert!(tree.nodes.contains_key(&0));
        assert_eq!(tree.next_id, 1);
    }

    #[test]
    fn test_can_free_skips_ancestors_after_concurrent_fork() {
        // Build a fork tree with chains A and B sharing ancestor (root 0)
        let mut tree = ForkTree::new();
        let active = HashSet::new();
        let (chain_a, chain_b) = tree.fork(0, &active).unwrap();

        // Simulate build_can_free: snapshot generation, fork tree, and active fork ids
        let generation = Arc::new(AtomicU64::new(0));
        let gen_at_snapshot = generation.load(Ordering::Acquire);
        let fork_gen = Arc::clone(&generation);
        let fork_tree_snapshot = tree.clone();
        let active_fork_ids: Vec<ForkId> = vec![chain_a, chain_b];
        let chain_fork_id = chain_a;

        let can_free = move |node_fork_id: ForkId| -> bool {
            if node_fork_id == chain_fork_id {
                return true;
            }
            if fork_gen.load(Ordering::Acquire) != gen_at_snapshot {
                return false;
            }
            fork_tree_snapshot.can_free(node_fork_id, chain_fork_id, &active_fork_ids)
        };

        // Own allocation: always freed
        assert!(can_free(chain_a), "own allocation should always be freed");

        // Ancestor (root 0): not freed because chain_b also descends from it
        assert!(
            !can_free(0),
            "ancestor shared with chain_b should not be freed"
        );

        // Simulate concurrent fork (bump generation)
        generation.fetch_add(1, Ordering::Release);

        // After generation bump, ancestor freeing is skipped
        assert!(
            !can_free(0),
            "ancestor should not be freed after generation change"
        );
        // Own allocation still freed
        assert!(
            can_free(chain_a),
            "own allocation should still be freed after generation change"
        );
    }

    #[test]
    fn test_prune_unreferenced_removes_phantom_entries() {
        // Create a fork tree and add phantom entries
        let mut tree = ForkTree::new();
        let active = HashSet::new();

        // Fork: 0 -> {1, 2}
        let (chain_a, chain_b) = tree.fork(0, &active).unwrap();
        // Fork chain_a: 1 -> {3, 4}
        let (chain_c, chain_d) = tree.fork(chain_a, &active).unwrap();

        // Only chain_b and chain_c are "active" (have validators)
        let active_ids: HashSet<ForkId> = [chain_b, chain_c].into_iter().collect();

        // chain_d (4) is a phantom leaf
        assert!(tree.nodes.contains_key(&chain_d));

        tree.prune_unreferenced(&active_ids);

        // chain_d should be removed (unreferenced leaf)
        assert!(
            !tree.nodes.contains_key(&chain_d),
            "phantom leaf should be pruned"
        );
        // chain_a (1) was an interior node that became a leaf after chain_d removal,
        // but it also lost chain_c... Actually chain_a still has chain_c as child.
        // chain_b and chain_c should survive
        assert!(
            tree.nodes.contains_key(&chain_b),
            "active chain_b should survive"
        );
        assert!(
            tree.nodes.contains_key(&chain_c),
            "active chain_c should survive"
        );
        // Root should always survive
        assert!(tree.nodes.contains_key(&0), "root should always survive");
    }

    #[test]
    fn test_prune_unreferenced_cascading_removal() {
        // Test that pruning cascades: removing a leaf may expose its parent as
        // a new unreferenced leaf.
        let mut tree = ForkTree::new();
        let active = HashSet::new();

        // 0 -> {1, 2}, 1 -> {3, 4}
        let (chain_a, chain_b) = tree.fork(0, &active).unwrap();
        let (_chain_c, chain_d) = tree.fork(chain_a, &active).unwrap();

        // Only chain_b is active; chain_a's subtree is entirely unreferenced
        let active_ids: HashSet<ForkId> = [chain_b].into_iter().collect();

        tree.prune_unreferenced(&active_ids);

        // chain_d (leaf, unreferenced) is removed first,
        // then chain_c becomes a leaf and is removed,
        // then chain_a becomes a leaf and is removed
        assert!(
            !tree.nodes.contains_key(&chain_d),
            "chain_d should be pruned"
        );
        assert!(
            !tree.nodes.contains_key(&chain_a),
            "chain_a should be pruned after cascade"
        );
        // chain_b and root survive
        assert!(tree.nodes.contains_key(&chain_b));
        assert!(tree.nodes.contains_key(&0));
    }
}
