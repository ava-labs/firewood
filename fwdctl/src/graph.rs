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
    log::debug!("dump database {opts:?}");
    let cfg = DbConfig::builder()
        .node_hash_algorithm(opts.database.node_hash_algorithm.into())
        .create_if_missing(false)
        .truncate(false);

    // Open via the runtime-selecting `open` so an existing database's persisted
    // hash mode is honored regardless of the requested `--hash-mode`.
    let db = Db::open(
        opts.database.dbpath.clone(),
        opts.database.resolve_node_hash_algorithm(),
        cfg.build(),
    )?;
    print!("{}", db.dump_to_string()?);
    db.close()
}
