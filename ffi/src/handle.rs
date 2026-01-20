// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::num::NonZeroUsize;

use firewood::{
    db::{Db, DbConfig},
    manager::RevisionManagerConfig,
    merkle::Merkle,
    v2::api::{
        self, ArcDynDbView, Db as _, DbView, Error, FrozenChangeProof, HashKey, HashKeyExt,
        IntoBatchIter, KeyType,
    },
};

use crate::{BorrowedBytes, CView, CreateProposalResult, KeyValuePair, arc_cache::ArcCache};

use crate::revision::{GetRevisionResult, RevisionHandle};
use firewood_storage::{FileBacked, MutableProposal, NodeStore, firewood_counter};

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
}

impl DatabaseHandle {
    /// Creates a new database handle from the given arguments.
    ///
    /// # Errors
    ///
    /// If the path is empty, or if the configuration is invalid, this will return an error.
    pub fn new(args: DatabaseHandleArgs<'_>) -> Result<Self, api::Error> {
        let cfg = DbConfig::builder()
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

    /// Returns a value from the database for the given key from the specified root hash.
    ///
    /// # Errors
    ///
    /// An error is returned if the root hash is invalid or if there was an i/o error
    /// while reading the value.
    pub fn get_from_root(
        &self,
        root: HashKey,
        key: impl KeyType,
    ) -> Result<Option<Box<[u8]>>, api::Error> {
        self.get_root(root)?.val(key.as_ref())
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
            firewood_counter!(
                "firewood.ffi.commit_ms",
                "FFI commit timing in milliseconds"
            )
            .increment(commit_time.as_millis());
        })?;

        firewood_counter!("firewood.ffi.batch_ms", "FFI batch timing in milliseconds")
            .increment(start_time.elapsed().as_millis());
        firewood_counter!("firewood.ffi.batch", "Number of FFI batch operations").increment(1);

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
            handle: RevisionHandle::new(view),
            root_hash: root,
        })
    }

    pub(crate) fn get_root(&self, root: HashKey) -> Result<ArcDynDbView, api::Error> {
        let mut cache_miss = false;
        let view = self.cached_view.get_or_try_insert_with(root, |key| {
            cache_miss = true;
            self.db.view(HashKey::clone(key))
        })?;

        if cache_miss {
            firewood_counter!(
                "firewood.ffi.cached_view.miss",
                "Number of FFI cached view misses"
            )
            .increment(1);
        } else {
            firewood_counter!(
                "firewood.ffi.cached_view.hit",
                "Number of FFI cached view hits"
            )
            .increment(1);
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

    pub(crate) fn mutable_to_propsal(
        &self,
        nodestore: NodeStore<MutableProposal, FileBacked>,
    ) -> Result<CreateProposalResult<'_>, api::Error> {
        CreateProposalResult::new(self, || self.db.mutable_to_proposal(nodestore))
    }

    /// Dumps the Trie structure of the latest revision to a DOT (Graphviz) format string.
    ///
    /// # Errors
    ///
    /// An error is returned if there was an i/o error while dumping the trie.
    pub fn dump_to_string(&self) -> Result<String, api::Error> {
        self.db.dump_to_string().map_err(api::Error::from)
    }

    /// Create a Change Proof between two revisions specified by the start and end hash.
    ///
    /// # Errors
    ///
    /// An error is returned if the revision cannot be found or if there is an I/O error.
    pub fn change_proof(
        &self,
        start_hash: HashKey,
        end_hash: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        limit: Option<NonZeroUsize>,
    ) -> Result<FrozenChangeProof, Error> {
        Merkle::from(self.db.revision(end_hash)?).change_proof(
            start_key,
            end_key,
            Merkle::from(self.db.revision(start_hash)?).nodestore(),
            limit,
        )
    }
    #[allow(clippy::missing_errors_doc)]
    pub fn apply_change_proof_to_parent(
        &self,
        start_hash: HashKey,
        key_values: &FrozenChangeProof,
    ) -> Result<NodeStore<MutableProposal, FileBacked>, Error> {
        // Verify that the current hash is the start_hash
        let start_hash = match self.db.root_hash()? {
            Some(root_hash) => {
                let start_hash = start_hash.into_triehash();
                if start_hash == root_hash {
                    Ok(start_hash)
                } else {
                    Err(api::Error::ParentNotLatest {
                        provided: Some(root_hash),
                        expected: Some(start_hash),
                    })
                }
            }
            None => Err(api::Error::LatestIsEmpty),
        }?;
        let parent = &self.db.revision(start_hash)?;
        self.db.apply_change_proof_to_parent(key_values, parent)
    }

    #[allow(clippy::missing_errors_doc)]
    pub fn apply_change_proof_to_proposal(
        &self,
        start_hash: HashKey,
        key_values: &FrozenChangeProof,
        proposal: NodeStore<MutableProposal, FileBacked>,
    ) -> Result<NodeStore<MutableProposal, FileBacked>, Error> {
        // TODO: Refactor start_hash verification into helper
        // Verify that the current hash is the start_hash
        match self.db.root_hash()? {
            Some(root_hash) => {
                let start_hash = start_hash.into_triehash();
                if start_hash == root_hash {
                    Ok(start_hash)
                } else {
                    Err(api::Error::ParentNotLatest {
                        provided: Some(root_hash),
                        expected: Some(start_hash),
                    })
                }
            }
            None => Err(api::Error::LatestIsEmpty),
        }?;
        self.db.apply_change_proof_to_proposal(key_values, proposal)
    }
}

impl From<Db> for DatabaseHandle {
    fn from(db: Db) -> Self {
        Self {
            db,
            cached_view: ArcCache::new(),
        }
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

fn invalid_data(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> api::Error {
    api::Error::IO(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}
