// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![expect(
    clippy::missing_errors_doc,
    reason = "Found 12 occurrences after enabling the lint."
)]

#[cfg(test)]
mod tests;

use crate::iter::MerkleKeyValueIter;
use crate::merkle::{Merkle, Value};
pub use crate::v2::api::BatchOp;
use crate::v2::api::{
    self, ArcDynDbView, DbView, FrozenChangeProof, FrozenProof, FrozenRangeProof, HashKey,
    IntoBatchIter, KeyType, KeyValuePair, OptionalHashKeyExt, ValidatorId,
};

use crate::manager::{ConfigManager, RevisionManager, RevisionManagerConfig};
use firewood_metrics::firewood_counter;
use firewood_storage::{
    CheckOpt, CheckerReport, Committed, FileBacked, FileIoError, HashedNodeReader,
    ImmutableProposal, NodeHashAlgorithm, NodeStore, Parentable, ReadableStorage, TrieReader,
};
use std::io::Write;
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;
use thiserror::Error;
use typed_builder::TypedBuilder;

use crate::merkle::parallel::ParallelMerkle;

#[derive(Error, Debug)]
#[non_exhaustive]
/// Represents the different types of errors that can occur in the database.
pub enum DbError {
    /// I/O error
    #[error("I/O error: {0:?}")]
    FileIo(#[from] FileIoError),
}

/// Metrics for the database.
/// TODO: Add more metrics
pub struct DbMetrics {
    proposals: metrics::Counter,
}

impl std::fmt::Debug for DbMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbMetrics").finish()
    }
}

impl<P: Parentable, S: ReadableStorage> api::DbView for NodeStore<P, S>
where
    NodeStore<P, S>: TrieReader,
{
    type Iter<'view>
        = MerkleKeyValueIter<'view, Self>
    where
        Self: 'view;

    fn root_hash(&self) -> Option<HashKey> {
        HashedNodeReader::root_hash(self).or_default_root_hash()
    }

    fn val<K: api::KeyType>(&self, key: K) -> Result<Option<Value>, api::Error> {
        let merkle = Merkle::from(self);
        Ok(merkle.get_value(key.as_ref())?)
    }

    fn single_key_proof<K: api::KeyType>(&self, key: K) -> Result<FrozenProof, api::Error> {
        let merkle = Merkle::from(self);
        merkle.prove(key.as_ref()).map_err(api::Error::from)
    }

    fn range_proof<K: api::KeyType>(
        &self,
        first_key: Option<K>,
        last_key: Option<K>,
        limit: Option<NonZeroUsize>,
    ) -> Result<FrozenRangeProof, api::Error> {
        Merkle::from(self).range_proof(
            first_key.as_ref().map(AsRef::as_ref),
            last_key.as_ref().map(AsRef::as_ref),
            limit,
        )
    }

    fn iter_option<K: KeyType>(&self, first_key: Option<K>) -> Result<Self::Iter<'_>, api::Error> {
        match first_key {
            Some(key) => Ok(MerkleKeyValueIter::from_key(self, key)),
            None => Ok(MerkleKeyValueIter::from(self)),
        }
    }

    fn dump_to_string(&self) -> Result<String, api::Error> {
        let merkle = Merkle::from(self);
        merkle.dump_to_string().map_err(api::Error::from)
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum UseParallel {
    Never,
    BatchSize(usize),
    Always,
}

/// Database configuration.
#[derive(Clone, TypedBuilder, Debug)]
#[non_exhaustive]
pub struct DbConfig {
    /// The algorithm used for hashing nodes (required).
    #[cfg_attr(test, builder(default = NodeHashAlgorithm::compile_option()))]
    pub node_hash_algorithm: NodeHashAlgorithm,
    /// Whether to create the DB if it doesn't exist.
    #[builder(default = true)]
    pub create_if_missing: bool,
    /// Whether to truncate the DB when opening it. If set, the DB will be reset and all its
    /// existing contents will be lost.
    #[builder(default = false)]
    pub truncate: bool,
    /// Revision manager configuration.
    #[builder(default = RevisionManagerConfig::builder().build())]
    pub manager: RevisionManagerConfig,
    // Whether to perform parallel proposal creation. If set to BatchSize, then firewood
    // performs parallel proposal creation if the batch is >= to the BatchSize value.
    // TODO: Experimentally determine the right value for BatchSize.
    #[builder(default = UseParallel::BatchSize(8))]
    pub use_parallel: UseParallel,
    /// Whether to enable `RootStore`.
    #[builder(default = false)]
    pub root_store: bool,
}

/// A database instance.
///
/// Callers **must** call `close()` when they are done with the database
/// to ensure the latest committed revision is persisted to disk. Dropping a
/// database without calling `close()` may result in committed data being lost.
#[derive(Debug)]
pub struct Db {
    metrics: Arc<DbMetrics>,
    manager: RevisionManager,
    use_parallel: UseParallel,
}

impl api::Db for Db {
    type Historical = NodeStore<Committed, FileBacked>;

    type Proposal<'db>
        = Proposal<'db>
    where
        Self: 'db;

    fn revision(&self, root_hash: HashKey) -> Result<Arc<Self::Historical>, api::Error> {
        let nodestore = self.manager.revision(root_hash)?;
        Ok(nodestore)
    }

    fn root_hash(&self) -> Option<HashKey> {
        self.manager.root_hash().or_default_root_hash()
    }

    fn propose(&self, batch: impl IntoBatchIter) -> Result<Self::Proposal<'_>, api::Error> {
        // Proposal created from db
        firewood_metrics::firewood_increment!(crate::registry::PROPOSALS_CREATED, 1, "base" => "db");

        self.propose_with_parent(batch, &self.manager.current_revision(), 0)
    }
}

impl Db {
    /// Create a new database instance.
    pub fn new<P: AsRef<Path>>(db_dir: P, cfg: DbConfig) -> Result<Self, api::Error> {
        let metrics = Arc::new(DbMetrics {
            proposals: firewood_counter!(crate::registry::PROPOSALS),
        });
        let config_manager = ConfigManager::builder()
            .root_dir(db_dir.as_ref().to_path_buf())
            .node_hash_algorithm(cfg.node_hash_algorithm)
            .create(cfg.create_if_missing)
            .truncate(cfg.truncate)
            .root_store(cfg.root_store)
            .manager(cfg.manager)
            .build();
        let manager = RevisionManager::new(config_manager)?;
        let db = Self {
            metrics,
            manager,
            use_parallel: cfg.use_parallel,
        };
        Ok(db)
    }

    /// Synchronously get a view, either committed or proposed
    pub fn view(&self, root_hash: HashKey) -> Result<ArcDynDbView, api::Error> {
        self.manager.view(root_hash).map_err(Into::into)
    }

    /// Dump the Trie of the latest revision.
    pub fn dump(&self, w: &mut dyn Write) -> Result<(), std::io::Error> {
        let latest_rev_nodestore = self.manager.current_revision();
        let merkle = Merkle::from(latest_rev_nodestore);
        merkle.dump(w).map_err(std::io::Error::other)
    }

    /// Dump the Trie of the latest revision to a String
    ///
    /// # Errors
    ///
    /// Returns an error if the dump operation fails
    pub fn dump_to_string(&self) -> Result<String, std::io::Error> {
        let latest_rev_nodestore = self.manager.current_revision();
        let merkle = Merkle::from(latest_rev_nodestore);
        merkle.dump_to_string().map_err(std::io::Error::other)
    }

    /// Get a copy of the database metrics
    pub fn metrics(&self) -> Arc<DbMetrics> {
        self.metrics.clone()
    }

    /// Check the database for consistency
    pub fn check(&self, opt: CheckOpt) -> CheckerReport {
        let latest_rev_nodestore = self.manager.current_revision();
        let header = self.manager.locked_header();
        latest_rev_nodestore.check(&header, opt)
    }

    /// Create a proposal with a specified parent. A proposal is created in parallel if `use_parallel`
    /// is `Always` or if `use_parallel` is `BatchSize` and the batch is >= to the `BatchSize` value.
    ///
    /// The `fork_id` tags all nodes allocated by this proposal for safe cross-chain
    /// reaping in multi-head mode. Pass 0 for single-head usage.
    ///
    /// # Panics
    ///
    /// Panics if the revision manager cannot create a thread pool.
    #[fastrace::trace(name = "propose")]
    pub(crate) fn propose_with_parent<F: Parentable>(
        &self,
        batch: impl IntoBatchIter,
        parent: &NodeStore<F, FileBacked>,
        fork_id: firewood_storage::ForkId,
    ) -> Result<Proposal<'_>, api::Error> {
        // Return immediately if the background thread is no longer running.
        self.manager.check_persist_error()?;
        // If use_parallel is BatchSize, then perform parallel proposal creation if the batch
        // size is >= BatchSize.
        let batch = batch.into_iter();
        let use_parallel = match self.use_parallel {
            UseParallel::Never => false,
            UseParallel::Always => true,
            UseParallel::BatchSize(required_size) => batch.size_hint().0 >= required_size,
        };
        let immutable = if use_parallel {
            let mut parallel_merkle = ParallelMerkle::default();
            let _span = fastrace::Span::enter_with_local_parent("parallel_merkle");
            parallel_merkle.create_proposal(parent, batch, self.manager.threadpool(), fork_id)?
        } else {
            let mut proposal = NodeStore::new(parent)?;
            proposal.set_fork_id(fork_id);
            let mut merkle = Merkle::from(proposal);
            let span = fastrace::Span::enter_with_local_parent("merkleops");
            for res in batch.into_batch_iter::<api::Error>() {
                match res? {
                    BatchOp::Put { key, value } => {
                        merkle.insert(key.as_ref(), value.as_ref().into())?;
                    }
                    BatchOp::Delete { key } => {
                        merkle.remove(key.as_ref())?;
                    }
                    BatchOp::DeleteRange { prefix } => {
                        merkle.remove_prefix(prefix.as_ref())?;
                    }
                }
            }

            drop(span);
            let _span = fastrace::Span::enter_with_local_parent("freeze");
            let nodestore = merkle.into_inner();
            Arc::new(nodestore.try_into()?)
        };
        self.manager.add_proposal(immutable.clone());

        self.metrics.proposals.increment(1);

        Ok(Proposal {
            nodestore: immutable,
            db: self,
        })
    }

    /// Merge a range of key-values into a new proposal on top of the current
    /// root revision.
    ///
    /// All items within the range `(first_key..=last_key)` will be replaced with
    /// the provided key-values from the iterator. I.e., any existing keys within
    /// the range that are not present in the provided key-values will be deleted,
    /// any duplicate keys will be overwritten, and any new keys will be inserted.
    ///
    /// Invariant: `key_values` must be sorted by key in ascending order; however,
    /// because debug assertions are disabled, this is not checked.
    pub fn merge_key_value_range(
        &self,
        first_key: Option<impl KeyType>,
        last_key: Option<impl KeyType>,
        key_values: impl IntoIterator<Item: KeyValuePair>,
    ) -> Result<Proposal<'_>, api::Error> {
        self.merge_key_value_range_with_parent(
            first_key,
            last_key,
            key_values,
            &self.manager.current_revision(),
        )
    }

    /// Merge a range of key-values into a new proposal on top of a specified parent.
    ///
    /// All items within the range `(first_key..=last_key)` will be replaced with
    /// the provided key-values from the iterator. I.e., any existing keys within
    /// the range that are not present in the provided key-values will be deleted,
    /// any duplicate keys will be overwritten, and any new keys will be inserted.
    ///
    /// Invariant: `key_values` must be sorted by key in ascending order; however,
    /// because debug assertions are disabled, this is not checked.
    pub fn merge_key_value_range_with_parent<F: Parentable>(
        &self,
        first_key: Option<impl KeyType>,
        last_key: Option<impl KeyType>,
        key_values: impl IntoIterator<Item: KeyValuePair>,
        parent: &NodeStore<F, FileBacked>,
    ) -> Result<Proposal<'_>, api::Error>
    where
        NodeStore<F, FileBacked>: TrieReader,
    {
        let merkle = Merkle::from(parent);
        let merge_ops = merkle.merge_key_value_range(first_key, last_key, key_values);
        self.propose_with_parent(merge_ops, merkle.nodestore(), 0)
    }

    pub fn apply_change_proof_to_parent<F: Parentable>(
        &self,
        batch_ops: impl IntoBatchIter,
        parent: &NodeStore<F, FileBacked>,
    ) -> Result<Proposal<'_>, api::Error>
    where
        NodeStore<F, FileBacked>: HashedNodeReader,
    {
        // Create a new proposal from the parent
        let merkle = Merkle::from(parent);
        self.propose_with_parent(batch_ops, merkle.nodestore(), 0)
    }

    /// Closes the database gracefully.
    ///
    /// Shuts down the background persistence worker and persists the latest
    /// committed revision to disk. This method **must** be called before the
    /// database is dropped as otherwise, any committed data may be lost.
    pub fn close(self) -> Result<(), api::Error> {
        self.manager.close().map_err(Into::into)
    }
}

#[derive(Debug)]
/// A user-visible database proposal
pub struct Proposal<'db> {
    pub(crate) nodestore: Arc<NodeStore<Arc<ImmutableProposal>, FileBacked>>,
    db: &'db Db,
}

impl api::DbView for Proposal<'_> {
    type Iter<'view>
        = MerkleKeyValueIter<'view, NodeStore<Arc<ImmutableProposal>, FileBacked>>
    where
        Self: 'view;

    fn root_hash(&self) -> Option<api::HashKey> {
        api::DbView::root_hash(&*self.nodestore)
    }

    fn val<K: KeyType>(&self, key: K) -> Result<Option<Value>, api::Error> {
        api::DbView::val(&*self.nodestore, key)
    }

    fn single_key_proof<K: KeyType>(&self, key: K) -> Result<FrozenProof, api::Error> {
        api::DbView::single_key_proof(&*self.nodestore, key)
    }

    fn range_proof<K: KeyType>(
        &self,
        first_key: Option<K>,
        last_key: Option<K>,
        limit: Option<NonZeroUsize>,
    ) -> Result<FrozenRangeProof, api::Error> {
        api::DbView::range_proof(&*self.nodestore, first_key, last_key, limit)
    }

    fn iter_option<K: KeyType>(&self, first_key: Option<K>) -> Result<Self::Iter<'_>, api::Error> {
        api::DbView::iter_option(&*self.nodestore, first_key)
    }

    fn dump_to_string(&self) -> Result<String, api::Error> {
        api::DbView::dump_to_string(&*self.nodestore)
    }
}

