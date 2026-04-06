// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;

use firewood::api::{self, Db as _, DbView as _};
use firewood::db::{Db, DbConfig};

use crate::DatabasePath;

#[derive(Debug, Args)]
pub struct Options {
    #[command(flatten)]
    pub database: DatabasePath,

    /// The key to get the value for
    #[arg(required = true, value_name = "KEY", help = "Key to get")]
    pub key: String,
}

pub(super) fn run(opts: &Options) -> Result<(), api::Error> {
    log::debug!("get key value pair {opts:?}");
    let cfg = DbConfig::builder()
        .create_if_missing(false)
        .truncate(false)
        .build();

    let db = Db::new(
        opts.database.dbpath.clone(),
        opts.database.node_hash_algorithm.into(),
        cfg,
    )?;

    let hash = db.root_hash();

    let Some(hash) = hash else {
        println!("Database is empty");
        return db.close();
    };

    let rev = db.revision(hash)?;

    match rev.val(opts.key.as_bytes()) {
        Ok(Some(val)) => {
            let s = String::from_utf8_lossy(val.as_ref());
            println!("{s:?}");
        }
        Ok(None) => {
            eprintln!("Key '{}' not found", opts.key);
        }
        Err(e) => return Err(e),
    }
    db.close()
}
