// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![expect(
    clippy::missing_errors_doc,
    reason = "FFI methods follow existing pattern without per-method error docs."
)]

use std::num::NonZeroUsize;

use firewood::{
    db::{MultiDb, MultiDbConfig},
    merkle::Merkle,
    v2::api::{
        self, ArcDynDbView, DbView, FrozenChangeProof, HashKey, IntoBatchIter, KeyType,
        OptionalHashKeyExt, Proposal as _, ValidatorId,
    },
};

use crate::{
    BatchOp, IteratorHandle,
    arc_cache::ArcCache,
    handle::DatabaseHandleArgs,
    iterator::CreateIteratorResult,
    metrics::MetricsContextExt,
    revision::{GetRevisionResult, RevisionHandle},
};
use firewood_metrics::{
    MetricsContext, firewood_increment, firewood_record, fwd_expensive_timed_result,
};

/// Arguments for creating or opening a multi-validator database.
#[repr(C)]
#[derive(Debug)]
pub struct MultiDatabaseHandleArgs<'a> {
    /// Base database arguments (reused from single-head).
    pub db_args: DatabaseHandleArgs<'a>,
    /// Maximum number of validators.
    pub max_validators: usize,
}

/// A handle to a multi-validator database, returned by [`fwd_multi_open_db`].
///
/// [`fwd_multi_open_db`]: crate::fwd_multi_open_db
#[derive(Debug)]
pub struct MultiDatabaseHandle {
    /// A single cached view to improve performance of repeated reads at the same root.
    cached_view: ArcCache<HashKey, dyn api::DynDbView>,
    multi_db: MultiDb,
    metrics_context: MetricsContext,
}

impl MultiDatabaseHandle {
    /// Creates a new multi-database handle from the given arguments.
    pub fn new(args: MultiDatabaseHandleArgs<'_>) -> Result<Self, api::Error> {
        let metrics_context = MetricsContext::new(args.db_args.expensive_metrics);

        let db_cfg = firewood::db::DbConfig::builder()
            .node_hash_algorithm(args.db_args.node_hash_algorithm.into())
            .truncate(args.db_args.truncate)
            .manager(args.db_args.as_rev_manager_config()?)
            .root_store(args.db_args.root_store)
            .build();

        let path =
            args.db_args.dir.as_str().map_err(|err| {
                invalid_data(format!("database path contains invalid utf-8: {err}"))
            })?;

        if path.is_empty() {
            return Err(invalid_data("database path cannot be empty"));
        }

        let cfg = MultiDbConfig::builder()
            .db(db_cfg)
            .max_validators(args.max_validators)
            .build();

        let multi_db = MultiDb::new(path, cfg)?;
        Ok(Self {
            cached_view: ArcCache::new(),
            multi_db,
            metrics_context,
        })
    }

    /// Register a validator.
    pub fn register_validator(&self, id: u64) -> Result<(), api::Error> {
        self.multi_db.register_validator(ValidatorId::new(id))
    }

    /// Deregister a validator.
    pub fn deregister_validator(&self, id: u64) -> Result<(), api::Error> {
        self.multi_db.deregister_validator(ValidatorId::new(id))
    }

    /// Get value from a validator's current head.
    pub fn get(&self, id: u64, key: impl KeyType) -> Result<Option<Box<[u8]>>, api::Error> {
        self.multi_db.get(ValidatorId::new(id), key.as_ref())
    }

    /// Get the root hash of a validator's current head.
    pub fn validator_root_hash(&self, id: u64) -> Result<Option<HashKey>, api::Error> {
        self.multi_db.validator_root_hash(ValidatorId::new(id))
    }

    /// Get a read-only view at a validator's current head.
    pub fn validator_view(&self, id: u64) -> Result<GetRevisionResult, api::Error> {
        let head = self.multi_db.validator_view(ValidatorId::new(id))?;
        let root_hash = head
            .root_hash()
            .or_default_root_hash()
            .unwrap_or_else(|| HashKey::from([0u8; 32]));
        let view = head as ArcDynDbView;
        Ok(GetRevisionResult {
            handle: RevisionHandle::new(view, self.metrics_context),
            root_hash,
        })
    }

