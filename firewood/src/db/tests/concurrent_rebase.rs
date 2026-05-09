// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Edge cases of `commit_with_rebase` that show up during avalanchego-style
//! sync, where many proposals are created off the same parent and serialize
//! through the manager via rebase. Each test pins one shape of the parent-
//! identity contract:
//!
//! - `test_concurrent_commit_with_rebase_no_freelist_corruption`: many
//!   threads commit disjoint batches under contention. After all commits,
//!   every key is present and the freelist is intact across reopen.
//!
//! - `test_concurrent_rebase_overlapping_paths`: many threads build
//!   proposals off the *same* parent (barrier-synced) and modify random
//!   keys whose tries share intermediate branches. Forces the manager to
//!   resolve sibling proposals via rebase rather than letting them all
//!   commit, which is the exact pattern that corrupted the freelist before
//!   `CommittedId` parent identity.
//!
//! - `test_trivial_rebase_succeeds_without_corruption`: two proposals
//!   making the same `Put` race to commit. After the first commits, the
//!   second's rebase produces a no-op (hash-identical to current). The
//!   trivial-commit fast path must absorb this without pushing a duplicate
//!   revision; otherwise the duplicate's deleted list aliases current's
//!   root address and reaping double-frees it.
//!
//! - `test_round_trip_rejects_stale_parent`: an A→B→A sequence produces
//!   two committed revisions with the same root hash but different
//!   `CommittedId`s. A proposal pinned to the first "A" must rebase rather
//!   than commit directly against the second "A" — its deleted list refers
//!   to nodes from the first A, which may already have been freed.

use std::thread;

use firewood_storage::{CheckOpt, CheckerError};

use crate::api::{Db as _, DbView as _, Proposal as _};
use crate::db::{BatchOp, DbConfig};
use crate::manager::RevisionManagerConfig;

use super::TestDb;

/// Number of concurrent commit threads. Each commits a single batch with
/// a disjoint keyspace, so all batches can succeed (one direct commit,
/// the rest rebased).
const THREADS: usize = 8;

/// Number of commits per thread. Set high enough that the total number of
/// commits exceeds the manager's `max_revisions` so that reaping fires
/// during the concurrent phase.
const COMMITS_PER_THREAD: usize = 32;

/// Number of keys per batch. Larger batches widen the time window in which
/// concurrent rebases interleave.
const KEYS_PER_BATCH: usize = 16;

/// Tighten `max_revisions` so reaping kicks in during the test.
const MAX_REVISIONS: usize = 16;

// Keys must be 32 bytes (64 nibbles) so the checker accepts them under
// the `ethhash` feature, where valid keys are sized like Ethereum account
// or storage trie entries.
#[expect(clippy::indexing_slicing, clippy::disallowed_methods)]
fn key(thread_idx: usize, commit_idx: usize, key_idx: usize) -> Vec<u8> {
    let mut k = vec![0u8; 32];
    k[..8].copy_from_slice(&(thread_idx as u64).to_be_bytes());
    k[8..16].copy_from_slice(&(commit_idx as u64).to_be_bytes());
    k[24..].copy_from_slice(&(key_idx as u64).to_be_bytes());
    k
}

fn value(thread_idx: usize, commit_idx: usize, key_idx: usize) -> Vec<u8> {
    format!("v{thread_idx}-{commit_idx}-{key_idx}").into_bytes()
}

/// `TestDb` with a tight `max_revisions` so reaping fires during the test.
fn test_db_with_tight_revisions() -> TestDb {
    TestDb::new_with_config(
        DbConfig::builder()
            .manager(
                RevisionManagerConfig::builder()
                    .max_revisions(MAX_REVISIONS)
                    .build(),
            )
            .build(),
    )
}

/// Run `db.check()` and assert no errors other than `AreaLeaks`.
fn assert_check_clean(db: &TestDb, label: &str) {
    let report = db.check(CheckOpt {
        hash_check: true,
        progress_bar: None,
    });
    let real_errors: Vec<_> = report
        .errors
        .iter()
        .filter(|e| !matches!(e, CheckerError::AreaLeaks(_)))
        .collect();
    assert!(real_errors.is_empty(), "{label}: {real_errors:?}");
}

/// Verify every disjoint `(t, c, k)` key produced by [`key`]/[`value`] is
/// present in `db`'s latest revision with the expected value.
fn assert_disjoint_keys_present(db: &TestDb, label: &str) {
    let latest_root = <crate::db::Db as crate::api::Db>::root_hash(db).unwrap();
    let view = <crate::db::Db as crate::api::Db>::revision(db, latest_root).unwrap();
    for t in 0..THREADS {
        for c in 0..COMMITS_PER_THREAD {
            for k in 0..KEYS_PER_BATCH {
                let got = view.val(key(t, c, k)).unwrap_or_else(|e| {
                    panic!(
                        "{label}: val() lookup failed at thread {t}, commit {c}, key idx {k}: {e:?}"
                    )
                });
                assert_eq!(
                    got.as_deref(),
                    Some(value(t, c, k).as_slice()),
                    "{label}: value mismatch at thread {t}, commit {c}, key idx {k}"
                );
            }
        }
    }
}

