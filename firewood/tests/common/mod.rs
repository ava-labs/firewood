// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::{env::temp_dir, fs::remove_dir_all, ops::Deref, path::PathBuf};

use firewood::db::{Db, DbConfig};
use typed_builder::TypedBuilder;

#[derive(Clone, Debug, TypedBuilder)]
pub struct TestDbCreator {
    #[builder(setter(into))]
    test_name: String,
    #[builder(default, setter(into))]
    path: Option<PathBuf>,
    #[builder(default = DbConfig::builder().truncate(true).build())]
    cfg: DbConfig,
}

pub struct TestDb {
    creator: TestDbCreator,
    preserve_on_drop: bool,
    db: Db,
}

impl TestDbCreator {
    pub async fn create(self) -> TestDb {
        let path = self.path.clone().unwrap_or_else(|| {
            let mut path: PathBuf = std::env::var_os("CARGO_TARGET_DIR")
                .unwrap_or(temp_dir().into())
                .into();
            if path.join("tmp").is_dir() {
                path.push("tmp");
            }
            path.join(&self.test_name)
        });
        let mut creator = self.clone();
        creator.path = path.clone().into();
        let db = Db::new(&path, &self.cfg).await.unwrap();
        TestDb {
            creator,
            db,
            preserve_on_drop: false,
        }
    }
}

impl Deref for TestDb {
    type Target = Db;

    fn deref(&self) -> &Self::Target {
        &self.db
    }
}

impl TestDb {
    /// reopen the database, consuming the old TestDb and giving you a new one
    pub async fn reopen(mut self) -> Self {
        let mut creator = self.creator.clone();
        self.preserve_on_drop = true;
        drop(self);
        creator.cfg.truncate = false;
        creator.create().await
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        if !self.preserve_on_drop {
            remove_dir_all(self.creator.path.as_ref().unwrap()).unwrap();
        }
    }
}
