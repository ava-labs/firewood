// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;

use firewood::api;
use firewood::db::{Db, DbConfig};

use crate::DatabasePath;

#[derive(Debug, Args)]
pub struct Options {
    #[command(flatten)]
    pub database: DatabasePath,
}

pub(super) fn run(opts: &Options) -> Result<(), api::Error> {
    let algorithm = opts.database.resolve_node_hash_algorithm();
    let cfg = DbConfig::builder()
        .node_hash_algorithm(algorithm)
        .create_if_missing(false)
        .truncate(false);

    let db = Db::open(opts.database.dbpath.clone(), algorithm, cfg.build())?;

    let hash = db.root_hash();

    println!("{hash:?}");
    db.close()
}