    /// Get a read-only view at any committed revision by hash.
    pub fn view(&self, hash: HashKey) -> Result<GetRevisionResult, api::Error> {
        let view = self.multi_db.view(hash.clone())?;
        Ok(GetRevisionResult {
            handle: RevisionHandle::new(view, self.metrics_context),
            root_hash: hash,
        })
    }

    /// Create a proposal for a validator.
    pub fn create_proposal<'a>(
        &'a self,
        id: u64,
        values: impl AsRef<[BatchOp<'a>]> + 'a,
    ) -> Result<MultiCreateProposalResult<'a>, api::Error> {
        let (proposal_result, propose_time) =
            fwd_expensive_timed_result!(crate::registry::PROPOSE_MS_BUCKET, {
                self.multi_db.propose(ValidatorId::new(id), values.as_ref())
            });
        let proposal = proposal_result?;
        firewood_increment!(crate::registry::PROPOSE_MS, propose_time.as_millis());
        firewood_increment!(crate::registry::PROPOSE_COUNT, 1);
        firewood_record!(
            crate::registry::PROPOSE_MS_BUCKET,
            propose_time.as_f64() * 1000.0,
            expensive
        );

        let hash_key = proposal.root_hash();

        Ok(MultiCreateProposalResult {
            handle: MultiProposalHandle {
                hash_key,
                proposal,
                handle: self,
                validator_id: id,
            },
        })
    }

    /// Propose and commit in one call.
    pub fn update<'a>(
        &self,
        id: u64,
        values: impl AsRef<[BatchOp<'a>]> + 'a,
    ) -> Result<Option<HashKey>, api::Error> {
        let (result, elapsed) = fwd_expensive_timed_result!(crate::registry::BATCH_MS_BUCKET, {
            self.multi_db.update(ValidatorId::new(id), values.as_ref())
        });
        let root_hash = result?;
        firewood_increment!(crate::registry::BATCH_MS, elapsed.as_millis());
        firewood_increment!(crate::registry::BATCH_COUNT, 1);
        firewood_record!(
            crate::registry::BATCH_MS_BUCKET,
            elapsed.as_f64() * 1000.0,
            expensive
        );
        Ok(root_hash)
    }

    /// Advance a validator's head to an existing revision by hash.
    pub fn advance_to_hash(&self, id: u64, hash: HashKey) -> Result<(), api::Error> {
        self.multi_db.advance_to_hash(ValidatorId::new(id), hash)
    }

    /// Dump the trie at a validator's current head.
    pub fn dump(&self, id: u64) -> Result<String, api::Error> {
        self.multi_db.dump_validator(ValidatorId::new(id))
    }

    /// Get a cached view by root hash for proof generation.
    pub(crate) fn get_root(&self, root: HashKey) -> Result<ArcDynDbView, api::Error> {
        let mut cache_miss = false;
        let view = self.cached_view.get_or_try_insert_with(root, |key| {
            cache_miss = true;
            self.multi_db.view(HashKey::clone(key))
        })?;

        if cache_miss {
            firewood_increment!(crate::registry::CACHED_VIEW_MISS, 1);
        } else {
            firewood_increment!(crate::registry::CACHED_VIEW_HIT, 1);
        }

        Ok(view)
    }

    /// Create a change proof between two revisions.
    pub(crate) fn change_proof(
        &self,
        start_hash: HashKey,
        end_hash: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        limit: Option<NonZeroUsize>,
    ) -> Result<FrozenChangeProof, api::Error> {
        // Get the end revision first so that EndRevisionNotFound is returned
        // when both revisions are missing, matching single-head behavior.
        let end_merkle = Merkle::from(self.multi_db.revision(end_hash).map_err(|err| {
            if let api::Error::RevisionNotFound { provided } = err {
                api::Error::EndRevisionNotFound { provided }
            } else {
                err
            }
        })?);

        let start_merkle = Merkle::from(self.multi_db.revision(start_hash).map_err(|err| {
            if let api::Error::RevisionNotFound { provided } = err {
                api::Error::StartRevisionNotFound { provided }
            } else {
                err
            }
        })?);

        end_merkle.change_proof(start_key, end_key, start_merkle.nodestore(), limit)
    }

    /// Merge key-value range for a validator (used by range proof commit).
    pub(crate) fn merge_key_value_range(
        &self,
        id: u64,
        first_key: Option<&[u8]>,
        last_key: Option<&[u8]>,
        key_values: impl IntoIterator<Item: api::KeyValuePair>,
    ) -> Result<MultiCreateProposalResult<'_>, api::Error> {
        let (proposal_result, propose_time) =
            fwd_expensive_timed_result!(crate::registry::PROPOSE_MS_BUCKET, {
                self.multi_db.merge_key_value_range(
                    ValidatorId::new(id),
                    first_key,
                    last_key,
                    key_values,
                )
            });
        let proposal = proposal_result?;
        firewood_increment!(crate::registry::PROPOSE_MS, propose_time.as_millis());
        firewood_increment!(crate::registry::PROPOSE_COUNT, 1);
        firewood_record!(
            crate::registry::PROPOSE_MS_BUCKET,
            propose_time.as_f64() * 1000.0,
            expensive
        );

        let hash_key = proposal.root_hash();
        Ok(MultiCreateProposalResult {
            handle: MultiProposalHandle {
                hash_key,
                proposal,
                handle: self,
                validator_id: id,
            },
        })
    }

    /// Apply a change proof to a parent revision for a validator.
    pub(crate) fn apply_change_proof_to_parent(
        &self,
        id: u64,
        start_hash: HashKey,
        change_proof: &FrozenChangeProof,
    ) -> Result<MultiCreateProposalResult<'_>, api::Error> {
        let (proposal_result, propose_time) =
            fwd_expensive_timed_result!(crate::registry::PROPOSE_MS_BUCKET, {
                self.multi_db
                    .apply_change_proof_to_parent(change_proof, start_hash)
            });
        let proposal = proposal_result?;
        firewood_increment!(crate::registry::PROPOSE_MS, propose_time.as_millis());
        firewood_increment!(crate::registry::PROPOSE_COUNT, 1);
        firewood_record!(
            crate::registry::PROPOSE_MS_BUCKET,
            propose_time.as_f64() * 1000.0,
            expensive
        );

        let hash_key = proposal.root_hash();
        Ok(MultiCreateProposalResult {
            handle: MultiProposalHandle {
                hash_key,
                proposal,
                handle: self,
                validator_id: id,
            },
        })
    }

    /// Close the database gracefully.
    pub fn close(self) -> Result<(), api::Error> {
        self.multi_db.close()
    }
}

