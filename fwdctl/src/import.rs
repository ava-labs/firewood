// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;
use firewood::api::{self, Db as _, Proposal as _};
use firewood::db::{BatchOp, Db, DbConfig};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use crate::DatabasePath;
use crate::dump::OutputFormat;

#[derive(Debug, Args)]
pub struct Options {
    #[command(flatten)]
    pub database: DatabasePath,

    /// The input format of database dump.
    /// Defaults to "json"
    #[arg(
        short = 'i',
        long,
        required = false,
        value_name = "INPUT_FORMAT",
        value_enum,
        default_value_t = OutputFormat::Json,
        help = "Input format of database dump, default to json. CSV and JSON formats are available."
    )]
    pub input_format: OutputFormat,

    /// The input file name of database dump.
    #[arg(
        short = 'f',
        long,
        required = true,
        value_name = "INPUT_FILE_NAME",
        help = "Input file name of database dump."
    )]
    pub input_file_name: PathBuf,

    #[arg(short = 'x', long, help = "Parse the keys and values as hex format.")]
    pub hex: bool,
}

const BATCH_SIZE: usize = 1000;

pub(super) fn run(opts: &Options) -> Result<(), api::Error> {
    log::debug!("import database {opts:?}");

    if opts.input_format == OutputFormat::Dot || opts.input_format == OutputFormat::Stdout {
        return Err(api::Error::InternalError(Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Import only supports CSV and JSON formats",
        ))));
    }

    let cfg = DbConfig::builder()
        .node_hash_algorithm(opts.database.node_hash_algorithm.into())
        .create_if_missing(true)
        .truncate(false);

    let db = Db::new(opts.database.dbpath.clone(), cfg.build())?;

    let file = File::open(&opts.input_file_name)?;

    let mut batch_ops: Vec<BatchOp<Vec<u8>, Vec<u8>>> = Vec::with_capacity(BATCH_SIZE);
    let mut total_imported: usize = 0;

    // Use a closure or macro to push ops and flush batches.
    let mut push_op = |key: Vec<u8>, value: Vec<u8>| -> Result<(), api::Error> {
        batch_ops.push(BatchOp::Put { key, value });
        if batch_ops.len() >= BATCH_SIZE {
            let mut batch = Vec::with_capacity(BATCH_SIZE);
            std::mem::swap(&mut batch_ops, &mut batch);
            let proposal = db.propose(batch)?;
            proposal.commit()?;
            total_imported = total_imported.saturating_add(BATCH_SIZE);
            log::info!("Imported {total_imported} records so far...");
        }
        Ok(())
    };

    match opts.input_format {
        OutputFormat::Csv => {
            let mut rdr = csv::ReaderBuilder::new()
                .has_headers(false)
                .from_reader(file);

            for result in rdr.records() {
                let record = result.map_err(|e| api::Error::InternalError(Box::new(e)))?;
                if record.len() >= 2 {
                    let key = parse_string(record.get(0).expect("len checked"), opts.hex)?;
                    let value = parse_string(record.get(1).expect("len checked"), opts.hex)?;
                    push_op(key, value)?;
                }
            }
        }
        OutputFormat::Json => {
            let reader = BufReader::new(file);
            for line in reader.lines() {
                let line = line?;
                let line = line.trim();

                if line == "{" || line == "}" {
                    continue;
                }

                // Line format is `"KEY": "VALUE",` or `"KEY": "VALUE"`
                // Remove trailing comma
                let line = line.strip_suffix(',').unwrap_or(line);

                // Now format is `"KEY": "VALUE"`
                if let Some((k_str, v_str)) = line.split_once("\": \"") {
                    let k_str = k_str.strip_prefix('"').unwrap_or(k_str);
                    let v_str = v_str.strip_suffix('"').unwrap_or(v_str);

                    let key = parse_string(k_str, opts.hex)?;
                    let value = parse_string(v_str, opts.hex)?;
                    push_op(key, value)?;
                }
            }
        }
        _ => unreachable!(),
    }

    // Insert remaining ops
    if !batch_ops.is_empty() {
        total_imported = total_imported.saturating_add(batch_ops.len());
        let proposal = db.propose(batch_ops)?;
        proposal.commit()?;
    }

    log::info!("Successfully imported {total_imported} records.");
    println!("Successfully imported {total_imported} records.");

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
