// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! The `Db` entry points for sized proofs: revision and proposal lookup, and
//! the error mapping on top of the merkle-level sizing tests.

use firewood_macros::hash_mode;
use firewood_storage::EthHash;
use firewood_storage::HashMode;
use firewood_storage::MerkleDbHash;

use super::TestDb;
use crate::api::Db as _;
use crate::api::DbView as _;
use crate::api::FrozenChangeProof;
use crate::api::FrozenRangeProof;
use crate::api::HashKey;
use crate::api::Proposal as _;
use crate::api::{self};

const BUDGET: usize = 64 * 1024;

fn kvs(range: std::ops::Range<u32>) -> Vec<(Vec<u8>, Vec<u8>)> {
    range
        .map(|i| (format!("key{i:05}").into_bytes(), vec![0x42; 48]))
        .collect()
}

#[hash_mode]
#[test]
fn test_db_sized_proofs_over_committed_revisions<H: HashMode>() {
    let db = TestDb::<H>::new();
    db.propose(kvs(0..500)).unwrap().commit().unwrap();
    let first: HashKey = db.root_hash().unwrap();
    db.propose(kvs(500..800)).unwrap().commit().unwrap();
    let second: HashKey = db.root_hash().unwrap();

    let range = db
        .range_proof_sized(first.clone(), None, BUDGET, None)
        .unwrap();
    assert!(range.wire().len() <= BUDGET);
    assert!(range.natural_end());
    assert_eq!(range.proof().key_values().len(), 500);
    let decoded = FrozenRangeProof::from_slice(range.wire()).unwrap();
    assert_eq!(decoded.key_values(), range.proof().key_values());

    let change = db
        .change_proof_sized(first, second, None, BUDGET, Some(range.hint()))
        .unwrap();
    assert!(change.wire().len() <= BUDGET);
    assert!(change.natural_end());
    assert_eq!(change.proof().batch_ops().len(), 300);
    let decoded = FrozenChangeProof::from_slice(change.wire()).unwrap();
    assert_eq!(decoded.batch_ops(), change.proof().batch_ops());
}

/// An uncommitted proposal's root resolves like it does for plain proofs.
#[hash_mode]
#[test]
fn test_db_sized_range_proof_over_a_proposal_root<H: HashMode>() {
    let db = TestDb::<H>::new();
    db.propose(kvs(0..100)).unwrap().commit().unwrap();
    let proposal = db.propose(kvs(100..200)).unwrap();
    let root: HashKey = proposal.root_hash().unwrap();

    let sized = db.range_proof_sized(root, None, BUDGET, None).unwrap();
    assert_eq!(sized.proof().key_values().len(), 200);
    assert!(sized.natural_end());
}

#[hash_mode]
#[test]
fn test_db_sized_proof_errors<H: HashMode>() {
    let db = TestDb::<H>::new();
    db.propose(kvs(0..10)).unwrap().commit().unwrap();
    let root: HashKey = db.root_hash().unwrap();
    let missing = HashKey::from([1u8; 32]);

    assert!(matches!(
        db.range_proof_sized(missing.clone(), None, BUDGET, None),
        Err(api::Error::RevisionNotFound { .. })
    ));
    assert!(matches!(
        db.range_proof_sized(root.clone(), None, 1, None),
        Err(api::Error::ProofOverBudget { budget: 1, .. })
    ));
    // The end revision is reported even when both are missing.
    assert!(matches!(
        db.change_proof_sized(missing.clone(), missing.clone(), None, BUDGET, None),
        Err(api::Error::EndRevisionNotFound { .. })
    ));
    assert!(matches!(
        db.change_proof_sized(missing, root.clone(), None, BUDGET, None),
        Err(api::Error::StartRevisionNotFound { .. })
    ));
    assert!(matches!(
        db.change_proof_sized(root.clone(), root, None, 1, None),
        Err(api::Error::ProofOverBudget { budget: 1, .. })
    ));
}
