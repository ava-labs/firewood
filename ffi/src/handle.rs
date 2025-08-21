// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::num::NonZeroUsize;

use firewood::{
    db::{Db, DbConfig},
    manager::RevisionManagerConfig,
    v2::api::{self, ArcDynDbView, Db as _, DbView, HashKey, Proposal as _},
};
use metrics::counter;

use crate::{BorrowedBytes, DatabaseHandle, KeyValuePair};

const DEFAULT_CACHE_SIZE: NonZeroUsize = const { NonZeroUsize::new(1_500_000).unwrap() };
const DEFAULT_FREE_LIST_CACHE_SIZE: NonZeroUsize = const { NonZeroUsize::new(40_000).unwrap() };
const DEFAULT_REVISIONS: usize = 128;

/// The cache read strategy to use for the database.
///
/// This controls what types of database operations are cached to improve
/// performance by avoiding redundant disk reads and computations.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum CacheReadStrategy {
    /// Only cache write operations. This is the most conservative strategy
    /// that minimizes memory usage but may result in more disk reads.
    WritesOnly = 0,

    /// Cache both write operations and branch node reads. This provides
    /// better performance for tree traversal operations while keeping
    /// memory usage moderate.
    BranchReads = 1,

    /// Cache all read and write operations. This provides the best performance
    /// but uses the most memory as it caches leaf nodes and values in addition
    /// to branch nodes.
    All = 2,
}

impl From<CacheReadStrategy> for firewood::manager::CacheReadStrategy {
    fn from(strategy: CacheReadStrategy) -> Self {
        match strategy {
            CacheReadStrategy::WritesOnly => firewood::manager::CacheReadStrategy::WritesOnly,
            CacheReadStrategy::BranchReads => firewood::manager::CacheReadStrategy::BranchReads,
            CacheReadStrategy::All => firewood::manager::CacheReadStrategy::All,
        }
    }
}

impl From<firewood::manager::CacheReadStrategy> for CacheReadStrategy {
    fn from(strategy: firewood::manager::CacheReadStrategy) -> Self {
        match strategy {
            firewood::manager::CacheReadStrategy::WritesOnly => CacheReadStrategy::WritesOnly,
            firewood::manager::CacheReadStrategy::BranchReads => CacheReadStrategy::BranchReads,
            firewood::manager::CacheReadStrategy::All => CacheReadStrategy::All,
        }
    }
}

/// Arguments for creating or opening a database. These are passed to [`fwd_open_db`]
///
/// [`fwd_open_db`]: crate::fwd_open_db
#[repr(C)]
#[derive(Debug)]
pub struct DatabaseHandleArgs<'a> {
    /// The path to the database file.
    ///
    /// This must be a valid UTF-8 string, even on Windows.
    ///
    /// If this is empty, an error will be returned.
    pub path: BorrowedBytes<'a>,

    /// The size of the node cache. If zero, the default size of 1,500,000 nodes
    /// will be used.
    pub cache_size: usize,

    /// The size of the free list cache. If zero, the default size of 40,000 nodes
    /// will be used.
    pub free_list_cache_size: usize,

    /// The maximum number of revisions to keep. If zero, the default of 128
    /// revisions will be used.
    ///
    /// If this is less than 2, an error will be returned.
    pub revisions: usize,

    /// The cache read strategy to use.
    pub strategy: CacheReadStrategy,

    /// Whether to truncate the database file if it exists.
    pub truncate: bool,
}

impl DatabaseHandleArgs<'_> {
    const fn cache_size(&self) -> NonZeroUsize {
        match NonZeroUsize::new(self.cache_size) {
            Some(size) => size,
            None => DEFAULT_CACHE_SIZE,
        }
    }

    const fn free_list_cache_size(&self) -> NonZeroUsize {
        match NonZeroUsize::new(self.free_list_cache_size) {
            Some(size) => size,
            None => DEFAULT_FREE_LIST_CACHE_SIZE,
        }
    }

    const fn revisions(&self) -> usize {
        if self.revisions == 0 {
            DEFAULT_REVISIONS
        } else {
            // if 1, building the revision manager will fail, but that's on the caller
            self.revisions
        }
    }

    fn as_rev_manager_config(&self) -> RevisionManagerConfig {
        RevisionManagerConfig::builder()
            .node_cache_size(self.cache_size())
            .free_list_cache_size(self.free_list_cache_size())
            .max_revisions(self.revisions())
            .cache_read_strategy(self.strategy.into())
            .build()
    }
}

