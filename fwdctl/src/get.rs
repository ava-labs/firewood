// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;

use firewood::api::{self, Db as _, DbView as _};
use firewood::db::{Db, DbConfig};

use crate::{DatabasePath, key};

#[derive(Debug, Args)]
pub struct Options {
    #[command(flatten)]
    pub database: DatabasePath,

    #[command(flatten)]
    pub key: key::Options,

    #[arg(value_name = "KEY", num_args = 0..=1)]
    pub args: Vec<String>,
}

pub(super) fn run(opts: &Options) -> Result<(), api::Error> {
    log::debug!("get key value pair {opts:?}");
    let raw_key = opts.key.raw_key(&opts.args, "get")?;
    let key = opts
        .key
        .resolve(raw_key, opts.database.node_hash_algorithm)?;

    let cfg = DbConfig::builder()
        .node_hash_algorithm(opts.database.node_hash_algorithm.into())
        .create_if_missing(false)
        .truncate(false);

    let db = Db::new(opts.database.dbpath.clone(), cfg.build())?;

    let hash = db.root_hash();

    let Some(hash) = hash else {
        println!("Database is empty");
        return db.close();
    };

    let rev = db.revision(hash)?;

    match rev.val(&key) {
        Ok(Some(val)) => {
            let s = String::from_utf8_lossy(val.as_ref());
            println!("{s:?}");
        }
        Ok(None) => {
            eprintln!("Key '{}' not found", opts.key.display(raw_key, &key));
        }
        Err(e) => return Err(e),
    }
    db.close()
}
