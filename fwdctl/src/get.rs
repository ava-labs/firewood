// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;

use firewood::api::{self, Db as _, DbView as _};
use firewood::db::{Db, DbConfig};

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

    /// The key to get the value for (as a string)
    #[arg(
        required_unless_present = "key_hex",
        value_name = "KEY",
        help = "Key to get (as a string)"
    )]
    pub key: Option<String>,

    /// The key to get the value for, in hex format
    #[arg(
        long,
        required_unless_present = "key",
        conflicts_with = "key",
        value_name = "KEY_HEX",
        help = "Key to get, in hex format (with or without 0x prefix)"
    )]
    pub key_hex: Option<String>,

    /// Output the value in hex format
    #[arg(short = 'x', long, help = "Print the value in hex format")]
    pub hex: bool,
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

    /// Get the key as a display string (for error messages)
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
    log::debug!("get key value pair {opts:?}");
    let cfg = DbConfig::builder()
        .node_hash_algorithm(opts.database.node_hash_algorithm.into())
        .create_if_missing(false)
        .truncate(false);

    let db = Db::new(opts.database.dbpath.clone(), cfg.build())?;

    let hash = db.root_hash();

    let Some(hash) = hash else {
        println!("Database is empty");
        return db.close();
    };

    let rev = db.revision(hash)?;

    let key_bytes = opts
        .key_bytes()
        .map_err(|e| api::Error::InternalError(Box::new(std::io::Error::new(std::io::ErrorKind::InvalidInput, e))))?;

    match rev.val(&key_bytes) {
        Ok(Some(val)) => {
            if opts.hex {
                println!("0x{}", hex::encode(val.as_ref()));
            } else {
                let s = String::from_utf8_lossy(val.as_ref());
                println!("{s:?}");
            }
        }
        Ok(None) => {
            eprintln!("Key '{}' not found", opts.key_display());
        }
        Err(e) => return Err(e),
    }
    db.close()
}
