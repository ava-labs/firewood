// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Simple diff performance probe for the change-proof diff implementation.
//!
//! This example:
//! - Creates a base revision with random key/value pairs.
//! - Applies a second batch of random modifications on top.
//! - Runs the change-proof diff between the two revisions.
//! - Reports elapsed time and the total number of trie nodes touched
//!   (across both tries) during the diff traversal.
//!
//! Usage (from the workspace root):
//! ```bash
//! cargo run --release -p firewood --example diff_metrics -- \
//!     --items 100_000 --modify 20_000 --db-path diff_db
//! ```

use clap::Parser;
use rand::{Rng, SeedableRng, rngs::StdRng};
use std::collections::HashSet;
use std::error::Error;
use std::num::NonZeroUsize;
use std::time::Instant;

use firewood::db::{BatchOp, Db, DbConfig};
use firewood::manager::RevisionManagerConfig;
use firewood::merkle::{Key, Merkle};
use firewood::proofs::{diff_merkle_iterator, diff_merkle_iterator_without_hash};
use firewood::v2::api::{Db as _, DbView as _, Proposal as _};

use metrics::{Key as MetricsKey, Label, Recorder};
use metrics_util::registry::{AtomicStorage, Registry};

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

/// Simple metrics recorder that lets us read back counter values.
#[derive(Clone)]
struct CliRecorder {
    registry: std::sync::Arc<Registry<MetricsKey, AtomicStorage>>,
}

impl CliRecorder {
    fn new() -> Self {
        Self {
            registry: std::sync::Arc::new(Registry::atomic()),
        }
    }

    fn get_counter_value(
        &self,
        key_name: &'static str,
        labels: &[(&'static str, &'static str)],
    ) -> u64 {
        use std::sync::atomic::Ordering;

        let key = if labels.is_empty() {
            MetricsKey::from_name(key_name)
        } else {
            let label_vec: Vec<Label> = labels.iter().map(|(k, v)| Label::new(*k, *v)).collect();
            MetricsKey::from_name(key_name).with_extra_labels(label_vec)
        };

        self.registry
            .get_counter_handles()
            .into_iter()
            .find(|(k, _)| k == &key)
            .map_or(0, |(_, counter)| counter.load(Ordering::Relaxed))
    }
}

impl Recorder for CliRecorder {
    fn describe_counter(
        &self,
        _key: metrics::KeyName,
        _unit: Option<metrics::Unit>,
        _description: metrics::SharedString,
    ) {
    }

    fn describe_gauge(
        &self,
        _key: metrics::KeyName,
        _unit: Option<metrics::Unit>,
        _description: metrics::SharedString,
    ) {
    }

    fn describe_histogram(
        &self,
        _key: metrics::KeyName,
        _unit: Option<metrics::Unit>,
        _description: metrics::SharedString,
    ) {
    }

    fn register_counter(
        &self,
        key: &MetricsKey,
        _metadata: &metrics::Metadata<'_>,
    ) -> metrics::Counter {
        self.registry
            .get_or_create_counter(key, |c| c.clone().into())
    }

    fn register_gauge(
        &self,
        key: &MetricsKey,
        _metadata: &metrics::Metadata<'_>,
    ) -> metrics::Gauge {
        self.registry.get_or_create_gauge(key, |c| c.clone().into())
    }

    fn register_histogram(
        &self,
        key: &MetricsKey,
        _metadata: &metrics::Metadata<'_>,
    ) -> metrics::Histogram {
        self.registry
            .get_or_create_histogram(key, |c| c.clone().into())
    }
}

/// Generates a set of unique random key-value pairs.
fn generate_unique_pairs(
    rng: &mut StdRng,
    count: usize,
) -> (Vec<Vec<u8>>, Vec<Vec<u8>>, HashSet<Vec<u8>>) {
    let mut keys: Vec<Vec<u8>> = Vec::with_capacity(count);
    let mut values: Vec<Vec<u8>> = Vec::with_capacity(count);
    let mut seen = HashSet::with_capacity(count * 2);

    println!("Generating {count} unique random keys...");
    while keys.len() < count {
        let key_length: usize = rng.random_range(8..=32);
        let value_length: usize = rng.random_range(16..=64);

        let key: Vec<u8> = (0..key_length).map(|_| rng.random()).collect();
        if !seen.insert(key.clone()) {
            continue;
        }

        let value: Vec<u8> = (0..value_length).map(|_| rng.random()).collect();
        keys.push(key);
        values.push(value);
    }

    (keys, values, seen)
}

/// Creates a batch operation for the initial base state.
fn create_base_batch(keys: &[Vec<u8>], values: &[Vec<u8>]) -> Vec<BatchOp<Vec<u8>, Vec<u8>>> {
    keys.iter()
        .cloned()
        .zip(values.iter().cloned())
        .map(|(key, value)| BatchOp::Put { key, value })
        .collect()
}

