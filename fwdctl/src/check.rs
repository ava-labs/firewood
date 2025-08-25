// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::HashMap;
use std::sync::Arc;
use std::{collections::BTreeMap, path::PathBuf};

use clap::Args;
use firewood::v2::api;
use firewood_storage::{CacheReadStrategy, CheckOpt, DBStats, FileBacked, NodeStore};
use handlebars::Handlebars;
use indicatif::{ProgressBar, ProgressFinish, ProgressStyle};
use nonzero_ext::nonzero;
use num_format::{Locale, ToFormattedString};

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

pub(super) fn run(opts: &Options) -> Result<(), api::Error> {
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

fn calculate_area_totals(area_counts: &BTreeMap<u64, u64>) -> (u64, u64) {
    let total_area_count = area_counts.values().sum::<u64>();
    let total_area_bytes = area_counts
        .iter()
        .map(|(area_size, count)| area_size.saturating_mul(*count))
        .sum::<u64>();
    (total_area_count, total_area_bytes)
}

const TEMPLATE: &str = r"
Basic Stats:
    Firewood Image Size / High Watermark (high_watermark): {{high_watermark}}
    Total Key-Value Count (kv_count): {{kv_count}}
    Total Key-Value Bytes (kv_bytes): {{kv_bytes}}

Trie Stats:
    Branching Factor Distribution: {{branching_factors}}
    Depth Distribution: {{depths}}

Branch Area Stats:
    Total Branch Data Bytes (branch_bytes): {{branch_bytes}}
    Total Branch Area Count (branch_area_count): {{total_branch_area_count}}
    Total Branch Area Bytes (branch_area_bytes): {{total_branch_area_bytes}}
    Branch Area Distribution: {{branch_area_counts}}
    Branches that Can Fit Into Smaller Area (low_occupancy_branch_area): {{low_occupancy_branch_area_count}} ({{low_occupancy_branch_area_percent}})

Leaf Area Stats:
    Total Leaf Data Bytes (leaf_bytes): {{leaf_bytes}}
    Total Leaf Area Count (leaf_area_count): {{total_leaf_area_count}}
    Total Leaf Area Bytes (leaf_area_bytes): {{total_leaf_area_bytes}}
    Leaf Area Distribution: {{leaf_area_counts}}
    Leaves that Can Fit Into Smaller Area (low_occupancy_leaf_area): {{low_occupancy_leaf_area_count}} ({{low_occupancy_leaf_area_percent}})

Free List Area Stats:
    Total Free List Area Count (free_list_area_count): {{total_free_list_area_count}}
    Total Free List Area Bytes (free_list_area_bytes): {{total_free_list_area_bytes}}
    Free List Area Distribution: {{free_list_area_counts}}

Alignment Stats:
    Trie Areas Spanning Extra Page Due to Unalignment: {{trie_area_extra_unaligned_page}} ({{trie_area_extra_unaligned_page_percent}})
    Free List Areas Spanning Extra Page Due to Unalignment: {{free_list_area_extra_unaligned_page}} ({{free_list_area_extra_unaligned_page_percent}})
    Trie Nodes Spanning Extra Page Due to Unalignment: {{trie_node_extra_unaligned_page}} ({{trie_node_extra_unaligned_page_percent}}%)

Advanced Stats:
    Storage Overhead: high_watermark / kv_bytes = {{storage_overhead}}
    Internal Fragmentation: 1 - (branch_bytes + leaf_bytes) / (branch_area_bytes + leaf_area_bytes) = {{internal_fragmentation}}
    Areas that Can Fit Into Smaller Area: low_occupancy_branch_area + low_occupancy_leaf_area = {{low_occupancy_area_count}} ({{low_occupancy_area_percent}})
";

fn format_u64(value: u64) -> String {
    value.to_formatted_string(&Locale::en)
}

fn format_map(map: &BTreeMap<impl ToFormattedString, impl ToFormattedString>) -> String {
    let str_map = map
        .iter()
        .map(|(key, value)| {
            (
                key.to_formatted_string(&Locale::en),
                value.to_formatted_string(&Locale::en),
            )
        })
        .collect::<BTreeMap<String, String>>();
    format!("{str_map:?}")
}

#[expect(clippy::cast_precision_loss)]
fn format_percent(numerator: u64, denominator: u64) -> String {
    format!("{:.2}%", (numerator as f64 / denominator as f64) * 100.0)
}

#[expect(clippy::cast_precision_loss)]
fn format_multiple(num: u64, base: u64) -> String {
    format!("{:.2}x", num as f64 / base as f64)
}

#[expect(clippy::too_many_lines)]
fn print_stats_report(db_stats: DBStats) {
    let (total_branch_area_count, total_branch_area_bytes) =
        calculate_area_totals(&db_stats.trie_stats.branch_area_counts);
    let (total_leaf_area_count, total_leaf_area_bytes) =
        calculate_area_totals(&db_stats.trie_stats.leaf_area_counts);
    let total_trie_area_count = total_branch_area_count.saturating_add(total_leaf_area_count);
    let total_trie_area_bytes = total_branch_area_bytes.saturating_add(total_leaf_area_bytes);

    let (total_free_list_area_count, total_free_list_area_bytes) =
        calculate_area_totals(&db_stats.free_list_stats.area_counts);

    let total_trie_bytes = db_stats
        .trie_stats
        .branch_bytes
        .saturating_add(db_stats.trie_stats.leaf_bytes);
    let total_low_occupancy_area_count = db_stats
        .trie_stats
        .low_occupancy_branch_area_count
        .saturating_add(db_stats.trie_stats.low_occupancy_leaf_area_count);

    let context = HashMap::from([
        // Basic stats
        ("high_watermark", format_u64(db_stats.high_watermark)),
        ("kv_count", format_u64(db_stats.trie_stats.kv_count)),
        ("kv_bytes", format_u64(db_stats.trie_stats.kv_bytes)),
        // Trie stats
        (
            "branching_factors",
            format_map(&db_stats.trie_stats.branching_factors),
        ),
        ("depths", format_map(&db_stats.trie_stats.depths)),
        // Branch area stats
        ("branch_bytes", format_u64(db_stats.trie_stats.branch_bytes)),
        (
            "total_branch_area_count",
            format_u64(total_branch_area_count),
        ),
        (
            "total_branch_area_bytes",
            format_u64(total_branch_area_bytes),
        ),
        (
            "branch_area_counts",
            format_map(&db_stats.trie_stats.branch_area_counts),
        ),
        (
            "low_occupancy_branch_area_count",
            format_u64(db_stats.trie_stats.low_occupancy_branch_area_count),
        ),
        (
            "low_occupancy_branch_area_percent",
            format_percent(
                db_stats.trie_stats.low_occupancy_branch_area_count,
                total_branch_area_count,
            ),
        ),
        // Leaf area stats
        ("leaf_bytes", format_u64(db_stats.trie_stats.leaf_bytes)),
        ("total_leaf_area_count", format_u64(total_leaf_area_count)),
        ("total_leaf_area_bytes", format_u64(total_leaf_area_bytes)),
        (
            "leaf_area_counts",
            format_map(&db_stats.trie_stats.leaf_area_counts),
        ),
        (
            "low_occupancy_leaf_area_count",
            format_u64(db_stats.trie_stats.low_occupancy_leaf_area_count),
        ),
        (
            "low_occupancy_leaf_area_percent",
            format_percent(
                db_stats.trie_stats.low_occupancy_leaf_area_count,
                total_leaf_area_count,
            ),
        ),
        // Free list area stats
        (
            "total_free_list_area_count",
            format_u64(total_free_list_area_count),
        ),
        (
            "total_free_list_area_bytes",
            format_u64(total_free_list_area_bytes),
        ),
        (
            "free_list_area_counts",
            format_map(&db_stats.free_list_stats.area_counts),
        ),
        // Alignment stats
        (
            "trie_area_extra_unaligned_page",
            format_u64(db_stats.trie_stats.area_extra_unaligned_page),
        ),
        (
            "trie_area_extra_unaligned_page_percent",
            format_percent(
                db_stats.trie_stats.area_extra_unaligned_page,
                total_trie_area_count,
            ),
        ),
        (
            "free_list_area_extra_unaligned_page",
            format_u64(db_stats.free_list_stats.area_extra_unaligned_page),
        ),
        (
            "free_list_area_extra_unaligned_page_percent",
            format_percent(
                db_stats.free_list_stats.area_extra_unaligned_page,
                total_free_list_area_count,
            ),
        ),
        (
            "trie_node_extra_unaligned_page",
            format_u64(db_stats.trie_stats.node_extra_unaligned_page),
        ),
        (
            "trie_node_extra_unaligned_page_percent",
            format_percent(
                db_stats.trie_stats.node_extra_unaligned_page,
                total_trie_area_count,
            ),
        ),
        (
            "storage_overhead",
            format_multiple(db_stats.high_watermark, db_stats.trie_stats.kv_bytes),
        ),
        (
            "internal_fragmentation",
            format_percent(
                total_trie_area_bytes.saturating_sub(total_trie_bytes),
                total_trie_area_bytes,
            ),
        ),
        (
            "low_occupancy_area_count",
            format_u64(total_low_occupancy_area_count),
        ),
        (
            "low_occupancy_area_percent",
            format_percent(total_low_occupancy_area_count, total_trie_area_count),
        ),
    ]);

    let reg = Handlebars::new();
    let report = reg
        .render_template(TEMPLATE, &context)
        .expect("valid template");
    println!("{report}");
}
