// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![expect(
    clippy::missing_errors_doc,
    reason = "Found 12 occurrences after enabling the lint."
)]

use crate::iter::MerkleKeyValueIter;
use crate::merkle::{Merkle, Value};
use crate::root_store::{NoOpStore, RootStore};
pub use crate::v2::api::BatchOp;
use crate::v2::api::{
    self, ArcDynDbView, FrozenProof, FrozenRangeProof, HashKey, KeyType, KeyValuePair,
    KeyValuePairIter, OptionalHashKeyExt,
};

use crate::manager::{ConfigManager, RevisionManager, RevisionManagerConfig};
use firewood_storage::{
    CheckOpt, CheckerReport, Committed, FileBacked, FileIoError, HashedNodeReader,
    ImmutableProposal, NodeStore, Parentable, ReadableStorage, TrieReader,
};
use metrics::{counter, describe_counter};
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

    fn root_hash(&self) -> Result<Option<HashKey>, api::Error> {
        Ok(HashedNodeReader::root_hash(self).or_default_root_hash())
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
}

/// Database configuration.
#[derive(Clone, TypedBuilder, Debug)]
#[non_exhaustive]
pub struct DbConfig {
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
}

#[derive(Debug)]
/// A database instance.
pub struct Db<RS = NoOpStore> {
    metrics: Arc<DbMetrics>,
    manager: RevisionManager<RS>,
}

impl<RS: RootStore> api::Db for Db<RS> {
    type Historical = NodeStore<Committed, FileBacked>;

    type Proposal<'db>
        = Proposal<'db, RS>
    where
        Self: 'db;

    fn revision(&self, root_hash: HashKey) -> Result<Arc<Self::Historical>, api::Error> {
        let nodestore = self.manager.revision(root_hash)?;
        Ok(nodestore)
    }

    fn root_hash(&self) -> Result<Option<HashKey>, api::Error> {
        Ok(self.manager.root_hash()?.or_default_root_hash())
    }

    fn all_hashes(&self) -> Result<Vec<HashKey>, api::Error> {
        Ok(self.manager.all_hashes())
    }

    #[fastrace::trace(short_name = true)]
    fn propose(
        &self,
        batch: impl IntoIterator<IntoIter: KeyValuePairIter>,
    ) -> Result<Self::Proposal<'_>, api::Error> {
        let parent = self.manager.current_revision();
        let proposal = NodeStore::new(&parent)?;
        let mut merkle = Merkle::from(proposal);
        let span = fastrace::Span::enter_with_local_parent("merkleops");
        for op in batch.into_iter().map_into_batch() {
            match op {
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
        let span = fastrace::Span::enter_with_local_parent("freeze");

        let nodestore = merkle.into_inner();
        let immutable: Arc<NodeStore<Arc<ImmutableProposal>, FileBacked>> =
            Arc::new(nodestore.try_into()?);

        drop(span);
        self.manager.add_proposal(immutable.clone());

        self.metrics.proposals.increment(1);

        Ok(Self::Proposal {
            nodestore: immutable,
            db: self,
        })
    }
}

impl Db<NoOpStore> {
    /// Create a new database instance.
    pub fn new<P: AsRef<Path>>(db_path: P, cfg: DbConfig) -> Result<Self, api::Error> {
        Self::with_root_store(db_path, cfg, NoOpStore {})
    }
}

impl<RS: RootStore> Db<RS> {
    fn with_root_store<P: AsRef<Path>>(
        db_path: P,
        cfg: DbConfig,
        root_store: RS,
    ) -> Result<Self, api::Error> {
        let metrics = Arc::new(DbMetrics {
            proposals: counter!("firewood.proposals"),
        });
        describe_counter!("firewood.proposals", "Number of proposals created");
        let config_manager = ConfigManager::builder()
            .create(cfg.create_if_missing)
            .truncate(cfg.truncate)
            .manager(cfg.manager)
            .build();

        let manager =
            RevisionManager::new(db_path.as_ref().to_path_buf(), config_manager, root_store)?;
        let db = Self { metrics, manager };
        Ok(db)
    }

