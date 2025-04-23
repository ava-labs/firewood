// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::merkle::Merkle;
use crate::proof::{Proof, ProofNode};
use crate::range_proof::RangeProof;
use crate::stream::MerkleKeyValueStream;
use crate::v2::api::{self, KeyType, ValueType};
pub use crate::v2::api::{Batch, BatchOp};

use crate::manager::{RevisionManager, RevisionManagerConfig};
use async_trait::async_trait;
use metrics::{counter, describe_counter};
use std::error::Error;
use std::fmt;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, RwLock};
use storage::{
    Committed, HashedNodeReader, ImmutableProposal, NodeStore, TrieHash, WritableStorage,
};
use crate::{FileBacked, MemStore};
use typed_builder::TypedBuilder;

#[derive(Debug)]
#[non_exhaustive]
/// Represents the different types of errors that can occur in the database.
pub enum DbError {
    /// I/O error occurred.
    IO(std::io::Error),
}

impl fmt::Display for DbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DbError::IO(e) => write!(f, "I/O error: {e:?}"),
        }
    }
}

impl From<std::io::Error> for DbError {
    fn from(e: std::io::Error) -> Self {
        DbError::IO(e)
    }
}

impl Error for DbError {}

type HistoricalRev<S> = NodeStore<Committed, S>;

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

impl Default for DbMetrics {
    fn default() -> Self {
        Self {
            proposals: counter!("firewood.proposals"),
        }
    }
}

/// A synchronous view of the database.
pub trait DbViewSync {
    /// find a value synchronously
    fn val_sync<K: KeyType>(&self, key: K) -> Result<Option<Box<[u8]>>, DbError>;
}

impl<S: WritableStorage> DbViewSync
    for HistoricalRev<S>
{
    fn val_sync<K: KeyType>(&self, key: K) -> Result<Option<Box<[u8]>>, DbError> {
        let merkle = Merkle::from(self);
        let value = merkle.get_value(key.as_ref())?;
        Ok(value)
    }
}

#[async_trait]
impl<S: WritableStorage> api::DbView<S>
    for HistoricalRev<S>
{
    type Stream<'a>
        = MerkleKeyValueStream<'a, Self>
    where
        Self: 'a;

    async fn root_hash(&self) -> Result<Option<api::HashKey>, api::Error> {
        HashedNodeReader::root_hash(self).map_err(api::Error::IO)
    }

    async fn val<K: api::KeyType>(&self, key: K) -> Result<Option<Box<[u8]>>, api::Error> {
        let merkle = Merkle::from(self);
        Ok(merkle.get_value(key.as_ref())?)
    }

    async fn single_key_proof<K: api::KeyType>(
        &self,
        key: K,
    ) -> Result<Proof<ProofNode>, api::Error> {
        let merkle = Merkle::from(self);
        merkle.prove(key.as_ref()).map_err(api::Error::from)
    }

    async fn range_proof<K: api::KeyType, V>(
        &self,
        _first_key: Option<K>,
        _last_key: Option<K>,
        _limit: Option<usize>,
    ) -> Result<Option<RangeProof<Box<[u8]>, Box<[u8]>, ProofNode>>, api::Error> {
        todo!()
    }

    fn iter_option<K: KeyType>(
        &self,
        _first_key: Option<K>,
    ) -> Result<Self::Stream<'_>, api::Error> {
        todo!()
    }
}

