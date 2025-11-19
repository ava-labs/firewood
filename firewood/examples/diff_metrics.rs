//! # Diff Algorithm Performance Metrics Example
//!
//! This example demonstrates and benchmarks the different diff algorithms provided by Firewood,
//! showcasing their performance characteristics on realistic workloads.
//!
//! ## Overview
//!
//! The example creates a large Firewood database with random key-value pairs, applies modifications,
//! and then compares the performance of three diff algorithms:
//!
//! 1. **Simple Diff**: Exhaustive O(n) algorithm that examines all keys
//! 2. **Optimized Diff**: Structural algorithm with hash-based pruning
//! 3. **Parallel Diff**: Multi-threaded version of the optimized algorithm
//!
//! ## Key Features
//!
//! - **Realistic Workload**: Generates random keys and values similar to blockchain state
//! - **Performance Metrics**: Tracks nodes visited, pruning rates, and execution time
//! - **Configurable**: Command-line arguments for database size and modification patterns
//!
//! ## Usage Examples
//!
//! ### Basic Usage
//!
//! Compare diff algorithms on a moderate-sized database:
//! ```bash
//! cargo run --release -p firewood --example diff_metrics
//! ```
//!
//! ### Large-Scale Testing
//!
//! Test with a large database (100K keys, 20K modifications):
//! ```bash
//! cargo run --release -p firewood --example diff_metrics -- \
//!     --items 100000 --modify 20000 --db-path diff_db
//! ```
//!
//! ### Custom Seed for Reproducibility
//!
//! Use a specific random seed for reproducible results:
//! ```bash
//! cargo run --release -p firewood --example diff_metrics -- \
//!     --seed 42 --items 50000 --modify 5000
//! ```
//!
//! ## Performance Insights
//!
//! This example helps understand:
//!
//! - **Pruning Effectiveness**: How many nodes the optimized algorithm skips
//! - **Speedup Factors**: Relative performance of each algorithm
//! - **Parallel Scalability**: Benefits of multi-threading for large tries
//! - **Memory Patterns**: Node visitation patterns and cache efficiency
//!
//! ## Expected Output
//!
//! The example outputs detailed metrics including:
//!
//! ```text
//! === Diff Metrics ===
//! Base keys:              100000
//! Requested modifications: 20000
//!
//! Simple diff:
//!   Operations:           15234
//!   Total nodes visited:  200000
//!   Elapsed:              450ms
//!
//! Optimized diff:
//!   Operations:           15234
//!   Nodes visited:        18542
//!   Nodes pruned:         16893
//!   Pruning rate:         91.1%
//!   Elapsed:              35ms
//! Time speedup: 12.8x
//!
//! Parallel diff:
//!   Operations:           15234
//!   Nodes visited:        18542
//!   Elapsed:              12ms
//! Time speedup: 37.5x
//! ```

// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Parser;
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::collections::HashSet;
use std::error::Error;
use std::num::NonZeroUsize;
use std::time::Instant;

use firewood::db::{BatchOp, Db, DbConfig};
use firewood::diff::{
    diff_merkle_optimized, diff_merkle_simple, ParallelDiff,
};
use firewood::manager::RevisionManagerConfig;
use firewood::merkle::{Key, Merkle};
use firewood::v2::api::{Db as _, DbView as _, Proposal as _};

/// Command-line arguments for the diff metrics example.
#[derive(Parser, Debug)]
struct Args {
    /// Path to the database directory
    #[arg(long, default_value = "diff_db")]
    db_path: String,

    /// Number of keys in the base state
    #[arg(long, default_value_t = 100_000)]
    items: usize,

    /// Number of random modifications applied on top of the base state
    #[arg(long, default_value_t = 20_000)]
    modify: usize,

    /// Seed for the random generator (if not set, a default is used)
    #[arg(long)]
    seed: Option<u64>,

    /// Whether to truncate (reset) the DB directory before running
    #[arg(long, default_value_t = true)]
    truncate: bool,

    /// Node cache size for the revision manager
    #[arg(long, default_value_t = NonZeroUsize::new(20_480).expect("non-zero"))]
    cache_size: NonZeroUsize,

    /// Maximum number of revisions kept in the DB
    #[arg(long, default_value_t = 128)]
    revisions: usize,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    println!(
        "Creating base state with {} keys, then applying {} modifications",
        args.items, args.modify
    );

    // Configure and open the database
    let mgrcfg = RevisionManagerConfig::builder()
        .node_cache_size(args.cache_size)
        .max_revisions(args.revisions)
        .build();
    let cfg = DbConfig::builder()
        .truncate(args.truncate)
        .manager(mgrcfg)
        .build();
    let db = Db::new(&args.db_path, cfg)?;