    /// Synchronously get a view, either committed or proposed
    pub fn view(&self, root_hash: HashKey) -> Result<ArcDynDbView, api::Error> {
        self.manager.view(root_hash).map_err(Into::into)
    }
}

impl<RS> Db<RS> {
    /// Dump the Trie of the latest revision.
    pub fn dump(&self, w: &mut dyn Write) -> Result<(), std::io::Error> {
        let latest_rev_nodestore = self.manager.current_revision();
        let merkle = Merkle::from(latest_rev_nodestore);
        merkle.dump(w).map_err(std::io::Error::other)
    }

    /// Get a copy of the database metrics
    pub fn metrics(&self) -> Arc<DbMetrics> {
        self.metrics.clone()
    }

    /// Check the database for consistency
    pub fn check(&self, opt: CheckOpt) -> CheckerReport {
        let latest_rev_nodestore = self.manager.current_revision();
        latest_rev_nodestore.check(opt)
    }

    /// Propose a new batch that is processed in parallel.
    ///
    /// # Panics
    ///
    /// Panics if the revision manager cannot create a thread pool.
    pub fn propose_parallel(
        &self,
        batch: impl IntoIterator<IntoIter: KeyValuePairIter>,
    ) -> Result<Proposal<'_, RS>, api::Error> {
        let parent = self.manager.current_revision();
        let mut parallel_merkle = ParallelMerkle::default();
        let immutable =
            parallel_merkle.create_proposal(&parent, batch, self.manager.threadpool())?;
        self.manager.add_proposal(immutable.clone());
        Ok(Proposal {
            nodestore: immutable,
            db: self,
        })
    }
}

#[derive(Debug)]
/// A user-visible database proposal
pub struct Proposal<'db, RS> {
    nodestore: Arc<NodeStore<Arc<ImmutableProposal>, FileBacked>>,
    db: &'db Db<RS>,
}

impl<RS> api::DbView for Proposal<'_, RS> {
    type Iter<'view>
        = MerkleKeyValueIter<'view, NodeStore<Arc<ImmutableProposal>, FileBacked>>
    where
        Self: 'view;

    fn root_hash(&self) -> Result<Option<api::HashKey>, api::Error> {
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
}

impl<'db, RS: RootStore> api::Proposal for Proposal<'db, RS> {
    type Proposal = Proposal<'db, RS>;

    #[fastrace::trace(short_name = true)]
    fn propose(
        &self,
        batch: impl IntoIterator<IntoIter: KeyValuePairIter>,
    ) -> Result<Self::Proposal, api::Error> {
        self.create_proposal(batch)
    }

    fn commit(self) -> Result<(), api::Error> {
        Ok(self.db.manager.commit(self.nodestore)?)
    }
}

