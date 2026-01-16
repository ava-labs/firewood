// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood::{
    db::{Db, DbConfig},
    manager::RevisionManagerConfig,
    v2::api::{self, ArcDynDbView, Db as _, DbView, HashKey, HashKeyExt, IntoBatchIter, KeyType},
};

use crate::{BorrowedBytes, CView, CreateProposalResult, KeyValuePair, arc_cache::ArcCache};

use crate::revision::{GetRevisionResult, RevisionHandle};
use firewood_metrics::{MetricsContext, firewood_increment, firewood_record};

/// The hashing mode to use for the database.
///
/// This determines the cryptographic hash function and trie structure used.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeHashAlgorithm {
    /// MerkleDB Firewood hashing (SHA-256 based)
    MerkleDB = 0,
    /// Ethereum-compatible hashing (Keccak-256 based)
    Ethereum = 1,
}

impl From<NodeHashAlgorithm> for firewood_storage::NodeHashAlgorithm {
    fn from(alg: NodeHashAlgorithm) -> Self {
        match alg {
            NodeHashAlgorithm::MerkleDB => firewood_storage::NodeHashAlgorithm::MerkleDB,
            NodeHashAlgorithm::Ethereum => firewood_storage::NodeHashAlgorithm::Ethereum,
        }
    }
}

/// Arguments for creating or opening a database. These are passed to [`fwd_open_db`]
///
/// [`fwd_open_db`]: crate::fwd_open_db
#[repr(C)]
#[derive(Debug)]
pub struct DatabaseHandleArgs<'a> {
    /// The path to the database directory.
    ///
    /// This must be a valid UTF-8 string.
    ///
    /// If this is empty, an error will be returned.
    pub dir: BorrowedBytes<'a>,

    /// Whether to enable `RootStore`.
    ///
    /// Note: Setting this feature will only track new revisions going forward
    /// and will not contain revisions from a prior database instance that didn't
    /// enable `root_store`.
    pub root_store: bool,

    /// The size of the node cache.
    ///
    /// Opening returns an error if this is zero.
    pub cache_size: usize,

    /// The size of the free list cache.
    ///
    /// Opening returns an error if this is zero.
    pub free_list_cache_size: usize,

    /// The maximum number of revisions to keep.
    pub revisions: usize,

    /// The cache read strategy to use.
    ///
    /// This must be one of the following:
    ///
    /// - `0`: No cache.
    /// - `1`: Cache only branch reads.
    /// - `2`: Cache all reads.
    ///
    /// Opening returns an error if this is not one of the above values.
    pub strategy: u8,

    /// Whether to truncate the database file if it exists.
    pub truncate: bool,

    /// Whether to enable expensive metrics recording for this database handle.
    ///
    /// Expensive metrics are disabled by default.
    pub expensive_metrics: bool,

    /// The hashing mode to use for the database.
    ///
    /// This must match the compile-time feature:
    /// - [`NodeHashAlgorithm::Ethereum`] if the `ethhash` feature is enabled
    /// - [`NodeHashAlgorithm::MerkleDB`] if the `ethhash` feature is disabled
    ///
    /// Opening returns an error if this does not match the compile-time feature.
    pub node_hash_algorithm: NodeHashAlgorithm,
}

impl DatabaseHandleArgs<'_> {
    fn as_rev_manager_config(&self) -> Result<RevisionManagerConfig, api::Error> {
        let cache_read_strategy = match self.strategy {
            0 => firewood::manager::CacheReadStrategy::WritesOnly,
            1 => firewood::manager::CacheReadStrategy::BranchReads,
            2 => firewood::manager::CacheReadStrategy::All,
            _ => return Err(invalid_data("invalid cache strategy")),
        };
        let config = RevisionManagerConfig::builder()
            .node_cache_size(
                self.cache_size
                    .try_into()
                    .map_err(|_| invalid_data("cache size should be non-zero"))?,
            )
            .max_revisions(self.revisions)
            .cache_read_strategy(cache_read_strategy)
            .free_list_cache_size(
                self.free_list_cache_size
                    .try_into()
                    .map_err(|_| invalid_data("free list cache size should be non-zero"))?,
            )
            .build();
        Ok(config)
    }
}

/// A handle to the database, returned by `fwd_open_db`.
///
/// These handles are passed to the other FFI functions.
///
#[derive(Debug)]
#[repr(C)]
pub struct DatabaseHandle {
    /// A single cached view to improve performance of reads while committing
    cached_view: ArcCache<HashKey, dyn api::DynDbView>,

    /// The database
    db: Db,

    metrics_context: MetricsContext,
}