#[test]
fn test_concurrent_commit_with_rebase_no_freelist_corruption() {
    let db = test_db_with_tight_revisions();

    // Seed with a small initial commit so the freelist has some starting state.
    let seed_key = vec![0xffu8; 32];
    db.propose(vec![BatchOp::Put {
        key: seed_key,
        value: b"v".to_vec(),
    }])
    .unwrap()
    .commit()
    .unwrap();

    // Spawn THREADS concurrent committers. Each runs COMMITS_PER_THREAD
    // commit_with_rebase calls; total commits = THREADS * COMMITS_PER_THREAD,
    // chosen to exceed MAX_REVISIONS so reaping fires during the concurrent
    // phase.
    thread::scope(|s| {
        let mut handles = Vec::with_capacity(THREADS);
        for t in 0..THREADS {
            let db_ref: &crate::db::Db = &db;
            handles.push(s.spawn(move || {
                for c in 0..COMMITS_PER_THREAD {
                    let batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> = (0..KEYS_PER_BATCH)
                        .map(|k| BatchOp::Put {
                            key: key(t, c, k),
                            value: value(t, c, k),
                        })
                        .collect();
                    let proposal = db_ref.propose(batch).expect("propose");
                    proposal
                        .commit_with_rebase()
                        .expect("commit_with_rebase must not error under contention");
                }
            }));
        }
        for h in handles {
            h.join().expect("thread panicked");
        }
    });

    assert_disjoint_keys_present(&db, "post-concurrency");
    assert_check_clean(&db, "post-concurrency");

    // Reopen to surface any deferred-persistence corruption.
    let db = db.reopen();
    assert_disjoint_keys_present(&db, "post-reopen");
    assert_check_clean(&db, "post-reopen");
}

/// Many concurrent proposals built off the same parent, each modifying
/// random keys distributed across the entire keyspace so threads' writes
/// share intermediate branch nodes. `max_revisions` is set low enough that
/// reaping fires during the concurrent phase.
#[test]
fn test_concurrent_rebase_overlapping_paths() {
    use std::sync::Barrier;

    const SHARED_THREADS: usize = 8;
    const SHARED_ROUNDS: usize = 32;
    const SHARED_KEYS_PER_BATCH: usize = 64;

    // xorshift PRNG so each thread gets a deterministic-but-varying key set
    // that's sprinkled across the keyspace. Different threads modify keys
    // whose paths share branches at multiple levels.
    fn random_key(seed: &mut u64) -> Vec<u8> {
        let mut k = vec![0u8; 32];
        for byte in &mut k {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            *byte = (*seed) as u8;
        }
        k
    }

    let db = test_db_with_tight_revisions();

    // Seed with a meaningful initial trie so subsequent proposals' COWs hit
    // shared branches at upper levels.
    let mut seed: Vec<BatchOp<Vec<u8>, Vec<u8>>> = Vec::with_capacity(64);
    for i in 0u8..64 {
        let mut k = vec![0u8; 32];
        #[expect(clippy::indexing_slicing, reason = "k is a 32-byte vec")]
        {
            k[0] = i.wrapping_mul(4); // spread across the first nibble
            k[31] = i;
        }
        seed.push(BatchOp::Put {
            key: k,
            value: vec![i; 8],
        });
    }
    db.propose(seed).unwrap().commit().unwrap();

    // Use a barrier so all threads create their proposal off the SAME parent,
    // then race to commit. This maximizes the chance that multiple proposals
    // against the same parent COW the same intermediate branches.
    let barrier = Barrier::new(SHARED_THREADS);

    thread::scope(|s| {
        let mut handles = Vec::with_capacity(SHARED_THREADS);
        for t in 0..SHARED_THREADS {
            let db_ref: &crate::db::Db = &db;
            let barrier = &barrier;
            handles.push(s.spawn(move || {
                let mut rng = 0xdead_beefu64.wrapping_add((t as u64).wrapping_mul(0x9e37_79b9));
                for _ in 0..SHARED_ROUNDS {
                    // Synchronize: all threads create their proposal at this point.
                    barrier.wait();
                    let mut batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> =
                        Vec::with_capacity(SHARED_KEYS_PER_BATCH);
                    for _ in 0..SHARED_KEYS_PER_BATCH {
                        let k = random_key(&mut rng);
                        let v = random_key(&mut rng);
                        batch.push(BatchOp::Put { key: k, value: v });
                    }
                    let proposal = db_ref.propose(batch).expect("propose");
                    // Synchronize again: all proposals exist concurrently before
                    // any of them try to commit.
                    barrier.wait();
                    proposal
                        .commit_with_rebase()
                        .expect("commit_with_rebase must not error under contention");
                }
            }));
        }
        for h in handles {
            h.join().expect("thread panicked");
        }
    });

    // Close + reopen forces deferred persistence to flush; freelist
    // corruption surfaces here.
    let db = db.reopen();
    assert_check_clean(&db, "post-reopen");
}