impl MetricsContextExt for MultiDatabaseHandle {
    fn metrics_context(&self) -> Option<MetricsContext> {
        Some(self.metrics_context)
    }
}

/// An opaque wrapper around a Proposal for multi-validator mode.
///
/// Stores the validator ID captured at propose time so that `commit_proposal`
/// can call `MultiDb::commit` without the caller needing to pass it again.
#[derive(Debug)]
pub struct MultiProposalHandle<'db> {
    hash_key: Option<HashKey>,
    proposal: firewood::db::Proposal<'db>,
    handle: &'db MultiDatabaseHandle,
    validator_id: u64,
}

impl<'db> DbView for MultiProposalHandle<'db> {
    type Iter<'view>
        = <firewood::db::Proposal<'db> as DbView>::Iter<'view>
    where
        Self: 'view;

    fn root_hash(&self) -> Option<HashKey> {
        self.proposal.root_hash()
    }

    fn val<K: KeyType>(&self, key: K) -> Result<Option<firewood::merkle::Value>, api::Error> {
        self.proposal.val(key)
    }

    fn single_key_proof<K: KeyType>(&self, key: K) -> Result<api::FrozenProof, api::Error> {
        self.proposal.single_key_proof(key)
    }

    fn range_proof<K: KeyType>(
        &self,
        first_key: Option<K>,
        last_key: Option<K>,
        limit: Option<std::num::NonZeroUsize>,
    ) -> Result<api::FrozenRangeProof, api::Error> {
        self.proposal.range_proof(first_key, last_key, limit)
    }

