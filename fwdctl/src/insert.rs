// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;
use firewood::api;
use firewood::db::{BatchOp, Db, DbConfig};

use crate::DatabasePath;

#[derive(Debug, Args)]
pub struct Options {
    #[command(flatten)]
    pub database: DatabasePath,

    /// The key to insert
    #[arg(required = true, value_name = "KEY", help = "Key to insert")]
    pub key: String,

    /// The value to insert
    #[arg(required = true, value_name = "VALUE", help = "Value to insert")]
    pub value: String,
}

pub(super) fn run(opts: &Options) -> Result<(), api::Error> {
    log::debug!("inserting key value pair {opts:?}");
    let cfg = DbConfig::builder()
        .node_hash_algorithm(opts.database.node_hash_algorithm.into())
        .create_if_missing(false)
        .truncate(false);

    let db: Box<dyn api::DynDb> = Box::new(Db::new(opts.database.dbpath.clone(), cfg.build())?);

    let batch: api::OwnedBatch = Box::new([BatchOp::Put {
        key: opts.key.clone().into_bytes().into_boxed_slice(),
        value: opts.value.clone().into_bytes().into_boxed_slice(),
    }]);
    let proposal = db.propose(batch)?;
    proposal.commit()?;

    println!("{}", opts.key);
    db.close()
}
