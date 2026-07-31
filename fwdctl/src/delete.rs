// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;
use firewood::api::{self, Db as _, Proposal as _};
use firewood::db::{BatchOp, Db, DbConfig};

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
    log::debug!("deleting key {opts:?}");
    let raw_key = opts.key.raw_key(&opts.args, "delete")?;
    let key = opts
        .key
        .resolve(raw_key, opts.database.node_hash_algorithm)?;
    let key_display = opts.key.display(raw_key, &key);

    let cfg = DbConfig::builder()
        .node_hash_algorithm(opts.database.node_hash_algorithm.into())
        .create_if_missing(false)
        .truncate(false);

    let db = Db::new(opts.database.dbpath.clone(), cfg.build())?;

    let batch: Vec<BatchOp<Box<[u8]>, Vec<u8>>> = vec![BatchOp::Delete { key }];
    let proposal = db.propose(batch)?;
    proposal.commit()?;

    println!("key {key_display} deleted successfully");
    db.close()
}
