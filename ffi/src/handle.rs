// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::num::{NonZeroU64, NonZeroUsize};
use std::sync::Arc;

use firewood::{
    api::{self, ArcDynDbView, DynDb, FrozenChangeProof, HashKey, IntoBatchIter, KeyType},
    db::{CommittedView, DbConfig},
    manager::RevisionManagerConfig,
    open,
};
use firewood_metrics::MetricsContext;

use crate::revision::{GetRevisionResult, RevisionHandle};
use crate::{BatchOp, BorrowedBytes, CView, CreateProposalResult};

/// The hashing mode to use for the database.
///
/// This determines the cryptographic hash function and trie structure used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub enum NodeHashAlgorithm {
    /// MerkleDB Firewood hashing (SHA-256 based)
    MerkleDB = 0,
    /// Ethereum-compatible hashing (Keccak-256 based)
    Ethereum = 1,
}

impl From<NodeHashAlgorithm> for firewood::NodeHashAlgorithm {
    fn from(alg: NodeHashAlgorithm) -> Self {
        match alg {
            NodeHashAlgorithm::MerkleDB => firewood::NodeHashAlgorithm::MerkleDB,
            NodeHashAlgorithm::Ethereum => firewood::NodeHashAlgorithm::Ethereum,
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

    /// The optional memory limit for the node cache in bytes.
    ///
    /// Set to `0` to leave this unset and rely on the default configured in
    /// `RevisionManagerConfig`.
    pub node_cache_memory_limit: usize,

    /// The memory limit for the free-list cache in kibibytes.
    ///
    /// Nothing is preallocated; this is an upper bound on the memory the cache
    /// may grow to.
    ///
    /// Opening returns an error if this is zero.
    pub freelist_memory_limit_kb: usize,

    /// The maximum number of revisions to keep.
    ///
    /// Must be > `deferred_persistence_commit_count`.
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

    /// Tag used to separate metrics and logs per database.
    ///
    /// This must be a valid UTF-8 string.
    ///
    /// If empty, no tag is applied and this database's metrics are recorded
    /// with the default `db_tag="untagged"` label.
    pub db_tag: BorrowedBytes<'a>,

    /// The hashing mode to use for the database.
    ///
    /// This is the per-database node-hashing scheme, selected at runtime. For
    /// an existing database it must match the scheme persisted in the file
    /// header (a mismatch is an error); for a fresh database it is the scheme
    /// to create with. A single binary can open both
    /// [`NodeHashAlgorithm::Ethereum`] and [`NodeHashAlgorithm::MerkleDB`]
    /// databases regardless of the database's runtime hash mode.
    pub node_hash_algorithm: NodeHashAlgorithm,

    /// The maximum number of unpersisted revisions that can exist at a given time.
    ///
    /// Note: `revisions` must be > `deferred_persistence_commit_count`.
    pub deferred_persistence_commit_count: u64,
}

impl DatabaseHandleArgs<'_> {
    fn as_rev_manager_config(&self) -> Result<RevisionManagerConfig, api::Error> {
        let cache_read_strategy = match self.strategy {
            0 => firewood::manager::CacheReadStrategy::WritesOnly,
            1 => firewood::manager::CacheReadStrategy::BranchReads,
            2 => firewood::manager::CacheReadStrategy::All,
            _ => return Err(invalid_data("invalid cache strategy")),
        };
        let freelist_memory_limit_kb = NonZeroUsize::new(self.freelist_memory_limit_kb)
            .ok_or_else(|| invalid_data("freelist memory limit should be non-zero"))?;
        let commit_count = NonZeroU64::new(self.deferred_persistence_commit_count)
            .ok_or(api::Error::ZeroCommitCount)?;

        let memory_limit = NonZeroUsize::new(self.node_cache_memory_limit);

        let config = {
            let builder = RevisionManagerConfig::builder()
                .max_revisions(self.revisions)
                .cache_read_strategy(cache_read_strategy)
                .freelist_memory_limit_kb(freelist_memory_limit_kb)
                .deferred_persistence_commit_count(commit_count);

            if let Some(memory_limit) = memory_limit {
                builder.node_cache_memory_limit(memory_limit).build()
            } else {
                builder.build()
            }
        };

        Ok(config)
    }
}

/// A handle to the database, returned by `fwd_open_db`.
///
/// These handles are passed to the other FFI functions.
#[derive(Debug)]
pub struct DatabaseHandle {
    /// The database, erased to the runtime-selected hash mode.
    db: Box<dyn DynDb>,
    metrics_context: MetricsContext,
}

impl DatabaseHandle {
    /// Creates a new database handle from the given arguments.
    ///
    /// # Errors
    ///
    /// If the path is empty, or if the configuration is invalid, this will return an error.
    pub fn new(args: DatabaseHandleArgs<'_>) -> Result<Self, api::Error> {
        let db_tag = parse_db_tag(args.db_tag)?;
        let metrics_context = MetricsContext::new(args.expensive_metrics).with_db_tag(db_tag);

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

        // Select the concrete `Db<H>` at runtime from the configured algorithm,
        // validated against the on-disk header, and hold it erased.
        let db = open(path, cfg)?;
        Ok(Self {
            db,
            metrics_context,
        })
    }

    /// Returns the current root hash of the database.
    ///
    /// # Errors
    ///
    /// Never errors.
    #[must_use]
    pub fn current_root_hash(&self) -> Option<HashKey> {
        self.db.root_hash()
    }

    /// Returns a value from the database for the given key from the latest root hash.
    ///
    /// # Errors
    ///
    /// An error is returned if there was an i/o error while reading the value.
    pub fn get_latest(&self, key: impl KeyType) -> Result<Option<Box<[u8]>>, api::Error> {
        let Some(root) = self.current_root_hash() else {
            return Err(api::Error::RevisionNotFound { provided: None });
        };

        self.db.revision(root)?.val(key.as_ref())
    }

