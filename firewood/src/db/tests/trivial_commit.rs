// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Space accounting across the trivial-commit fast path.

use firewood_macros::hash_mode;
use firewood_storage::CheckOpt;
use firewood_storage::CheckerError;
use firewood_storage::CheckerReport;
use firewood_storage::EthHash;
use firewood_storage::HashMode;
use firewood_storage::MerkleDbHash;

use super::TestDb;
use crate::api;
use crate::api::BatchOp;
use crate::api::Db as _;
use crate::api::Proposal as _;
use crate::db::DbConfig;
use crate::manager::RevisionManagerConfig;

type Op = BatchOp<Vec<u8>, Vec<u8>>;

/// A 32-byte key, valid under both hash schemes, with a value that is a
/// well-formed account under ethhash.
fn kv(b0: u8, b1: u8) -> (Vec<u8>, Vec<u8>) {
    let mut key = vec![0u8; 32];
    key[0] = b0;
    key[1] = b1;
    // [nonce, balance, storageRoot, codeHash]
    let mut rlp = rlp::RlpStream::new_list(4);
    rlp.append(&1u64);
    rlp.append(&u64::from(b1));
    rlp.append(&[0u8; 32].as_slice());
    rlp.append(&[b0; 32].as_slice());
    (key, rlp.out().to_vec())
}

fn put(b0: u8, b1: u8) -> Op {
    let (key, value) = kv(b0, b1);
    BatchOp::Put { key, value }
}

fn leaked_bytes(report: &CheckerReport) -> u64 {
    report
        .errors
        .iter()
        .map(|e| match e {
            CheckerError::AreaLeaks(ranges) => ranges
                .into_iter()
                .map(|r| r.end.get().saturating_sub(r.start.get()))
                .sum(),
            _ => 0,
        })
        .sum()
}

/// Seeds a database, runs `workload` on it, commits until every revision the
/// workload touched has aged out, and returns the bytes the checker reports as
/// leaked.
///
/// Panics if the checker reports anything other than leaked areas.
fn leaked_after<H: HashMode>(workload: impl FnOnce(&TestDb<H>)) -> u64 {
    let cfg = DbConfig::builder()
        .manager(RevisionManagerConfig::builder().max_revisions(2).build())
        .build();
    let db = TestDb::<H>::new_with_config(cfg);
    let seed: Vec<Op> = (0u8..32).map(|i| put(i.wrapping_mul(37), i)).collect();
    db.propose(seed).unwrap().commit().unwrap();

    workload(&db);

    // age every revision above out of the in-memory window
    for i in 0..8 {
        db.propose(vec![put(0xc0, i)]).unwrap().commit().unwrap();
    }
    db.manager.wait_persisted();

    let report = db.check(CheckOpt {
        hash_check: true,
        progress_bar: None,
    });
    let errors: Vec<_> = report
        .errors
        .iter()
        .filter(|e| !matches!(e, CheckerError::AreaLeaks(_)))
        .collect();
    assert!(errors.is_empty(), "checker errors: {errors:?}");
    leaked_bytes(&report)
}

/// Asserts that `workload` leaves the space accounting that committing each of
/// `direct` on the latest revision in turn leaves.
fn assert_leaks_like<H: HashMode>(direct: &[Op], workload: impl FnOnce(&TestDb<H>)) {
    let expected = leaked_after::<H>(|db| {
        for op in direct {
            db.propose(vec![op.clone()]).unwrap().commit().unwrap();
        }
    });
    assert_eq!(leaked_after::<H>(workload), expected);
}

/// A proposal whose root hash equals its parent's commits through the trivial
/// path, which drops it along with its deleted list. A child proposal built
/// on it is reparented onto the current revision, but the nodes of that
/// revision the dropped proposal had replaced are on no deleted list. Once
/// the revision ages out their areas are neither reachable nor free.
#[hash_mode]
#[test]
fn test_trivial_commit_with_child_frees_replaced_nodes<H: HashMode>() {
    assert_leaks_like::<H>(&[put(0xaa, 0)], |db| {
        let absorbed = db.propose(Vec::<Op>::new()).unwrap();
        let child = absorbed.propose(vec![put(0xaa, 0)]).unwrap();
        absorbed.commit().unwrap();
        child.commit().unwrap();
    });
}

/// Both children inherit the same nodes, and only the one that commits on the
/// current revision frees them.
#[hash_mode]
#[test]
fn test_trivial_commit_with_two_children_and_a_rebase<H: HashMode>() {
    assert_leaks_like::<H>(&[put(0xaa, 0), put(0xbb, 0)], |db| {
        let absorbed = db.propose(Vec::<Op>::new()).unwrap();
        let first = absorbed.propose(vec![put(0xaa, 0)]).unwrap();
        let second = absorbed.propose(vec![put(0xbb, 0)]).unwrap();
        absorbed.commit().unwrap();
        first.commit().unwrap();
        second.commit_with_rebase().unwrap();
    });
}

/// Inherited nodes pass through a second trivial commit, and a sibling that
/// holds some of the same nodes frees nothing when its commit is rejected.
#[hash_mode]
#[test]
fn test_trivial_commit_chain_with_a_rejected_sibling<H: HashMode>() {
    assert_leaks_like::<H>(&[put(0xaa, 0)], |db| {
        let absorbed = db.propose(Vec::<Op>::new()).unwrap();
        let absorbed_child = absorbed.propose(Vec::<Op>::new()).unwrap();
        let sibling = absorbed.propose(vec![put(0xbb, 0)]).unwrap();
        let grandchild = absorbed_child.propose(vec![put(0xaa, 0)]).unwrap();
        absorbed.commit().unwrap();
        absorbed_child.commit().unwrap();
        grandchild.commit().unwrap();
        assert!(matches!(
            sibling.commit(),
            Err(api::Error::ParentNotLatest { .. })
        ));
    });
}

/// The nodes a rebased child inherited belong to a revision that a commit in
/// between has already replaced, so the rebase frees none of them.
#[hash_mode]
#[test]
fn test_trivial_commit_child_rebased_over_a_sibling<H: HashMode>() {
    assert_leaks_like::<H>(&[put(0xbb, 0), put(0xaa, 0)], |db| {
        let absorbed = db.propose(Vec::<Op>::new()).unwrap();
        let child = absorbed.propose(vec![put(0xaa, 0)]).unwrap();
        let sibling = db.propose(vec![put(0xbb, 0)]).unwrap();
        absorbed.commit().unwrap();
        sibling.commit().unwrap();
        child.commit_with_rebase().unwrap();
    });
}
