// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;
use firewood::api::{self, Db as _, Proposal as _};
use firewood::db::{BatchOp, Db, DbConfig};

use crate::DatabasePath;

/// Parse a hex string into bytes
fn parse_hex(s: &str) -> Result<Vec<u8>, String> {
    // Strip 0x prefix if present
    let s = s.strip_prefix("0x").unwrap_or(s);
    hex::decode(s).map_err(|e| format!("Invalid hex string: {e}"))
}

#[derive(Debug, Args)]
pub struct Options {
    #[command(flatten)]
    pub database: DatabasePath,

    /// The key to insert (as a string)
    #[arg(
        required_unless_present = "key_hex",
        value_name = "KEY",
        help = "Key to insert (as a string)"
    )]
    pub key: Option<String>,

    /// The value to insert (as a string)
    #[arg(
        required_unless_present = "value_hex",
        value_name = "VALUE",
        help = "Value to insert (as a string)"
    )]
    pub value: Option<String>,

    /// The key to insert, in hex format
    #[arg(
        long,
        required_unless_present = "key",
        conflicts_with = "key",
        value_name = "KEY_HEX",
        help = "Key to insert, in hex format (with or without 0x prefix)"
    )]
    pub key_hex: Option<String>,

    /// The value to insert, in hex format
    #[arg(
        long,
        required_unless_present = "value",
        conflicts_with = "value",
        value_name = "VALUE_HEX",
        help = "Value to insert, in hex format (with or without 0x prefix)"
    )]
    pub value_hex: Option<String>,
}

impl Options {
    /// Get the key as bytes
    fn key_bytes(&self) -> Result<Vec<u8>, String> {
        if let Some(ref key) = self.key {
            Ok(key.as_bytes().to_vec())
        } else if let Some(ref key_hex) = self.key_hex {
            parse_hex(key_hex)
        } else {
            Err("No key provided".to_string())
        }
    }

    /// Get the value as bytes
    fn value_bytes(&self) -> Result<Vec<u8>, String> {
        if let Some(ref value) = self.value {
            Ok(value.as_bytes().to_vec())
        } else if let Some(ref value_hex) = self.value_hex {
            parse_hex(value_hex)
        } else {
            Err("No value provided".to_string())
        }
    }

    /// Get the key as a display string (for output)
    fn key_display(&self) -> String {
        if let Some(ref key) = self.key {
            key.clone()
        } else if let Some(ref key_hex) = self.key_hex {
            format!("0x{}", key_hex.strip_prefix("0x").unwrap_or(key_hex))
        } else {
            String::from("<no key>")
        }
    }
}

pub(super) fn run(opts: &Options) -> Result<(), api::Error> {
    log::debug!("inserting key value pair {opts:?}");
    let cfg = DbConfig::builder()
        .node_hash_algorithm(opts.database.node_hash_algorithm.into())
        .create_if_missing(false)
        .truncate(false);

    let db = Db::new(opts.database.dbpath.clone(), cfg.build())?;

    let key_bytes = opts
        .key_bytes()
        .map_err(|e| api::Error::InternalError(Box::new(std::io::Error::new(std::io::ErrorKind::InvalidInput, e))))?;
    let value_bytes = opts
        .value_bytes()
        .map_err(|e| api::Error::InternalError(Box::new(std::io::Error::new(std::io::ErrorKind::InvalidInput, e))))?;

    let batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> = vec![BatchOp::Put {
        key: key_bytes,
        value: value_bytes,
    }];
    let proposal = db.propose(batch)?;
    proposal.commit()?;

    println!("{}", opts.key_display());
    db.close()
}