impl<'db> api::Proposal for Proposal<'db> {
    type Proposal = Proposal<'db>;

    fn propose(&self, batch: impl IntoBatchIter) -> Result<Self::Proposal, api::Error> {
        self.create_proposal(batch)
    }

    fn commit(self) -> Result<(), api::Error> {
        Ok(self.db.manager.commit(self.nodestore)?)
    }
}

impl Proposal<'_> {
    #[crate::metrics("proposal.create", "database proposal creation")]
    fn create_proposal(&self, batch: impl IntoBatchIter) -> Result<Self, api::Error> {
        // Proposal created based on another proposal
        firewood_metrics::firewood_increment!(crate::registry::PROPOSALS_CREATED, 1, "base" => "proposal");

        self.db.propose_with_parent(batch, &self.nodestore, 0)
    }

    /// Returns the view backing this proposal.
    #[must_use]
    pub fn view(&self) -> ArcDynDbView {
        self.nodestore.clone()
    }
}

/// Configuration for multi-validator mode.
#[derive(Clone, Debug, TypedBuilder)]
pub struct MultiDbConfig {
    /// Base database configuration.
    pub db: DbConfig,
    /// Maximum number of validators (1..=`MAX_VALIDATORS`).
    #[builder(default = firewood_storage::MAX_VALIDATORS)]
    pub max_validators: usize,
}

use crate::manager::CommittedRevision;

/// A multi-validator Firewood database.
///
/// Wraps a single `Db` instance and provides per-validator
/// propose/commit/view operations with automatic deduplication.
/// Identical revisions (same root hash) are stored once and shared
/// by all validators that produce them.
#[derive(Debug)]
pub struct MultiDb {
    db: Db,
}

impl MultiDb {
    /// Create a new multi-validator database.
    pub fn new<P: AsRef<Path>>(db_dir: P, cfg: MultiDbConfig) -> Result<Self, api::Error> {
        let max_validators = cfg.max_validators;
        let mut db = Db::new(db_dir, cfg.db)?;
        db.manager
            .enable_multi_head(max_validators)
            .map_err(api::Error::from)?;
        Ok(Self { db })
    }

    /// Register a new validator.
    ///
    /// The validator starts at the latest persisted revision.
    pub fn register_validator(&self, id: ValidatorId) -> Result<(), api::Error> {
        self.db.manager.register_validator(id).map_err(Into::into)
    }

    /// Deregister a validator, freeing its header slot.
    ///
    /// Any revisions unique to this validator become candidates for cleanup.
    pub fn deregister_validator(&self, id: ValidatorId) -> Result<(), api::Error> {
        self.db.manager.deregister_validator(id).map_err(Into::into)
    }

    /// Create a proposal for a validator from its current head.
    ///
    /// Nodes allocated by this proposal are tagged with the validator's
    /// fork ID for safe cross-chain reaping in multi-head mode.
    pub fn propose(
        &self,
        id: ValidatorId,
        batch: impl IntoBatchIter,
    ) -> Result<Proposal<'_>, api::Error> {
        let (head, fork_id) = self.db.manager.validator_view_and_fork_id(id)?;
        self.db.propose_with_parent(batch, &head, fork_id)
    }

    /// Commit a proposal for a validator.
    ///
    /// If a revision with the same root hash already exists (committed by
    /// another validator), the proposal is discarded and the validator's
    /// head advances to the existing revision (deduplication).
    pub fn commit(&self, id: ValidatorId, proposal: Proposal<'_>) -> Result<(), api::Error> {
        self.commit_with_source(id, proposal, "consensus")
    }

    /// Commit a proposal for a validator with a specified source label.
    ///
    /// The `source` label is attached to any divergence metrics emitted during
    /// this commit (e.g., `"consensus"` or `"proof"`).
    pub fn commit_with_source(
        &self,
        id: ValidatorId,
        proposal: Proposal<'_>,
        source: &str,
    ) -> Result<(), api::Error> {
        self.db
            .manager
            .commit_for_validator(id, proposal.nodestore, source)
            .map_err(Into::into)
    }

    /// Advance a validator's head to an existing revision by hash.
    ///
    /// This is the skip-propose optimization: if a validator knows the
    /// expected root hash (e.g., from a block header), it can advance
    /// without computing a proposal.
    pub fn advance_to_hash(&self, id: ValidatorId, hash: HashKey) -> Result<(), api::Error> {
        self.db
            .manager
            .advance_validator_to_hash(id, hash)
            .map_err(Into::into)
    }

    /// Get a read-only view at a validator's current head.
    pub fn validator_view(&self, id: ValidatorId) -> Result<CommittedRevision, api::Error> {
        self.db.manager.validator_view(id).map_err(Into::into)
    }

    /// Get a read-only view at any committed revision by hash.
    pub fn view(&self, hash: HashKey) -> Result<ArcDynDbView, api::Error> {
        self.db.view(hash)
    }

    /// Returns the root hash of a validator's current head.
    pub fn validator_root_hash(&self, id: ValidatorId) -> Result<Option<HashKey>, api::Error> {
        let head = self.db.manager.validator_view(id)?;
        Ok(head.root_hash().or_default_root_hash())
    }

    /// Read a value from a validator's current head.
    pub fn get(&self, id: ValidatorId, key: &[u8]) -> Result<Option<Value>, api::Error> {
        let head = self.db.manager.validator_view(id)?;
        head.val(key)
    }

    /// Propose and commit in one call (convenience for single-batch workflows).
    pub fn update(
        &self,
        id: ValidatorId,
        batch: impl IntoBatchIter,
    ) -> Result<Option<HashKey>, api::Error> {
        let proposal = self.propose(id, batch)?;
        let hash = proposal.root_hash();
        self.commit(id, proposal)?;
        Ok(hash)
    }

    /// Dump the trie at a validator's current head.
    pub fn dump_validator(&self, id: ValidatorId) -> Result<String, api::Error> {
        let head = self.db.manager.validator_view(id)?;
        let merkle = Merkle::from(head);
        merkle.dump_to_string().map_err(api::Error::from)
    }

    /// Get a committed revision by hash, returning the concrete `Arc` type.
    ///
    /// Unlike [`MultiDb::view`], this returns the concrete `CommittedRevision`
    /// instead of a type-erased `ArcDynDbView`, which is needed for operations
    /// that require access to the underlying `NodeStore` (e.g., proof generation).
    pub fn revision(&self, root_hash: HashKey) -> Result<CommittedRevision, api::Error> {
        self.db.manager.revision(root_hash).map_err(Into::into)
    }

    /// Merge a range of key-values into a new proposal on top of a validator's
    /// current head.
    ///
    /// All items within the range `(first_key..=last_key)` will be replaced with
    /// the provided key-values. Existing keys within the range that are not in
    /// the provided set will be deleted.
    pub fn merge_key_value_range(
        &self,
        id: ValidatorId,
        first_key: Option<impl KeyType>,
        last_key: Option<impl KeyType>,
        key_values: impl IntoIterator<Item: KeyValuePair>,
    ) -> Result<Proposal<'_>, api::Error> {
        let head = self.db.manager.validator_view(id)?;
        self.db
            .merge_key_value_range_with_parent(first_key, last_key, key_values, &head)
    }

    /// Apply the `BatchOp`s of a change proof to a parent revision for a
    /// validator.
    pub fn apply_change_proof_to_parent(
        &self,
        change_proof: &FrozenChangeProof,
        start_hash: HashKey,
    ) -> Result<Proposal<'_>, api::Error> {
        let parent = &self.db.manager.revision(start_hash)?;
        self.db.apply_change_proof_to_parent(change_proof, parent)
    }

    /// Close the database gracefully.
    pub fn close(self) -> Result<(), api::Error> {
        self.db.close()
    }
}

#[cfg(test)]
mod test {
    #![expect(clippy::unwrap_used)]

    use core::iter::Take;
    use std::collections::HashMap;
    use std::iter::Peekable;
    use std::num::{NonZeroU64, NonZeroUsize};
    use std::ops::{Deref, DerefMut};
    use std::path::Path;

    use firewood_storage::{
        CheckOpt, CheckerError, HashedNodeReader, IntoHashType, LinearAddress, MaybePersistedNode,
        NodeStore, TrieHash,
    };
    use nonzero_ext::nonzero;

    use crate::db::{Db, Proposal, UseParallel};
    use crate::manager::RevisionManagerConfig;
    use crate::v2::api::{Db as _, DbView, HashKeyExt, Proposal as _};

    use super::{BatchOp, DbConfig};

    /// A chunk of an iterator, provided by [`IterExt::chunk_fold`] to the folding
    /// function.
    type Chunk<'chunk, 'base, T> = &'chunk mut Take<&'base mut Peekable<T>>;

    trait IterExt: Iterator {
        /// Asynchronously folds the iterator with chunks of a specified size. The last
        /// chunk may be smaller than the specified size.
        ///
        /// The folding function is a closure that takes an accumulator and a
        /// chunk of the underlying iterator, and returns a new accumulator.
        ///
        /// # Panics
        ///
        /// If the folding function does not consume the entire chunk, it will panic.
        ///
        /// If the folding function panics, the iterator will be dropped (because this
        /// method consumes `self`).
        fn chunk_fold<B, F>(self, chunk_size: NonZeroUsize, init: B, mut f: F) -> B
        where
            Self: Sized,
            F: for<'a, 'b> FnMut(B, Chunk<'a, 'b, Self>) -> B,
        {
            let chunk_size = chunk_size.get();
            let mut iter = self.peekable();
            let mut acc = init;
            while iter.peek().is_some() {
                let mut chunk = iter.by_ref().take(chunk_size);
                acc = f(acc, chunk.by_ref());
                assert!(chunk.next().is_none(), "entire chunk was not consumed");
            }
            acc
        }
    }

    impl<T: Iterator> IterExt for T {}

    impl Db {
        /// Wait until all pending commits have been persisted.
        fn wait_persisted(&self) {
            self.manager.wait_persisted();
        }
    }

    impl super::MultiDb {
        /// Wait until all pending commits have been persisted.
        fn wait_persisted(&self) {
            self.db.wait_persisted();
        }
    }

    #[test]
    fn test_proposal_reads() {
        let db = TestDb::new();
        let batch = vec![BatchOp::Put {
            key: b"k",
            value: b"v",
        }];
        let proposal = db.propose(batch).unwrap();
        assert_eq!(&*proposal.val(b"k").unwrap().unwrap(), b"v");

        assert_eq!(proposal.val(b"notfound").unwrap(), None);
        proposal.commit().unwrap();

        let batch = vec![BatchOp::Put {
            key: b"k",
            value: b"v2",
        }];
        let proposal = db.propose(batch).unwrap();
        assert_eq!(&*proposal.val(b"k").unwrap().unwrap(), b"v2");

        let committed = db.root_hash().unwrap();
        let historical = db.revision(committed).unwrap();
        assert_eq!(&*historical.val(b"k").unwrap().unwrap(), b"v");
    }

    #[test]
    fn reopen_test() {
        let db = TestDb::new();
        let initial_root = db.root_hash();
        let batch = vec![
            BatchOp::Put {
                key: b"a",
                value: b"1",
            },
            BatchOp::Put {
                key: b"b",
                value: b"2",
            },
        ];
        let proposal = db.propose(batch).unwrap();
        proposal.commit().unwrap();
        println!("{:?}", db.root_hash().unwrap());

        let db = db.reopen();
        println!("{:?}", db.root_hash().unwrap());
        let committed = db.root_hash().unwrap();
        let historical = db.revision(committed).unwrap();
        assert_eq!(&*historical.val(b"a").unwrap().unwrap(), b"1");
        drop(historical);

        let db = db.replace();
        let final_root = db.root_hash();
        println!("{final_root:?}");
        assert!(final_root == initial_root);
    }

    #[test]
    // test that dropping a proposal removes it from the list of known proposals
    //    /-> P1 - will get committed
    // R1 --> P2 - will get dropped
    //    \-> P3 - will get orphaned, but it's still known
    fn test_proposal_scope_historic() {
        let db = TestDb::new();
        let batch1 = vec![BatchOp::Put {
            key: b"k1",
            value: b"v1",
        }];
        let proposal1 = db.propose(batch1).unwrap();
        assert_eq!(&*proposal1.val(b"k1").unwrap().unwrap(), b"v1");

        let batch2 = vec![BatchOp::Put {
            key: b"k2",
            value: b"v2",
        }];
        let proposal2 = db.propose(batch2).unwrap();
        assert_eq!(&*proposal2.val(b"k2").unwrap().unwrap(), b"v2");

        let batch3 = vec![BatchOp::Put {
            key: b"k3",
            value: b"v3",
        }];
        let proposal3 = db.propose(batch3).unwrap();
        assert_eq!(&*proposal3.val(b"k3").unwrap().unwrap(), b"v3");

        // the proposal is dropped here, but the underlying
        // nodestore is still accessible because it's referenced by the revision manager
        // The third proposal remains referenced
        let p2hash = proposal2.root_hash().unwrap();
        assert!(db.manager.proposal_hashes().contains(&p2hash));
        drop(proposal2);

        // commit the first proposal
        proposal1.commit().unwrap();
        // Ensure we committed the first proposal's data
        let committed = db.root_hash().unwrap();
        let historical = db.revision(committed).unwrap();
        assert_eq!(&*historical.val(b"k1").unwrap().unwrap(), b"v1");

        // the second proposal shouldn't be available to commit anymore
        assert!(!db.manager.proposal_hashes().contains(&p2hash));

        // the third proposal should still be contained within the all_hashes list
        // would be deleted if another proposal was committed and proposal3 was dropped here
        let hash3 = proposal3.root_hash().unwrap();
        assert!(db.manager.proposal_hashes().contains(&hash3));
    }

