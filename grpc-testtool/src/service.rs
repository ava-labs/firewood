// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood::db::{Db, DbConfig};
use firewood::merkle::MerkleKeyValueStream;
use firewood::storage::WalConfig;
use firewood::v2::{api::Db as _, api::Error};

use std::fmt::Debug;
use std::path::Path;
use std::pin::Pin;
use std::sync::atomic::AtomicU32;
use std::{
    collections::HashMap,
    ops::Deref,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::sync::Mutex;
use tonic::Status;

pub mod database;
pub mod db;
pub mod merkle;
pub mod process;

trait IntoStatusResultExt<T> {
    fn into_status_result(self) -> Result<T, Status>;
}

impl<T> IntoStatusResultExt<T> for Result<T, Error> {
    // We map errors from bad arguments into Status::invalid_argument; all other errors are Status::internal errors
    fn into_status_result(self) -> Result<T, Status> {
        self.map_err(|err| match err {
            Error::IncorrectRootHash { .. } | Error::HashNotFound { .. } | Error::RangeTooSmall => {
                Status::invalid_argument(err.to_string())
            }
            Error::IO { .. } | Error::InternalError { .. } | Error::InvalidProposal => {
                Status::internal(err.to_string())
            }
            _ => Status::internal(err.to_string()),
        })
    }
}

#[derive(Debug)]
pub struct Database {
    db: Db,
    iterators: Arc<Mutex<Iterators>>,
    views: Arc<Mutex<Views>>,
}
#[derive(Default, Debug)]
struct Views {
    map: HashMap<u32, View>,
    next_id: AtomicU32,
}

impl Views {
    fn insert(&mut self, view: View) -> u32 {
        let next_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.map.insert(next_id, view);
        next_id
    }

    fn delete(&mut self, view_id: u32) -> Option<View> {
        self.map.remove(&view_id)
    }

    fn get(&self, view_id: u32) -> Option<&View> {
        self.map.get(&view_id)
    }
}

use futures::Stream;

enum View {
    Historical(Arc<<firewood::db::Db as firewood::v2::api::Db>::Historical>),
    Proposal(Arc<<firewood::db::Db as firewood::v2::api::Db>::Proposal>),
}

// TODO: We manually implement Debug since Proposal does not, but probably should
impl Debug for View {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Historical(arg0) => f.debug_tuple("Historical").field(arg0).finish(),
            Self::Proposal(_arg0) => f.debug_tuple("Proposal").finish(),
        }
    }
}

impl Database {
    pub async fn new<P: AsRef<Path>>(path: P, history_length: u32) -> Result<Self, Error> {
        // try to create the parents for this directory, but it's okay if it fails; it will get caught in Db::new
        std::fs::create_dir_all(&path).ok();
        // TODO: truncate should be false
        // see https://github.com/ava-labs/firewood/issues/418
        let cfg = DbConfig::builder()
            .wal(WalConfig::builder().max_revisions(history_length).build())
            .truncate(true)
            .build();

        let db = Db::new(path, &cfg).await?;

        Ok(Self {
            db,
            iterators: Default::default(),
            views: Default::default(),
        })
    }
}

impl Deref for Database {
    type Target = Db;

    fn deref(&self) -> &Self::Target {
        &self.db
    }
}

impl Database {
    async fn latest(&self) -> Result<Arc<<Db as firewood::v2::api::Db>::Historical>, Error> {
        let root_hash = self.root_hash().await?;
        self.revision(root_hash).await
    }
}

trait DebugStream: Stream + Debug + Send {}

impl<S: Send + Sync + Debug + firewood::shale::ShaleStore<firewood::merkle::Node>, T: Send + Sync + Debug> DebugStream for MerkleKeyValueStream<'_, S, T> {}
type Iter = dyn DebugStream<Item = Result<(Box<[u8]>, Vec<u8>), firewood::v2::api::Error>>;

type IteratorID = u64;

#[derive(Default, Debug)]
struct Iterators {
    map: HashMap<IteratorID, Pin<Box<Iter>>>,
    next_id: AtomicU64,
}

impl Iterators {
    fn insert(&mut self, iter: Pin<Box<Iter>>) -> IteratorID {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.map.insert(id, iter);
        id
    }

    fn get(&self, id: IteratorID) -> Option<&Pin<Box<Iter>>> {
        self.map.get(&id)
    }

    fn get_mut(&mut self, id: IteratorID) -> Option<&mut Pin<Box<Iter>>> {
        self.map.get_mut(&id)
    }

    fn remove(&mut self, id: IteratorID) {
        self.map.remove(&id);
    }
}

#[derive(Debug)]
pub struct ProcessServer;
