// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;
use firewood::db::{Db, DbConfig};
use firewood::v2::api;
use std::io::stdout;

use crate::DatabaseDir;

#[derive(Debug, Args)]
pub struct Options {
    #[command(flatten)]
    pub database: DatabaseDir,
}

pub(super) fn run(opts: &Options) -> Result<(), api::Error> {
    log::debug!("dump database {opts:?}");
    let cfg = DbConfig::builder().create_if_missing(false).truncate(false);

    let db = Db::new(opts.database.dbdir.clone(), cfg.build())?;
    db.dump(&mut stdout())?;
    Ok(())
}
