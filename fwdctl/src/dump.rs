// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;
use firewood::{
    db::{Db, DbConfig, WalConfig},
    v2::api::{self},
};
use log;

#[derive(Debug, Args)]
pub struct Options {
    /// The database path (if no path is provided, return an error). Defaults to firewood.
    #[arg(
        required = true,
        value_name = "DB_NAME",
        default_value_t = String::from("firewood"),
        help = "Name of the database"
    )]
    pub db: String,
}

pub async fn run(opts: &Options) -> Result<(), api::Error> {
    log::debug!("dump database {:?}", opts);
    let cfg = DbConfig::builder()
        .truncate(false)
        .wal(WalConfig::builder().max_revisions(10).build());

    let db = Db::new(opts.db.as_str(), &cfg.build()).await?;
    Ok(db
        .kv_dump(&mut std::io::stdout().lock())?)
}
