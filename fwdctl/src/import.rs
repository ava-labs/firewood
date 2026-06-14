// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;
use firewood::api::{self, Db as _, Proposal as _};
use firewood::db::{BatchOp, Db, DbConfig};
use humantime::{format_duration, parse_duration};
use std::fs::File;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::DatabasePath;

#[derive(Debug, clap::ValueEnum, Clone, PartialEq)]
pub enum InputFormat {
    Csv,
}

#[derive(Debug, Args)]
pub struct Options {
    #[command(flatten)]
    pub database: DatabasePath,

    /// The input format of database dump.
    /// Defaults to "csv"
    #[arg(
        short = 'i',
        long,
        required = false,
        value_name = "INPUT_FORMAT",
        value_enum,
        default_value_t = InputFormat::Csv,
        help = "Input format of database dump, default to csv."
    )]
    pub input_format: InputFormat,

    /// The input file name of database dump.
    #[arg(
        short = 'f',
        long,
        required = false,
        value_name = "INPUT_FILE_NAME",
        help = "Input file name of database dump. Reads from stdin if not provided."
    )]
    pub input_file_name: Option<PathBuf>,

    /// Parse the keys and values as hex format.
    #[arg(
        short = 'x',
        long,
        help = "Parse the keys and values as hex format."
    )]
    pub hex: bool,

    /// Number of records to batch per database commit.
    #[arg(
        short = 'b',
        long,
        default_value_t = 10000,
        help = "Number of records to batch per database commit."
    )]
    pub batch_size: usize,

    /// Time interval between status updates.
    #[arg(
        long,
        default_value = "1m",
        value_parser = parse_duration,
        help = "Time interval between status updates (e.g., 1m, 30s)."
    )]
    pub status_interval: Duration,
}

pub(super) fn run(opts: &Options) -> Result<(), api::Error> {
    log::debug!("import database {opts:?}");

    let cfg = DbConfig::builder()
        .node_hash_algorithm(opts.database.node_hash_algorithm.into())
        .create_if_missing(true)
        .truncate(false);

    let db = Db::new(opts.database.dbpath.clone(), cfg.build())?;

    let reader: Box<dyn std::io::Read> = if let Some(path) = &opts.input_file_name {
        Box::new(File::open(path)?)
    } else {
        Box::new(std::io::stdin())
    };

    let mut batch_ops: Vec<BatchOp<Vec<u8>, Vec<u8>>> = Vec::with_capacity(opts.batch_size);
    let mut total_imported: usize = 0;

    let start_time = Instant::now();
    let mut last_status_time = start_time;

    let mut push_op = |key: Vec<u8>, value: Vec<u8>| -> Result<(), api::Error> {
        batch_ops.push(BatchOp::Put { key, value });
        if batch_ops.len() >= opts.batch_size {
            let count = batch_ops.len();
            let proposal = db.propose(batch_ops.drain(..))?;
            proposal.commit()?;
            total_imported = total_imported.saturating_add(count);

            let now = Instant::now();
            if now.duration_since(last_status_time) >= opts.status_interval {
                let duration = now.duration_since(start_time);
                let duration_secs = duration.as_secs_f64();
                let rate = if duration_secs > 0.0 {
                    (total_imported as f64 / duration_secs) as usize
                } else {
                    0
                };
                let formatted_duration = format_duration(Duration::from_secs(duration.as_secs()));
                log::info!("{total_imported} keys in {formatted_duration} ({rate} keys/s)");
                last_status_time = now;
            }
        }
        Ok(())
    };

    match opts.input_format {
        InputFormat::Csv => {
            let mut rdr = csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(reader);

            let mut consecutive_errors = 0;
            const MAX_CONSECUTIVE_ERRORS: usize = 100;

            for (row_idx, result) in rdr.deserialize().enumerate() {
                let (key_str, value_str): (String, String) = match result {
                    Ok(r) => r,
                    Err(e) => {
                        consecutive_errors += 1;
                        if consecutive_errors > MAX_CONSECUTIVE_ERRORS {
                            return Err(api::Error::InternalError(Box::new(std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                "Too many consecutive malformed rows. Aborting import.",
                            ))));
                        }
                        log::warn!("Skipping malformed CSV row {}: {}", row_idx + 1, e);
                        continue;
                    }
                };

                let key = match parse_string(&key_str, opts.hex) {
                    Ok(k) => k,
                    Err(e) => {
                        consecutive_errors += 1;
                        if consecutive_errors > MAX_CONSECUTIVE_ERRORS {
                            return Err(api::Error::InternalError(Box::new(std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                "Too many consecutive malformed rows. Aborting import.",
                            ))));
                        }
                        log::warn!("Skipping row {}: failed to parse key: {}", row_idx + 1, e);
                        continue;
                    }
                };
                let value = match parse_string(&value_str, opts.hex) {
                    Ok(v) => v,
                    Err(e) => {
                        consecutive_errors += 1;
                        if consecutive_errors > MAX_CONSECUTIVE_ERRORS {
                            return Err(api::Error::InternalError(Box::new(std::io::Error::new(
                                std::io::ErrorKind::InvalidInput,
                                "Too many consecutive malformed rows. Aborting import.",
                            ))));
                        }
                        log::warn!("Skipping row {}: failed to parse value: {}", row_idx + 1, e);
                        continue;
                    }
                };
                
                consecutive_errors = 0;
                push_op(key, value)?;
            }
        }
    }

    // Insert remaining ops
    if !batch_ops.is_empty() {
        let remaining = batch_ops.len();
        let proposal = db.propose(batch_ops.drain(..))?;
        proposal.commit()?;
        total_imported = total_imported.saturating_add(remaining);
    }

    let total_duration = start_time.elapsed();
    let duration_secs = total_duration.as_secs_f64();
    let final_rate = if duration_secs > 0.0 {
        (total_imported as f64 / duration_secs) as usize
    } else {
        0
    };
    let formatted_total_duration =
        format_duration(Duration::from_secs(total_duration.as_secs()));

    let final_msg = format!(
        "Successfully imported {total_imported} keys in {formatted_total_duration} ({final_rate} keys/s)"
    );
    log::info!("{final_msg}");
    println!("{final_msg}");

    db.close()
}

fn parse_string(s: &str, hex: bool) -> Result<Vec<u8>, api::Error> {
    if hex {
        hex::decode(s).map_err(|e| {
            api::Error::InternalError(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                e,
            )))
        })
    } else {
        Ok(s.as_bytes().to_vec())
    }
}
