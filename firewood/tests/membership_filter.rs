// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! End-to-end checks for the membership filter's soundness rules.
//!
//! The filter is configured by environment and bound once per process, so
//! each scenario runs in a child process: the parent re-executes this test
//! binary with the scenario name in `FILTER_TEST_SCENARIO` and the filter
//! variables set, and the `scenario` test dispatches on it.

#![cfg(feature = "filter")]
#![expect(clippy::unwrap_used, reason = "test code")]

use std::path::{Path, PathBuf};
use std::process::Command;

use firewood::api::{self, DynDb};
use firewood::db::{BatchOp, DbConfig};
use firewood::open;
use firewood_storage::filter::load_counting;
use firewood_storage::{DefaultHashMode, HashMode, TrieHash};

const SCENARIO: &str = "FILTER_TEST_SCENARIO";
const DIR: &str = "FILTER_TEST_DIR";

fn open_db(dir: &Path) -> Box<dyn DynDb> {
    let cfg = DbConfig::builder()
        .node_hash_algorithm(DefaultHashMode::ALGORITHM)
        .truncate(false)
        .build();
    open(dir.join("db"), cfg).unwrap()
}

fn key(i: u32) -> Vec<u8> {
    format!("key-{i:08}").into_bytes()
}

fn insert_range(db: &dyn DynDb, range: std::ops::Range<u32>) {
    let batch: Vec<_> = range
        .map(|i| BatchOp::Put {
            key: key(i),
            value: i.to_le_bytes().to_vec(),
        })
        .collect();
    db.propose(api::collect_owned_batch(batch).unwrap())
        .unwrap()
        .commit()
        .unwrap();
}

fn assert_reads(db: &dyn DynDb, present: std::ops::Range<u32>, absent: std::ops::Range<u32>) {
    let rev = db.revision(db.root_hash().unwrap()).unwrap();
    for i in present {
        assert_eq!(
            rev.val(&key(i)).unwrap().as_deref(),
            Some(i.to_le_bytes().as_slice()),
            "present key {i} must be found"
        );
    }
    for i in absent {
        assert_eq!(
            rev.val(&key(i)).unwrap(),
            None,
            "absent key {i} must be None"
        );
    }
}

fn filter_tag(path: &Path) -> Option<TrieHash> {
    load_counting(path).unwrap().1
}

/// Fresh empty database + `FIREWOOD_FILTER_EXPECTED_KEYS`: the filter is
/// enabled, reads stay correct in skip mode, and close writes a checkpoint
/// tagged with the closing root.
fn fresh_db_enables_and_checkpoints_on_close(dir: &Path) {
    let db = open_db(dir);
    assert!(firewood::membership_filter_enabled());
    insert_range(db.as_ref(), 0..500);
    assert_reads(db.as_ref(), 0..500, 500..1000);
    let root = db.root_hash();
    db.close().unwrap();
    assert_eq!(filter_tag(&dir.join("f.fwdfilter")), root);
}

/// Reopening at the root the checkpoint was tagged with loads it; further
/// commits and a clean close re-tag it.
fn reopen_matching_root_loads(dir: &Path) {
    let db = open_db(dir);
    assert!(firewood::membership_filter_enabled());
    assert_reads(db.as_ref(), 0..500, 500..1000);
    insert_range(db.as_ref(), 500..800);
    assert_reads(db.as_ref(), 0..800, 800..1000);
    let root = db.root_hash();
    db.close().unwrap();
    assert_eq!(filter_tag(&dir.join("f.fwdfilter")), root);
}

/// A checkpoint tagged with a root the database is not at is refused, and
/// reads fall back to walking the trie.
fn stale_checkpoint_is_refused(dir: &Path) {
    let db = open_db(dir);
    assert!(!firewood::membership_filter_enabled());
    assert_reads(db.as_ref(), 0..800, 800..1000);
    db.close().unwrap();
}

/// `FIREWOOD_FILTER_EXPECTED_KEYS` without a checkpoint is refused for a
/// non-empty database: an empty filter would call every existing key absent.
fn empty_filter_refused_for_nonempty_db(dir: &Path) {
    let db = open_db(dir);
    assert!(!firewood::membership_filter_enabled());
    assert_reads(db.as_ref(), 0..800, 800..1000);
    db.close().unwrap();
}

#[test]
fn scenario() {
    let Ok(name) = std::env::var(SCENARIO) else {
        return;
    };
    let dir = PathBuf::from(std::env::var(DIR).unwrap());
    match name.as_str() {
        "fresh" => fresh_db_enables_and_checkpoints_on_close(&dir),
        "reopen" => reopen_matching_root_loads(&dir),
        "stale" => stale_checkpoint_is_refused(&dir),
        "empty_refused" => empty_filter_refused_for_nonempty_db(&dir),
        other => panic!("unknown scenario {other}"),
    }
    // Proves to the parent that the scenario ran, not just that the child exited 0.
    std::fs::write(dir.join(format!("ran-{name}")), b"").unwrap();
}

fn run_scenario(name: &str, dir: &Path, filter_path: &Path, extra: &[(&str, &str)]) {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", "scenario", "--nocapture"])
        .env(SCENARIO, name)
        .env(DIR, dir)
        .env("FIREWOOD_FILTER_PATH", filter_path)
        .env("FIREWOOD_FILTER_CHECKPOINT", "0");
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let output = cmd.output().unwrap();
    assert!(
        output.status.success() && dir.join(format!("ran-{name}")).exists(),
        "scenario {name} failed or did not run:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn checkpoint_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let filter = dir.join("f.fwdfilter");
    let sized = [("FIREWOOD_FILTER_EXPECTED_KEYS", "10000")];

    run_scenario("fresh", dir, &filter, &sized);
    run_scenario("reopen", dir, &filter, &[]);

    // Retag the checkpoint with a root the database never had.
    let (f, _) = load_counting(&filter).unwrap();
    firewood_storage::filter::MembershipFilter::save(
        &f,
        &filter,
        Some(&TrieHash::from_bytes([0x42; 32])),
    )
    .unwrap();
    run_scenario("stale", dir, &filter, &[]);

    // No checkpoint at all, but a request to size a fresh one.
    std::fs::remove_file(&filter).unwrap();
    run_scenario("empty_refused", dir, &filter, &sized);
}