    // Build deterministic RNG
    let seed = args
        .seed
        .unwrap_or(0xD1FF_C0DE_BAAD_F00D_u64);
    let mut rng = StdRng::seed_from_u64(seed);

    // ------------------------------------------------------------------------
    // Step 1: Build a large base state and commit it
    // ------------------------------------------------------------------------
    let mut keys: Vec<Vec<u8>> = Vec::with_capacity(args.items);
    let mut values: Vec<Vec<u8>> = Vec::with_capacity(args.items);
    let mut seen = HashSet::with_capacity(args.items * 2);

    println!("Generating {} unique random keys...", args.items);
    while keys.len() < args.items {
        let klen: usize = rng.random_range(8..=32);
        let vlen: usize = rng.random_range(16..=64);

        let key: Vec<u8> = (0..klen).map(|_| rng.random()).collect();
        if !seen.insert(key.clone()) {
            continue;
        }

        let value: Vec<u8> = (0..vlen).map(|_| rng.random()).collect();
        keys.push(key);
        values.push(value);
    }

    let base_batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> = keys
        .iter()
        .cloned()
        .zip(values.iter().cloned())
        .map(|(key, value)| BatchOp::Put { key, value })
        .collect();

    println!("Committing base revision...");
    let t_insert_start = Instant::now();
    let proposal = db.propose(base_batch)?;
    let base_hash = proposal
        .root_hash()?
        .expect("base proposal should have a root hash");
    proposal.commit()?;
    let insert_duration = t_insert_start.elapsed();

    println!(
        "Base commit done: {} keys in {:?}",
        args.items, insert_duration
    );

    // ------------------------------------------------------------------------
    // Step 2: Apply random modifications on top of the base state
    // ------------------------------------------------------------------------
    let mut mods: Vec<BatchOp<Vec<u8>, Vec<u8>>> = Vec::with_capacity(args.modify);

    println!(
        "Generating {} random modifications (deletes / updates / inserts)...",
        args.modify
    );
    for _ in 0..args.modify {
        let op_choice = rng.random_range(0..100);
        if !keys.is_empty() && op_choice < 25 {
            // Delete an existing key (25%)
            let idx = rng.random_range(0..keys.len());
            let key = keys[idx].clone();
            mods.push(BatchOp::Delete { key });
        } else if !keys.is_empty() && op_choice < 75 {
            // Update an existing key (50%)
            let idx = rng.random_range(0..keys.len());
            let key = keys[idx].clone();
            let vlen: usize = rng.random_range(16..=64);
            let new_value: Vec<u8> = (0..vlen).map(|_| rng.random()).collect();
            mods.push(BatchOp::Put {
                key,
                value: new_value,
            });
        } else {
            // Insert a new key (25%)
            let key = loop {
                let klen: usize = rng.random_range(8..=32);
                let candidate: Vec<u8> = (0..klen).map(|_| rng.random()).collect();
                if seen.insert(candidate.clone()) {
                    break candidate;
                }
            };
            let vlen: usize = rng.random_range(16..=64);
            let value: Vec<u8> = (0..vlen).map(|_| rng.random()).collect();
            keys.push(key.clone());
            values.push(value.clone());
            mods.push(BatchOp::Put { key, value });
        }
    }

    println!("Committing modified revision...");
    let t_modify_start = Instant::now();
    let proposal2 = db.propose(mods)?;
    let modified_hash = proposal2
        .root_hash()?
        .expect("modified proposal should have a root hash");
    proposal2.commit()?;
    let modify_duration = t_modify_start.elapsed();

    println!(
        "Modified commit done: {} modifications in {:?}",
        args.modify, modify_duration
    );

    // ------------------------------------------------------------------------
    // Step 3: Build Merkles for both revisions and run diffs
    // ------------------------------------------------------------------------
    println!("\nBuilding Merkle views for diff...");
    let left_store = db.revision(base_hash.clone())?;
    let right_store = db.revision(modified_hash.clone())?;

    let left_merkle: Merkle<_> = Merkle::from(left_store);
    let right_merkle: Merkle<_> = Merkle::from(right_store);

    let start_key: Key = Box::new([]);

    println!("\nRunning simple diff...");
    let t_simple_start = Instant::now();
    let (ops_simple, simple_left_nodes, simple_right_nodes) =
        diff_merkle_simple(&left_merkle, &right_merkle, start_key.clone());
    let t_simple = t_simple_start.elapsed();