    #[test]
    // test that dropping a proposal removes it from the list of known proposals
    // R1 - base revision
    //  \-> P1 - will get committed
    //   \-> P2 - will get dropped
    //    \-> P3 - will get orphaned, but it's still known
    fn test_proposal_scope_orphan() {
        let db = TestDb::new();
        let batch1 = vec![BatchOp::Put {
            key: b"k1",
            value: b"v1",
        }];
        let proposal1 = db.propose(batch1).unwrap();
        assert_eq!(&*proposal1.val(b"k1").unwrap().unwrap(), b"v1");

        let batch2 = vec![BatchOp::Put {
            key: b"k2",
            value: b"v2",
        }];
        let proposal2 = proposal1.propose(batch2).unwrap();
        assert_eq!(&*proposal2.val(b"k2").unwrap().unwrap(), b"v2");

        let batch3 = vec![BatchOp::Put {
            key: b"k3",
            value: b"v3",
        }];
        let proposal3 = proposal2.propose(batch3).unwrap();
        assert_eq!(&*proposal3.val(b"k3").unwrap().unwrap(), b"v3");

        // the proposal is dropped here, but the underlying
        // nodestore is still accessible because it's referenced by the revision manager
        // The third proposal remains referenced
        let p2hash = proposal2.root_hash().unwrap();
        assert!(db.manager.proposal_hashes().contains(&p2hash));
        drop(proposal2);

        // commit the first proposal
        proposal1.commit().unwrap();
        // Ensure we committed the first proposal's data
        let committed = db.root_hash().unwrap();
        let historical = db.revision(committed).unwrap();
        assert_eq!(&*historical.val(b"k1").unwrap().unwrap(), b"v1");

        // the second proposal shouldn't be available to commit anymore
        assert!(!db.manager.proposal_hashes().contains(&p2hash));

        // the third proposal should still be contained within the all_hashes list
        let hash3 = proposal3.root_hash().unwrap();
        assert!(db.manager.proposal_hashes().contains(&hash3));

        // moreover, the data from the second and third proposals should still be available
        // through proposal3
        assert_eq!(&*proposal3.val(b"k2").unwrap().unwrap(), b"v2");
        assert_eq!(&*proposal3.val(b"k3").unwrap().unwrap(), b"v3");
    }

    #[test]
    fn test_view_sync() {
        let db = TestDb::new();

        // Create and commit some data to get a historical revision
        let batch = vec![BatchOp::Put {
            key: b"historical_key",
            value: b"historical_value",
        }];
        let proposal = db.propose(batch).unwrap();
        let historical_hash = proposal.root_hash().unwrap();
        proposal.commit().unwrap();

        // Create a new proposal (uncommitted)
        let batch = vec![BatchOp::Put {
            key: b"proposal_key",
            value: b"proposal_value",
        }];
        let proposal = db.propose(batch).unwrap();
        let proposal_hash = proposal.root_hash().unwrap();

        // Test that view_sync can find the historical revision
        let historical_view = db.view(historical_hash).unwrap();
        let value = historical_view.val(b"historical_key").unwrap().unwrap();
        assert_eq!(&*value, b"historical_value");

        // Test that view_sync can find the proposal
        let proposal_view = db.view(proposal_hash).unwrap();
        let value = proposal_view.val(b"proposal_key").unwrap().unwrap();
        assert_eq!(&*value, b"proposal_value");
    }

    #[test]
    fn test_propose_parallel_reopen() {
        fn insert_commit(db: &TestDb, kv: u8) {
            let keys: Vec<[u8; 1]> = vec![[kv; 1]];
            let vals: Vec<Box<[u8]>> = vec![Box::new([kv; 1])];
            let kviter = keys.iter().zip(vals.iter());
            let proposal = db.propose(kviter).unwrap();
            proposal.commit().unwrap();
        }

        // Create, insert, close, open, insert
        let db = TestDb::new_with_config(
            DbConfig::builder()
                .use_parallel(UseParallel::Always)
                .build(),
        );
        insert_commit(&db, 1);
        let db = db.reopen();
        insert_commit(&db, 2);
        // Check that the keys are still there after the commits
        let committed = db.revision(db.root_hash().unwrap()).unwrap();
        let keys: Vec<[u8; 1]> = vec![[1; 1], [2; 1]];
        let vals: Vec<Box<[u8]>> = vec![Box::new([1; 1]), Box::new([2; 1])];
        let kviter = keys.iter().zip(vals.iter());
        for (k, v) in kviter {
            assert_eq!(&committed.val(k).unwrap().unwrap(), v);
        }
        drop(db);

        // Open-db1, insert, open-db2, insert
        let db1 = TestDb::new_with_config(
            DbConfig::builder()
                .use_parallel(UseParallel::Always)
                .build(),
        );
        insert_commit(&db1, 1);
        let db2 = TestDb::new_with_config(
            DbConfig::builder()
                .use_parallel(UseParallel::Always)
                .build(),
        );
        insert_commit(&db2, 2);
        let committed1 = db1.revision(db1.root_hash().unwrap()).unwrap();
        let committed2 = db2.revision(db2.root_hash().unwrap()).unwrap();
        let keys: Vec<[u8; 1]> = vec![[1; 1], [2; 1]];
        let vals: Vec<Box<[u8]>> = vec![Box::new([1; 1]), Box::new([2; 1])];
        let mut kviter = keys.iter().zip(vals.iter());
        let (k, v) = kviter.next().unwrap();
        assert_eq!(&committed1.val(k).unwrap().unwrap(), v);
        let (k, v) = kviter.next().unwrap();
        assert_eq!(&committed2.val(k).unwrap().unwrap(), v);
    }

    #[test]
    fn test_propose_parallel() {
        const N: usize = 100;
        let db = TestDb::new_with_config(
            DbConfig::builder()
                .use_parallel(UseParallel::Always)
                .build(),
        );

        // Test an empty proposal
        let keys: Vec<[u8; 0]> = Vec::new();
        let vals: Vec<Box<[u8]>> = Vec::new();

        let kviter = keys.iter().zip(vals.iter());
        let proposal = db.propose(kviter).unwrap();
        proposal.commit().unwrap();

        // Create a proposal consisting of a single entry and an empty key.
        let keys: Vec<[u8; 0]> = vec![[0; 0]];

        // Note that if the value is [], then it is interpreted as a DeleteRange.
        // Instead, set value to [0]
        let vals: Vec<Box<[u8]>> = vec![Box::new([0; 1])];

        let kviter = keys.iter().zip(vals.iter());
        let proposal = db.propose(kviter).unwrap();

        let kviter = keys.iter().zip(vals.iter());
        for (k, v) in kviter {
            assert_eq!(&proposal.val(k).unwrap().unwrap(), v);
        }
        proposal.commit().unwrap();

        // Check that the key is still there after the commit
        let committed = db.revision(db.root_hash().unwrap()).unwrap();
        let kviter = keys.iter().zip(vals.iter());
        for (k, v) in kviter {
            assert_eq!(&committed.val(k).unwrap().unwrap(), v);
        }

        // Create a proposal that deletes the previous entry
        let vals: Vec<Box<[u8]>> = vec![Box::new([0; 0])];
        let kviter = keys.iter().zip(vals.iter());
        let proposal = db.propose(kviter).unwrap();

        let kviter = keys.iter().zip(vals.iter());
        for (k, _v) in kviter {
            assert_eq!(proposal.val(k).unwrap(), None);
        }
        proposal.commit().unwrap();

        // Create a proposal that inserts 0 to 999
        let (keys, vals): (Vec<_>, Vec<_>) = (0..1000)
            .map(|i| {
                (
                    format!("key{i}").into_bytes(),
                    Box::from(format!("value{i}").as_bytes()),
                )
            })
            .unzip();

        let kviter = keys.iter().zip(vals.iter());
        let proposal = db.propose(kviter).unwrap();
        let kviter = keys.iter().zip(vals.iter());
        for (k, v) in kviter {
            assert_eq!(&proposal.val(k).unwrap().unwrap(), v);
        }
        proposal.commit().unwrap();

        // Create a proposal that deletes all of the even entries
        let (keys, vals): (Vec<_>, Vec<_>) = (0..1000)
            .filter_map(|i| {
                if i % 2 != 0 {
                    Some::<(Vec<u8>, Box<[u8]>)>((format!("key{i}").into_bytes(), Box::new([])))
                } else {
                    None
                }
            })
            .unzip();

        let kviter = keys.iter().zip(vals.iter());
        let proposal = db.propose(kviter).unwrap();
        let kviter = keys.iter().zip(vals.iter());
        for (k, _v) in kviter {
            assert_eq!(proposal.val(k).unwrap(), None);
        }
        proposal.commit().unwrap();

        // Create a proposal that deletes using empty prefix
        let keys: Vec<[u8; 0]> = vec![[0; 0]];
        let vals: Vec<Box<[u8]>> = vec![Box::new([0; 0])];
        let kviter = keys.iter().zip(vals.iter());
        let proposal = db.propose(kviter).unwrap();
        proposal.commit().unwrap();

        // Create N keys and values like (key0, value0)..(keyN, valueN)
        let rng = firewood_storage::SeededRng::from_env_or_random();
        let (keys, vals): (Vec<_>, Vec<_>) = (0..N)
            .map(|i| {
                (
                    rng.random::<[u8; 32]>(),
                    Box::from(format!("value{i}").as_bytes()),
                )
            })
            .unzip();

        // Looping twice to test that we are reusing the thread pool.
        for _ in 0..2 {
            let kviter = keys.iter().zip(vals.iter());
            let proposal = db.propose(kviter).unwrap();

            // iterate over the keys and values again, checking that the values are in the correct proposal
            let kviter = keys.iter().zip(vals.iter());

            for (k, v) in kviter {
                assert_eq!(&proposal.val(k).unwrap().unwrap(), v);
            }
            proposal.commit().unwrap();
        }
    }

    #[test]
    fn test_propose_parallel_vs_normal_propose() {
        fn persisted_deleted(nodes: &[MaybePersistedNode]) -> Vec<LinearAddress> {
            let mut addresses: Vec<_> = nodes
                .iter()
                .filter_map(MaybePersistedNode::as_linear_address)
                .collect();
            addresses.sort();
            addresses
        }

        let parallel_db = TestDb::new_with_config(
            DbConfig::builder()
                .use_parallel(UseParallel::Always)
                .build(),
        );
        let single_threaded_db =
            TestDb::new_with_config(DbConfig::builder().use_parallel(UseParallel::Never).build());

        // First batch: insert two keys with different first nibbles so they are
        // handled by different workers in the parallel merkle implementation.
        let initial_keys: Vec<[u8; 1]> = vec![[0x00], [0x10]];
        let initial_values: Vec<Box<[u8]>> = vec![Box::new([1u8]), Box::new([2u8])];

        let initial_parallel_proposal = parallel_db
            .propose(initial_keys.iter().zip(initial_values.iter()))
            .unwrap();
        let initial_single_proposal = single_threaded_db
            .propose(initial_keys.iter().zip(initial_values.iter()))
            .unwrap();

        initial_parallel_proposal.commit().unwrap();
        initial_single_proposal.commit().unwrap();

        // Second batch: update only the first key.
        let update_keys: Vec<[u8; 1]> = vec![[0x00]];
        let update_values: Vec<Box<[u8]>> = vec![Box::new([3u8])];

        let update_parallel_proposal = parallel_db
            .propose(update_keys.iter().zip(update_values.iter()))
            .unwrap();
        let update_single_proposal = single_threaded_db
            .propose(update_keys.iter().zip(update_values.iter()))
            .unwrap();

        // Wait for background persistence to complete before creating update proposals
        // so both DBs are in the same persisted state
        parallel_db.wait_persisted();
        single_threaded_db.wait_persisted();

        let parallel_deleted = update_parallel_proposal.nodestore.deleted().to_vec();
        let single_deleted = update_single_proposal.nodestore.deleted().to_vec();

        assert_eq!(
            persisted_deleted(&parallel_deleted),
            persisted_deleted(&single_deleted),
            "persisted deleted nodes should match between parallel and single proposals",
        );

        update_parallel_proposal.commit().unwrap();
        update_single_proposal.commit().unwrap();
    }

    /// Test that proposing on a proposal works as expected
    ///
    /// Test creates two batches and proposes them, and verifies that the values are in the correct proposal.
    /// It then commits them one by one, and verifies the latest committed version is correct.
    #[test]
    fn test_propose_on_proposal() {
        // number of keys and values to create for this test
        const N: usize = 20;

        let db = TestDb::new();

        // create N keys and values like (key0, value0)..(keyN, valueN)
        let (keys, vals): (Vec<_>, Vec<_>) = (0..N)
            .map(|i| {
                (
                    format!("key{i}").into_bytes(),
                    Box::from(format!("value{i}").as_bytes()),
                )
            })
            .unzip();

        // create two batches, one with the first half of keys and values, and one with the last half keys and values
        let mut kviter = keys.iter().zip(vals.iter());

        // create two proposals, second one has a base of the first one
        let proposal1 = db.propose(kviter.by_ref().take(N / 2)).unwrap();
        let proposal2 = proposal1.propose(kviter).unwrap();

        // iterate over the keys and values again, checking that the values are in the correct proposal
        let mut kviter = keys.iter().zip(vals.iter());

        // first half of the keys should be in both proposals
        for (k, v) in kviter.by_ref().take(N / 2) {
            assert_eq!(&proposal1.val(k).unwrap().unwrap(), v);
            assert_eq!(&proposal2.val(k).unwrap().unwrap(), v);
        }

        // remaining keys should only be in the second proposal
        for (k, v) in kviter {
            // second half of keys are in the second proposal
            assert_eq!(&proposal2.val(k).unwrap().unwrap(), v);
            // but not in the first
            assert_eq!(proposal1.val(k).unwrap(), None);
        }

        proposal1.commit().unwrap();

        // all keys are still in the second proposal (first is no longer accessible)
        for (k, v) in keys.iter().zip(vals.iter()) {
            assert_eq!(&proposal2.val(k).unwrap().unwrap(), v);
        }

        // commit the second proposal
        proposal2.commit().unwrap();

        // all keys are in the database
        let committed = db.root_hash().unwrap();
        let revision = db.revision(committed).unwrap();

        for (k, v) in keys.into_iter().zip(vals.into_iter()) {
            assert_eq!(revision.val(k).unwrap().unwrap(), v);
        }
    }