impl DatabaseHandle {
    /// Creates a new database handle from the given arguments.
    ///
    /// # Errors
    ///
    /// If the path is empty, or if the configuration is invalid, this will return an error.
    pub fn new(args: DatabaseHandleArgs<'_>) -> Result<Self, api::Error> {
        let metrics_context = MetricsContext::new(args.expensive_metrics);

        let cfg = DbConfig::builder()
            .node_hash_algorithm(args.node_hash_algorithm.into())
            .truncate(args.truncate)
            .manager(args.as_rev_manager_config()?)
            .root_store(args.root_store)
            .build();

        let path = args
            .dir
            .as_str()
            .map_err(|err| invalid_data(format!("database path contains invalid utf-8: {err}")))?;

        if path.is_empty() {
            return Err(invalid_data("database path cannot be empty"));
        }

        let db = Db::new(path, cfg)?;
        Ok(Self {
            cached_view: ArcCache::new(),
            db,
            metrics_context,
        })
    }

    /// Returns the current root hash of the database.
    ///
    /// # Errors
    ///
    /// An error is returned if there was an i/o error while reading the root hash.
    pub fn current_root_hash(&self) -> Result<Option<HashKey>, api::Error> {
        self.db.root_hash()
    }

    /// Returns a value from the database for the given key from the latest root hash.
    ///
    /// # Errors
    ///
    /// An error is returned if there was an i/o error while reading the value.
    pub fn get_latest(&self, key: impl KeyType) -> Result<Option<Box<[u8]>>, api::Error> {
        let Some(root) = self.current_root_hash()? else {
            return Err(api::Error::RevisionNotFound {
                provided: HashKey::default_root_hash(),
            });
        };

        self.db.revision(root)?.val(key)
    }

    /// Creates a proposal with the given values and returns the proposal and the start time.
    ///
    /// # Errors
    ///
    /// An error is returned if the proposal could not be created.
    pub fn create_batch<'kvp>(
        &self,
        values: impl AsRef<[KeyValuePair<'kvp>]> + 'kvp,
    ) -> Result<Option<HashKey>, api::Error> {
        let CreateProposalResult { handle, start_time } =
            self.create_proposal_handle(values.as_ref())?;

        let root_hash = handle.commit_proposal(|commit_time| {
            firewood_increment!(crate::registry::COMMIT_MS, commit_time.as_millis());
            firewood_record!(
                crate::registry::COMMIT_MS_BUCKET,
                commit_time.as_f64() * 1000.0,
                expensive
            );
        })?;

        let elapsed = start_time.elapsed();
        firewood_increment!(crate::registry::BATCH_MS, elapsed.as_millis());
        firewood_increment!(crate::registry::BATCH_COUNT, 1);
        firewood_record!(
            crate::registry::BATCH_MS_BUCKET,
            elapsed.as_f64() * 1000.0,
            expensive
        );

        Ok(root_hash)
    }

    /// Returns an owned handle to the revision corresponding to the provided root hash.
    ///
    /// # Errors
    ///
    /// Returns an error if could not get the view from underlying database for the specified
    /// root hash, for example when the revision does not exist or an I/O error occurs while
    /// accessing the database.
    pub fn get_revision(&self, root: HashKey) -> Result<GetRevisionResult, api::Error> {
        let view = self.db.view(root.clone())?;
        Ok(GetRevisionResult {
            handle: RevisionHandle::new(view, self.metrics_context),
            root_hash: root,
        })
    }

    #[must_use]
    pub(crate) const fn metrics_context(&self) -> MetricsContext {
        self.metrics_context
    }

    pub(crate) fn get_root(&self, root: HashKey) -> Result<ArcDynDbView, api::Error> {
        let mut cache_miss = false;
        let view = self.cached_view.get_or_try_insert_with(root, |key| {
            cache_miss = true;
            self.db.view(HashKey::clone(key))
        })?;

        if cache_miss {
            firewood_increment!(crate::registry::CACHED_VIEW_MISS, 1);
        } else {
            firewood_increment!(crate::registry::CACHED_VIEW_HIT, 1);
        }

        Ok(view)
    }

    pub(crate) fn clear_cached_view(&self) {
        self.cached_view.clear();
    }

    pub(crate) fn merge_key_value_range(
        &self,
        first_key: Option<impl KeyType>,
        last_key: Option<impl KeyType>,
        key_values: impl IntoIterator<Item: api::KeyValuePair>,
    ) -> Result<CreateProposalResult<'_>, api::Error> {
        CreateProposalResult::new(self, || {
            self.db
                .merge_key_value_range(first_key, last_key, key_values)
        })
    }

    /// Dumps the Trie structure of the latest revision to a DOT (Graphviz) format string.
    ///
    /// # Errors
    ///
    /// An error is returned if there was an i/o error while dumping the trie.
    pub fn dump_to_string(&self) -> Result<String, api::Error> {
        self.db.dump_to_string().map_err(api::Error::from)
    }
}

impl<'db> CView<'db> for &'db crate::DatabaseHandle {
    fn handle(&self) -> &'db crate::DatabaseHandle {
        self
    }

    fn create_proposal(
        self,
        values: impl IntoBatchIter,
    ) -> Result<firewood::db::Proposal<'db>, api::Error> {
        self.db.propose(values)
    }
}

impl crate::HasContext for DatabaseHandle {
    fn metrics(&self) -> Option<MetricsContext> {
        Some(self.metrics_context)
    }
}

fn invalid_data(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> api::Error {
    api::Error::IO(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}
