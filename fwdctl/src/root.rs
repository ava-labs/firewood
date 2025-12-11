// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;

use firewood::db::{Db, DbConfig};
use firewood::v2::api::{self, Db as _};

use crate::DatabaseDir;

#[derive(Debug, Args)]
pub struct Options {
    #[command(flatten)]
    pub database_dir: DatabaseDir,
}

pub(super) fn run(opts: &Options) -> Result<(), api::Error> {
    let cfg = DbConfig::builder().create_if_missing(false).truncate(false);

    let db = Db::new(opts.database_dir.dbdir.clone(), cfg.build())?;

    let hash = db.root_hash()?;

    println!("{hash:?}");
    Ok(())
}
