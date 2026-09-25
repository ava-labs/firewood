// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::path::PathBuf;
use std::time::Instant;

use clap::Args;
use firewood::api::{self, DynDbView};
use firewood::db::DbConfig;
use firewood::open;
use firewood_storage::filter::{CountingBloom, MembershipFilter};

use crate::DatabasePath;

const DEFAULT_COUNTERS_PER_KEY: u32 = 12;

#[derive(Debug, Args)]
pub struct Options {
    #[command(flatten)]
    pub database: DatabasePath,

    /// Checkpoint file to write; pass the same path as `FIREWOOD_FILTER_PATH`
    /// when running the database.
    #[arg(short = 'o', long, value_name = "FILE")]
    pub output: PathBuf,

    /// Counters per key. Roughly bits-per-key for a plain bloom filter; 12
    /// gives ~0.3% false positives at 4× the memory of a plain filter.
    #[arg(long, value_name = "N", default_value_t = DEFAULT_COUNTERS_PER_KEY)]
    pub counters_per_key: u32,

    /// Size the filter for this many keys instead of counting them first.
    /// Saves one full iteration of the database; undersizing raises the
    /// false-positive rate but is never unsafe.
    #[arg(long, value_name = "N")]
    pub expected_keys: Option<u64>,
}

pub(super) fn run(opts: &Options) -> Result<(), api::Error> {
    let algorithm = opts.database.node_hash_algorithm()?;
    let cfg = DbConfig::builder()
        .node_hash_algorithm(algorithm)
        .create_if_missing(false)
        .truncate(false);
    let db = open(opts.database.dbpath.clone(), cfg.build())?;

    let Some(root) = db.root_hash() else {
        println!("Database is empty; nothing to build");
        return db.close();
    };
    let latest = db.revision(root.clone())?;

    let expected = if let Some(expected) = opts.expected_keys {
        expected
    } else {
        let start = Instant::now();
        let count = count_keys(latest.as_ref())?;
        println!(
            "counted {count} keys in {:.1}s",
            start.elapsed().as_secs_f64()
        );
        count
    };
    if expected == 0 {
        println!("Database is empty; nothing to build");
        return db.close();
    }

    let filter = CountingBloom::new(expected, opts.counters_per_key);
    println!(
        "sized filter: {} MiB ({} counters/key) for {expected} keys",
        filter.size_bytes() >> 20,
        opts.counters_per_key
    );

    let start = Instant::now();
    let mut inserted = 0u64;
    for item in latest.iter()? {
        let (key, _) = item?;
        filter.insert(&key);
        inserted = inserted.saturating_add(1);
    }
    let stats = filter.stats();
    println!(
        "inserted {inserted} keys in {:.1}s: fill {:.4}, saturated {}, est. fpp {:.5}",
        start.elapsed().as_secs_f64(),
        stats.fill_ratio(),
        stats.saturated,
        stats.estimated_fpp(),
    );

    filter.save(&opts.output, Some(&root))?;
    println!("wrote {} for root {root}", opts.output.display());
    db.close()
}

fn count_keys(view: &dyn DynDbView) -> Result<u64, api::Error> {
    let mut count = 0u64;
    for item in view.iter()? {
        item?;
        count = count.saturating_add(1);
    }
    Ok(count)
}