    #[test]
    fn test_slow_fuzz_checker() {
        let rng = firewood_storage::SeededRng::from_env_or_random();

        let db = TestDb::new();

        // takes about 0.3s on a mac to run 50 times
        for _ in 0..50 {
            // create a batch of 10 random key-value pairs
            let batch = (0..10).fold(vec![], |mut batch, _| {
                let key: [u8; 32] = rng.random();
                let value: [u8; 8] = rng.random();
                batch.push(BatchOp::Put {
                    key: key.to_vec(),
                    value,
                });
                if rng.random_range(0..5) == 0 {
                    let addon: [u8; 32] = rng.random();
                    let key = [key, addon].concat();
                    let value: [u8; 8] = rng.random();
                    batch.push(BatchOp::Put { key, value });
                }
                batch
            });
            let proposal = db.propose(batch).unwrap();
            proposal.commit().unwrap();

            // Wait for background persistence to complete before checking consistency
            db.wait_persisted();

            // check the database for consistency, sometimes checking the hashes
            let hash_check = rng.random();
            let report = db.check(CheckOpt {
                hash_check,
                progress_bar: None,
            });
            if report
                .errors
                .iter()
                .filter(|e| !matches!(e, CheckerError::AreaLeaks(_)))
                .count()
                != 0
            {
                db.dump(&mut std::io::stdout()).unwrap();
                panic!("error: {:?}", report.errors);
            }
        }
    }

    #[test]
    fn test_deep_propose() {
        const NUM_KEYS: NonZeroUsize = const { NonZeroUsize::new(2).unwrap() };
        const NUM_PROPOSALS: usize = 100;

        let db = TestDb::new();

        let ops = (0..(NUM_KEYS.get() * NUM_PROPOSALS))
            .map(|i| (format!("key{i}"), format!("value{i}")))
            .collect::<Vec<_>>();

        let proposals = ops.iter().chunk_fold(
            NUM_KEYS,
            Vec::<Proposal<'_>>::with_capacity(NUM_PROPOSALS),
            |mut proposals, ops| {
                let proposal = if let Some(parent) = proposals.last() {
                    parent.propose(ops).unwrap()
                } else {
                    db.propose(ops).unwrap()
                };

                proposals.push(proposal);
                proposals
            },
        );

        let last_proposal_root_hash = proposals.last().unwrap().root_hash().unwrap();

        // commit the proposals
        for proposal in proposals {
            proposal.commit().unwrap();
        }

        // get the last committed revision
        let last_root_hash = db.root_hash().unwrap();
        let committed = db.revision(last_root_hash.clone()).unwrap();

        // the last root hash should be the same as the last proposal root hash
        assert_eq!(last_root_hash, last_proposal_root_hash);

        // check that all the keys and values are still present
        for (k, v) in &ops {
            let found = committed.val(k).unwrap();
            assert_eq!(
                found.as_deref(),
                Some(v.as_bytes()),
                "Value for key {k:?} should be {v:?} but was {found:?}",
            );
        }
    }

    /// Test that reading from a proposal during commit works as expected
    #[test]
    fn test_slow_read_during_commit() {
        use crate::db::Proposal;

        const CHANNEL_CAPACITY: usize = 8;

        let testdb = TestDb::new();
        let db = &*testdb;

        let (tx, rx) = std::sync::mpsc::sync_channel::<Proposal<'_>>(CHANNEL_CAPACITY);
        let (result_tx, result_rx) = std::sync::mpsc::sync_channel(CHANNEL_CAPACITY);

        // scope will block until all scope-spawned threads finish
        std::thread::scope(|scope| {
            // Commit task
            scope.spawn(move || {
                while let Ok(proposal) = rx.recv() {
                    let result = proposal.commit();
                    // send result back to the main thread, both for synchronization and stopping the
                    // test on error
                    result_tx.send(result).unwrap();
                }
            });
            scope.spawn(move || {
                // Proposal creation
                for id in 0u32..500 {
                    // insert a key of length 32 and a value of length 8,
                    // rotating between all zeroes through all 255
                    let batch = vec![BatchOp::Put {
                        key: [id as u8; 32],
                        value: [id as u8; 8],
                    }];
                    let proposal = db.propose(batch).unwrap();
                    let last_hash = proposal.root_hash().unwrap();
                    let view = db.view(last_hash).unwrap();

                    tx.send(proposal).unwrap();

                    let key = [id as u8; 32];
                    let value = view.val(&key).unwrap().unwrap();
                    assert_eq!(&*value, &[id as u8; 8]);
                    result_rx.recv().unwrap().unwrap();
                }
                // close the channel, which will cause the commit task to exit
                drop(tx);
            });
        });
    }

    #[test]
    fn test_resurrect_unpersisted_root() {
        let db = TestDb::new();

        // First, create a revision to retrieve
        let key = b"key";
        let value = b"value";
        let batch = vec![BatchOp::Put { key, value }];

        let proposal = db.propose(batch).unwrap();
        let root_hash = proposal.root_hash().unwrap();
        proposal.commit().unwrap();

        // Wait for background persistence to complete
        db.wait_persisted();

        let root_address = db
            .revision(root_hash.clone())
            .unwrap()
            .root_address()
            .unwrap();

        // Next, overwrite the kv-pair with a new revision
        let new_value = b"new_value";
        let batch = vec![BatchOp::Put {
            key,
            value: new_value,
        }];

        let proposal = db.propose(batch).unwrap();
        proposal.commit().unwrap();

        // Finally, reopen the database and make sure that we can retrieve the first revision
        let db = db.reopen();

        let latest_root_hash = db.root_hash().unwrap();
        let latest_revision = db.revision(latest_root_hash).unwrap();

        let latest_value = latest_revision.val(key).unwrap().unwrap();
        assert_eq!(new_value, latest_value.as_ref());

        let node_store = NodeStore::with_root(
            root_hash.into_hash_type(),
            root_address,
            latest_revision.get_storage(),
        );

        let retrieved_value = node_store.val(key).unwrap().unwrap();
        assert_eq!(value, retrieved_value.as_ref());
    }

    /// Verifies that persisted revisions are still accessible when reopening the database.
    #[test]
    fn test_root_store() {
        let db = TestDb::new_with_config(DbConfig::builder().root_store(true).build());

        // First, create a revision to retrieve
        let key = b"key";
        let value = b"value";
        let batch = vec![BatchOp::Put { key, value }];

        let proposal = db.propose(batch).unwrap();
        let root_hash = proposal.root_hash().unwrap();
        proposal.commit().unwrap();

        // Next, overwrite the kv-pair with a new revision
        let new_value = b"new_value";
        let batch = vec![BatchOp::Put {
            key,
            value: new_value,
        }];

        let proposal = db.propose(batch).unwrap();
        proposal.commit().unwrap();

        // Reopen the database and verify that the database can access a persisted revision
        let db = db.reopen();

        let view = db.view(root_hash).unwrap();
        let retrieved_value = view.val(key).unwrap().unwrap();
        assert_eq!(value, retrieved_value.as_ref());
    }

    #[test]
    fn test_rootstore_empty_db_reopen() {
        let db = TestDb::new_with_config(DbConfig::builder().root_store(true).build());

        db.reopen();
    }

    /// Verifies that revisions exceeding the in-memory limit can still be retrieved.
    #[test]
    fn test_root_store_with_capped_max_revisions() {
        const NUM_REVISIONS: usize = 10;

        let dbconfig = DbConfig::builder()
            .manager(RevisionManagerConfig::builder().max_revisions(5).build())
            .root_store(true)
            .build();
        let db = TestDb::new_with_config(dbconfig);

        // Create and commit 10 proposals
        let key = b"root_store";
        let revisions: HashMap<TrieHash, _> = (0..NUM_REVISIONS)
            .map(|i| {
                let value = i.to_be_bytes();
                let batch = vec![BatchOp::Put { key, value }];
                let proposal = db.propose(batch).unwrap();
                let root_hash = proposal.root_hash().unwrap();
                proposal.commit().unwrap();

                (root_hash, value)
            })
            .collect();

        // Verify that we can access all revisions with their correct values
        for (root_hash, value) in &revisions {
            let revision = db.revision(root_hash.clone()).unwrap();
            let retrieved_value = revision.val(key).unwrap().unwrap();
            assert_eq!(value.as_slice(), retrieved_value.as_ref());
        }

        let db = db.reopen();

        // Verify that we can access all revisions with their correct values
        // after reopening
        for (root_hash, value) in &revisions {
            let revision = db.revision(root_hash.clone()).unwrap();
            let retrieved_value = revision.val(key).unwrap().unwrap();
            assert_eq!(value.as_slice(), retrieved_value.as_ref());
        }
    }

    /// Verifies that `RootStore` is truncated as well if we truncate the database.
    #[test]
    fn test_root_store_truncation() {
        let db =
            TestDb::new_with_config(DbConfig::builder().root_store(true).truncate(true).build());

        // Create a revision to store
        let batch = vec![BatchOp::Put {
            key: b"foo",
            value: b"bar",
        }];
        let proposal = db.propose(batch).unwrap();
        let root_hash = proposal.root_hash().unwrap();
        proposal.commit().unwrap();

        let db = db.reopen();
        assert!(db.view(root_hash).is_err());
    }

    /// Verifies that opening a database fails if the directory doesn't exist.
    #[test]
    fn test_nonexistent_directory() {
        let tmpdir = tempfile::tempdir().unwrap();

        assert!(Db::new(tmpdir, DbConfig::builder().create_if_missing(false).build()).is_err());
    }

    #[test]
    fn test_backwards_compatible_magic_string() {
        use std::os::unix::fs::FileExt;

        let testdb = TestDb::new();

        testdb
            .propose([(b"key", b"value")])
            .unwrap()
            .commit()
            .unwrap();

        // Wait for background persistence to complete before reading the file
        testdb.wait_persisted();

        let rh = testdb.root_hash().unwrap();

        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .append(false)
            .truncate(false)
            .open(testdb.path().join(crate::manager::DB_FILE_NAME))
            .unwrap();

        let mut version = [0_u8; 16];
        file.read_exact_at(&mut version, 0).unwrap();
        assert_eq!(&version, b"firewood-v1\0\0\0\0\0");

        // overwrite the magic string to simulate an older version
        file.write_all_at(b"firewood 0.0.18\0", 0).unwrap();
        drop(file);

        let testdb = testdb.reopen();

        assert_eq!(rh, testdb.root_hash().unwrap());
    }

    #[test]
    fn test_deferred_persist_multiple_commits_with_commit_count_one() {
        const NUM_REVISIONS: usize = 20;

        let dbcfg = DbConfig::builder()
            .manager(
                RevisionManagerConfig::builder()
                    .max_revisions(NUM_REVISIONS)
                    .build(),
            )
            .build();

        let db = TestDb::new_with_config(dbcfg);

        // Create and commit NUM_REVISIONS proposals, storing their root hashes
        let root_hashes: Vec<TrieHash> = (0..NUM_REVISIONS)
            .map(|i| {
                let batch = vec![BatchOp::Put {
                    key: format!("key{i}").as_bytes().to_vec(),
                    value: format!("value{i}").as_bytes().to_vec(),
                }];
                let proposal = db.propose(batch).unwrap();
                let root_hash = proposal.root_hash().unwrap();
                proposal.commit().unwrap();
                root_hash
            })
            .collect();

        // Wait for the background thread to finish persisting
        db.wait_persisted();

        // Verify that all revisions were persisted
        for (i, root_hash) in root_hashes.iter().enumerate() {
            let is_persisted = db
                .manager
                .revision_persist_status(root_hash.clone())
                .unwrap();

            assert!(
                is_persisted,
                "Revision {i} with root hash {root_hash:?} should be persisted"
            );
        }
    }

    #[test]
    fn test_deferred_persist_close_with_commit_count_one() {
        let dbcfg = DbConfig::builder().build();

        let db = TestDb::new_with_config(dbcfg);

        // Then, commit once and see what the latest revision is
        let key = b"foo";
        let value = b"bar";
        let batch = vec![BatchOp::Put { key, value }];
        let proposal = db.propose(batch).unwrap();
        let root_hash = proposal.root_hash().unwrap();

        proposal.commit().unwrap();
        let db = db.reopen();

        let revision = db.view(root_hash).unwrap();
        let new_value = revision.val(b"foo").unwrap().unwrap();

        assert_eq!(value, new_value.as_ref());
    }

    #[test]
    fn test_deferred_persist_close_with_high_commit_count() {
        const HIGH_COMMIT_COUNT: NonZeroU64 = nonzero!(1_000_000u64);
        const MAX_REVISIONS: usize = HIGH_COMMIT_COUNT.get() as usize + 1;

        // Set commit count to an arbitrarily high number so persist happens
        // only on shutdown
        let dbcfg = DbConfig::builder()
            .manager(
                RevisionManagerConfig::builder()
                    .max_revisions(MAX_REVISIONS)
                    .deferred_persistence_commit_count(HIGH_COMMIT_COUNT)
                    .build(),
            )
            .build();

        let db = TestDb::new_with_config(dbcfg);

        // Then, commit once and see what the latest revision is
        let key = b"foo";
        let value = b"bar";
        let batch = vec![BatchOp::Put { key, value }];
        let proposal = db.propose(batch).unwrap();
        let root_hash = proposal.root_hash().unwrap();

        proposal.commit().unwrap();
        let db = db.reopen();

        let revision = db.view(root_hash).unwrap();
        let new_value = revision.val(b"foo").unwrap().unwrap();

        assert_eq!(value, new_value.as_ref());
    }

    #[test]
    fn test_deferred_persist_with_multiple_commit_count() {
        const COMMIT_COUNT: NonZeroU64 = nonzero!(5u64);
        const NUM_REVISIONS: u64 = COMMIT_COUNT.get() + 1;

        let dbcfg = DbConfig::builder()
            .manager(
                RevisionManagerConfig::builder()
                    .deferred_persistence_commit_count(COMMIT_COUNT)
                    .build(),
            )
            .build();

        let db = TestDb::new_with_config(dbcfg);

        let mut root_hashes = Vec::new();

        for i in 0..NUM_REVISIONS {
            let batch = vec![BatchOp::Put {
                key: format!("key{i}").as_bytes().to_vec(),
                value: format!("value{i}").as_bytes().to_vec(),
            }];
            let proposal = db.propose(batch).unwrap();
            root_hashes.push(proposal.root_hash().unwrap());
            proposal.commit().unwrap();
        }

        // Verify that at least one of the last COMMIT_COUNT revisions is persisted.
        let commit_count = COMMIT_COUNT.get() as usize;
        let any_persisted = root_hashes
            .iter()
            .rev()
            .take(commit_count)
            .any(|hash| db.manager.revision_persist_status(hash.clone()).unwrap());

        assert!(
            any_persisted,
            "At least one of the last {COMMIT_COUNT} revisions should be persisted"
        );
    }