impl DatabaseHandle<'_> {
    /// Creates a new database handle from the given arguments.
    ///
    /// # Errors
    ///
    /// If the path is empty, or if the configuration is invalid, this will return an error.
    pub fn new(args: DatabaseHandleArgs<'_>) -> Result<Self, api::Error> {
        let cfg = DbConfig::builder()
            .truncate(args.truncate)
            .manager(args.as_rev_manager_config())
            .build();

        let path = args
            .path
            .as_str()
            .map_err(|err| invalid_data(format!("database path contains invalid utf-8: {err}")))?;

        if path.is_empty() {
            return Err(invalid_data("database path cannot be empty"));
        }

        Db::new(path, cfg).map(Self::from)
    }

    /// Returns the current root hash of the database.
    ///
    /// # Errors
    ///
    /// An error is returned if there was an i/o error while reading the root hash.
    pub fn current_root_hash(&self) -> Result<Option<HashKey>, api::Error> {
        self.db.root_hash()
    }

    /// Creates a proposal with the given values and returns the proposal and the start time.
    ///
    /// # Errors
    ///
    /// An error is returned if the proposal could not be created.
    pub fn create_batch<'kvp>(
        &self,
        values: (impl AsRef<[KeyValuePair<'kvp>]> + 'kvp),
    ) -> Result<Option<HashKey>, api::Error> {
        let start = coarsetime::Instant::now();

        let proposal = self.db.propose(values.as_ref())?;

        let propose_time = start.elapsed().as_millis();
        counter!("firewood.ffi.propose_ms").increment(propose_time);

        let hash_val = proposal.root_hash()?;

        proposal.commit()?;

        let propose_plus_commit_time = start.elapsed().as_millis();
        counter!("firewood.ffi.batch_ms").increment(propose_plus_commit_time);
        counter!("firewood.ffi.commit_ms")
            .increment(propose_plus_commit_time.saturating_sub(propose_time));
        counter!("firewood.ffi.batch").increment(1);

        Ok(hash_val)
    }

    /// Commits a proposal with the given ID.
    ///
    /// # Errors
    ///
    /// An error is returned if the proposal could not be committed, or if the proposal ID is invalid.
    pub fn commit_proposal(&self, proposal_id: u32) -> Result<Option<HashKey>, String> {
        let proposal = self
            .proposals
            .write()
            .map_err(|_| "proposal lock is poisoned")?
            .remove(&proposal_id)
            .ok_or("proposal not found")?;

        // Get the proposal hash and cache the view. We never cache an empty proposal.
        let proposal_hash = proposal.root_hash().map_err(|e| e.to_string())?;

        if let Some(ref hash_key) = proposal_hash {
            _ = self.get_root(hash_key.clone());
        }

        // Commit the proposal
        let result = proposal.commit().map_err(|e| e.to_string());

        // Clear the cache, which will force readers after this point to find the committed root hash
        self.clear_cached_view();

        result.map(|()| proposal_hash)
    }

    pub(crate) fn get_root(&self, root: HashKey) -> Result<ArcDynDbView, api::Error> {
        // construct outside of the closure so that when the closure is dropped
        // without executing, it triggers the hit counter
        let token = CachedViewHitOnDrop;
        self.cached_view.get_or_try_insert_with(root, move |key| {
            token.miss();
            self.db.view(HashKey::clone(key))
        })
    }

    pub(crate) fn clear_cached_view(&self) {
        self.cached_view.clear();
    }
}

/// A RAII metrics helper that tracks cache hits and misses for database views.
///
/// This type uses the drop pattern to automatically record cache metrics:
/// - By default, dropping this type records a cache hit
/// - Calling [`Self::miss`] before dropping records a cache miss instead
///
/// This ensures that every cache lookup is properly tracked in metrics without
/// requiring manual instrumentation at each call site.
///
/// # Metrics Recorded
///
/// - `firewood.ffi.cached_view.hit` - Incremented when the cache lookup succeeds
/// - `firewood.ffi.cached_view.miss` - Incremented when the cache lookup fails
struct CachedViewHitOnDrop;

impl Drop for CachedViewHitOnDrop {
    fn drop(&mut self) {
        counter!("firewood.ffi.cached_view.hit").increment(1);
    }
}

impl CachedViewHitOnDrop {
    fn miss(self) {
        std::mem::forget(self);
        counter!("firewood.ffi.cached_view.miss").increment(1);
    }
}

type BoxErr = Box<dyn std::error::Error + Send + Sync>;

fn invalid_data(error: impl Into<BoxErr>) -> api::Error {
    api::Error::IO(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}
