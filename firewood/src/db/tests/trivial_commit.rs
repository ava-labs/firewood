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

/// Runs one fixed workload and returns the bytes the checker reports as
/// leaked at its end. With `through_empty_parent`, the middle batch is
/// proposed on top of an empty proposal that commits first.
fn leaked_after_workload<H: HashMode>(through_empty_parent: bool) -> u64 {
    let cfg = DbConfig::builder()
        .manager(RevisionManagerConfig::builder().max_revisions(2).build())
        .build();
    let db = TestDb::<H>::new_with_config(cfg);
    let seed: Vec<Op> = (0u8..32).map(|i| put(i.wrapping_mul(37), i)).collect();
    db.propose(seed).unwrap().commit().unwrap();

    let batch = vec![put(0xaa, 0)];
    if through_empty_parent {
        let empty = db.propose(Vec::<Op>::new()).unwrap();
        let child = empty.propose(batch).unwrap();
        empty.commit().unwrap();
        child.commit().unwrap();
    } else {
        db.propose(batch).unwrap().commit().unwrap();
    }

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

/// A proposal whose root hash equals its parent's commits through the trivial
/// path, which drops it along with its deleted list. A child proposal built
/// on it is reparented onto the current revision, but the nodes of that
/// revision the dropped proposal had replaced are on no deleted list. Once
/// the revision ages out their areas are neither reachable nor free.
#[hash_mode]
#[test]
fn test_trivial_commit_with_child_frees_replaced_nodes<H: HashMode>() {
    let direct = leaked_after_workload::<H>(false);
    let through_empty_parent = leaked_after_workload::<H>(true);
    assert_eq!(
        through_empty_parent, direct,
        "the same changes leak {through_empty_parent} bytes through an empty parent proposal \
         and {direct} bytes without one"
    );
}