    /// Verifies that an unpersisted revision which wipes the database is
    /// persisted when the database closes.
    #[test]
    fn test_deferred_persistence_closing_on_empty_trie() {
        const COMMIT_COUNT: NonZeroU64 = nonzero!(10u64);

        let dbcfg = DbConfig::builder()
            .manager(
                RevisionManagerConfig::builder()
                    .deferred_persistence_commit_count(COMMIT_COUNT)
                    .build(),
            )
            .build();

        let db = TestDb::new_with_config(dbcfg);

        // Commit COMMIT_COUNT proposals to trigger the first persist
        for i in 0..COMMIT_COUNT.get() {
            let batch = vec![BatchOp::Put {
                key: format!("key{i}").as_bytes().to_vec(),
                value: format!("value{i}").as_bytes().to_vec(),
            }];
            let proposal = db.propose(batch).unwrap();
            proposal.commit().unwrap();
        }

        // Empty the trie
        let batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> = vec![BatchOp::DeleteRange { prefix: vec![] }];
        let proposal = db.propose(batch).unwrap();
        proposal.commit().unwrap();

        let db = db.reopen();

        // Verify that the latest committed revision is empty.
        let last_committed_hash = db.root_hash();
        assert_eq!(last_committed_hash, TrieHash::default_root_hash());
    }

    #[test]
    fn test_deferred_persistence_root_store() {
        const NUM_COMMITS: usize = 20;
        const COMMIT_COUNT: NonZeroU64 = nonzero!(10u64);
        const MAX_REVISIONS: usize = COMMIT_COUNT.get() as usize + 1;

        let dbcfg = DbConfig::builder()
            .manager(
                RevisionManagerConfig::builder()
                    .max_revisions(MAX_REVISIONS)
                    .deferred_persistence_commit_count(COMMIT_COUNT)
                    .build(),
            )
            .root_store(true)
            .build();

        let db = TestDb::new_with_config(dbcfg);

        let mut root_hashes = Vec::new();

        let key = b"key";
        for i in 0..NUM_COMMITS {
            let batch = vec![BatchOp::Put {
                key,
                value: format!("{i}").as_bytes().to_vec(),
            }];
            let proposal = db.propose(batch).unwrap();
            root_hashes.push(proposal.root_hash().unwrap());
            proposal.commit().unwrap();
        }

        let db = db.reopen();

        // Verify that we never went more than COMMIT_COUNT revisions without
        // persisting.
        let commit_count = COMMIT_COUNT.get() as usize;
        let mut last_persisted: Option<usize> = None;

        for (i, hash) in root_hashes.iter().enumerate() {
            let is_persisted = db
                .manager
                .revision_persist_status(hash.clone())
                .is_ok_and(|persisted| persisted);

            if is_persisted {
                let gap = match last_persisted {
                    Some(prev) => i.wrapping_sub(prev),
                    None => i.wrapping_add(1),
                };
                assert!(
                    gap <= commit_count,
                    "Gap of {gap} between persisted revisions exceeds COMMIT_COUNT of {commit_count}"
                );
                last_persisted = Some(i);
            }
        }

        assert!(
            last_persisted.is_some(),
            "At least one revision should be persisted"
        );
    }

    /// Verifies that non-persisted revisions are lost after reopening the database.
    #[test]
    fn test_deferred_persistence_unpersisted_revisions() {
        const COMMIT_COUNT: NonZeroU64 = nonzero!(10u64);

        let dbcfg = DbConfig::builder()
            .manager(
                RevisionManagerConfig::builder()
                    .deferred_persistence_commit_count(COMMIT_COUNT)
                    .build(),
            )
            .root_store(true)
            .build();

        let db = TestDb::new_with_config(dbcfg);

        let mut root_hashes = Vec::new();

        let key = b"key";
        for i in 0..COMMIT_COUNT.get() {
            let batch = vec![BatchOp::Put {
                key,
                value: format!("{i}").as_bytes().to_vec(),
            }];
            let proposal = db.propose(batch).unwrap();
            root_hashes.push(proposal.root_hash().unwrap());
            proposal.commit().unwrap();
        }

        let db = db.reopen();

        let persisted_count = root_hashes
            .iter()
            .filter(|&hash| {
                db.manager
                    .revision_persist_status(hash.clone())
                    .is_ok_and(|is_persisted| is_persisted)
            })
            .count();

        assert!(
            persisted_count > 0 && persisted_count <= 2,
            "Expected at most 2 persisted revisions, but found {persisted_count}"
        );
    }

    // Testdb is a helper struct for testing the Db. Once it's dropped, the directory and file disappear
    pub(super) struct TestDb {
        db: Option<Db>,
        tmpdir: tempfile::TempDir,
        dbconfig: DbConfig,
    }
    impl Drop for TestDb {
        fn drop(&mut self) {
            if let Some(db) = self.db.take() {
                db.close().unwrap();
            }
        }
    }
    impl Deref for TestDb {
        type Target = Db;
        fn deref(&self) -> &Self::Target {
            self.db.as_ref().unwrap()
        }
    }
    impl DerefMut for TestDb {
        fn deref_mut(&mut self) -> &mut Self::Target {
            self.db.as_mut().unwrap()
        }
    }

    impl TestDb {
        pub fn new() -> Self {
            TestDb::new_with_config(DbConfig::builder().build())
        }

        pub fn new_with_config(dbconfig: DbConfig) -> Self {
            let tmpdir = tempfile::tempdir().unwrap();
            let db = Db::new(tmpdir.as_ref(), dbconfig.clone()).unwrap();
            TestDb {
                db: Some(db),
                tmpdir,
                dbconfig,
            }
        }

        /// Reopens the database at the same path, preserving existing data.
        ///
        /// This method closes the current database instance (releasing the advisory lock),
        /// then opens it again at the same path while keeping the same configuration.
        pub fn reopen(mut self) -> Self {
            self.db.take().unwrap().close().unwrap();

            let db = Db::new(self.tmpdir.path(), self.dbconfig.clone()).unwrap();
            self.db = Some(db);
            self
        }

        /// Replaces the database with a fresh instance at the same path.
        ///
        /// This method closes the current database instance (releasing the advisory lock),
        /// and creates a new database. This completely resets the database, removing all
        /// existing data and starting fresh. The new database instance will use the default
        /// configuration with truncation enabled.
        ///
        /// This is useful for testing scenarios where you want to start with a clean slate
        /// while maintaining the same temporary directory structure.
        pub fn replace(mut self) -> Self {
            self.db.take().unwrap().close().unwrap();

            self.dbconfig = DbConfig::builder().truncate(true).build();
            let db = Db::new(self.tmpdir.path(), self.dbconfig.clone()).unwrap();
            self.db = Some(db);
            self
        }

        pub fn path(&self) -> &Path {
            self.tmpdir.path()
        }
    }

    // === MultiDb Integration Tests ===

    use super::{MultiDb, MultiDbConfig, ValidatorId};
    use std::sync::Arc as StdArc;

    fn create_multi_db() -> (MultiDb, tempfile::TempDir) {
        let tmpdir = tempfile::tempdir().unwrap();
        let cfg = MultiDbConfig::builder()
            .db(DbConfig::builder().build())
            .build();
        let db = MultiDb::new(tmpdir.as_ref(), cfg).unwrap();
        (db, tmpdir)
    }

