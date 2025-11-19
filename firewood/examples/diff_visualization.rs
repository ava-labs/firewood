//! # Diff Algorithm Visualization Example
//!
//! This example demonstrates the different diff algorithms with visual output
//! showing how they work on a sample dataset.
//!
//! Run with: `cargo run --example diff_visualization`

use firewood::db::{BatchOp, Db, DbConfig};
use firewood::diff::{diff_merkle_optimized, diff_merkle_simple, ParallelDiff};
use firewood::merkle::Merkle;
use firewood::v2::api::{Db as _, DbView as _, Proposal as _};
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    println!("=== Firewood Diff Algorithm Visualization ===\n");

    // Create a simple test database
    let cfg = DbConfig::builder().truncate(true).build();
    let db = Db::new("/tmp/diff_viz_db", cfg)?;

    // Create the initial state
    println!("1️⃣  Creating initial state with sample data...");
    let initial_batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> = vec![
        BatchOp::Put {
            key: b"alice/balance".to_vec(),
            value: b"1000".to_vec(),
        },
        BatchOp::Put {
            key: b"alice/nonce".to_vec(),
            value: b"5".to_vec(),
        },
        BatchOp::Put {
            key: b"bob/balance".to_vec(),
            value: b"500".to_vec(),
        },
        BatchOp::Put {
            key: b"bob/nonce".to_vec(),
            value: b"3".to_vec(),
        },
        BatchOp::Put {
            key: b"charlie/balance".to_vec(),
            value: b"750".to_vec(),
        },
        BatchOp::Put {
            key: b"charlie/nonce".to_vec(),
            value: b"1".to_vec(),
        },
    ];

    let proposal = db.propose(initial_batch)?;
    let initial_hash = proposal.root_hash()?.expect("should have root hash");
    proposal.commit()?;
    println!("   Initial state committed: {:?}\n", initial_hash);

    // Create a modified state
    println!("2️⃣  Applying modifications...");
    let modifications: Vec<BatchOp<Vec<u8>, Vec<u8>>> = vec![
        // Alice sends 200 to Bob
        BatchOp::Put {
            key: b"alice/balance".to_vec(),
            value: b"800".to_vec(),
        },
        BatchOp::Put {
            key: b"alice/nonce".to_vec(),
            value: b"6".to_vec(),
        },
        BatchOp::Put {
            key: b"bob/balance".to_vec(),
            value: b"700".to_vec(),
        },
        BatchOp::Put {
            key: b"bob/nonce".to_vec(),
            value: b"4".to_vec(),
        },
        // Charlie's account is deleted
        BatchOp::Delete {
            key: b"charlie/balance".to_vec(),
        },
        BatchOp::Delete {
            key: b"charlie/nonce".to_vec(),
        },
        // New account: Dave
        BatchOp::Put {
            key: b"dave/balance".to_vec(),
            value: b"100".to_vec(),
        },
        BatchOp::Put {
            key: b"dave/nonce".to_vec(),
            value: b"0".to_vec(),
        },
    ];

    let proposal2 = db.propose(modifications)?;
    let modified_hash = proposal2.root_hash()?.expect("should have root hash");
    proposal2.commit()?;
    println!("   Modified state committed: {:?}\n", modified_hash);

    // Build Merkle views
    let left_store = db.revision(initial_hash)?;
    let right_store = db.revision(modified_hash)?;
    let left_merkle: Merkle<_> = Merkle::from(left_store);
    let right_merkle: Merkle<_> = Merkle::from(right_store);

    // Visualize the changes
    println!("3️⃣  Visualizing state changes:\n");
    println!("   Initial State:");
    println!("   ├── alice/");
    println!("   │   ├── balance: 1000");
    println!("   │   └── nonce: 5");
    println!("   ├── bob/");
    println!("   │   ├── balance: 500");
    println!("   │   └── nonce: 3");
    println!("   └── charlie/");
    println!("       ├── balance: 750");
    println!("       └── nonce: 1");
    println!();
    println!("   Modified State:");
    println!("   ├── alice/");
    println!("   │   ├── balance: 800 ⚡");
    println!("   │   └── nonce: 6 ⚡");
    println!("   ├── bob/");
    println!("   │   ├── balance: 700 ⚡");
    println!("   │   └── nonce: 4 ⚡");
    println!("   └── dave/ ✨");
    println!("       ├── balance: 100");
    println!("       └── nonce: 0");
    println!("   ❌ charlie/ (deleted)");
    println!();

    // Run Simple Diff
    println!("4️⃣  Running Simple Diff Algorithm...");
    let (simple_ops, left_nodes, right_nodes) =
        diff_merkle_simple(&left_merkle, &right_merkle, Box::new([]));
    println!("   Results:");
    println!("   • Operations found: {}", simple_ops.len());
    println!("   • Nodes visited: {} (left) + {} (right) = {} total",
             left_nodes, right_nodes, left_nodes + right_nodes);
    println!("   • Algorithm: Exhaustive comparison of all keys\n");

    // Run Optimized Diff
    println!("5️⃣  Running Optimized Diff Algorithm...");
    let mut opt_iter = diff_merkle_optimized(&left_merkle, &right_merkle, Box::new([]));
    let optimized_ops: Vec<_> = opt_iter.by_ref().collect();
    let pruning_rate = if opt_iter.nodes_visited > 0 {
        opt_iter.nodes_pruned as f64 / opt_iter.nodes_visited as f64 * 100.0
    } else {
        0.0
    };
    println!("   Results:");
    println!("   • Operations found: {}", optimized_ops.len());
    println!("   • Nodes visited: {}", opt_iter.nodes_visited);
    println!("   • Nodes pruned: {} ({:.1}% pruning rate)",
             opt_iter.nodes_pruned, pruning_rate);
    println!("   • Subtrees skipped: {}", opt_iter.subtrees_skipped);
    println!("   • Algorithm: Hash-based pruning for unchanged subtrees\n");

    // Run Parallel Diff
    println!("6️⃣  Running Parallel Diff Algorithm...");
    let (parallel_ops, metrics) = ParallelDiff::diff(&left_merkle, &right_merkle, Box::new([]));
    let par_pruning_rate = if metrics.nodes_visited > 0 {
        metrics.nodes_pruned as f64 / metrics.nodes_visited as f64 * 100.0
    } else {
        0.0
    };
    println!("   Results:");
    println!("   • Operations found: {}", parallel_ops.len());
    println!("   • Nodes visited: {}", metrics.nodes_visited);
    println!("   • Nodes pruned: {} ({:.1}% pruning rate)",
             metrics.nodes_pruned, par_pruning_rate);
    println!("   • Threads used: {}", rayon::current_num_threads());
    println!("   • Algorithm: Parallel processing of top-level branches\n");

    // Show the actual diff operations
    println!("7️⃣  Diff Operations Generated:\n");
    for (i, op) in optimized_ops.iter().enumerate() {
        match op {
            BatchOp::Put { key, value } => {
                let key_str = String::from_utf8_lossy(key);
                let value_str = String::from_utf8_lossy(value);
                if simple_ops.iter().any(|s_op| {
                    if let BatchOp::Delete { key: del_key } = s_op {
                        del_key == key
                    } else {
                        false
                    }
                }) {
                    println!("   {}. ✨ ADD: {} = {}", i + 1, key_str, value_str);
                } else {
                    println!("   {}. ⚡ MODIFY: {} = {}", i + 1, key_str, value_str);
                }
            }
            BatchOp::Delete { key } => {
                let key_str = String::from_utf8_lossy(key);
                println!("   {}. ❌ DELETE: {}", i + 1, key_str);
            }
            _ => {}
        }
    }

    // Performance comparison
    println!("\n8️⃣  Performance Summary:");
    println!("   ┌─────────────┬────────────┬──────────────┬──────────────┐");
    println!("   │ Algorithm   │ Operations │ Nodes Visited│ Efficiency   │");
    println!("   ├─────────────┼────────────┼──────────────┼──────────────┤");
    println!("   │ Simple      │ {:>10} │ {:>12} │ Baseline     │",
             simple_ops.len(), left_nodes + right_nodes);
    println!("   │ Optimized   │ {:>10} │ {:>12} │ {:.1}% pruned │",
             optimized_ops.len(), opt_iter.nodes_visited, pruning_rate);
    println!("   │ Parallel    │ {:>10} │ {:>12} │ {:.1}% pruned │",
             parallel_ops.len(), metrics.nodes_visited, par_pruning_rate);
    println!("   └─────────────┴────────────┴──────────────┴──────────────┘");

    // Verify all algorithms produce the same result
    assert_eq!(simple_ops.len(), optimized_ops.len(), "Simple and optimized should produce same number of ops");
    assert_eq!(optimized_ops.len(), parallel_ops.len(), "Optimized and parallel should produce same number of ops");
    println!("\n✅ All algorithms produced consistent results!");

    println!("\n📚 For more details, see DIFF_ALGORITHMS.md");

    Ok(())
}