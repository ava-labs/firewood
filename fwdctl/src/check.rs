// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Args;
use firewood::v2::api;
use firewood_storage::{CacheReadStrategy, CheckOpt, DBStats, FileBacked, NodeStore};
use indicatif::{ProgressBar, ProgressFinish, ProgressStyle};
use nonzero_ext::nonzero;

use crate::DatabasePath;

// TODO: (optionally) add a fix option
#[derive(Args)]
pub struct Options {
    #[command(flatten)]
    pub database: DatabasePath,

    /// Whether to perform hash check
    #[arg(
        long,
        required = false,
        default_value_t = false,
        help = "Should perform hash check"
    )]
    pub hash_check: bool,

    /// Whether to fix observed inconsistencies
    #[arg(
        long,
        required = false,
        default_value_t = false,
        help = "Should fix observed inconsistencies"
    )]
    pub fix: bool,
}

pub(super) async fn run(opts: &Options) -> Result<(), api::Error> {
    let db_path = PathBuf::from(&opts.database.dbpath);
    let node_cache_size = nonzero!(1usize);
    let free_list_cache_size = nonzero!(1usize);

    let fb = FileBacked::new(
        db_path,
        node_cache_size,
        free_list_cache_size,
        false,
        false,                         // don't create if missing
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

    let check_ops = CheckOpt {
        hash_check: opts.hash_check,
        progress_bar: Some(progress_bar),
    };

    let nodestore = NodeStore::open(storage)?;
    let db_stats = if opts.fix {
        let report = nodestore.check_and_fix(check_ops);
        println!("Fixed Errors ({}):", report.fixed.len());
        for error in report.fixed {
            println!("\t{error}");
        }
        println!();
        println!("Unfixable Errors ({}):", report.unfixable.len(),);
        for (error, io_error) in report.unfixable {
            println!("\t{error}");
            if let Some(io_error) = io_error {
                println!("\t\tError encountered while fixing: {io_error}");
            }
        }
        report.db_stats
    } else {
        let report = nodestore.check(check_ops);
        println!("Errors ({}):", report.errors.len());
        for error in report.errors {
            println!("\t{error}");
        }
        report.db_stats
    };
    println!();

    print_stats_report(db_stats);

    Ok(())
}

#[expect(clippy::cast_precision_loss)]
fn print_stats_report(db_stats: DBStats) {
    // print the raw data
    println!("\nRaw Data: {db_stats:#?}");

    println!("Advanced Data: ");
    let total_trie_area_bytes = db_stats
        .trie_stats
        .area_counts
        .iter()
        .map(|(area_size, count)| area_size.saturating_mul(*count))
        .sum::<u64>();
    println!(
        "\tStorage Overhead: {} / {} = {:.2}x",
        db_stats.high_watermark,
        db_stats.trie_stats.kv_bytes,
        (db_stats.high_watermark as f64 / db_stats.trie_stats.kv_bytes as f64)
    );
    println!(
        "\tInternal Fragmentation: 1 - ({} / {total_trie_area_bytes}) = {:.2}%",
        db_stats.trie_stats.trie_bytes,
        (1f64 - (db_stats.trie_stats.trie_bytes as f64 / total_trie_area_bytes as f64)) * 100.0
    );
    println!(
        "\tTrie Area Size Distribution: {:?}",
        db_stats.trie_stats.area_counts
    );
    println!(
        "\tFree List Area Size Distribution: {:?}",
        db_stats.free_list_stats.area_counts
    );
    println!(
        "\tBranching Factor Distribution: {:?}",
        db_stats.trie_stats.branching_factors
    );
    println!("\tDepth Distribution: {:?}", db_stats.trie_stats.depths);
    let total_trie_area_count = db_stats.trie_stats.area_counts.values().sum::<u64>();
    println!(
        "\tLow Occupancy Rate: {} / {total_trie_area_count} = {:.2}%",
        db_stats.trie_stats.low_occupancy_area_count,
        (db_stats.trie_stats.low_occupancy_area_count as f64 / total_trie_area_count as f64)
            * 100.0
    );
    println!(
        "\tTrie Extra Unaligned Page Read Rate: {} / {total_trie_area_count} = {:.2}%",
        db_stats.trie_stats.extra_unaligned_page_read,
        (db_stats.trie_stats.extra_unaligned_page_read as f64 / total_trie_area_count as f64)
            * 100.0
    );
    let total_free_list_area_count = db_stats.free_list_stats.area_counts.values().sum::<u64>();
    println!(
        "\tFree List Extra Unaligned Page Read Rate: {} / {total_free_list_area_count} = {:.2}%",
        db_stats.free_list_stats.extra_unaligned_page_read,
        (db_stats.free_list_stats.extra_unaligned_page_read as f64
            / total_free_list_area_count as f64)
            * 100.0
    );
}