    #[test]
    fn test_multi_db_create_and_close() {
        let (db, _tmpdir) = create_multi_db();
        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_register_single_validator() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();
        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_register_multiple_validators() {
        let (db, _tmpdir) = create_multi_db();
        for i in 0..4u64 {
            db.register_validator(ValidatorId::new(i)).unwrap();
        }
        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_register_duplicate_returns_error() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();
        let err = db.register_validator(v0);
        assert!(err.is_err());
        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_deregister_validator() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();
        db.deregister_validator(v0).unwrap();
        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_deregister_nonexistent_returns_error() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        let err = db.deregister_validator(v0);
        assert!(err.is_err());
        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_deregister_then_reregister() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();
        db.deregister_validator(v0).unwrap();
        db.register_validator(v0).unwrap();
        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_propose_commit_single_validator() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();

        let batch = vec![BatchOp::Put {
            key: b"key1".to_vec(),
            value: b"value1".to_vec(),
        }];
        let proposal = db.propose(v0, batch).unwrap();
        db.commit(v0, proposal).unwrap();

        // Verify the value is readable
        let head = db.validator_view(v0).unwrap();
        assert_eq!(
            head.val(b"key1" as &[u8]).unwrap(),
            Some(b"value1".to_vec().into_boxed_slice())
        );
        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_two_validators_same_batch_deduplicates() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        // Both validators propose the same batch
        let batch = vec![BatchOp::Put {
            key: b"key1".to_vec(),
            value: b"value1".to_vec(),
        }];

        // V0 commits first
        let proposal0 = db.propose(v0, batch.clone()).unwrap();
        db.commit(v0, proposal0).unwrap();

        // V1 proposes and commits the same batch
        let proposal1 = db.propose(v1, batch).unwrap();
        db.commit(v1, proposal1).unwrap();

        // Both validators should see the same data
        let hash0 = db.validator_root_hash(v0).unwrap();
        let hash1 = db.validator_root_hash(v1).unwrap();
        assert_eq!(hash0, hash1);

        // Both should be able to read the value
        let head0 = db.validator_view(v0).unwrap();
        let head1 = db.validator_view(v1).unwrap();
        assert_eq!(
            head0.val(b"key1" as &[u8]).unwrap(),
            Some(b"value1".to_vec().into_boxed_slice())
        );
        assert_eq!(
            head1.val(b"key1" as &[u8]).unwrap(),
            Some(b"value1".to_vec().into_boxed_slice())
        );

        // Both heads should be the same Arc (deduplication)
        assert!(StdArc::ptr_eq(&head0, &head1));

        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_advance_to_hash() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        // V0 commits a block
        let batch = vec![BatchOp::Put {
            key: b"key1".to_vec(),
            value: b"value1".to_vec(),
        }];
        let proposal = db.propose(v0, batch).unwrap();
        db.commit(v0, proposal).unwrap();

        // V1 advances to V0's hash without proposing
        let hash0 = db.validator_root_hash(v0).unwrap().unwrap();
        db.advance_to_hash(v1, hash0).unwrap();

        // Both should see the same data
        let head1 = db.validator_view(v1).unwrap();
        assert_eq!(
            head1.val(b"key1" as &[u8]).unwrap(),
            Some(b"value1".to_vec().into_boxed_slice())
        );

        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_advance_to_nonexistent_hash_returns_error() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();

        let fake_hash = TrieHash::from([0xFFu8; 32]);
        let err = db.advance_to_hash(v0, fake_hash);
        assert!(err.is_err());
        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_validator_view_returns_correct_head() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        // V0 commits a block with key1
        let batch = vec![BatchOp::Put {
            key: b"key1".to_vec(),
            value: b"value1".to_vec(),
        }];
        let proposal = db.propose(v0, batch).unwrap();
        db.commit(v0, proposal).unwrap();

        // V0 should see key1, V1 should not (V1 is still at the initial empty revision)
        let head0 = db.validator_view(v0).unwrap();
        let head1 = db.validator_view(v1).unwrap();
        assert_eq!(
            head0.val(b"key1" as &[u8]).unwrap(),
            Some(b"value1".to_vec().into_boxed_slice())
        );
        assert_eq!(head1.val(b"key1" as &[u8]).unwrap(), None);

        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_commit_wrong_parent_returns_error() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        // V0 commits a block
        let batch = vec![BatchOp::Put {
            key: b"key1".to_vec(),
            value: b"value1".to_vec(),
        }];
        let proposal0 = db.propose(v0, batch.clone()).unwrap();
        db.commit(v0, proposal0).unwrap();

        // V0 proposes another block from its new head
        let batch2 = vec![BatchOp::Put {
            key: b"key2".to_vec(),
            value: b"value2".to_vec(),
        }];
        let proposal_v0_2 = db.propose(v0, batch2).unwrap();

        // Try to commit V0's second proposal under V1 (wrong parent)
        let err = db.commit(v1, proposal_v0_2);
        assert!(err.is_err());

        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_commit_for_nonexistent_validator() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();

        let batch = vec![BatchOp::Put {
            key: b"key1".to_vec(),
            value: b"value1".to_vec(),
        }];
        let proposal = db.propose(v0, batch).unwrap();

        // Try to commit under an unregistered validator
        let err = db.commit(v1, proposal);
        assert!(err.is_err());

        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_divergent_produces_separate_revision() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        // V0 commits key1=value1
        let batch0 = vec![BatchOp::Put {
            key: b"key1".to_vec(),
            value: b"value1".to_vec(),
        }];
        let proposal0 = db.propose(v0, batch0).unwrap();
        db.commit(v0, proposal0).unwrap();

        // V1 commits different data from the same parent (divergent)
        let batch1 = vec![BatchOp::Put {
            key: b"key1".to_vec(),
            value: b"DIVERGENT".to_vec(),
        }];
        let proposal1 = db.propose(v1, batch1).unwrap();
        db.commit(v1, proposal1).unwrap();

        // Hashes should differ
        let hash0 = db.validator_root_hash(v0).unwrap();
        let hash1 = db.validator_root_hash(v1).unwrap();
        assert_ne!(hash0, hash1);

        // Each validator sees its own data
        let head0 = db.validator_view(v0).unwrap();
        let head1 = db.validator_view(v1).unwrap();
        assert_eq!(
            head0.val(b"key1" as &[u8]).unwrap(),
            Some(b"value1".to_vec().into_boxed_slice())
        );
        assert_eq!(
            head1.val(b"key1" as &[u8]).unwrap(),
            Some(b"DIVERGENT".to_vec().into_boxed_slice())
        );

        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_max_validators_limit() {
        let (db, _tmpdir) = create_multi_db();

        // Register up to MAX_VALIDATORS
        for i in 0..firewood_storage::MAX_VALIDATORS as u64 {
            db.register_validator(ValidatorId::new(i)).unwrap();
        }

        // One more should fail
        let err = db.register_validator(ValidatorId::new(firewood_storage::MAX_VALIDATORS as u64));
        assert!(err.is_err());

        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_three_way_dedup() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        let v2 = ValidatorId::new(2);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();
        db.register_validator(v2).unwrap();

        let batch = vec![BatchOp::Put {
            key: b"key1".to_vec(),
            value: b"value1".to_vec(),
        }];

        // All three validators commit the same batch
        let p0 = db.propose(v0, batch.clone()).unwrap();
        db.commit(v0, p0).unwrap();

        let p1 = db.propose(v1, batch.clone()).unwrap();
        db.commit(v1, p1).unwrap();

        let p2 = db.propose(v2, batch).unwrap();
        db.commit(v2, p2).unwrap();

        // All should have the same hash
        let h0 = db.validator_root_hash(v0).unwrap();
        let h1 = db.validator_root_hash(v1).unwrap();
        let h2 = db.validator_root_hash(v2).unwrap();
        assert_eq!(h0, h1);
        assert_eq!(h1, h2);

        // All should share the same Arc
        let head0 = db.validator_view(v0).unwrap();
        let head1 = db.validator_view(v1).unwrap();
        let head2 = db.validator_view(v2).unwrap();
        assert!(StdArc::ptr_eq(&head0, &head1));
        assert!(StdArc::ptr_eq(&head1, &head2));

        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_propose_for_deregistered_validator() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();
        db.deregister_validator(v0).unwrap();

        let batch = vec![BatchOp::Put {
            key: b"key1".to_vec(),
            value: b"value1".to_vec(),
        }];
        let err = db.propose(v0, batch);
        assert!(err.is_err());

        db.close().unwrap();
    }

    #[test]
    fn test_multi_db_sequential_commits() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();

        // Commit multiple blocks sequentially
        for i in 0..5u8 {
            let batch = vec![BatchOp::Put {
                key: format!("key{i}").into_bytes(),
                value: format!("value{i}").into_bytes(),
            }];
            let proposal = db.propose(v0, batch).unwrap();
            db.commit(v0, proposal).unwrap();
        }

        // Verify all values are readable
        let head = db.validator_view(v0).unwrap();
        for i in 0..5u8 {
            let key = format!("key{i}");
            let expected = format!("value{i}").into_bytes().into_boxed_slice();
            assert_eq!(head.val(key.as_bytes()).unwrap(), Some(expected));
        }

        db.close().unwrap();
    }

    // === Per-Chain Revision Budget Tests ===

    fn create_multi_db_with_max_revisions(max_revisions: usize) -> (MultiDb, tempfile::TempDir) {
        let tmpdir = tempfile::tempdir().unwrap();
        let cfg = MultiDbConfig::builder()
            .db(DbConfig::builder()
                .manager(
                    crate::manager::RevisionManagerConfig::builder()
                        .max_revisions(max_revisions)
                        .build(),
                )
                .build())
            .build();
        let db = MultiDb::new(tmpdir.as_ref(), cfg).unwrap();
        (db, tmpdir)
    }

    #[test]
    fn test_chain_fork_creates_new_chain() {
        // Two validators commit different batches from the same parent → fork
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        // Both start at the same base. v0 commits first.
        let batch_a = vec![BatchOp::Put {
            key: b"key_a".to_vec(),
            value: b"value_a".to_vec(),
        }];
        let p0 = db.propose(v0, batch_a).unwrap();
        let hash_a = p0.root_hash();
        db.commit(v0, p0).unwrap();

        // v1 commits a DIFFERENT batch from the same parent → divergence/fork
        let batch_b = vec![BatchOp::Put {
            key: b"key_b".to_vec(),
            value: b"value_b".to_vec(),
        }];
        let p1 = db.propose(v1, batch_b).unwrap();
        let hash_b = p1.root_hash();
        db.commit(v1, p1).unwrap();

        // Different hashes → fork happened
        assert_ne!(hash_a, hash_b);

        // Both views should work independently
        let head0 = db.validator_view(v0).unwrap();
        let head1 = db.validator_view(v1).unwrap();
        assert_eq!(
            head0.val(b"key_a").unwrap(),
            Some(b"value_a".to_vec().into_boxed_slice())
        );
        assert_eq!(
            head1.val(b"key_b").unwrap(),
            Some(b"value_b".to_vec().into_boxed_slice())
        );

        db.close().unwrap();
    }

    #[test]
    fn test_chain_dedup_moves_validator_to_existing_chain() {
        // Two validators commit the same batch → dedup, same chain
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        let batch = vec![BatchOp::Put {
            key: b"key".to_vec(),
            value: b"value".to_vec(),
        }];

        // v0 commits first
        let p0 = db.propose(v0, batch.clone()).unwrap();
        let hash0 = p0.root_hash();
        db.commit(v0, p0).unwrap();

        // v1 commits same batch → dedup
        let p1 = db.propose(v1, batch).unwrap();
        let hash1 = p1.root_hash();
        db.commit(v1, p1).unwrap();

        // Same hashes → dedup happened
        assert_eq!(hash0, hash1);

        // Both heads point to the same revision (Arc::ptr_eq)
        let head0 = db.validator_view(v0).unwrap();
        let head1 = db.validator_view(v1).unwrap();
        assert!(StdArc::ptr_eq(&head0, &head1));

        db.close().unwrap();
    }

    #[test]
    fn test_chain_advance_moves_validator_to_target_chain() {
        // v0 commits, v1 advances to v0's hash → joins v0's chain
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        let batch = vec![BatchOp::Put {
            key: b"key".to_vec(),
            value: b"value".to_vec(),
        }];
        let p0 = db.propose(v0, batch).unwrap();
        let hash = p0.root_hash().unwrap();
        db.commit(v0, p0).unwrap();

        // v1 advances to v0's hash
        db.advance_to_hash(v1, hash).unwrap();

        // Both heads should be the same Arc
        let head0 = db.validator_view(v0).unwrap();
        let head1 = db.validator_view(v1).unwrap();
        assert!(StdArc::ptr_eq(&head0, &head1));

        db.close().unwrap();
    }

    #[test]
    fn test_chain_deregister_last_validator_cleans_up_chain() {
        // Fork, then deregister one validator → its unique chain is cleaned up
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        // v0 commits
        let batch_a = vec![BatchOp::Put {
            key: b"key_a".to_vec(),
            value: b"value_a".to_vec(),
        }];
        let p0 = db.propose(v0, batch_a).unwrap();
        db.commit(v0, p0).unwrap();

        // v1 commits different batch → fork
        let batch_b = vec![BatchOp::Put {
            key: b"key_b".to_vec(),
            value: b"value_b".to_vec(),
        }];
        let p1 = db.propose(v1, batch_b).unwrap();
        db.commit(v1, p1).unwrap();

        // Deregister v1 → its chain should be cleaned up
        db.deregister_validator(v1).unwrap();

        // v0 should still work
        let head0 = db.validator_view(v0).unwrap();
        assert_eq!(
            head0.val(b"key_a").unwrap(),
            Some(b"value_a".to_vec().into_boxed_slice())
        );

        db.close().unwrap();
    }

    #[test]
    fn test_chain_register_assigns_to_tip_chain() {
        // After commits, a new validator should join the most popular chain
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();

        // v0 commits some data
        let batch = vec![BatchOp::Put {
            key: b"key".to_vec(),
            value: b"value".to_vec(),
        }];
        let p0 = db.propose(v0, batch).unwrap();
        let hash = p0.root_hash().unwrap();
        db.commit(v0, p0).unwrap();

        // Register v1 → should start at v0's chain tip
        let v1 = ValidatorId::new(1);
        db.register_validator(v1).unwrap();

        let head1 = db.validator_view(v1).unwrap();
        assert_eq!(head1.root_hash().unwrap(), hash);

        db.close().unwrap();
    }

    #[test]
    fn test_per_chain_reaping_independent() {
        // Two chains: fast chain gets reaped without affecting slow chain
        let (db, _tmpdir) = create_multi_db_with_max_revisions(5);
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        // v0 commits to create initial state
        let batch_init = vec![BatchOp::Put {
            key: b"init".to_vec(),
            value: b"value".to_vec(),
        }];
        let p0 = db.propose(v0, batch_init).unwrap();
        db.commit(v0, p0).unwrap();

        // v1 forks with different data
        let batch_fork = vec![BatchOp::Put {
            key: b"fork".to_vec(),
            value: b"value".to_vec(),
        }];
        let p1 = db.propose(v1, batch_fork).unwrap();
        db.commit(v1, p1).unwrap();

        // Now v0 commits many revisions (exceeding max_revisions)
        for i in 0..10u32 {
            let batch = vec![BatchOp::Put {
                key: format!("key{i}").into_bytes(),
                value: format!("val{i}").into_bytes(),
            }];
            let p = db.propose(v0, batch).unwrap();
            db.commit(v0, p).unwrap();
        }

        // v0's chain should have been reaped, but v0 should still work
        let head0 = db.validator_view(v0).unwrap();
        assert_eq!(
            head0.val(b"key9").unwrap(),
            Some(b"val9".to_vec().into_boxed_slice())
        );

        // v1's chain should be unaffected
        let head1 = db.validator_view(v1).unwrap();
        assert_eq!(
            head1.val(b"fork").unwrap(),
            Some(b"value".to_vec().into_boxed_slice())
        );

        db.close().unwrap();
    }

    #[test]
    fn test_per_chain_reaping_stops_at_min_head() {
        // Single chain with two validators at different positions
        let (db, _tmpdir) = create_multi_db_with_max_revisions(5);
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        // Both commit the same initial block
        let batch = vec![BatchOp::Put {
            key: b"key0".to_vec(),
            value: b"val0".to_vec(),
        }];
        let p0 = db.propose(v0, batch.clone()).unwrap();
        let hash0 = p0.root_hash().unwrap();
        db.commit(v0, p0).unwrap();
        db.advance_to_hash(v1, hash0).unwrap();

        // v0 advances many blocks while v1 stays behind
        for i in 1..10u32 {
            let batch = vec![BatchOp::Put {
                key: format!("key{i}").into_bytes(),
                value: format!("val{i}").into_bytes(),
            }];
            let p = db.propose(v0, batch).unwrap();
            db.commit(v0, p).unwrap();
        }

        // v1 should still be able to read its head (reaping shouldn't touch it)
        let head1 = db.validator_view(v1).unwrap();
        assert_eq!(
            head1.val(b"key0").unwrap(),
            Some(b"val0".to_vec().into_boxed_slice())
        );

        db.close().unwrap();
    }

    #[test]
    fn test_advance_to_unknown_hash_returns_revision_not_found() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();

        let fake_hash = TrieHash::from([0xABu8; 32]);
        let err = db.advance_to_hash(v0, fake_hash);
        assert!(err.is_err());
        let err_msg = format!("{}", err.unwrap_err());
        assert!(
            err_msg.contains("not found"),
            "Expected RevisionNotFound, got: {err_msg}"
        );
    }

    #[test]
    fn test_hash_to_chain_cleanup_on_reap() {
        // After reaping, hash_to_chain entries for reaped revisions are cleaned up
        let (db, _tmpdir) = create_multi_db_with_max_revisions(3);
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();

        // Commit enough blocks to trigger reaping
        for i in 0..10u32 {
            let batch = vec![BatchOp::Put {
                key: format!("key{i}").into_bytes(),
                value: format!("val{i}").into_bytes(),
            }];
            let p = db.propose(v0, batch).unwrap();
            db.commit(v0, p).unwrap();
        }

        // Latest should still be readable
        let head = db.validator_view(v0).unwrap();
        assert_eq!(
            head.val(b"key9").unwrap(),
            Some(b"val9".to_vec().into_boxed_slice())
        );

        db.close().unwrap();
    }

    #[test]
    fn test_by_hash_preserved_when_validator_head_references() {
        // If a validator's head references a hash, by_hash should keep it
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        // Both commit same block
        let batch = vec![BatchOp::Put {
            key: b"key".to_vec(),
            value: b"val".to_vec(),
        }];
        let p0 = db.propose(v0, batch.clone()).unwrap();
        let hash = p0.root_hash().unwrap();
        db.commit(v0, p0).unwrap();

        let p1 = db.propose(v1, batch).unwrap();
        db.commit(v1, p1).unwrap();

        // v0 advances further
        let batch2 = vec![BatchOp::Put {
            key: b"key2".to_vec(),
            value: b"val2".to_vec(),
        }];
        let p = db.propose(v0, batch2).unwrap();
        db.commit(v0, p).unwrap();

        // v1 still at the old hash — it should still be viewable
        let view = db.view(hash);
        assert!(view.is_ok());

        db.close().unwrap();
    }

    // === Concurrency Tests ===

