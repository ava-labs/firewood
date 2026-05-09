// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Concurrent `commit_with_rebase` correctness.
//!
//! Multiple threads each propose a disjoint batch off the same parent and
//! call `commit_with_rebase`. The first commit wins outright; the rest see
//! a stale parent and rebase. After all threads finish, the database must:
//!   - have committed every batch (no lost updates),
//!   - pass `check()` with no freelist or structural errors, and
//!   - reopen cleanly without surfacing "Invalid `FreeArea` marker" errors.
//!
//! Tracks issue #1985.

use std::thread;

use firewood_storage::{CheckOpt, CheckerError};

use crate::api::{Db as _, DbView as _, Proposal as _};
use crate::db::{BatchOp, DbConfig};

use super::TestDb;

/// Number of concurrent commit threads. Each commits a single batch with
/// a disjoint keyspace, so all batches can succeed (one direct commit,
/// the rest rebased).
const THREADS: usize = 8;

/// Number of keys per thread. Larger batches widen the time window in which
/// concurrent rebases interleave.
const KEYS_PER_THREAD: usize = 16;

fn key(thread_idx: usize, key_idx: usize) -> Vec<u8> {
    let mut k = Vec::with_capacity(8);
    k.extend_from_slice(b"t");
    k.extend_from_slice(&(thread_idx as u32).to_be_bytes());
    k.extend_from_slice(b"k");
    k.extend_from_slice(&(key_idx as u32).to_be_bytes());
    k
}

fn value(thread_idx: usize, key_idx: usize) -> Vec<u8> {
    format!("v{thread_idx}-{key_idx}").into_bytes()
}

#[test]
#[ignore = "reproduces issue #1985: concurrent commit_with_rebase corrupts freelist/index"]
fn test_concurrent_commit_with_rebase_no_freelist_corruption() {
    let db = TestDb::new_with_config(DbConfig::builder().build());

    // Seed with a small initial commit so the freelist has some starting state.
    let seed: Vec<BatchOp<&[u8], &[u8]>> = vec![BatchOp::Put {
        key: b"seed",
        value: b"v",
    }];
    db.propose(seed).unwrap().commit().unwrap();

    // Spawn THREADS concurrent committers. Each builds a proposal off the
    // current revision and calls commit_with_rebase. With disjoint key sets,
    // every commit must eventually succeed (one direct, the rest via rebase).
    thread::scope(|s| {
        let mut handles = Vec::with_capacity(THREADS);
        for t in 0..THREADS {
            let db_ref: &crate::db::Db = &db;
            handles.push(s.spawn(move || {
                let batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> = (0..KEYS_PER_THREAD)
                    .map(|k| BatchOp::Put {
                        key: key(t, k),
                        value: value(t, k),
                    })
                    .collect();
                let proposal = db_ref.propose(batch).expect("propose");
                proposal
                    .commit_with_rebase()
                    .expect("commit_with_rebase must not error under contention")
            }));
        }
        for h in handles {
            let _ = h.join().expect("thread panicked");
        }
    });

    // Every key from every thread must be present in the latest revision.
    let latest_root = <crate::db::Db as crate::api::Db>::root_hash(&db).unwrap();
    let view = <crate::db::Db as crate::api::Db>::revision(&db, latest_root).unwrap();
    for t in 0..THREADS {
        for k in 0..KEYS_PER_THREAD {
            let got = view.val(key(t, k)).unwrap();
            assert_eq!(
                got.as_deref(),
                Some(value(t, k).as_slice()),
                "missing key from thread {t}, key idx {k} — a concurrent commit_with_rebase \
                 must not lose updates"
            );
        }
    }
    drop(view);

    // No freelist or structural corruption.
    let report = db.check(CheckOpt {
        hash_check: true,
        progress_bar: None,
    });
    let real_errors: Vec<_> = report
        .errors
        .iter()
        .filter(|e| !matches!(e, CheckerError::AreaLeaks(_)))
        .collect();
    assert!(
        real_errors.is_empty(),
        "post-concurrency check found errors: {real_errors:?}"
    );

    // Reopen to surface any deferred persistence corruption (e.g. the
    // "Invalid `FreeArea` marker" the Go report mentioned).
    let db = db.reopen();
    let latest_root = <crate::db::Db as crate::api::Db>::root_hash(&db).unwrap();
    let view = <crate::db::Db as crate::api::Db>::revision(&db, latest_root).unwrap();
    for t in 0..THREADS {
        for k in 0..KEYS_PER_THREAD {
            let got = view.val(key(t, k)).unwrap();
            assert_eq!(
                got.as_deref(),
                Some(value(t, k).as_slice()),
                "missing key after reopen from thread {t}, key idx {k}"
            );
        }
    }
    drop(view);
    let report = db.check(CheckOpt {
        hash_check: true,
        progress_bar: None,
    });
    let real_errors: Vec<_> = report
        .errors
        .iter()
        .filter(|e| !matches!(e, CheckerError::AreaLeaks(_)))
        .collect();
    assert!(
        real_errors.is_empty(),
        "post-reopen check found errors: {real_errors:?}"
    );
}
