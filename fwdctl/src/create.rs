// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::{Args, ValueEnum, value_parser};
use firewood::api;
use firewood::db::DbConfig;
use firewood::open;

use crate::DatabasePath;

// Clap-facing mirror of firewood_storage::NodeHashAlgorithm. Keeping ValueEnum
// here avoids adding a Clap dependency to the storage crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum)]
enum HashModeArg {
    #[value(name = "merkle-db")]
    MerkleDB,
    #[value(name = "ethereum")]
    Ethereum,
}

impl From<HashModeArg> for firewood_storage::NodeHashAlgorithm {
    fn from(hash_mode: HashModeArg) -> Self {
        match hash_mode {
            HashModeArg::MerkleDB => Self::MerkleDB,
            HashModeArg::Ethereum => Self::Ethereum,
        }
    }
}

#[derive(Args, Debug)]
pub struct Options {
    #[command(flatten)]
    pub database: DatabasePath,

    /// The node hash algorithm to persist in the new database header.
    #[arg(
        long,
        visible_alias = "hash-mode",
        value_enum,
        required = true,
        help = "The node hash algorithm for the new database"
    )]
    node_hash_algorithm: HashModeArg,

    #[arg(
        long,
        required = false,
        value_parser = value_parser!(bool),
        default_missing_value = "false",
        default_value_t = true,
        value_name = "TRUNCATE",
        help = "Whether to truncate the DB when opening it. If set, the DB will be reset and all its
    existing contents will be lost"
    )]
    pub truncate: bool,

    /// WAL Config
    #[arg(
        long,
        required = false,
        default_value_t = 22,
        value_name = "WAL_FILE_NBIT",
        help = "Size of WAL file."
    )]
    file_nbit: u64,

    #[arg(
        long,
        required = false,
        default_value_t = 100,
        value_name = "Wal_MAX_REVISIONS",
        help = "Number of revisions to keep from the past. This preserves a rolling window
    of the past N commits to the database."
    )]
    max_revisions: u32,
}

pub(super) fn new(opts: &Options) -> DbConfig {
    DbConfig::builder()
        .node_hash_algorithm(opts.node_hash_algorithm.into())
        .truncate(opts.truncate)
        .build()
}

pub(super) fn run(opts: &Options) -> Result<(), api::Error> {
    let db_config = new(opts);
    log::debug!("database configuration parameters: \n{db_config:?}\n");

    let db = open(opts.database.dbpath.clone(), db_config)?;
    println!(
        "created firewood database in {}",
        opts.database.dbpath.display()
    );
    db.close()
}
