// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;
use firewood::api;
use firewood::db::BatchOp;
use firewood::db::DbConfig;
use firewood::open;

use crate::DatabasePath;
use crate::key::KeyArgument;

#[derive(Debug, Args)]
pub struct Options {
    #[command(flatten)]
    pub database: DatabasePath,

    #[command(flatten)]
    pub key: KeyArgument,
}

pub(super) fn run(opts: &Options) -> Result<(), api::Error> {
    log::debug!("deleting key {opts:?}");
    let key = opts.key.database_key()?;
    let hex_key = hex::encode(&key);
    let algorithm = opts.database.node_hash_algorithm()?;
    let cfg = DbConfig::builder()
        .node_hash_algorithm(algorithm)
        .create_if_missing(false)
        .truncate(false);

    let db = open(opts.database.dbpath.clone(), cfg.build())?;

    let batch: api::OwnedBatch = Box::new([BatchOp::Delete {
        key: key.into_boxed_slice(),
    }]);
    let proposal = db.propose(batch)?;
    proposal.commit()?;

    println!("key 0x{hex_key} deleted successfully");
    db.close()
}
