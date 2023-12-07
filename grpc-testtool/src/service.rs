// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood::v2::{
    api::{Db, Error},
    emptydb::EmptyDb,
};
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
pub mod process;

trait IntoStatusResultExt<T> {
    fn into_status_result(self) -> Result<T, Status>;
}

impl<T> IntoStatusResultExt<T> for Result<T, Error> {
    fn into_status_result(self) -> Result<T, Status> {
        self.map_err(|err| match err {
            Error::HashNotFound { provided: _ } => todo!(),
            Error::IncorrectRootHash {
                provided: _,
                current: _,
            } => todo!(),
            Error::IO(_) => todo!(),
            Error::InvalidProposal => todo!(),
            _ => todo!(),
        })
    }
}
pub struct Database {
    db: EmptyDb,
    iterators: Arc<Mutex<Iterators>>,
}

impl Default for Database {
    fn default() -> Self {
        Self {
            db: EmptyDb,
            iterators: Default::default(),
        }
    }
}

impl Deref for Database {
    type Target = EmptyDb;

    fn deref(&self) -> &Self::Target {
        &self.db
    }
}

impl Database {
    async fn latest(&self) -> Result<Arc<<EmptyDb as firewood::v2::api::Db>::Historical>, Error> {
        let root_hash = self.root_hash().await?;
        self.revision(root_hash).await
    }
}

// TODO: implement Iterator
struct Iter;

#[derive(Default)]
struct Iterators {
    map: HashMap<u64, Iter>,
    next_id: AtomicU64,
}

impl Iterators {
    fn insert(&mut self, iter: Iter) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.map.insert(id, iter);
        id
    }

    fn _get(&self, id: u64) -> Option<&Iter> {
        self.map.get(&id)
    }

    fn remove(&mut self, id: u64) {
        self.map.remove(&id);
    }
}

#[derive(Debug)]
pub struct ProcessServer;
