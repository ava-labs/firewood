// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};
use firewood::db::{Db, DbConfig};
use firewood::manager::RevisionManagerConfig;
use firewood::v2::api::Db as _;
use metrics_exporter_prometheus::PrometheusBuilder;

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

        /// Max commits
        #[arg(long, value_name = "MAX_COMMITS")]
        max_commits: Option<u64>,

        /// Path to metrics output
        #[arg(long, value_name = "METRICS_PATH")]
        metrics: Option<PathBuf>,
    },

    BinarySearch {},
    Plots {},
    ExportLogsToMessagepack {
        /// Path to the replay log file.
        #[arg(long, value_name = "LOGS_PATH")]
        logs: PathBuf,
        /// Path to desired output file for messagepack encoded logs.
        #[arg(long, value_name = "OUT_PATH")]
        output: PathBuf,
    },
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let builder = PrometheusBuilder::new();
    let handle = builder.install_recorder()?;

    let cli = Cli::parse();

    match cli.command {
        Command::ReExecute {
            log,
            db,
            max_commits,
            metrics,
        } => {
            let cfg = DbConfig::builder()
                .truncate(false)
                .manager(RevisionManagerConfig::builder().build())
                .build();
            let db = Db::new(db, cfg)?;
            let res = firewood_replay::replay_log_from_file(log, &db, max_commits)?.unwrap();
            let root = db.root_hash()?.unwrap();
            if *root.as_ref() == *res {
                println!("replay successful!");
                println!("new root: {:?}", root);
            } else {
                println!("replay done but root hash mismatch detected!");
                println!("new root: {:?}", root);
                println!("expected root: {}", hex::encode(&res));
            }

            if let Some(metrics) = metrics {
                let r = handle.render();
                fs::write(metrics, r)?;
            }
        }
        Command::BinarySearch { .. } => {
            firewood_replay::search::binary_search_performance().unwrap();
        }
        Command::Plots { .. } => {
            firewood_replay::search::plot_replay_times_from_dir(
                Path::new("run-metrics"),
                Path::new("x.html"),
            )
            .unwrap();
        }
        Command::ExportLogsToMessagepack { logs, output } => {
            firewood_replay::convert_rkyv_log_to_rmp_file(logs, output).unwrap();
        }
    }

    Ok(())
}