/// Database configuration.
#[derive(Clone, TypedBuilder, Debug)]
pub struct DbConfig {
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
pub struct Db<S> {
    metrics: Arc<DbMetrics>,
    // TODO: consider using https://docs.rs/lock_api/latest/lock_api/struct.RwLock.html#method.upgradable_read
    // TODO: This should probably use an async RwLock
    manager: RwLock<RevisionManager<S>>,
}

#[async_trait]
impl<S: WritableStorage> api::Db<S> for Db<S>
where
    for<'p> Proposal<'p, S>: api::Proposal<S>,
{
    type Historical = NodeStore<Committed, S>;

    type Proposal<'p>
        = Proposal<'p, S>
    where
        Self: 'p;

    async fn revision(&self, root_hash: TrieHash) -> Result<Arc<Self::Historical>, api::Error> {
        let nodestore = self
            .manager
            .read()
            .expect("poisoned lock")
            .revision(root_hash)?;
        Ok(nodestore)
    }

    async fn root_hash(&self) -> Result<Option<TrieHash>, api::Error> {
        Ok(self.manager.read().expect("poisoned lock").root_hash()?)
    }

    async fn all_hashes(&self) -> Result<Vec<TrieHash>, api::Error> {
        Ok(self.manager.read().expect("poisoned lock").all_hashes())
    }

    #[fastrace::trace(short_name = true)]
    async fn propose<'p, K: KeyType, V: ValueType>(
        &'p self,
        batch: api::Batch<K, V>,
    ) -> Result<Arc<Self::Proposal<'p>>, api::Error>
    where
        Self: 'p,
    {
        let parent = self
            .manager
            .read()
            .expect("poisoned lock")
            .current_revision();
        let proposal = NodeStore::new(parent)?;
        let mut merkle = Merkle::from(proposal);
        let span = fastrace::Span::enter_with_local_parent("merkleops");
        for op in batch {
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
        let immutable: Arc<NodeStore<Arc<ImmutableProposal>, S>> = Arc::new(nodestore.try_into()?);

        drop(span);
        self.manager
            .write()
            .expect("poisoned lock")
            .add_proposal(immutable.clone());

        self.metrics.proposals.increment(1);

        Ok(Self::Proposal {
            nodestore: immutable,
            db: self,
        }
        .into())
    }
}

impl Db<MemStore> {
    /// Create a new database instance backed by an in-memory store.
    pub async fn new_in_memory(cfg: DbConfig) -> Result<Self, api::Error> {
        let metrics = Arc::new(DbMetrics::default());
        let manager = RevisionManager::<MemStore>::new_in_memory(cfg.manager)?;
        Ok(Self {
            metrics,
            manager: manager.into(),
        })
    }
}

impl Db<FileBacked> {
    /// Create a new database instance with asynchronous I/O.
    pub async fn new<P: AsRef<Path>>(db_path: P, cfg: DbConfig) -> Result<Self, api::Error> {
        let metrics = Arc::new(DbMetrics::default());
        describe_counter!("firewood.proposals", "Number of proposals created");
        let manager = RevisionManager::<FileBacked>::new(
            db_path.as_ref().to_path_buf(),
            cfg.truncate,
            cfg.manager.clone(),
        )?;
        Ok(Self {
            metrics,
            manager: manager.into(),
        })
    }
    /// Create a new database instance with synchronous I/O.
    pub fn new_sync<P: AsRef<Path>>(db_path: P, cfg: DbConfig) -> Result<Self, api::Error> {
        let metrics = Arc::new(DbMetrics::default());
        describe_counter!("firewood.proposals", "Number of proposals created");
        let manager = RevisionManager::<FileBacked>::new(
            db_path.as_ref().to_path_buf(),
            cfg.truncate,
            cfg.manager.clone(),
        )?;
        Ok(Self {
            metrics,
            manager: manager.into(),
        })
    }
}

impl<S: WritableStorage> Db<S> {
    /// Synchronously get the root hash of the latest revision.
    pub fn root_hash_sync(&self) -> Result<Option<TrieHash>, api::Error> {
        Ok(self.manager.read().expect("poisoned lock").root_hash()?)
    }
    /// Synchronously get a revision from a root hash
    pub fn revision_sync(&self, root_hash: TrieHash) -> Result<Arc<HistoricalRev<S>>, api::Error> {
        let nodestore = self
            .manager
            .read()
            .expect("poisoned lock")
            .revision(root_hash)?;
        Ok(nodestore)
    }

    /// propose a new batch synchronously
    pub fn propose_sync<K: KeyType, V: ValueType>(
        &'_ self,
        batch: Batch<K, V>,
    ) -> Result<Arc<Proposal<'_, S>>, api::Error> {
        let parent = self
            .manager
            .read()
            .expect("poisoned lock")
            .current_revision();
        let proposal = NodeStore::new(parent)?;
        let mut merkle = Merkle::from(proposal);
        for op in batch {
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
        let nodestore = merkle.into_inner();
        let immutable: Arc<NodeStore<Arc<ImmutableProposal>, S>> = Arc::new(nodestore.try_into()?);
        self.manager
            .write()
            .expect("poisoned lock")
            .add_proposal(immutable.clone());

        self.metrics.proposals.increment(1);

        Ok(Arc::new(Proposal {
            nodestore: immutable,
            db: self,
        }))
    }

    /// Dump the Trie of the latest revision.
    pub async fn dump(&self, w: &mut dyn Write) -> Result<(), std::io::Error> {
        self.dump_sync(w)
    }

