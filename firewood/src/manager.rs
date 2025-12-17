// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![expect(
    clippy::cast_precision_loss,
    reason = "Found 2 occurrences after enabling the lint."
)]
#![expect(
    clippy::default_trait_access,
    reason = "Found 3 occurrences after enabling the lint."
)]

use parking_lot::{Mutex, RwLock};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::num::NonZero;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use firewood_storage::logger::{trace, warn};
use metrics::gauge;
use rayon::{ThreadPool, ThreadPoolBuilder};
use typed_builder::TypedBuilder;

use crate::merkle::Merkle;
use crate::root_store::RootStore;
use crate::v2::api::{ArcDynDbView, HashKey, OptionalHashKeyExt};

pub use firewood_storage::CacheReadStrategy;
use firewood_storage::{
    BranchNode, Committed, FileBacked, FileIoError, HashedNodeReader, ImmutableProposal, Lockable,
    MemStore, NodeStore, TrieHash, WritableStorage,
};

/// Trait for storage backends that may support historical revision queries.
///
/// This trait enables the `RevisionManager` to have a single `revision` method
/// that behaves correctly for all storage types. For `FileBacked` storage with
/// `RootStore` enabled, it can retrieve revisions from persistent storage.
/// For other storage types (like `MemStore`), it returns `None`.
pub(crate) trait HistoricalRevisionStorage: WritableStorage + Lockable + Sized {
    /// Attempt to retrieve a historical revision from persistent storage.
    ///
    /// Returns `Ok(Some(revision))` if the revision was found in historical storage,
    /// `Ok(None)` if historical storage is not available or doesn't contain the revision,
    /// or an error if the lookup failed.
    fn get_historical_revision(
        manager: &RevisionManager<Self>,
        hash: &TrieHash,
    ) -> Result<Option<CommittedRevision<Self>>, RevisionManagerError>;
}

impl HistoricalRevisionStorage for FileBacked {
    fn get_historical_revision(
        manager: &RevisionManager<Self>,
        hash: &TrieHash,
    ) -> Result<Option<CommittedRevision<Self>>, RevisionManagerError> {
        if let Some(root_store) = &manager.root_store {
            root_store
                .get(hash)
                .map_err(RevisionManagerError::RootStoreError)
        } else {
            Ok(None)
        }
    }
}

impl HistoricalRevisionStorage for MemStore {
    fn get_historical_revision(
        _manager: &RevisionManager<Self>,
        _hash: &TrieHash,
    ) -> Result<Option<CommittedRevision<Self>>, RevisionManagerError> {
        // MemStore has no historical storage
        Ok(None)
    }
}

const DB_FILE_NAME: &str = "firewood.db";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, TypedBuilder)]
/// Revision manager configuratoin
pub struct RevisionManagerConfig {
    /// The number of committed revisions to keep in memory.
    #[builder(default = 128)]
    max_revisions: usize,

    /// The size of the node cache
    #[builder(default_code = "NonZero::new(1500000).expect(\"non-zero\")")]
    node_cache_size: NonZero<usize>,

    #[builder(default_code = "NonZero::new(40000).expect(\"non-zero\")")]
    free_list_cache_size: NonZero<usize>,

    #[builder(default = CacheReadStrategy::WritesOnly)]
    cache_read_strategy: CacheReadStrategy,
}

#[derive(Clone, Debug, TypedBuilder)]
#[non_exhaustive]
/// Configuration manager that contains both truncate and revision manager config
pub struct ConfigManager {
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

/// A committed revision backed by storage type `S`.
pub type CommittedRevision<S> = Arc<NodeStore<Committed, S>>;
type ProposedRevision<S> = Arc<NodeStore<Arc<ImmutableProposal>, S>>;

/// Manages database revisions and proposals for a given storage backend.
///
/// The `RevisionManager` is generic over the storage type `S`, which must implement
/// both `WritableStorage` and `Lockable`. This allows it to work with different
/// storage backends, such as `FileBacked` for production use or `MemStore` for testing.
#[derive(Debug)]
pub(crate) struct RevisionManager<S: WritableStorage + Lockable> {
    /// Maximum number of revisions to keep in memory.
    ///
    /// When this limit is exceeded, the oldest revision is removed from memory.
    /// If `root_store` is `None`, the oldest revision's nodes are added to the
    /// free list for space reuse. Otherwise, the oldest revision
    /// is preserved on disk for historical queries.
    max_revisions: usize,