/// Generates random modifications (inserts, updates, deletes) to apply to the base state.
fn generate_modifications(
    rng: &mut StdRng,
    keys: &mut Vec<Vec<u8>>,
    values: &mut Vec<Vec<u8>>,
    seen: &mut HashSet<Vec<u8>>,
    modification_count: usize,
) -> Vec<BatchOp<Vec<u8>, Vec<u8>>> {
    let mut modifications: Vec<BatchOp<Vec<u8>, Vec<u8>>> = Vec::with_capacity(modification_count);

    println!(
        "Generating {modification_count} random modifications (deletes / updates / inserts)..."
    );

    for _ in 0..modification_count {
        let operation_choice = rng.random_range(0..100);

        if !keys.is_empty() && operation_choice < 25 {
            // Delete an existing key (25% probability)
            let index = rng.random_range(0..keys.len());
            let key = keys[index].clone();
            modifications.push(BatchOp::Delete { key });
        } else if !keys.is_empty() && operation_choice < 75 {
            // Update an existing key (50% probability)
            let index = rng.random_range(0..keys.len());
            let key = keys[index].clone();
            let value_length: usize = rng.random_range(16..=64);
            let new_value: Vec<u8> = (0..value_length).map(|_| rng.random()).collect();
            modifications.push(BatchOp::Put {
                key,
                value: new_value,
            });
        } else {
            // Insert a new key (25% probability)
            let key = loop {
                let key_length: usize = rng.random_range(8..=32);
                let candidate: Vec<u8> = (0..key_length).map(|_| rng.random()).collect();
                if seen.insert(candidate.clone()) {
                    break candidate;
                }
            };
            let value_length: usize = rng.random_range(16..=64);
            let value: Vec<u8> = (0..value_length).map(|_| rng.random()).collect();
            keys.push(key.clone());
            values.push(value.clone());
            modifications.push(BatchOp::Put { key, value });
        }
    }

    modifications
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    println!(
        "Creating base state with {} keys, then applying {} modifications",
        args.items, args.modify
    );

    // Set up metrics recorder so we can read node traversal counts.
    let recorder = CliRecorder::new();
    metrics::set_global_recorder(recorder.clone())?;

    // Configure and open the database
    let manager_config = RevisionManagerConfig::builder()
        .node_cache_size(args.cache_size)
        .max_revisions(args.revisions)
        .build();
    let database_config = DbConfig::builder()
        .truncate(args.truncate)
        .manager(manager_config)
        .build();
    let db = Db::new(&args.db_path, database_config)?;

    // Build deterministic RNG for reproducible results
    let seed = args.seed.unwrap_or(0xD1FF_C0DE_BAAD_F00D_u64);
    let mut rng = StdRng::seed_from_u64(seed);

    // ------------------------------------------------------------------------
    // Step 1: Build a large base state and commit it
    // ------------------------------------------------------------------------
    let (mut keys, mut values, mut seen) = generate_unique_pairs(&mut rng, args.items);
    let base_batch = create_base_batch(&keys, &values);

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
    let modifications =
        generate_modifications(&mut rng, &mut keys, &mut values, &mut seen, args.modify);

    println!("Committing modified revision...");
    let t_modify_start = Instant::now();
    let proposal2 = db.propose(modifications)?;
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
    // Step 3: Build Merkles for both revisions and run the change-proof diff
    // ------------------------------------------------------------------------
    println!("\nBuilding Merkle views for diff...");
    let left_store = db.revision(base_hash.clone())?;
    let right_store = db.revision(modified_hash.clone())?;

    let left_merkle: Merkle<_> = Merkle::from(left_store);
    let right_merkle: Merkle<_> = Merkle::from(right_store);

    let start_key: Key = Box::new([]);

    println!("\nRunning change-proof diff...");
    let t_diff_start = Instant::now();

    let nodes_before = recorder.get_counter_value("firewood.diff_merkle", &[("traversal", "next")]);

    let diff_iter = diff_merkle_iterator_without_hash(&left_merkle, &right_merkle, start_key)?;
    let ops: Vec<_> = diff_iter.collect::<Result<Vec<_>, _>>()?;

    let nodes_after = recorder.get_counter_value("firewood.diff_merkle", &[("traversal", "next")]);
    let diff_duration = t_diff_start.elapsed();

    let nodes_touched = nodes_after.saturating_sub(nodes_before);

    // ------------------------------------------------------------------------
    // Step 4: Print metrics and relative efficiency
    // ------------------------------------------------------------------------
    println!("\n=== Change-Proof Diff Metrics ===");
    println!("Base keys:              {}", args.items);
    println!("Requested modifications: {}", args.modify);
    println!("Seed:                   {seed}");
    println!("Base root hash:         {base_hash:?}");
    println!("Modified root hash:     {modified_hash:?}");

    println!("\nChange-proof diff:");
    println!("  Operations:           {}", ops.len());
    println!("  Nodes touched (total): {nodes_touched}");
    println!("  Elapsed:              {:?}", diff_duration);

    Ok(())
}
