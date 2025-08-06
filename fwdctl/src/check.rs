// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Args;
use firewood::v2::api;
use firewood_storage::{CacheReadStrategy, CheckOpt, CheckerReport, FileBacked, NodeStore};
use indicatif::{ProgressBar, ProgressFinish, ProgressStyle};
use nonzero_ext::nonzero;

// TODO: (optionally) add a fix option
#[derive(Args)]
pub struct Options {
    /// The database path (if no path is provided, return an error). Defaults to firewood.
    #[arg(
        long,
        required = false,
        value_name = "DB_NAME",
        default_value_t = String::from("firewood"),
        help = "Name of the database"
    )]
    pub db: String,

    /// Whether to perform hash check
    #[arg(
        long,
        required = false,
        default_value_t = false,
        help = "Should perform hash check"
    )]
    pub hash_check: bool,
}

pub(super) async fn run(opts: &Options) -> Result<(), api::Error> {
    let db_path = PathBuf::from(&opts.db);
    let node_cache_size = nonzero!(1usize);
    let free_list_cache_size = nonzero!(1usize);

    let fb = FileBacked::new(
        db_path,
        node_cache_size,
        free_list_cache_size,
        false,
        CacheReadStrategy::WritesOnly, // we scan the database once - no need to cache anything
    )?;
    let storage = Arc::new(fb);

    let progress_bar = ProgressBar::no_length()
        .with_style(
            ProgressStyle::with_template("{wide_bar} {bytes}/{total_bytes} [{msg}]")
                .expect("valid template")
                .progress_chars("#>-"),
        )
        .with_finish(ProgressFinish::WithMessage("Check Completed!".into()));

    let report = NodeStore::open(storage)?.check(CheckOpt {
        hash_check: opts.hash_check,
        progress_bar: Some(progress_bar),
    });

    print_checker_report(report);

    Ok(())
}

#[expect(clippy::cast_precision_loss)]
fn print_checker_report(report: CheckerReport) {
    println!("Raw Report: {report:?}\n");

    println!("Advanced Data: ");
    let total_trie_area_bytes = report
        .trie_stats
        .area_counts
        .iter()
        .map(|(area_size, count)| area_size.saturating_mul(*count))
        .sum::<u64>();
    println!(
        "\tStorage Space Utilization: {} / {} = {:.2}%",
        report.trie_stats.trie_bytes,
        report.physical_bytes,
        (report.trie_stats.trie_bytes as f64 / report.physical_bytes as f64) * 100.0
    );
    println!(
        "\tInternal Fragmentation: {} / {total_trie_area_bytes} = {:.2}%",
        report.trie_stats.trie_bytes,
        (report.trie_stats.trie_bytes as f64 / total_trie_area_bytes as f64) * 100.0
    );
}
