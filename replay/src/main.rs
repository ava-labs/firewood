// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::error::Error;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use firewood::db::{Db, DbConfig};
use firewood::manager::RevisionManagerConfig;

/// Command-line utility for applying Firewood block replay logs.
#[derive(Debug, Parser)]
#[command(author, version, about = "Firewood block replay CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Re-execute a replay log against a fresh database.
    ReExecute {
        /// Path to the replay log file.
        #[arg(long, value_name = "LOG_PATH")]
        log: PathBuf,

        /// Path to the Firewood database to create/truncate.
        #[arg(long, value_name = "DB_PATH")]
        db: PathBuf,
    },

    /// Placeholder for a future binary search helper over replay logs.
    BinarySearch {
        /// Path to the replay log file.
        #[arg(long, value_name = "LOG_PATH")]
        log: PathBuf,

        /// Path to the Firewood database to use.
        #[arg(long, value_name = "DB_PATH")]
        db: PathBuf,
    },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Command::ReExecute { log, db } => {
            let cfg = DbConfig::builder()
                .truncate(true)
                .manager(RevisionManagerConfig::builder().build())
                .build();
            let db = Db::new(db, cfg)?;
            firewood_replay::replay_log_from_file(log, &db)?;
        }
        Command::BinarySearch { .. } => {
            eprintln!("binary-search subcommand is not implemented yet.");
            std::process::exit(1);
        }
    }

    Ok(())
}