    fn iter_option<K: KeyType>(&self, first_key: Option<K>) -> Result<Self::Iter<'_>, api::Error> {
        self.proposal.iter_option(first_key)
    }

    fn dump_to_string(&self) -> Result<String, api::Error> {
        self.proposal.dump_to_string()
    }
}

impl MultiProposalHandle<'_> {
    /// Returns the root hash of the proposal.
    #[must_use]
    pub fn hash_key(&self) -> Option<crate::HashKey> {
        self.hash_key.clone().map(Into::into)
    }

    /// Consume and commit this proposal using the stored validator ID.
    pub fn commit_proposal(self) -> Result<Option<HashKey>, api::Error> {
        self.commit_proposal_with_source("consensus")
    }

    /// Consume and commit this proposal with a specified divergence source label.
    pub fn commit_proposal_with_source(self, source: &str) -> Result<Option<HashKey>, api::Error> {
        let MultiProposalHandle {
            hash_key,
            proposal,
            handle,
            validator_id,
        } = self;

        let (commit_result, commit_time) = fwd_expensive_timed_result!(
            crate::registry::COMMIT_MS_BUCKET,
            handle
                .multi_db
                .commit_with_source(ValidatorId::new(validator_id), proposal, source)
        );
        commit_result?;

        firewood_increment!(crate::registry::COMMIT_MS, commit_time.as_millis());
        firewood_increment!(crate::registry::COMMIT_COUNT, 1);

        Ok(hash_key)
    }

    /// Creates an iterator on the proposal starting from the given key.
    #[must_use]
    #[allow(clippy::missing_panics_doc)]
    pub fn iter_from(&self, first_key: Option<&[u8]>) -> CreateIteratorResult<'_> {
        let it = self
            .iter_option(first_key)
            .expect("infallible; see issue #1329");
        CreateIteratorResult(IteratorHandle::new(
            self.proposal.view(),
            Box::new(it) as api::BoxKeyValueIter<'_>,
            self.handle.metrics_context(),
        ))
    }
}

impl MetricsContextExt for MultiProposalHandle<'_> {
    fn metrics_context(&self) -> Option<MetricsContext> {
        self.handle.metrics_context()
    }
}

/// Result of creating a multi-validator proposal.
#[derive(Debug)]
pub struct MultiCreateProposalResult<'db> {
    pub handle: MultiProposalHandle<'db>,
}

/// Helper to create a child proposal on an existing `MultiProposalHandle`.
impl<'db> MultiProposalHandle<'db> {
    /// Create a child proposal that inherits the validator ID.
    pub fn create_proposal(
        &self,
        values: impl IntoBatchIter,
    ) -> Result<MultiCreateProposalResult<'db>, api::Error> {
        let (proposal_result, propose_time) =
            fwd_expensive_timed_result!(crate::registry::PROPOSE_MS_BUCKET, {
                self.proposal.propose(values)
            });
        let proposal = proposal_result?;
        firewood_increment!(crate::registry::PROPOSE_MS, propose_time.as_millis());
        firewood_increment!(crate::registry::PROPOSE_COUNT, 1);
        firewood_record!(
            crate::registry::PROPOSE_MS_BUCKET,
            propose_time.as_f64() * 1000.0,
            expensive
        );

        let hash_key = proposal.root_hash();

        Ok(MultiCreateProposalResult {
            handle: MultiProposalHandle {
                hash_key,
                proposal,
                handle: self.handle,
                validator_id: self.validator_id,
            },
        })
    }
}

use crate::invalid_data;
