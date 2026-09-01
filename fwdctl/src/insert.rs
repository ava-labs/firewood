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

    /// The value to insert
    #[arg(required = true, value_name = "VALUE", help = "Value to insert")]
    pub value: String,
}

pub(super) fn run(opts: &Options) -> Result<(), api::Error> {
    log::debug!("inserting key value pair {opts:?}");
    let key = opts.key.database_key()?;
    let hex_key = hex::encode(&key);
    let algorithm = opts.database.node_hash_algorithm()?;
    let cfg = DbConfig::builder()
        .node_hash_algorithm(algorithm)
        .create_if_missing(false)
        .truncate(false);

    let db = open(opts.database.dbpath.clone(), cfg.build())?;

    let batch: api::OwnedBatch = Box::new([BatchOp::Put {
        key: key.into_boxed_slice(),
        value: opts.value.clone().into_bytes().into_boxed_slice(),
    }]);
    let proposal = db.propose(batch)?;
    proposal.commit()?;

    println!("0x{hex_key}");
    db.close()
}