impl<RS> Proposal<'_, RS> {
    #[crate::metrics("firewood.proposal.create", "database proposal creation")]
    fn create_proposal(
        &self,
        batch: impl IntoIterator<IntoIter: KeyValuePairIter>,
    ) -> Result<Self, api::Error> {
        let parent = self.nodestore.clone();
        let proposal = NodeStore::new(&parent)?;
        let mut merkle = Merkle::from(proposal);
        for op in batch {
            match op.into_batch() {
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
        let nodestore = merkle.into_inner();
        let immutable: Arc<NodeStore<Arc<ImmutableProposal>, FileBacked>> =
            Arc::new(nodestore.try_into()?);
        self.db.manager.add_proposal(immutable.clone());

        Ok(Self {
            nodestore: immutable,
            db: self.db,
        })
    }
}

#[cfg(test)]
mod test {
    #![expect(clippy::unwrap_used)]

    use core::iter::Take;
    use std::iter::Peekable;
    use std::num::NonZeroUsize;
    use std::ops::{Deref, DerefMut};
    use std::path::PathBuf;

    use firewood_storage::{
        CheckOpt, CheckerError, HashedNodeReader, IntoHashType, NodeStore, TrieHash,
    };

    use crate::db::{Db, Proposal};
    use crate::root_store::{MockStore, NoOpStore, RootStore};
    use crate::v2::api::{Db as _, DbView, KeyValuePairIter, Proposal as _};

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

        let committed = db.root_hash().unwrap().unwrap();
        let historical = db.revision(committed).unwrap();
        assert_eq!(&*historical.val(b"k").unwrap().unwrap(), b"v");
    }

    #[test]
    fn reopen_test() {
        let db = TestDb::new();
        let initial_root = db.root_hash().unwrap();
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
        println!("{:?}", db.root_hash().unwrap().unwrap());

        let db = db.reopen();
        println!("{:?}", db.root_hash().unwrap().unwrap());
        let committed = db.root_hash().unwrap().unwrap();
        let historical = db.revision(committed).unwrap();
        assert_eq!(&*historical.val(b"a").unwrap().unwrap(), b"1");
        drop(historical);

        let db = db.replace();
        println!("{:?}", db.root_hash().unwrap());
        assert!(db.root_hash().unwrap() == initial_root);
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
        let p2hash = proposal2.root_hash().unwrap().unwrap();
        assert!(db.all_hashes().unwrap().contains(&p2hash));
        drop(proposal2);

        // commit the first proposal
        proposal1.commit().unwrap();
        // Ensure we committed the first proposal's data
        let committed = db.root_hash().unwrap().unwrap();
        let historical = db.revision(committed).unwrap();
        assert_eq!(&*historical.val(b"k1").unwrap().unwrap(), b"v1");

        // the second proposal shouldn't be available to commit anymore
        assert!(!db.all_hashes().unwrap().contains(&p2hash));

        // the third proposal should still be contained within the all_hashes list
        // would be deleted if another proposal was committed and proposal3 was dropped here
        let hash3 = proposal3.root_hash().unwrap().unwrap();
        assert!(db.manager.all_hashes().contains(&hash3));
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
        let p2hash = proposal2.root_hash().unwrap().unwrap();
        assert!(db.all_hashes().unwrap().contains(&p2hash));
        drop(proposal2);

        // commit the first proposal
        proposal1.commit().unwrap();
        // Ensure we committed the first proposal's data
        let committed = db.root_hash().unwrap().unwrap();
        let historical = db.revision(committed).unwrap();
        assert_eq!(&*historical.val(b"k1").unwrap().unwrap(), b"v1");

        // the second proposal shouldn't be available to commit anymore
        assert!(!db.all_hashes().unwrap().contains(&p2hash));

        // the third proposal should still be contained within the all_hashes list
        let hash3 = proposal3.root_hash().unwrap().unwrap();
        assert!(db.manager.all_hashes().contains(&hash3));

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
        let historical_hash = proposal.root_hash().unwrap().unwrap();
        proposal.commit().unwrap();

        // Create a new proposal (uncommitted)
        let batch = vec![BatchOp::Put {
            key: b"proposal_key",
            value: b"proposal_value",
        }];
        let proposal = db.propose(batch).unwrap();
        let proposal_hash = proposal.root_hash().unwrap().unwrap();

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
            let kviter = keys.iter().zip(vals.iter()).map_into_batch();
            let proposal = db.propose_parallel(kviter).unwrap();
            proposal.commit().unwrap();
        }

        // Create, insert, close, open, insert
        let db = TestDb::new();
        insert_commit(&db, 1);
        let db = db.reopen();
        insert_commit(&db, 2);
        // Check that the keys are still there after the commits
        let committed = db.revision(db.root_hash().unwrap().unwrap()).unwrap();
        let keys: Vec<[u8; 1]> = vec![[1; 1], [2; 1]];
        let vals: Vec<Box<[u8]>> = vec![Box::new([1; 1]), Box::new([2; 1])];
        let kviter = keys.iter().zip(vals.iter());
        for (k, v) in kviter {
            assert_eq!(&committed.val(k).unwrap().unwrap(), v);
        }
        drop(db);

        // Open-db1, insert, open-db2, insert
        let db1 = TestDb::new();
        insert_commit(&db1, 1);
        let db2 = TestDb::new();
        insert_commit(&db2, 2);
        let committed1 = db1.revision(db1.root_hash().unwrap().unwrap()).unwrap();
        let committed2 = db2.revision(db2.root_hash().unwrap().unwrap()).unwrap();
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
        let db = TestDb::new();

        // Test an empty proposal
        let keys: Vec<[u8; 0]> = Vec::new();
        let vals: Vec<Box<[u8]>> = Vec::new();

        let kviter = keys.iter().zip(vals.iter()).map_into_batch();
        let proposal = db.propose_parallel(kviter).unwrap();
        proposal.commit().unwrap();

        // Create a proposal consisting of a single entry and an empty key.
        let keys: Vec<[u8; 0]> = vec![[0; 0]];

        // Note that if the value is [], then it is interpreted as a DeleteRange.
        // Instead, set value to [0]
        let vals: Vec<Box<[u8]>> = vec![Box::new([0; 1])];

        let kviter = keys.iter().zip(vals.iter()).map_into_batch();
        let proposal = db.propose_parallel(kviter).unwrap();

        let kviter = keys.iter().zip(vals.iter());
        for (k, v) in kviter {
            assert_eq!(&proposal.val(k).unwrap().unwrap(), v);
        }
        proposal.commit().unwrap();

        // Check that the key is still there after the commit
        let committed = db.revision(db.root_hash().unwrap().unwrap()).unwrap();
        let kviter = keys.iter().zip(vals.iter());
        for (k, v) in kviter {
            assert_eq!(&committed.val(k).unwrap().unwrap(), v);
        }

        // Create a proposal that deletes the previous entry
        let vals: Vec<Box<[u8]>> = vec![Box::new([0; 0])];
        let kviter = keys.iter().zip(vals.iter()).map_into_batch();
        let proposal = db.propose_parallel(kviter).unwrap();

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

        let kviter = keys.iter().zip(vals.iter()).map_into_batch();
        let proposal = db.propose_parallel(kviter).unwrap();
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

        let kviter = keys.iter().zip(vals.iter()).map_into_batch();
        let proposal = db.propose_parallel(kviter).unwrap();
        let kviter = keys.iter().zip(vals.iter());
        for (k, _v) in kviter {
            assert_eq!(proposal.val(k).unwrap(), None);
        }
        proposal.commit().unwrap();

        // Create a proposal that deletes using empty prefix
        let keys: Vec<[u8; 0]> = vec![[0; 0]];
        let vals: Vec<Box<[u8]>> = vec![Box::new([0; 0])];
        let kviter = keys.iter().zip(vals.iter()).map_into_batch();
        let proposal = db.propose_parallel(kviter).unwrap();
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
            let kviter = keys.iter().zip(vals.iter()).map_into_batch();
            let proposal = db.propose_parallel(kviter).unwrap();

            // iterate over the keys and values again, checking that the values are in the correct proposal
            let kviter = keys.iter().zip(vals.iter());

            for (k, v) in kviter {
                assert_eq!(&proposal.val(k).unwrap().unwrap(), v);
            }
            proposal.commit().unwrap();
        }
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
        let mut kviter = keys.iter().zip(vals.iter()).map_into_batch();

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
        let committed = db.root_hash().unwrap().unwrap();
        let revision = db.revision(committed).unwrap();

        for (k, v) in keys.into_iter().zip(vals.into_iter()) {
            assert_eq!(revision.val(k).unwrap().unwrap(), v);
        }
    }

    #[test]
    fn fuzz_checker() {
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
            Vec::<Proposal<'_, _>>::with_capacity(NUM_PROPOSALS),
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

        let last_proposal_root_hash = proposals.last().unwrap().root_hash().unwrap().unwrap();

        // commit the proposals
        for proposal in proposals {
            proposal.commit().unwrap();
        }

        // get the last committed revision
        let last_root_hash = db.root_hash().unwrap().unwrap();
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
    fn test_read_during_commit() {
        use crate::db::Proposal;

        const CHANNEL_CAPACITY: usize = 8;

        let testdb = TestDb::new();
        let db = &testdb.db;

        let (tx, rx) = std::sync::mpsc::sync_channel::<Proposal<'_, _>>(CHANNEL_CAPACITY);
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
                for id in 0u32..5000 {
                    // insert a key of length 32 and a value of length 8,
                    // rotating between all zeroes through all 255
                    let batch = vec![BatchOp::Put {
                        key: [id as u8; 32],
                        value: [id as u8; 8],
                    }];
                    let proposal = db.propose(batch).unwrap();
                    let last_hash = proposal.root_hash().unwrap().unwrap();
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
        let root_hash = proposal.root_hash().unwrap().unwrap();
        proposal.commit().unwrap();

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

        let latest_root_hash = db.root_hash().unwrap().unwrap();
        let latest_revision = db.revision(latest_root_hash).unwrap();

        let latest_value = latest_revision.val(key).unwrap().unwrap();
        assert_eq!(new_value, latest_value.as_ref());

        let node_store =
            NodeStore::with_root(root_hash.into_hash_type(), root_address, latest_revision);

        let retrieved_value = node_store.val(key).unwrap().unwrap();
        assert_eq!(value, retrieved_value.as_ref());
    }

    #[test]
    fn test_root_store() {
        let mock_store = MockStore::default();
        let db = TestDb::with_mockstore(mock_store);

        // First, create a revision to retrieve
        let key = b"key";
        let value = b"value";
        let batch = vec![BatchOp::Put { key, value }];

        let proposal = db.propose(batch).unwrap();
        let root_hash = proposal.root_hash().unwrap().unwrap();
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
    fn test_root_store_errs() {
        let mock_store = MockStore::with_failures();
        let db = TestDb::with_mockstore(mock_store);

        let view = db.view(TrieHash::empty());
        assert!(view.is_err());

        let batch = vec![BatchOp::Put {
            key: b"k",
            value: b"v",
        }];

        let proposal = db.propose(batch).unwrap();
        assert!(proposal.commit().is_err());
    }

    #[test]
    fn test_rootstore_empty_db_reopen() {
        let mock_store = MockStore::default();
        let db = TestDb::with_mockstore(mock_store);

        db.reopen();
    }

    // Testdb is a helper struct for testing the Db. Once it's dropped, the directory and file disappear
    struct TestDb<RS = NoOpStore> {
        db: Db<RS>,
        tmpdir: tempfile::TempDir,
    }
    impl<RS> Deref for TestDb<RS> {
        type Target = Db<RS>;
        fn deref(&self) -> &Self::Target {
            &self.db
        }
    }
    impl<RS> DerefMut for TestDb<RS> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.db
        }
    }

    impl TestDb {
        fn new() -> Self {
            let tmpdir = tempfile::tempdir().unwrap();
            let dbpath: PathBuf = [tmpdir.path().to_path_buf(), PathBuf::from("testdb")]
                .iter()
                .collect();
            let dbconfig = DbConfig::builder().build();
            let db = Db::new(dbpath, dbconfig).unwrap();
            TestDb { db, tmpdir }
        }
    }

    impl TestDb<MockStore> {
        fn with_mockstore(mock_store: MockStore) -> Self {
            let tmpdir = tempfile::tempdir().unwrap();
            let dbpath: PathBuf = [tmpdir.path().to_path_buf(), PathBuf::from("testdb")]
                .iter()
                .collect();
            let dbconfig = DbConfig::builder().build();
            let db = Db::with_root_store(dbpath, dbconfig, mock_store).unwrap();
            TestDb { db, tmpdir }
        }
    }

    impl<RS: Clone + RootStore> TestDb<RS> {
        fn reopen(self) -> Self {
            let path = self.path();
            let root_store = self.manager.root_store();
            drop(self.db);
            let dbconfig = DbConfig::builder().truncate(false).build();

            let db = Db::with_root_store(path, dbconfig, root_store).unwrap();
            TestDb {
                db,
                tmpdir: self.tmpdir,
            }
        }
        fn replace(self) -> Self {
            let path = self.path();
            let root_store = self.manager.root_store();
            drop(self.db);
            let dbconfig = DbConfig::builder().truncate(true).build();

            let db = Db::with_root_store(path, dbconfig, root_store).unwrap();
            TestDb {
                db,
                tmpdir: self.tmpdir,
            }
        }
    }

    impl<RS> TestDb<RS> {
        fn path(&self) -> PathBuf {
            [self.tmpdir.path().to_path_buf(), PathBuf::from("testdb")]
                .iter()
                .collect()
        }
    }
}