    /// Creates and commits a proposal with the given values.
    ///
    /// # Errors
    ///
    /// An error is returned if the proposal could not be created.
    pub fn create_batch<'a>(
        &self,
        values: impl AsRef<[BatchOp<'a>]> + 'a,
    ) -> Result<Option<HashKey>, api::Error> {
        let CreateProposalResult { handle } = self.create_proposal_handle(values.as_ref())?;
        handle.commit_proposal()
    }

    /// Returns an owned handle to the revision corresponding to the provided root hash.
    ///
    /// # Errors
    ///
    /// Returns an error if could not get the view from underlying database for the specified
    /// root hash, for example when the revision does not exist or an I/O error occurs while
    /// accessing the database.
    pub fn get_revision(&self, root: HashKey) -> Result<GetRevisionResult<'_>, api::Error> {
        let view = self.db.view(root.clone())?;
        // The opaque committed parent is `None` when `root` names a proposal
        // rather than a committed revision (see [`DynDb::committed_view`]).
        let historical = self.db.committed_view(root.clone())?;
        Ok(GetRevisionResult {
            handle: RevisionHandle::new(view, historical, self.metrics_context.clone(), self),
            root_hash: root,
        })
    }

    /// Reconstructs a view on top of an existing historical revision.
    ///
    /// # Errors
    ///
    /// Returns an error if reconstruction fails.
    pub fn reconstruct_from_view<'db>(
        &'db self,
        parent: &CommittedView,
        batch: impl IntoBatchIter,
    ) -> Result<firewood::db::ReconstructedView<'db>, api::Error> {
        let ops = crate::proposal::collect_owned_batch(batch)?;
        self.db.reconstruct_from_view(parent, ops)
    }

    pub(crate) fn view(&self, root: HashKey) -> Result<ArcDynDbView, api::Error> {
        self.db.view(root)
    }

    pub(crate) fn merge_key_value_range(
        &self,
        first_key: Option<impl KeyType>,
        last_key: Option<impl KeyType>,
        key_values: impl IntoIterator<Item: api::KeyValuePair>,
    ) -> Result<CreateProposalResult<'_>, api::Error> {
        let first_key = first_key.map(|k| k.as_ref().to_vec());
        let last_key = last_key.map(|k| k.as_ref().to_vec());
        let key_values: api::OwnedKeyValuePairs = key_values
            .into_iter()
            .map(|pair| {
                let (key, value) = api::KeyValuePair::try_into_tuple(pair)
                    .map_err(|e| api::Error::from(e.into()))?;
                Ok::<_, api::Error>((key.as_ref().into(), value.as_ref().into()))
            })
            .collect::<Result<_, api::Error>>()?;
        CreateProposalResult::new(self, || {
            self.db
                .merge_key_value_range(first_key.as_deref(), last_key.as_deref(), key_values)
        })
    }

    /// Create a Change Proof between two revisions specified by the start and end hash.
    ///
    /// Delegates to [`firewood::db::Db::change_proof`].
    pub(crate) fn change_proof(
        &self,
        start_hash: HashKey,
        end_hash: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        limit: Option<NonZeroUsize>,
    ) -> Result<FrozenChangeProof, api::Error> {
        self.db
            .change_proof(start_hash, end_hash, start_key, end_key, limit)
    }

    /// Verify a change proof and create a proposal from it.
    ///
    /// Performs structural validation, applies batch ops to the latest
    /// revision, and verifies the root hash against `end_root`. The proof
    /// is borrowed, not consumed.
    ///
    /// # Errors
    ///
    /// Returns an error if structural validation fails or the root hash
    /// doesn't match `end_root`.
    pub fn verify_change_proof(
        &self,
        proof: &FrozenChangeProof,
        end_root: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<CreateProposalResult<'_>, api::Error> {
        CreateProposalResult::new(self, || {
            self.db
                .verify_change_proof(proof, end_root, start_key, end_key, max_length)
        })
    }

    /// Dumps the Trie structure of the latest revision to a DOT (Graphviz) format string.
    ///
    /// # Errors
    ///
    /// An error is returned if there was an i/o error while dumping the trie.
    pub fn dump_to_string(&self) -> Result<String, api::Error> {
        self.db.dump_to_string()
    }

    /// Closes the database gracefully.
    ///
    /// # Errors
    ///
    /// An error is returned if the persistence background thread panicked or
    /// errored during execution.
    pub fn close(self) -> Result<(), api::Error> {
        self.db.close()
    }
}

impl<'db> CView<'db> for &'db crate::DatabaseHandle {
    fn handle(&self) -> &'db crate::DatabaseHandle {
        self
    }

    fn create_proposal(
        self,
        values: impl IntoBatchIter,
    ) -> Result<Box<dyn api::DynProposal<'db> + 'db>, api::Error> {
        let ops = crate::proposal::collect_owned_batch(values)?;
        self.db.propose(ops)
    }
}

impl crate::MetricsContextExt for DatabaseHandle {
    fn metrics_context(&self) -> Option<MetricsContext> {
        Some(self.metrics_context.clone())
    }
}

fn invalid_data(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> api::Error {
    api::Error::IO(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

fn parse_db_tag(db_tag: BorrowedBytes<'_>) -> Result<Option<metrics::SharedString>, api::Error> {
    let db_tag = db_tag
        .as_str()
        .map_err(|err| invalid_data(format!("database tag contains invalid utf-8: {err}")))?;

    // Arc<str> keeps the per-recording clone of the tag a refcount bump.
    Ok((!db_tag.is_empty()).then(|| metrics::SharedString::from(Arc::<str>::from(db_tag))))
}