    println!("Running optimized diff...");
    let t_opt_start = Instant::now();
    let mut opt_iter = diff_merkle_optimized(&left_merkle, &right_merkle, start_key);
    let ops_optimized: Vec<_> = opt_iter.by_ref().collect();
    let t_opt = t_opt_start.elapsed();

    println!("Running parallel optimized diff...");
    let t_par_start = Instant::now();
    let (ops_parallel, par_metrics) = ParallelDiff::diff(&left_merkle, &right_merkle, Box::new([]));
    let t_par = t_par_start.elapsed();

    // ------------------------------------------------------------------------
    // Step 4: Print metrics and relative efficiency
    // ------------------------------------------------------------------------
    let simple_total_nodes = simple_left_nodes + simple_right_nodes;
    let opt_nodes_visited = opt_iter.nodes_visited;
    let opt_nodes_pruned = opt_iter.nodes_pruned;
    let opt_subtrees_skipped = opt_iter.subtrees_skipped;
    let par_nodes_visited = par_metrics.nodes_visited;
    let par_nodes_pruned = par_metrics.nodes_pruned;
    let par_subtrees_skipped = par_metrics.subtrees_skipped;

    println!("\n=== Diff Metrics ===");
    println!("Base keys:              {}", args.items);
    println!("Requested modifications: {}", args.modify);
    println!("Seed:                   {seed}");
    println!("Base root hash:         {base_hash:?}");
    println!("Modified root hash:     {modified_hash:?}");

    println!("\nSimple diff:");
    println!("  Operations:           {}", ops_simple.len());
    println!("  Left nodes visited:   {}", simple_left_nodes);
    println!("  Right nodes visited:  {}", simple_right_nodes);
    println!("  Total nodes visited:  {}", simple_total_nodes);
    println!("  Elapsed:              {:?}", t_simple);

    println!("\nOptimized diff:");
    println!("  Operations:           {}", ops_optimized.len());
    println!("  Nodes visited:        {}", opt_nodes_visited);
    println!("  Nodes pruned:         {}", opt_nodes_pruned);
    println!("  Subtrees skipped:     {}", opt_subtrees_skipped);
    println!("  Elapsed:              {:?}", t_opt);

    if opt_nodes_visited > 0 {
        let prune_rate =
            opt_nodes_pruned as f64 / opt_nodes_visited as f64 * 100.0;
        println!("  Pruning rate:         {:.1}%", prune_rate);
    }

    if simple_total_nodes > 0 && opt_nodes_visited > 0 {
        let traversal_reduction =
            100.0 - (opt_nodes_visited as f64 / simple_total_nodes as f64 * 100.0);
        println!(
            "Traversal reduction vs simple: {:.1}%",
            traversal_reduction
        );
    }

    if !ops_simple.is_empty() && !ops_optimized.is_empty() {
        let t_simple_s = t_simple.as_secs_f64();
        let t_opt_s = t_opt.as_secs_f64();
        if t_simple_s > 0.0 && t_opt_s > 0.0 {
            let speedup = t_simple_s / t_opt_s;
            println!("Time speedup (simple / optimized): {:.2}x", speedup);
        }
    }

    println!("\nParallel optimized diff:");
    println!("  Operations:           {}", ops_parallel.len());
    println!("  Nodes visited:        {}", par_nodes_visited);
    println!("  Nodes pruned:         {}", par_nodes_pruned);
    println!("  Subtrees skipped:     {}", par_subtrees_skipped);
    println!("  Elapsed:              {:?}", t_par);

    if par_nodes_visited > 0 {
        let prune_rate =
            par_nodes_pruned as f64 / par_nodes_visited as f64 * 100.0;
        println!("  Pruning rate:         {:.1}%", prune_rate);
    }

    if simple_total_nodes > 0 && par_nodes_visited > 0 {
        let traversal_reduction =
            100.0 - (par_nodes_visited as f64 / simple_total_nodes as f64 * 100.0);
        println!(
            "Traversal reduction vs simple: {:.1}%",
            traversal_reduction
        );
    }

    if !ops_optimized.is_empty() && !ops_parallel.is_empty() {
        let t_opt_s = t_opt.as_secs_f64();
        let t_par_s = t_par.as_secs_f64();
        if t_opt_s > 0.0 && t_par_s > 0.0 {
            let speedup = t_opt_s / t_par_s;
            println!("Time speedup (optimized / parallel): {:.2}x", speedup);
        }
    }

    if ops_optimized != ops_parallel {
        println!("WARNING: parallel optimized diff produced different ops than single-threaded optimized diff");
    }

    Ok(())
}