    /// Dump the Trie of the latest revision, synchronously.
    pub fn dump_sync(&self, w: &mut dyn Write) -> Result<(), std::io::Error> {
        let latest_rev_nodestore = self
            .manager
            .read()
            .expect("poisoned lock")
            .current_revision();
        let merkle = Merkle::from(latest_rev_nodestore);
        // TODO: This should be a stream
        let output = merkle.dump()?;
        write!(w, "{}", output)
    }

    /// Get a copy of the database metrics
    pub fn metrics(&self) -> Arc<DbMetrics> {
        self.metrics.clone()
    }
}

#[derive(Debug)]
/// A user-visible database proposal
pub struct Proposal<'p, S> {
    nodestore: Arc<NodeStore<Arc<ImmutableProposal>, S>>,
    db: &'p Db<S>,
}

#[async_trait]
impl<S: WritableStorage> api::DbView<S>
    for Proposal<'_, S>
{
    type Stream<'b>
        = MerkleKeyValueStream<'b, NodeStore<Arc<ImmutableProposal>, S>>
    where
        Self: 'b;

    async fn root_hash(&self) -> Result<Option<api::HashKey>, api::Error> {
        self.nodestore.root_hash().map_err(api::Error::from)
    }

    async fn val<K: KeyType>(&self, key: K) -> Result<Option<Box<[u8]>>, api::Error> {
        let merkle = Merkle::from(self.nodestore.clone());
        merkle.get_value(key.as_ref()).map_err(api::Error::from)
    }

    async fn single_key_proof<K: KeyType>(&self, key: K) -> Result<Proof<ProofNode>, api::Error> {
        let merkle = Merkle::from(self.nodestore.clone());
        merkle.prove(key.as_ref()).map_err(api::Error::from)
    }

    async fn range_proof<K: KeyType, V>(
        &self,
        _first_key: Option<K>,
        _last_key: Option<K>,
        _limit: Option<usize>,
    ) -> Result<Option<api::RangeProof<Box<[u8]>, Box<[u8]>, ProofNode>>, api::Error> {
        todo!()
    }

    fn iter_option<K: KeyType>(
        &self,
        _first_key: Option<K>,
    ) -> Result<Self::Stream<'_>, api::Error> {
        todo!()
    }
}

