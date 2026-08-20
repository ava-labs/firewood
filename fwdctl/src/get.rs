// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;

use firewood::api;
use firewood::db::DbConfig;
use firewood::open;

use crate::{DatabasePath, key::KeyArgument};

#[derive(Debug, Args)]
pub struct Options {
    #[command(flatten)]
    pub database: DatabasePath,

    #[command(flatten)]
    pub key: KeyArgument,
}

pub(super) fn run(opts: &Options) -> Result<(), api::Error> {
    log::debug!("get key value pair {opts:?}");
    let key = opts.key.database_key()?;
    let algorithm = opts.database.resolve_node_hash_algorithm();
    let cfg = DbConfig::builder()
        .node_hash_algorithm(algorithm)
        .create_if_missing(false)
        .truncate(false);

    let db = open(opts.database.dbpath.clone(), cfg.build())?;

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
            eprintln!("Key '0x{}' not found", hex::encode(&key));
        }
        Err(e) => return Err(e),
    }
    db.close()
}
