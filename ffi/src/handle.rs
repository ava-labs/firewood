// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::num::NonZeroUsize;

use firewood::{
    db::{Db, DbConfig},
    manager::RevisionManagerConfig,
    v2::api::{self, ArcDynDbView, HashKey},
};
use metrics::counter;

use crate::{BorrowedBytes, DatabaseHandle};

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

    pub(crate) fn get_root(&self, root: HashKey) -> Result<ArcDynDbView, api::Error> {
        let mut cache_miss = false;
        let view = self.cached_view.get_or_try_insert_with(root, |key| {
            cache_miss = true;
            self.db.view(HashKey::clone(key))
        })?;

        if cache_miss {
            counter!("firewood.ffi.cached_view.miss").increment(1);
        } else {
            counter!("firewood.ffi.cached_view.hit").increment(1);
        }

        Ok(view)
    }

    pub(crate) fn clear_cached_view(&self) {
        self.cached_view.clear();
    }
}

fn invalid_data(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> api::Error {
    api::Error::IO(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}