    #[test]
    fn test_concurrent_commits_all_succeed() {
        use std::sync::Barrier;
        use std::thread;

        let (db, _tmpdir) = create_multi_db();
        let db = StdArc::new(db);

        let n_validators = 4u64;
        for i in 0..n_validators {
            db.register_validator(ValidatorId::new(i)).unwrap();
        }

        let barrier = StdArc::new(Barrier::new(n_validators as usize));

        let mut handles = Vec::new();

        for i in 0..n_validators {
            let db = StdArc::clone(&db);
            let barrier = StdArc::clone(&barrier);
            handles.push(thread::spawn(move || {
                let vid = ValidatorId::new(i);
                let batch = vec![BatchOp::Put {
                    key: format!("key_{i}").into_bytes(),
                    value: format!("val_{i}").into_bytes(),
                }];
                let proposal = db.propose(vid, batch).unwrap();
                barrier.wait();
                db.commit(vid, proposal).unwrap();
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // All validators should have committed successfully
        for i in 0..n_validators {
            let vid = ValidatorId::new(i);
            let head = db.validator_view(vid).unwrap();
            let val = head.val(format!("key_{i}").as_bytes()).unwrap();
            assert_eq!(
                val,
                Some(format!("val_{i}").into_bytes().into_boxed_slice())
            );
        }
    }

    #[test]
    fn test_concurrent_same_batch_deduplicates() {
        use std::sync::Barrier;
        use std::thread;

        let (db, _tmpdir) = create_multi_db();
        let db = StdArc::new(db);
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        let barrier = StdArc::new(Barrier::new(2));
        let batch = vec![BatchOp::Put {
            key: b"shared_key".to_vec(),
            value: b"shared_val".to_vec(),
        }];

        // Extract nodestores before spawning threads (Proposal borrows Db)
        let ns0 = db.propose(v0, batch.clone()).unwrap().nodestore.clone();
        let ns1 = db.propose(v1, batch).unwrap().nodestore.clone();

        let db0 = StdArc::clone(&db);
        let b0 = StdArc::clone(&barrier);
        let h0 = thread::spawn(move || {
            b0.wait();
            db0.db
                .manager
                .commit_for_validator(v0, ns0, "consensus")
                .unwrap();
        });

        let db1 = StdArc::clone(&db);
        let b1 = StdArc::clone(&barrier);
        let h1 = thread::spawn(move || {
            b1.wait();
            db1.db
                .manager
                .commit_for_validator(v1, ns1, "consensus")
                .unwrap();
        });

        h0.join().unwrap();
        h1.join().unwrap();

        // Both heads should be the same Arc (dedup)
        let head0 = db.validator_view(v0).unwrap();
        let head1 = db.validator_view(v1).unwrap();
        assert!(StdArc::ptr_eq(&head0, &head1));
    }

    #[test]
    fn test_concurrent_different_batches_creates_fork() {
        use std::sync::Barrier;
        use std::thread;

        let (db, _tmpdir) = create_multi_db();
        let db = StdArc::new(db);
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        let batch_a = vec![BatchOp::Put {
            key: b"key_a".to_vec(),
            value: b"val_a".to_vec(),
        }];
        let batch_b = vec![BatchOp::Put {
            key: b"key_b".to_vec(),
            value: b"val_b".to_vec(),
        }];

        // Extract nodestores before spawning threads
        let ns0 = db.propose(v0, batch_a).unwrap().nodestore.clone();
        let ns1 = db.propose(v1, batch_b).unwrap().nodestore.clone();

        let barrier = StdArc::new(Barrier::new(2));

        let db0 = StdArc::clone(&db);
        let b0 = StdArc::clone(&barrier);
        let h0 = thread::spawn(move || {
            b0.wait();
            db0.db
                .manager
                .commit_for_validator(v0, ns0, "consensus")
                .unwrap();
        });

        let db1 = StdArc::clone(&db);
        let b1 = StdArc::clone(&barrier);
        let h1 = thread::spawn(move || {
            b1.wait();
            db1.db
                .manager
                .commit_for_validator(v1, ns1, "consensus")
                .unwrap();
        });

        h0.join().unwrap();
        h1.join().unwrap();

        // Different hashes → fork
        let head0 = db.validator_view(v0).unwrap();
        let head1 = db.validator_view(v1).unwrap();
        assert_ne!(head0.root_hash(), head1.root_hash());

        // Both should read their own data
        assert_eq!(
            head0.val(b"key_a").unwrap(),
            Some(b"val_a".to_vec().into_boxed_slice())
        );
        assert_eq!(
            head1.val(b"key_b").unwrap(),
            Some(b"val_b".to_vec().into_boxed_slice())
        );
    }

    #[test]
    fn test_read_during_commit_sees_consistent_state() {
        use std::sync::Barrier;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::thread;

        let (db, _tmpdir) = create_multi_db();
        let db = StdArc::new(db);
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        let stop = StdArc::new(AtomicBool::new(false));
        let barrier = StdArc::new(Barrier::new(2));

        // Reader thread: continuously reads v1's view
        let db_r = StdArc::clone(&db);
        let stop_r = StdArc::clone(&stop);
        let barrier_r = StdArc::clone(&barrier);
        let reader = thread::spawn(move || {
            barrier_r.wait();
            while !stop_r.load(Ordering::Relaxed) {
                let _head = db_r.validator_view(v1).unwrap();
                thread::yield_now();
            }
        });

        // Committer thread: commits for v0
        let db_w = StdArc::clone(&db);
        let stop_w = StdArc::clone(&stop);
        let barrier_w = StdArc::clone(&barrier);
        let writer = thread::spawn(move || {
            barrier_w.wait();
            for i in 0..20u32 {
                let batch = vec![BatchOp::Put {
                    key: format!("key{i}").into_bytes(),
                    value: format!("val{i}").into_bytes(),
                }];
                let p = db_w.propose(v0, batch).unwrap();
                db_w.commit(v0, p).unwrap();
            }
            stop_w.store(true, Ordering::Relaxed);
        });

        writer.join().unwrap();
        reader.join().unwrap();
    }

    #[test]
    fn test_multiple_concurrent_readers() {
        use std::sync::Barrier;
        use std::thread;

        let (db, _tmpdir) = create_multi_db();
        let db = StdArc::new(db);

        for i in 0..4u64 {
            db.register_validator(ValidatorId::new(i)).unwrap();
            let batch = vec![BatchOp::Put {
                key: format!("key_{i}").into_bytes(),
                value: format!("val_{i}").into_bytes(),
            }];
            let p = db.propose(ValidatorId::new(i), batch).unwrap();
            db.commit(ValidatorId::new(i), p).unwrap();
        }

        let barrier = StdArc::new(Barrier::new(4));
        let mut handles = Vec::new();

        for i in 0..4u64 {
            let db = StdArc::clone(&db);
            let barrier = StdArc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                for _ in 0..100 {
                    let head = db.validator_view(ValidatorId::new(i)).unwrap();
                    let val = head.val(format!("key_{i}").as_bytes()).unwrap();
                    assert!(val.is_some());
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn test_reader_holds_view_during_reap() {
        let (db, _tmpdir) = create_multi_db_with_max_revisions(3);
        let db = StdArc::new(db);
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();

        // Commit first block and grab a view
        let batch = vec![BatchOp::Put {
            key: b"early_key".to_vec(),
            value: b"early_val".to_vec(),
        }];
        let p = db.propose(v0, batch).unwrap();
        let early_hash = p.root_hash().unwrap();
        db.commit(v0, p).unwrap();

        let held_view = db.validator_view(v0).unwrap();

        // Commit more blocks to trigger reaping
        for i in 0..10u32 {
            let batch = vec![BatchOp::Put {
                key: format!("key{i}").into_bytes(),
                value: format!("val{i}").into_bytes(),
            }];
            let p = db.propose(v0, batch).unwrap();
            db.commit(v0, p).unwrap();
        }

        // The held view should still be valid (Arc keeps it alive)
        assert_eq!(held_view.root_hash().unwrap(), early_hash);
    }

    #[test]
    fn test_commit_while_advance() {
        use std::sync::Barrier;
        use std::thread;

        let (db, _tmpdir) = create_multi_db();
        let db = StdArc::new(db);
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        // v0 commits block 1
        let batch = vec![BatchOp::Put {
            key: b"key1".to_vec(),
            value: b"val1".to_vec(),
        }];
        let p = db.propose(v0, batch).unwrap();
        let hash1 = p.root_hash().unwrap();
        db.commit(v0, p).unwrap();

        // Concurrently: v0 commits block 2, v1 advances to block 1
        let barrier = StdArc::new(Barrier::new(2));

        let db0 = StdArc::clone(&db);
        let b0 = StdArc::clone(&barrier);
        let committer = thread::spawn(move || {
            let batch = vec![BatchOp::Put {
                key: b"key2".to_vec(),
                value: b"val2".to_vec(),
            }];
            let p = db0.propose(v0, batch).unwrap();
            b0.wait();
            db0.commit(v0, p).unwrap();
        });

        let db1 = StdArc::clone(&db);
        let b1 = StdArc::clone(&barrier);
        let advancer = thread::spawn(move || {
            b1.wait();
            db1.advance_to_hash(v1, hash1).unwrap();
        });

        committer.join().unwrap();
        advancer.join().unwrap();
    }

    #[test]
    fn test_two_validators_advance_same_hash() {
        use std::sync::Barrier;
        use std::thread;

        let (db, _tmpdir) = create_multi_db();
        let db = StdArc::new(db);
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        let v2 = ValidatorId::new(2);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();
        db.register_validator(v2).unwrap();

        let batch = vec![BatchOp::Put {
            key: b"key".to_vec(),
            value: b"val".to_vec(),
        }];
        let p = db.propose(v0, batch).unwrap();
        let hash = p.root_hash().unwrap();
        db.commit(v0, p).unwrap();

        let barrier = StdArc::new(Barrier::new(2));
        let hash1 = hash.clone();
        let hash2 = hash;

        let db1 = StdArc::clone(&db);
        let b1 = StdArc::clone(&barrier);
        let h1 = thread::spawn(move || {
            b1.wait();
            db1.advance_to_hash(v1, hash1).unwrap();
        });

        let db2 = StdArc::clone(&db);
        let b2 = StdArc::clone(&barrier);
        let h2 = thread::spawn(move || {
            b2.wait();
            db2.advance_to_hash(v2, hash2).unwrap();
        });

        h1.join().unwrap();
        h2.join().unwrap();

        // All three should have the same head
        let head0 = db.validator_view(v0).unwrap();
        let head1 = db.validator_view(v1).unwrap();
        let head2 = db.validator_view(v2).unwrap();
        assert!(StdArc::ptr_eq(&head0, &head1));
        assert!(StdArc::ptr_eq(&head1, &head2));
    }

    #[test]
    fn test_commit_while_propose() {
        use std::sync::Barrier;
        use std::thread;

        let (db, _tmpdir) = create_multi_db();
        let db = StdArc::new(db);
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        let barrier = StdArc::new(Barrier::new(2));

        let db0 = StdArc::clone(&db);
        let b0 = StdArc::clone(&barrier);
        let committer = thread::spawn(move || {
            let batch = vec![BatchOp::Put {
                key: b"key_commit".to_vec(),
                value: b"val_commit".to_vec(),
            }];
            let p = db0.propose(v0, batch).unwrap();
            b0.wait();
            db0.commit(v0, p).unwrap();
        });

        let db1 = StdArc::clone(&db);
        let b1 = StdArc::clone(&barrier);
        let proposer = thread::spawn(move || {
            b1.wait();
            let batch = vec![BatchOp::Put {
                key: b"key_propose".to_vec(),
                value: b"val_propose".to_vec(),
            }];
            // propose is lock-free, should succeed concurrently
            let _p = db1.propose(v1, batch).unwrap();
        });

        committer.join().unwrap();
        proposer.join().unwrap();
    }

    #[test]
    fn test_register_during_commit() {
        use std::sync::Barrier;
        use std::thread;

        let (db, _tmpdir) = create_multi_db();
        let db = StdArc::new(db);
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();

        let barrier = StdArc::new(Barrier::new(2));

        let db0 = StdArc::clone(&db);
        let b0 = StdArc::clone(&barrier);
        let committer = thread::spawn(move || {
            let batch = vec![BatchOp::Put {
                key: b"key".to_vec(),
                value: b"val".to_vec(),
            }];
            let p = db0.propose(v0, batch).unwrap();
            b0.wait();
            db0.commit(v0, p).unwrap();
        });

        let db1 = StdArc::clone(&db);
        let b1 = StdArc::clone(&barrier);
        let registerer = thread::spawn(move || {
            b1.wait();
            db1.register_validator(ValidatorId::new(1)).unwrap();
        });

        committer.join().unwrap();
        registerer.join().unwrap();
    }

    #[test]
    fn test_deregister_during_commit() {
        use std::sync::Barrier;
        use std::thread;

        let (db, _tmpdir) = create_multi_db();
        let db = StdArc::new(db);
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        let barrier = StdArc::new(Barrier::new(2));

        let db0 = StdArc::clone(&db);
        let b0 = StdArc::clone(&barrier);
        let committer = thread::spawn(move || {
            let batch = vec![BatchOp::Put {
                key: b"key".to_vec(),
                value: b"val".to_vec(),
            }];
            let p = db0.propose(v0, batch).unwrap();
            b0.wait();
            db0.commit(v0, p).unwrap();
        });

        let db1 = StdArc::clone(&db);
        let b1 = StdArc::clone(&barrier);
        let deregisterer = thread::spawn(move || {
            b1.wait();
            db1.deregister_validator(v1).unwrap();
        });

        committer.join().unwrap();
        deregisterer.join().unwrap();
    }

    #[test]
    fn test_8_validators_50_cycles_concurrent() {
        use std::sync::Barrier;
        use std::thread;

        let (db, _tmpdir) = create_multi_db_with_max_revisions(64);
        let db = StdArc::new(db);

        let n = 8u64;
        for i in 0..n {
            db.register_validator(ValidatorId::new(i)).unwrap();
        }

        let barrier = StdArc::new(Barrier::new(n as usize));
        let mut handles = Vec::new();

        for i in 0..n {
            let db = StdArc::clone(&db);
            let barrier = StdArc::clone(&barrier);
            handles.push(thread::spawn(move || {
                let vid = ValidatorId::new(i);
                barrier.wait();

                for cycle in 0..50u32 {
                    let batch = vec![BatchOp::Put {
                        key: format!("v{i}_c{cycle}").into_bytes(),
                        value: format!("val_{i}_{cycle}").into_bytes(),
                    }];
                    let p = db.propose(vid, batch).unwrap();
                    db.commit(vid, p).unwrap();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // Verify final state: each validator's head is accessible
        for i in 0..n {
            let vid = ValidatorId::new(i);
            let head = db.validator_view(vid).unwrap();
            let val = head.val(format!("v{i}_c49").as_bytes()).unwrap();
            assert_eq!(
                val,
                Some(format!("val_{i}_49").into_bytes().into_boxed_slice())
            );
        }
    }

    // === Tests for MultiDb convenience methods (get, update, dump_validator) ===

    #[test]
    fn test_multi_db_get_from_validator_head() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();

        let batch = vec![BatchOp::Put {
            key: b"key",
            value: b"value",
        }];
        let proposal = db.propose(v0, batch).unwrap();
        db.commit(v0, proposal).unwrap();

        let val = db.get(v0, b"key").unwrap();
        assert_eq!(val, Some(b"value".to_vec().into_boxed_slice()));
    }

    #[test]
    fn test_multi_db_get_unknown_validator_returns_error() {
        let (db, _tmpdir) = create_multi_db();
        let result = db.get(ValidatorId::new(99), b"key");
        assert!(result.is_err());
    }

    #[test]
    fn test_multi_db_update_propose_and_commit() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();

        let batch = vec![BatchOp::Put {
            key: b"k",
            value: b"v",
        }];
        let hash = db.update(v0, batch).unwrap();
        assert!(hash.is_some());

        let val = db.get(v0, b"k").unwrap();
        assert_eq!(val, Some(b"v".to_vec().into_boxed_slice()));
    }

    #[test]
    fn test_multi_db_dump_validator() {
        let (db, _tmpdir) = create_multi_db();
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();

        let batch = vec![BatchOp::Put {
            key: b"k",
            value: b"v",
        }];
        db.update(v0, batch).unwrap();

        let dump = db.dump_validator(v0).unwrap();
        assert!(!dump.is_empty());
    }

    #[test]
    fn test_multi_db_dump_unknown_validator_returns_error() {
        let (db, _tmpdir) = create_multi_db();
        let result = db.dump_validator(ValidatorId::new(99));
        assert!(result.is_err());
    }

    #[test]
    fn test_multi_db_update_unknown_validator_returns_error() {
        let (db, _tmpdir) = create_multi_db();
        let batch = vec![BatchOp::Put {
            key: b"k",
            value: b"v",
        }];
        let result = db.update(ValidatorId::new(99), batch);
        assert!(result.is_err());
    }

    // ---- Fork ID Integration Tests ----

    #[test]
    fn test_fork_reaping_does_not_corrupt_other_chain() {
        // Two validators diverge, one reaps, the other reads safely.
        let (db, _tmpdir) = create_multi_db_with_max_revisions(5);
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        // Both start with shared state
        let batch_shared = vec![BatchOp::Put {
            key: b"shared".to_vec(),
            value: b"value".to_vec(),
        }];
        let p0 = db.propose(v0, batch_shared.clone()).unwrap();
        let hash0 = p0.root_hash().unwrap();
        db.commit(v0, p0).unwrap();
        db.advance_to_hash(v1, hash0).unwrap();

        // V0 and V1 diverge
        let batch_a = vec![BatchOp::Put {
            key: b"key_a".to_vec(),
            value: b"val_a".to_vec(),
        }];
        let p0 = db.propose(v0, batch_a).unwrap();
        db.commit(v0, p0).unwrap();

        let batch_b = vec![BatchOp::Put {
            key: b"key_b".to_vec(),
            value: b"val_b".to_vec(),
        }];
        let p1 = db.propose(v1, batch_b).unwrap();
        db.commit(v1, p1).unwrap();

        // V0 commits many more revisions to trigger reaping
        for i in 0..15u32 {
            let batch = vec![BatchOp::Put {
                key: format!("v0key{i}").into_bytes(),
                value: format!("v0val{i}").into_bytes(),
            }];
            let p = db.propose(v0, batch).unwrap();
            db.commit(v0, p).unwrap();
        }

        // V1's chain should be unaffected by V0's reaping
        let head1 = db.validator_view(v1).unwrap();
        assert_eq!(
            head1.val(b"shared").unwrap(),
            Some(b"value".to_vec().into_boxed_slice()),
            "shared ancestor data should survive reaping"
        );
        assert_eq!(
            head1.val(b"key_b").unwrap(),
            Some(b"val_b".to_vec().into_boxed_slice()),
            "v1's own data should survive reaping"
        );

        // V0 should also work fine
        let head0 = db.validator_view(v0).unwrap();
        assert_eq!(
            head0.val(b"v0key14").unwrap(),
            Some(b"v0val14".to_vec().into_boxed_slice())
        );

        db.close().unwrap();
    }

    #[test]
    fn test_fork_reaping_frees_own_allocations() {
        // After fork, a chain's reaping frees its own nodes.
        let (db, _tmpdir) = create_multi_db_with_max_revisions(3);
        let v0 = ValidatorId::new(0);
        db.register_validator(v0).unwrap();

        // Commit enough revisions to trigger reaping
        for i in 0..10u32 {
            let batch = vec![BatchOp::Put {
                key: format!("key{i}").into_bytes(),
                value: format!("val{i}").into_bytes(),
            }];
            let p = db.propose(v0, batch).unwrap();
            db.commit(v0, p).unwrap();
        }

        // The database should still work (reaping freed old nodes successfully)
        let head = db.validator_view(v0).unwrap();
        assert_eq!(
            head.val(b"key9").unwrap(),
            Some(b"val9".to_vec().into_boxed_slice())
        );
        // Earlier keys should also be present (they were never deleted)
        assert_eq!(
            head.val(b"key0").unwrap(),
            Some(b"val0".to_vec().into_boxed_slice())
        );

        db.close().unwrap();
    }

    #[test]
    fn test_shared_ancestor_nodes_not_freed_during_fork() {
        // When two chains share ancestor nodes, reaping on either chain
        // does not corrupt the shared state.
        let (db, _tmpdir) = create_multi_db_with_max_revisions(3);
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        // Build shared base with multiple keys
        for i in 0..5u32 {
            let batch = vec![BatchOp::Put {
                key: format!("base{i}").into_bytes(),
                value: format!("bval{i}").into_bytes(),
            }];
            let p = db.propose(v0, batch).unwrap();
            let hash = p.root_hash().unwrap();
            db.commit(v0, p).unwrap();
            db.advance_to_hash(v1, hash).unwrap();
        }

        // V0 diverges — modifies base keys
        let batch_a = vec![BatchOp::Put {
            key: b"base0".to_vec(),
            value: b"modified_by_v0".to_vec(),
        }];
        let p0 = db.propose(v0, batch_a).unwrap();
        db.commit(v0, p0).unwrap();

        // V1 diverges — different modification
        let batch_b = vec![BatchOp::Put {
            key: b"base1".to_vec(),
            value: b"modified_by_v1".to_vec(),
        }];
        let p1 = db.propose(v1, batch_b).unwrap();
        db.commit(v1, p1).unwrap();

        // V0 commits more to trigger reaping
        for i in 0..10u32 {
            let batch = vec![BatchOp::Put {
                key: format!("v0extra{i}").into_bytes(),
                value: format!("v0eval{i}").into_bytes(),
            }];
            let p = db.propose(v0, batch).unwrap();
            db.commit(v0, p).unwrap();
        }

        // V1's shared ancestor data should still be intact
        let head1 = db.validator_view(v1).unwrap();
        assert_eq!(
            head1.val(b"base2").unwrap(),
            Some(b"bval2".to_vec().into_boxed_slice()),
            "shared base keys should survive v0's reaping"
        );
        assert_eq!(
            head1.val(b"base3").unwrap(),
            Some(b"bval3".to_vec().into_boxed_slice())
        );
        assert_eq!(
            head1.val(b"base1").unwrap(),
            Some(b"modified_by_v1".to_vec().into_boxed_slice()),
            "v1's modifications should survive v0's reaping"
        );

        db.close().unwrap();
    }

    #[test]
    fn test_convergence_resumes_full_reaping() {
        // After chains converge (all validators on same chain),
        // reaping works normally.
        let (db, _tmpdir) = create_multi_db_with_max_revisions(3);
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();

        // Diverge
        let p0 = db
            .propose(
                v0,
                vec![BatchOp::Put {
                    key: b"k".to_vec(),
                    value: b"v0".to_vec(),
                }],
            )
            .unwrap();
        db.commit(v0, p0).unwrap();

        let p1 = db
            .propose(
                v1,
                vec![BatchOp::Put {
                    key: b"k".to_vec(),
                    value: b"v1".to_vec(),
                }],
            )
            .unwrap();
        db.commit(v1, p1).unwrap();

        // Converge: v1 advances to v0's hash
        let hash0 = db.validator_root_hash(v0).unwrap().unwrap();
        db.advance_to_hash(v1, hash0).unwrap();

        // Now both are on the same chain. Commit more to trigger reaping.
        for i in 0..10u32 {
            let batch = vec![BatchOp::Put {
                key: format!("post{i}").into_bytes(),
                value: format!("pval{i}").into_bytes(),
            }];
            let p = db.propose(v0, batch.clone()).unwrap();
            let hash = p.root_hash().unwrap();
            db.commit(v0, p).unwrap();
            db.advance_to_hash(v1, hash).unwrap();
        }

        // Both validators should see the same data
        let head0 = db.validator_view(v0).unwrap();
        let head1 = db.validator_view(v1).unwrap();
        assert_eq!(
            head0.val(b"post9").unwrap(),
            Some(b"pval9".to_vec().into_boxed_slice())
        );
        assert_eq!(head0.root_hash(), head1.root_hash());

        db.close().unwrap();
    }

    #[test]
    fn test_fork_of_fork_reaping_safety() {
        // Three-way fork: reaping on any chain doesn't corrupt others.
        let (db, _tmpdir) = create_multi_db_with_max_revisions(5);
        let v0 = ValidatorId::new(0);
        let v1 = ValidatorId::new(1);
        let v2 = ValidatorId::new(2);
        db.register_validator(v0).unwrap();
        db.register_validator(v1).unwrap();
        db.register_validator(v2).unwrap();

        // Shared base
        let batch = vec![BatchOp::Put {
            key: b"base".to_vec(),
            value: b"shared".to_vec(),
        }];
        let p0 = db.propose(v0, batch).unwrap();
        let base_hash = p0.root_hash().unwrap();
        db.commit(v0, p0).unwrap();
        db.advance_to_hash(v1, base_hash.clone()).unwrap();
        db.advance_to_hash(v2, base_hash).unwrap();

        // Three-way divergence
        for (v, name) in [(v0, "v0"), (v1, "v1"), (v2, "v2")] {
            let batch = vec![BatchOp::Put {
                key: format!("{name}_key").into_bytes(),
                value: format!("{name}_val").into_bytes(),
            }];
            let p = db.propose(v, batch).unwrap();
            db.commit(v, p).unwrap();
        }

        // V0 commits many to trigger reaping
        for i in 0..15u32 {
            let batch = vec![BatchOp::Put {
                key: format!("v0extra{i}").into_bytes(),
                value: format!("v0eval{i}").into_bytes(),
            }];
            let p = db.propose(v0, batch).unwrap();
            db.commit(v0, p).unwrap();
        }

        // V1 and V2 should be unaffected
        let head1 = db.validator_view(v1).unwrap();
        assert_eq!(
            head1.val(b"base").unwrap(),
            Some(b"shared".to_vec().into_boxed_slice()),
            "shared base should survive three-way fork reaping"
        );
        assert_eq!(
            head1.val(b"v1_key").unwrap(),
            Some(b"v1_val".to_vec().into_boxed_slice())
        );

        let head2 = db.validator_view(v2).unwrap();
        assert_eq!(
            head2.val(b"base").unwrap(),
            Some(b"shared".to_vec().into_boxed_slice())
        );
        assert_eq!(
            head2.val(b"v2_key").unwrap(),
            Some(b"v2_val".to_vec().into_boxed_slice())
        );

        db.close().unwrap();
    }

    #[test]
    fn test_fork_tree_survives_reopen() {
        // Close and reopen the DB. Verify data persists and new commits work.
        let tmpdir = tempfile::tempdir().unwrap();
        let cfg = MultiDbConfig::builder()
            .db(DbConfig::builder()
                .manager(
                    crate::manager::RevisionManagerConfig::builder()
                        .max_revisions(10)
                        .build(),
                )
                .build())
            .build();

        let shared_hash;
        {
            let db = MultiDb::new(tmpdir.as_ref(), cfg.clone()).unwrap();
            let v0 = ValidatorId::new(0);
            db.register_validator(v0).unwrap();

            // V0 commits data that will be persisted
            let batch = vec![BatchOp::Put {
                key: b"survive".to_vec(),
                value: b"reopen".to_vec(),
            }];
            let p = db.propose(v0, batch).unwrap();
            shared_hash = p.root_hash().unwrap();
            db.commit(v0, p).unwrap();

            db.wait_persisted();
            db.close().unwrap();
        }

        // Reopen and verify data survives
        {
            let db = MultiDb::new(tmpdir.as_ref(), cfg).unwrap();
            let v0 = ValidatorId::new(0);
            db.register_validator(v0).unwrap();

            // Advance to the persisted root
            db.advance_to_hash(v0, shared_hash).unwrap();

            let head0 = db.validator_view(v0).unwrap();
            assert_eq!(
                head0.val(b"survive").unwrap(),
                Some(b"reopen".to_vec().into_boxed_slice()),
                "data from before close should survive"
            );

            // Commit new data on top
            let batch = vec![BatchOp::Put {
                key: b"after_reopen".to_vec(),
                value: b"works".to_vec(),
            }];
            let p = db.propose(v0, batch).unwrap();
            db.commit(v0, p).unwrap();

            let head0 = db.validator_view(v0).unwrap();
            assert_eq!(
                head0.val(b"survive").unwrap(),
                Some(b"reopen".to_vec().into_boxed_slice())
            );
            assert_eq!(
                head0.val(b"after_reopen").unwrap(),
                Some(b"works".to_vec().into_boxed_slice()),
                "new data after reopen should work"
            );

            db.wait_persisted();
            db.close().unwrap();
        }
    }

    #[test]
    fn test_mixed_v0_v1_nodes_read_correctly() {
        // Open as single-head (v0 format), then reopen as multi-head (v1 format).
        // Both node formats should coexist and be readable.
        let tmpdir = tempfile::tempdir().unwrap();

        // Phase 1: Write nodes in single-head mode (v0 format, fork_id=0)
        {
            use crate::v2::api::Proposal as _;
            let cfg = DbConfig::builder().build();
            let db = Db::new(tmpdir.as_ref(), cfg).unwrap();
            let batch = vec![BatchOp::Put {
                key: b"v0node".to_vec(),
                value: b"legacy".to_vec(),
            }];
            let proposal = db.propose(batch).unwrap();
            proposal.commit().unwrap();
            db.close().unwrap();
        }

        // Phase 2: Reopen as multi-head and write more nodes (v1 format with fork_id)
        {
            let cfg = MultiDbConfig::builder()
                .db(DbConfig::builder().build())
                .build();
            let db = MultiDb::new(tmpdir.as_ref(), cfg).unwrap();
            let v0 = ValidatorId::new(0);
            db.register_validator(v0).unwrap();

            // The v0-format node should be readable
            let head = db.validator_view(v0).unwrap();
            assert_eq!(
                head.val(b"v0node").unwrap(),
                Some(b"legacy".to_vec().into_boxed_slice()),
                "v0 format node should be readable in multi-head mode"
            );

            // Write a new node (v1 format since this is multi-head)
            let batch = vec![BatchOp::Put {
                key: b"v1node".to_vec(),
                value: b"new_format".to_vec(),
            }];
            let p = db.propose(v0, batch).unwrap();
            db.commit(v0, p).unwrap();

            // Both v0 and v1 nodes should be readable
            let head = db.validator_view(v0).unwrap();
            assert_eq!(
                head.val(b"v0node").unwrap(),
                Some(b"legacy".to_vec().into_boxed_slice())
            );
            assert_eq!(
                head.val(b"v1node").unwrap(),
                Some(b"new_format".to_vec().into_boxed_slice())
            );

            db.close().unwrap();
        }
    }
}