#[async_trait]
impl<'a, S: WritableStorage> api::Proposal<S>
    for Proposal<'a, S>
{
    type Proposal = Proposal<'a, S>;

    #[fastrace::trace(short_name = true)]
    async fn propose<K: KeyType, V: ValueType>(
        self: Arc<Self>,
        batch: api::Batch<K, V>,
    ) -> Result<Arc<Self::Proposal>, api::Error> {
        let parent = self.nodestore.clone();
        let proposal = NodeStore::new(parent)?;
        let mut merkle = Merkle::from(proposal);
        for op in batch {
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
        let nodestore = merkle.into_inner();
        let immutable: Arc<NodeStore<Arc<ImmutableProposal>, S>> = Arc::new(nodestore.try_into()?);
        self.db
            .manager
            .write()
            .expect("poisoned lock")
            .add_proposal(immutable.clone());

        Ok(Self::Proposal {
            nodestore: immutable,
            db: self.db,
        }
        .into())
    }

    async fn commit(self: Arc<Self>) -> Result<(), api::Error> {
        match Arc::into_inner(self) {
            Some(proposal) => {
                let mut manager = proposal.db.manager.write().expect("poisoned lock");
                Ok(manager.commit(proposal.nodestore)?)
            }
            None => Err(api::Error::CannotCommitClonedProposal),
        }
    }
}

impl<S: WritableStorage> Proposal<'_, S> {
    /// Commit a proposal synchronously
    pub fn commit_sync(self: Arc<Self>) -> Result<(), api::Error> {
        match Arc::into_inner(self) {
            Some(proposal) => {
                let mut manager = proposal.db.manager.write().expect("poisoned lock");
                Ok(manager.commit(proposal.nodestore)?)
            }
            None => Err(api::Error::CannotCommitClonedProposal),
        }
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod test {
    use std::ops::{Deref, DerefMut};
    use std::path::PathBuf;
    use std::sync::Arc;

    use async_trait::async_trait;
    use storage::{FileBacked, WritableStorage};

    use crate::db::Db;
    use crate::v2::api::{self, Db as _, DbView as _, Error, KeyType, Proposal as _, ValueType};
    use storage::TrieHash;

    use super::{BatchOp, DbConfig, HistoricalRev};

    #[async_trait]
    impl api::Db<FileBacked> for TestDb<FileBacked> {
        type Historical = HistoricalRev<FileBacked>;
        type Proposal<'p> = super::Proposal<'p, FileBacked>;

        async fn revision(&self, hash: TrieHash) -> Result<Arc<Self::Historical>, api::Error> {
            self.db.revision(hash).await
        }

        async fn root_hash(&self) -> Result<Option<TrieHash>, api::Error> {
            self.db.root_hash().await
        }

        async fn all_hashes(&self) -> Result<Vec<TrieHash>, api::Error> {
            self.db.all_hashes().await
        }

        async fn propose<'p, K: KeyType, V: ValueType>(
            &'p self,
            data: api::Batch<K, V>,
        ) -> Result<Arc<Self::Proposal<'p>>, api::Error> {
            self.db.propose(data).await
        }
    }

    #[tokio::test]
    async fn test_cloned_proposal_error() {
        let db = testdb().await;
        let proposal = db
            .propose::<Vec<u8>, Vec<u8>>(Default::default())
            .await
            .unwrap();
        let cloned = proposal.clone();

        // attempt to commit the clone; this should fail
        let result = cloned.commit().await;
        assert!(
            matches!(result, Err(Error::CannotCommitClonedProposal)),
            "{result:?}"
        );

        // the prior attempt consumed the Arc though, so cloned is no longer valid
        // that means the actual proposal can be committed
        let result = proposal.commit().await;
        assert!(matches!(result, Ok(())), "{result:?}");
    }

    #[tokio::test]
    async fn test_proposal_reads() {
        let db = testdb().await;
        let batch = vec![BatchOp::Put {
            key: b"k",
            value: b"v",
        }];
        let proposal = db.propose(batch).await.unwrap();
        assert_eq!(&*proposal.val(b"k").await.unwrap().unwrap(), b"v");

        assert_eq!(proposal.val(b"notfound").await.unwrap(), None);
        proposal.commit().await.unwrap();

        let batch = vec![BatchOp::Put {
            key: b"k",
            value: b"v2",
        }];
        let proposal = db.propose(batch).await.unwrap();
        assert_eq!(&*proposal.val(b"k").await.unwrap().unwrap(), b"v2");

        let committed = db.root_hash().await.unwrap().unwrap();
        let historical = db.revision(committed).await.unwrap();
        assert_eq!(&*historical.val(b"k").await.unwrap().unwrap(), b"v");
    }

    #[tokio::test]
    async fn reopen_test() {
        let db = testdb().await;
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
        let proposal = db.propose(batch).await.unwrap();
        proposal.commit().await.unwrap();
        println!("{:?}", db.root_hash().await.unwrap().unwrap());

        let db = db.reopen().await;
        println!("{:?}", db.root_hash().await.unwrap().unwrap());
        let committed = db.root_hash().await.unwrap().unwrap();
        let historical = db.revision(committed).await.unwrap();
        assert_eq!(&*historical.val(b"a").await.unwrap().unwrap(), b"1");
    }

    // Testdb is a helper struct for testing the Db. Once it's dropped, the directory and file disappear
    struct TestDb<S: WritableStorage> {
        db: Db<S>,
        tmpdir: tempfile::TempDir,
    }
    impl<S: WritableStorage> Deref for TestDb<S> {
        type Target = Db<S>;
        fn deref(&self) -> &Self::Target {
            &self.db
        }
    }
    impl<S: WritableStorage> DerefMut for TestDb<S> {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.db
        }
    }

    async fn testdb() -> TestDb<FileBacked> {
        let tmpdir = tempfile::tempdir().unwrap();
        let dbpath: PathBuf = [tmpdir.path().to_path_buf(), PathBuf::from("testdb")]
            .iter()
            .collect();
        let dbconfig = DbConfig::builder().truncate(true).build();
        let db = Db::new(dbpath, dbconfig).await.unwrap();
        TestDb { db, tmpdir }
    }

    impl TestDb<FileBacked> {
        fn path(&self) -> PathBuf {
            [self.tmpdir.path().to_path_buf(), PathBuf::from("testdb")]
                .iter()
                .collect()
        }
        async fn reopen(self) -> Self {
            let path = self.path();
            drop(self.db);
            let dbconfig = DbConfig::builder().truncate(false).build();

            let db = Db::new(path, dbconfig).await.unwrap();
            TestDb {
                db,
                tmpdir: self.tmpdir,
            }
        }
    }
}