    /// FIFO queue of committed revisions kept in memory. The queue always
    /// contains at least one revision.
    in_memory_revisions: RwLock<VecDeque<CommittedRevision<S>>>,

    /// Hash-based index of committed revisions kept in memory.
    ///
    /// Maps root hash to the corresponding committed revision for O(1) lookup
    /// performance. This allows efficient retrieval of revisions without
    /// scanning through the `in_memory_revisions` queue.
    by_hash: RwLock<HashMap<TrieHash, CommittedRevision<S>>>,

    /// Active proposals that have not yet been committed.
    proposals: Mutex<Vec<ProposedRevision<S>>>,

    /// Lazily initialized thread pool for parallel operations.
    threadpool: OnceLock<ThreadPool>,

    /// Optional persistent store for historical root addresses.
    ///
    /// When present, enables retrieval of revisions beyond `max_revisions` by
    /// persisting root hash to disk address mappings. This allows historical
    /// queries of arbitrarily old revisions without keeping them in memory.
    /// Note: `RootStore` only works with `FileBacked` storage.
    root_store: Option<RootStore>,
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
}

impl RevisionManager<FileBacked> {
    /// Create a new revision manager backed by file-based storage.
    ///
    /// This constructor creates a `FileBacked` storage instance and optionally
    /// enables `RootStore` for historical revision queries.
    pub fn new(db_dir: PathBuf, config: ConfigManager) -> Result<Self, RevisionManagerError> {
        if config.create {
            std::fs::create_dir_all(&db_dir).map_err(RevisionManagerError::IOError)?;
        }

        let file = db_dir.join(DB_FILE_NAME);
        let fb = FileBacked::new(
            file,
            config.manager.node_cache_size,
            config.manager.free_list_cache_size,
            config.truncate,
            config.create,
            config.manager.cache_read_strategy,
        )?;

        // Acquire an advisory lock on the database file to prevent multiple processes
        // from opening the same database simultaneously
        fb.lock()?;

        let storage = Arc::new(fb);
        let nodestore = Arc::new(NodeStore::open(storage.clone())?);
        let root_store = config.root_store.then_some({
            let root_store_dir = db_dir.join("root_store");

            RootStore::new(root_store_dir, storage.clone(), config.truncate)
                .map_err(RevisionManagerError::RootStoreError)?
        });

        let manager = Self {
            max_revisions: config.manager.max_revisions,
            in_memory_revisions: RwLock::new(VecDeque::from([nodestore.clone()])),
            by_hash: RwLock::new(Default::default()),
            proposals: Mutex::new(Default::default()),
            threadpool: OnceLock::new(),
            root_store,
        };

        if let Some(hash) = nodestore.root_hash().or_default_root_hash() {
            manager.by_hash.write().insert(hash, nodestore.clone());
        }

        if config.truncate {
            nodestore.flush_header_with_padding()?;
        }

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
}

impl<S: WritableStorage + Lockable + 'static> RevisionManager<S> {
    /// Commit a proposal
    /// To commit a proposal involves a few steps:
    /// 1. Commit check.
    ///    The proposal's parent must be the last committed revision, otherwise the commit fails.
    ///    It only contains the address of the nodes that are deleted, which should be very small.
    /// 2. Revision reaping.
    ///    If more than the maximum number of revisions are kept in memory, the
    ///    oldest revision is removed from memory. If `RootStore` does not exist,
    ///    the oldest revision's nodes are added to the free list for space reuse.
    ///    Otherwise, the oldest revision's nodes are preserved on disk, which
    ///    is useful for historical queries.
    /// 3. Persist to disk. This includes flushing everything to disk.
    /// 4. Persist the revision to `RootStore`.
    /// 5. Set last committed revision.
    ///    Set last committed revision in memory.
    /// 6. Proposal Cleanup.
    ///    Any other proposals that have this proposal as a parent should be reparented to the committed version.
    #[fastrace::trace(short_name = true)]
    #[crate::metrics("firewood.proposal.commit", "proposal commit to storage")]
    pub fn commit(&self, proposal: ProposedRevision<S>) -> Result<(), RevisionManagerError> {
        // 1. Commit check
        let current_revision = self.current_revision();
        if !proposal.parent_hash_is(current_revision.root_hash()) {
            return Err(RevisionManagerError::NotLatest {
                provided: proposal.root_hash(),
                expected: current_revision.root_hash(),
            });
        }

        let mut committed = proposal.as_committed(&current_revision);

        // 2. Revision reaping
        // When we exceed max_revisions, remove the oldest revision from memory.
        // If `RootStore` does not exist, add the oldest revision's nodes to the free list.
        // If you crash after freeing some of these, then the free list will point to nodes that are not actually free.
        // TODO: Handle the case where we get something off the free list that is not free
        while self.in_memory_revisions.read().len() >= self.max_revisions {
            let oldest = self
                .in_memory_revisions
                .write()
                .pop_front()
                .expect("must be present");
            let oldest_hash = oldest.root_hash().or_default_root_hash();
            if let Some(ref hash) = oldest_hash {
                self.by_hash.write().remove(hash);
            }

            // We reap the revision's nodes only if `RootStore` does not exist.
            if self.root_store.is_none() {
                // This `try_unwrap` is safe because nobody else will call `try_unwrap` on this Arc
                // in a different thread, so we don't have to worry about the race condition where
                // the Arc we get back is not usable as indicated in the docs for `try_unwrap`.
                // This guarantee is there because we have a `&mut self` reference to the manager, so
                // the compiler guarantees we are the only one using this manager.
                match Arc::try_unwrap(oldest) {
                    Ok(oldest) => oldest.reap_deleted(&mut committed)?,
                    Err(original) => {
                        warn!("Oldest revision could not be reaped; still referenced");
                        self.in_memory_revisions.write().push_front(original);
                        break;
                    }
                }
            }
            gauge!("firewood.active_revisions").set(self.in_memory_revisions.read().len() as f64);
            gauge!("firewood.max_revisions").set(self.max_revisions as f64);
        }

        // 3. Persist to disk.
        // TODO: We can probably do this in another thread, but it requires that
        // we move the header out of NodeStore, which is in a future PR.
        committed.persist()?;

        // 4. Persist revision to root store
        if let Some(store) = &self.root_store
            && let (Some(hash), Some(address)) = (committed.root_hash(), committed.root_address())
        {
            store
                .add_root(&hash, &address)
                .map_err(RevisionManagerError::RootStoreError)?;
        }

        // 5. Set last committed revision
        // The revision is added to `by_hash` here while it still exists in `proposals`.
        // The `view()` method relies on this ordering - it checks `proposals` first,
        // then `by_hash`, ensuring the revision is always findable during the transition.
        let committed: CommittedRevision<S> = committed.into();
        self.in_memory_revisions
            .write()
            .push_back(committed.clone());
        if let Some(hash) = committed.root_hash().or_default_root_hash() {
            self.by_hash.write().insert(hash, committed.clone());
        }

        // 6. Proposal Cleanup
        // Free proposal that is being committed as well as any proposals no longer
        // referenced by anyone else.
        self.proposals
            .lock()
            .retain(|p| !Arc::ptr_eq(&proposal, p) && Arc::strong_count(p) > 1);

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
    ///
    /// Checks proposals first, then committed revisions (both in-memory and
    /// historical storage if available).
    pub fn view(&self, root_hash: HashKey) -> Result<ArcDynDbView, RevisionManagerError>
    where
        S: HistoricalRevisionStorage,
    {
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

    pub fn add_proposal(&self, proposal: ProposedRevision<S>) {
        self.proposals.lock().push(proposal);
    }

    /// Retrieve a committed revision by its root hash.
    ///
    /// This method first checks the in-memory revision cache, then falls back
    /// to historical storage if the storage backend supports it (e.g., `RootStore`
    /// for `FileBacked` storage).
    pub fn revision(&self, root_hash: HashKey) -> Result<CommittedRevision<S>, RevisionManagerError>
    where
        S: HistoricalRevisionStorage,
    {
        // 1. Check the in-memory revision cache.
        if let Some(revision) = self.by_hash.read().get(&root_hash).cloned() {
            return Ok(revision);
        }

        // 2. Check historical storage (implementation depends on S).
        if let Some(revision) = S::get_historical_revision(self, &root_hash)? {
            return Ok(revision);
        }

        Err(RevisionManagerError::RevisionNotFound {
            provided: root_hash,
        })
    }

    pub fn root_hash(&self) -> Result<Option<HashKey>, RevisionManagerError> {
        Ok(self.current_revision().root_hash())
    }

    pub fn current_revision(&self) -> CommittedRevision<S> {
        self.in_memory_revisions
            .read()
            .back()
            .expect("there is always one revision")
            .clone()
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
}

/*
Durable, but all nodes are also kept in memory
In memory, but not durable
*/

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    impl<S: WritableStorage + Lockable + 'static> RevisionManager<S> {
        /// Create a new revision manager with an existing storage backend.
        ///
        /// This constructor accepts a pre-constructed storage backend and does not
        /// support `RootStore` (historical revision queries beyond `max_revisions`).
        /// Use this for in-memory storage backends or when you need more control
        /// over the storage initialization.
        pub fn with_storage(
            storage: Arc<S>,
            config: RevisionManagerConfig,
            truncate: bool,
        ) -> Result<Self, RevisionManagerError> {
            storage.lock()?;

            let nodestore = Arc::new(NodeStore::open(storage)?);

            let manager = Self {
                max_revisions: config.max_revisions,
                in_memory_revisions: RwLock::new(VecDeque::from([nodestore.clone()])),
                by_hash: RwLock::new(Default::default()),
                proposals: Mutex::new(Default::default()),
                threadpool: OnceLock::new(),
                root_store: None, // RootStore not supported with generic storage
            };

            if let Some(hash) = nodestore.root_hash().or_default_root_hash() {
                manager.by_hash.write().insert(hash, nodestore.clone());
            }

            if truncate {
                nodestore.flush_header_with_padding()?;
            }

            Ok(manager)
        }
    }

    impl RevisionManager<FileBacked> {
        /// Get all proposal hashes available.
        pub fn proposal_hashes(&self) -> Vec<TrieHash> {
            self.proposals
                .lock()
                .iter()
                .filter_map(|p| p.root_hash().or_default_root_hash())
                .collect()
        }
    }

    #[test]
    fn test_file_advisory_lock() {
        // Create a temporary file for testing
        let db_dir = tempfile::tempdir().unwrap();

        let config = ConfigManager::builder()
            .create(true)
            .truncate(false)
            .build();

        // First database instance should open successfully
        let first_manager = RevisionManager::new(db_dir.as_ref().to_path_buf(), config.clone());
        assert!(
            first_manager.is_ok(),
            "First database should open successfully"
        );

        // Second database instance should fail to open due to file locking
        let second_manager = RevisionManager::new(db_dir.as_ref().to_path_buf(), config.clone());
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
        let third_manager = RevisionManager::new(db_dir.as_ref().to_path_buf(), config);
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
            .create(true)
            .manager(
                RevisionManagerConfig::builder()
                    .max_revisions(100_000) // Set very high to prevent reaping during test
                    .build(),
            )
            .build();

        let manager =
            Arc::new(RevisionManager::new(db_dir.as_ref().to_path_buf(), config).unwrap());

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
}