/// Two proposals built off the same parent that produce the same `Put`. The
/// first commits directly; the second's `commit_with_rebase` produces a
/// trivial result (rebased batch is a no-op against current). The trivial
/// path must succeed and not push a duplicate revision; the post-state must
/// match the first commit and the freelist must be intact.
#[test]
fn test_trivial_rebase_succeeds_without_corruption() {
    let db = TestDb::new();

    let key = vec![0x42u8; 32];
    let val = vec![0xaa, 0xbb, 0xcc];

    let p_a = db
        .propose(vec![BatchOp::Put {
            key: key.clone(),
            value: val.clone(),
        }])
        .unwrap();
    let p_b = db
        .propose(vec![BatchOp::Put {
            key: key.clone(),
            value: val.clone(),
        }])
        .unwrap();

    p_a.commit().unwrap();
    let after_a = <crate::db::Db as crate::api::Db>::root_hash(&db).unwrap();

    // p_b's recorded parent is stale; rebase diff applied to current is a
    // no-op. commit_with_rebase must succeed and return the same hash as
    // current (trivial result).
    let returned = p_b
        .commit_with_rebase()
        .expect("trivial rebase must succeed")
        .expect("commit produces a hash");
    assert_eq!(returned, after_a);

    let view = <crate::db::Db as crate::api::Db>::revision(&db, after_a).unwrap();
    assert_eq!(view.val(&key).unwrap().as_deref(), Some(val.as_slice()));
    drop(view);

    // Reopen forces the persist worker to flush before we run check();
    // otherwise the checker may race the worker and report `UnpersistedRoot`.
    let db = db.reopen();
    assert_check_clean(&db, "post-trivial-rebase");
}

/// A→B→A: commit `Put(K, V)`, then `Delete(K)`, then `Put(K, V)`. The first
/// and third revisions have the same root hash but different `CommittedId`s.
/// A proposal built off the first revision must NOT be commitable directly
/// against the third — `parent_id_is` must reject hash-equal-but-different
/// parents and force a rebase. After rebase the proposal commits cleanly
/// with no freelist corruption.
#[test]
fn test_round_trip_rejects_stale_parent() {
    let db = TestDb::new();

    let key = vec![0x42u8; 32];
    let val = vec![0xaa, 0xbb, 0xcc];

    // C1: K=V.
    db.propose(vec![BatchOp::Put {
        key: key.clone(),
        value: val.clone(),
    }])
    .unwrap()
    .commit()
    .unwrap();
    let c1_hash = <crate::db::Db as crate::api::Db>::root_hash(&db).unwrap();

    // P off C1 adds a separate key.
    let key2 = vec![0x43u8; 32];
    let val2 = vec![0xdd, 0xee];
    let proposal = db
        .propose(vec![BatchOp::Put {
            key: key2.clone(),
            value: val2.clone(),
        }])
        .unwrap();

    // C2: K removed (empty trie).
    db.propose(vec![BatchOp::<_, Vec<u8>>::Delete { key: key.clone() }])
        .unwrap()
        .commit()
        .unwrap();

    // C3: K=V again. Same content as C1 → same root hash, different id.
    db.propose(vec![BatchOp::Put {
        key: key.clone(),
        value: val.clone(),
    }])
    .unwrap()
    .commit()
    .unwrap();
    let c3_hash = <crate::db::Db as crate::api::Db>::root_hash(&db).unwrap();
    assert_eq!(
        c1_hash, c3_hash,
        "A→B→A should reach the same root hash twice"
    );

    // proposal's recorded parent id is C1's; current is C3 (same hash but
    // different id). Must rebase rather than commit directly. Rebase
    // applies [Put(key2, val2)] to current → non-trivial result.
    proposal
        .commit_with_rebase()
        .expect("commit_with_rebase must succeed via rebase");

    let final_hash = <crate::db::Db as crate::api::Db>::root_hash(&db).unwrap();
    let view = <crate::db::Db as crate::api::Db>::revision(&db, final_hash).unwrap();
    assert_eq!(view.val(&key).unwrap().as_deref(), Some(val.as_slice()));
    assert_eq!(view.val(&key2).unwrap().as_deref(), Some(val2.as_slice()));
    drop(view);

    // Reopen forces persist before check() to avoid `UnpersistedRoot` races.
    let db = db.reopen();
    assert_check_clean(&db, "post-round-trip");
}
