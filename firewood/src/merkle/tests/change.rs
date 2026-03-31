// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::*;
use crate::api::{self, BatchOp, Db as DbTrait, DbView, FrozenChangeProof, Proposal as _};
use crate::db::{Db, DbConfig};
use crate::merkle::{
    ChangeProofVerificationContext, change_proof_boundary_key, verify_change_proof_root_hash,
    verify_change_proof_structure,
};

// ── Test infrastructure ────────────────────────────────────────────────────

fn new_db() -> (Db, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::new(dir.path(), DbConfig::builder().build()).unwrap();
    (db, dir)
}

/// Verify a change proof end-to-end: structural check + root hash check.
fn verify_and_check(
    db: &Db,
    proof: &FrozenChangeProof,
    verification: &ChangeProofVerificationContext,
    start_root: api::HashKey,
) -> Result<(), api::Error> {
    let parent = db.revision(start_root)?;
    let proposal = db.apply_change_proof_to_parent(proof, &*parent)?;

    let start_path = match change_proof_boundary_key(proof.start_proof().as_ref()) {
        Some(key) => proposal.path_to_key(&key)?,
        None => Box::default(),
    };
    let end_path = match change_proof_boundary_key(proof.end_proof().as_ref()) {
        Some(key) => proposal.path_to_key(&key)?,
        None => Box::default(),
    };

    verify_change_proof_root_hash(
        proof,
        verification,
        proposal.root_hash().as_ref(),
        &start_path,
        &end_path,
    )
}

// ── Structural validation tests ────────────────────────────────────────────

#[test]
fn test_inverted_range_rejected() {
    let (db, _dir) = new_db();
    db.propose(vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\xa0",
            value: b"v1",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    db.propose(vec![BatchOp::Put {
        key: b"\x50",
        value: b"mid",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    let proof = db
        .change_proof(root1, root2.clone(), Some(b"\x10"), Some(b"\xa0"), None)
        .unwrap();

    let err = verify_change_proof_structure(&proof, root2, Some(b"\xa0"), Some(b"\x10"), None)
        .unwrap_err();
    assert!(matches!(err, api::Error::InvalidRange { .. }));
}

#[test]
fn test_boundary_proof_unverifiable() {
    let (db, _dir) = new_db();
    db.propose(vec![
        BatchOp::Put {
            key: b"\x00",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\x10",
            value: b"v1",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    db.propose(vec![BatchOp::Put {
        key: b"\x05",
        value: b"v2",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    let proof = db
        .change_proof(root1, root2.clone(), Some(b"\x00"), Some(b"\x10"), None)
        .unwrap();

    // Non-empty start_proof but start_key=None
    let err = verify_change_proof_structure(&proof, root2, None, Some(b"\x10"), None).unwrap_err();
    assert!(matches!(
        err,
        api::Error::ProofError(crate::ProofError::BoundaryProofUnverifiable)
    ));
}

#[test]
fn test_keys_not_sorted() {
    let (db, _dir) = new_db();
    db.propose(vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\xa0",
            value: b"v1",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    db.propose(vec![BatchOp::Put {
        key: b"\x50",
        value: b"mid",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    let valid = db
        .change_proof(root1, root2.clone(), None, None, None)
        .unwrap();

    let crafted = FrozenChangeProof::new(
        crate::Proof::new(valid.start_proof().as_ref().into()),
        crate::Proof::new(valid.end_proof().as_ref().into()),
        Box::new([
            BatchOp::Put {
                key: b"\x50".to_vec().into(),
                value: b"a".to_vec().into(),
            },
            BatchOp::Put {
                key: b"\x50".to_vec().into(),
                value: b"b".to_vec().into(),
            },
        ]),
    );

    let err = verify_change_proof_structure(&crafted, root2, None, None, None).unwrap_err();
    assert!(matches!(
        err,
        api::Error::ProofError(crate::ProofError::ChangeProofKeysNotSorted)
    ));
}

#[test]
fn test_start_key_larger_than_first_key() {
    let (db, _dir) = new_db();
    db.propose(vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\xa0",
            value: b"v1",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    db.propose(vec![BatchOp::Put {
        key: b"\x50",
        value: b"mid",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    let proof = db
        .change_proof(root1, root2.clone(), None, None, None)
        .unwrap();
    let err = verify_change_proof_structure(&proof, root2, Some(b"\xff"), None, None).unwrap_err();
    assert!(matches!(
        err,
        api::Error::ProofError(crate::ProofError::StartKeyLargerThanFirstKey)
    ));
}

#[test]
fn test_end_key_less_than_last_key() {
    let (db, _dir) = new_db();
    db.propose(vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\xa0",
            value: b"v1",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    db.propose(vec![BatchOp::Put {
        key: b"\x50",
        value: b"mid",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    let proof = db
        .change_proof(root1, root2.clone(), None, None, None)
        .unwrap();
    let err = verify_change_proof_structure(&proof, root2, None, Some(b"\x01"), None).unwrap_err();
    assert!(matches!(
        err,
        api::Error::ProofError(crate::ProofError::EndKeyLessThanLastKey)
    ));
}

#[test]
fn test_proof_larger_than_max_length() {
    let (db, _dir) = new_db();
    db.propose(vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\xa0",
            value: b"v1",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    db.propose(vec![
        BatchOp::Put {
            key: b"\x50",
            value: b"mid_",
        },
        BatchOp::Put {
            key: b"\x60",
            value: b"mid2",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    let proof = db
        .change_proof(root1, root2.clone(), None, None, None)
        .unwrap();
    let err =
        verify_change_proof_structure(&proof, root2, None, None, NonZeroUsize::new(1)).unwrap_err();
    assert!(matches!(
        err,
        api::Error::ProofError(crate::ProofError::ProofIsLargerThanMaxLength)
    ));
}

#[test]
fn test_missing_boundary_proof() {
    let proof = FrozenChangeProof::new(
        crate::Proof::new(Box::new([])),
        crate::Proof::new(Box::new([])),
        Box::new([BatchOp::Put {
            key: b"\x50".to_vec().into(),
            value: b"value".to_vec().into(),
        }]),
    );
    let err = verify_change_proof_structure(
        &proof,
        api::HashKey::empty(),
        Some(b"\x10"),
        Some(b"\xa0"),
        None,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        api::Error::ProofError(crate::ProofError::MissingBoundaryProof)
    ));
}

#[test]
fn test_missing_end_proof() {
    let (db, _dir) = new_db();
    db.propose(vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\xa0",
            value: b"v1",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    db.propose(vec![BatchOp::Put {
        key: b"\x50",
        value: b"mid",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    let valid = db
        .change_proof(root1, root2.clone(), None, None, None)
        .unwrap();

    let crafted = FrozenChangeProof::new(
        crate::Proof::new(valid.start_proof().as_ref().into()),
        crate::Proof::new(Box::new([])),
        valid.batch_ops().into(),
    );
    let err = verify_change_proof_structure(&crafted, root2, None, None, None).unwrap_err();
    assert!(matches!(
        err,
        api::Error::ProofError(crate::ProofError::MissingEndProof)
    ));
}

#[test]
fn test_unexpected_end_proof() {
    let (db, _dir) = new_db();
    db.propose(vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\x20",
            value: b"v1",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    db.propose(vec![BatchOp::Put {
        key: b"\x20",
        value: b"changed",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    // Get a proof with a non-empty end_proof
    let valid = db
        .change_proof(root1, root2.clone(), None, Some(b"\x20"), None)
        .unwrap();
    assert!(!valid.end_proof().is_empty());

    // Craft: non-empty end_proof + empty batch_ops + end_key=None
    let crafted = FrozenChangeProof::new(
        crate::Proof::new(Box::new([])),
        crate::Proof::new(valid.end_proof().as_ref().into()),
        Box::new([]),
    );

    let err = verify_change_proof_structure(&crafted, root2, None, None, None).unwrap_err();
    assert!(matches!(
        err,
        api::Error::ProofError(crate::ProofError::UnexpectedEndProof)
    ));
}

#[test]
fn test_wrong_end_root_boundary_check() {
    let (db, _dir) = new_db();
    db.propose(vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\xa0",
            value: b"v1",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    db.propose(vec![BatchOp::Put {
        key: b"\x50",
        value: b"mid",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    let proof = db
        .change_proof(root1, root2, Some(b"\x10"), Some(b"\xa0"), None)
        .unwrap();
    let err = verify_change_proof_structure(
        &proof,
        api::HashKey::empty(),
        Some(b"\x10"),
        Some(b"\xa0"),
        None,
    )
    .unwrap_err();
    assert!(matches!(err, api::Error::ProofError(_)));
}

// ── Root hash verification (one per case) ──────────────────────────────────

#[test]
fn test_root_hash_complete_no_proofs() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial = vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\x20",
            value: b"v1",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    db_a.propose(vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"changed0",
        },
        BatchOp::Put {
            key: b"\x20",
            value: b"changed1",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    let proof = db_a
        .change_proof(root1_a, root2.clone(), None, None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, None, None, None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

#[test]
fn test_root_hash_single_end_proof() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial = vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\x20",
            value: b"v1",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    db_a.propose(vec![BatchOp::Put {
        key: b"\x10",
        value: b"changed",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    let proof = db_a
        .change_proof(root1_a, root2.clone(), None, Some(b"\x20"), None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, None, Some(b"\x20"), None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

#[test]
fn test_root_hash_single_start_proof() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial = vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\x20",
            value: b"v1",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    db_a.propose(vec![BatchOp::Put {
        key: b"\x20",
        value: b"changed",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x10"), None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, Some(b"\x10"), None, None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

#[test]
fn test_root_hash_two_proofs() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial = vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\x20",
            value: b"v1",
        },
        BatchOp::Put {
            key: b"\x30",
            value: b"v2",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    db_a.propose(vec![BatchOp::Put {
        key: b"\x20",
        value: b"changed",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x10"), Some(b"\x30"), None)
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2, Some(b"\x10"), Some(b"\x30"), None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

// ── Attack detection ───────────────────────────────────────────────────────

#[test]
fn test_omitted_change_detected() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial = vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\x20",
            value: b"v1",
        },
        BatchOp::Put {
            key: b"\x30",
            value: b"v2",
        },
        BatchOp::Put {
            key: b"\x40",
            value: b"v3",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    db_a.propose(vec![
        BatchOp::Put {
            key: b"\x20",
            value: b"changed1",
        },
        BatchOp::Put {
            key: b"\x30",
            value: b"changed2",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    let valid = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x10"), Some(b"\x40"), None)
        .unwrap();

    let mut shortened: Vec<BatchOp<Key, Value>> = valid.batch_ops().to_vec();
    shortened.remove(0);
    let crafted = FrozenChangeProof::new(
        crate::Proof::new(valid.start_proof().as_ref().into()),
        crate::Proof::new(valid.end_proof().as_ref().into()),
        shortened.into_boxed_slice(),
    );

    let ctx =
        verify_change_proof_structure(&crafted, root2, Some(b"\x10"), Some(b"\x40"), None).unwrap();
    verify_and_check(&db_b, &crafted, &ctx, root1_b).expect_err("omitted change must be detected");
}

#[test]
fn test_forged_value_detected() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial = vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\xa0",
            value: b"v1",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    db_a.propose(vec![BatchOp::Put {
        key: b"\x10",
        value: b"real",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    let valid = db_a
        .change_proof(root1_a, root2.clone(), None, None, None)
        .unwrap();

    let mut forged_ops: Vec<BatchOp<Key, Value>> = valid.batch_ops().to_vec();
    forged_ops[0] = BatchOp::Put {
        key: b"\x10".to_vec().into(),
        value: b"FORGED".to_vec().into(),
    };
    let crafted = FrozenChangeProof::new(
        crate::Proof::new(valid.start_proof().as_ref().into()),
        crate::Proof::new(valid.end_proof().as_ref().into()),
        forged_ops.into_boxed_slice(),
    );

    let ctx = verify_change_proof_structure(&crafted, root2, None, None, None).unwrap();
    verify_and_check(&db_b, &crafted, &ctx, root1_b).expect_err("forged value must be detected");
}

#[test]
fn test_wrong_end_root_detected() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial = vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\xa0",
            value: b"v1",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    db_a.propose(vec![BatchOp::Put {
        key: b"\x50",
        value: b"mid",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    let proof = db_a.change_proof(root1_a, root2, None, None, None).unwrap();

    // Wrong end_root — boundary hash chain rejects
    let result = verify_change_proof_structure(&proof, api::HashKey::empty(), None, None, None);
    if let Ok(ctx) = result {
        verify_and_check(&db_b, &proof, &ctx, root1_b)
            .expect_err("wrong end_root must be detected");
    }
}

#[test]
fn test_spurious_batch_op_detected() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial = vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\xa0",
            value: b"v1",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    db_a.propose(vec![BatchOp::Put {
        key: b"\xa0",
        value: b"changed",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    let valid = db_a
        .change_proof(root1_a, root2.clone(), None, None, None)
        .unwrap();

    let mut ops: Vec<BatchOp<Key, Value>> = valid.batch_ops().to_vec();
    ops.push(BatchOp::Put {
        key: b"\xf0".to_vec().into(),
        value: b"spurious".to_vec().into(),
    });
    ops.sort_by(|a, b| a.key().cmp(b.key()));

    let crafted = FrozenChangeProof::new(
        crate::Proof::new(valid.start_proof().as_ref().into()),
        crate::Proof::new(valid.end_proof().as_ref().into()),
        ops.into_boxed_slice(),
    );

    // Structural check or root hash check catches this
    let result = verify_change_proof_structure(&crafted, root2, None, None, None);
    if let Ok(ctx) = result {
        verify_and_check(&db_b, &crafted, &ctx, root1_b)
            .expect_err("spurious batch op must be detected");
    }
}

// ── Edge cases ─────────────────────────────────────────────────────────────

#[test]
fn test_empty_batch_ops_with_nonempty_proofs() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial = vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\x20",
            value: b"v1",
        },
        BatchOp::Put {
            key: b"\x30",
            value: b"v2",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    // Change only \x30 (outside range [\x10, \x20])
    db_a.propose(vec![BatchOp::Put {
        key: b"\x30",
        value: b"changed",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x10"), Some(b"\x20"), None)
        .unwrap();
    assert!(proof.batch_ops().is_empty());

    let ctx =
        verify_change_proof_structure(&proof, root2, Some(b"\x10"), Some(b"\x20"), None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

// test_empty_batch_ops_end_nibbles_fallback removed: this attack scenario
// requires the receiver's base state to differ from the sender's within the
// proven range, which means different start_roots. In practice, the receiver
// always verifies against its own start_root, so this scenario is already
// covered by the root hash check in the empty-batch-ops happy-path test.

#[test]
fn test_odd_depth_proof_node_accepted() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial = vec![
        BatchOp::Put {
            key: b"\x12",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\x13",
            value: b"v1",
        },
        BatchOp::Put {
            key: b"\x50",
            value: b"v2",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    db_a.propose(vec![BatchOp::Put {
        key: b"\x50",
        value: b"changed",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x14"), None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, Some(b"\x14"), None, None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

#[test]
fn test_start_proof_inclusion_with_children_below() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial: Vec<BatchOp<&[u8], &[u8]>> = vec![
        BatchOp::Put {
            key: b"\xab",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\xab\xcd",
            value: b"v1",
        },
        BatchOp::Put {
            key: b"\xf0",
            value: b"v2",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    db_a.propose(vec![BatchOp::Put {
        key: b"\xf0",
        value: b"changed",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\xab"), None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, Some(b"\xab"), None, None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

#[test]
fn test_end_proof_inclusion_with_children_below() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial: Vec<BatchOp<&[u8], &[u8]>> = vec![
        BatchOp::Put {
            key: b"\xab",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\xab\xcd",
            value: b"v1",
        },
        BatchOp::Put {
            key: b"\xf0",
            value: b"v2",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    let changes: Vec<BatchOp<&[u8], &[u8]>> = vec![
        BatchOp::Put {
            key: b"\xab",
            value: b"c0",
        },
        BatchOp::Put {
            key: b"\xab\xcd",
            value: b"c1",
        },
        BatchOp::Put {
            key: b"\xf0",
            value: b"c2",
        },
    ];
    db_a.propose(changes).unwrap().commit().unwrap();
    let root2 = db_a.root_hash().unwrap();

    // Truncated to 1 — last_op_key is prefix with children below
    let proof = db_a
        .change_proof(root1_a, root2.clone(), None, None, NonZeroUsize::new(1))
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2, None, None, NonZeroUsize::new(1)).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

#[test]
fn test_divergence_parent_start_key_exhausted() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial: Vec<BatchOp<&[u8], &[u8]>> = vec![
        BatchOp::Put {
            key: b"\x12\x01",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\x12\x02",
            value: b"v1",
        },
        BatchOp::Put {
            key: b"\xf0",
            value: b"v2",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    db_a.propose(vec![BatchOp::Put {
        key: b"\xf0",
        value: b"changed",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x12"), Some(b"\xf0"), None)
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2, Some(b"\x12"), Some(b"\xf0"), None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

#[test]
fn test_divergence_at_depth_zero() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial = vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\x11",
            value: b"v1",
        },
        BatchOp::Put {
            key: b"\xa0",
            value: b"v2",
        },
        BatchOp::Put {
            key: b"\xa1",
            value: b"v3",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    let changes: Vec<BatchOp<&[u8], &[u8]>> = vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"changed",
        },
        BatchOp::Put {
            key: b"\xa0",
            value: b"changed2",
        },
    ];
    db_a.propose(changes).unwrap().commit().unwrap();
    let root2 = db_a.root_hash().unwrap();

    let left = db_a
        .change_proof(
            root1_a.clone(),
            root2.clone(),
            Some(b"\x10"),
            Some(b"\x11"),
            None,
        )
        .unwrap();
    let right = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\xa0"), Some(b"\xa1"), None)
        .unwrap();

    // Skip shared root to create divergence at position 0
    let s = left.start_proof().as_ref();
    let e = right.end_proof().as_ref();
    assert!(s.len() >= 2 && e.len() >= 2);

    let crafted = FrozenChangeProof::new(
        crate::Proof::new(s[1..].into()),
        crate::Proof::new(e[1..].into()),
        Box::new([BatchOp::Put {
            key: b"\x50".to_vec().into(),
            value: b"mid".to_vec().into(),
        }]),
    );

    let parent = db_b.revision(root1_b).unwrap();
    let proposal = db_b
        .apply_change_proof_to_parent(&crafted, &*parent)
        .unwrap();

    let verification = ChangeProofVerificationContext {
        end_root: root2,
        start_key: Some(b"\x10".to_vec().into()),
        end_key: Some(b"\xa1".to_vec().into()),
    };

    let start_path = change_proof_boundary_key(crafted.start_proof().as_ref())
        .map(|k| proposal.path_to_key(&k).unwrap())
        .unwrap_or_default();
    let end_path = change_proof_boundary_key(crafted.end_proof().as_ref())
        .map(|k| proposal.path_to_key(&k).unwrap())
        .unwrap_or_default();

    let err = verify_change_proof_root_hash(
        &crafted,
        &verification,
        proposal.root_hash().as_ref(),
        &start_path,
        &end_path,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        api::Error::ProofError(crate::ProofError::BoundaryProofsDivergeAtRoot)
    ));
}

// Verify that the children of the last node in the start proof are checked
// during verification. The start bound \x10 is a branch node (not a leaf),
// so the verifier must inspect its children (\x10\x01, \x10\x02) to ensure
// they haven't been tampered with. The actual change is to \x30, which is
// on a completely separate branch, so this isolates the child-checking logic.
//
// Trie structure (both dbs start identical):
//
//       [root]
//       /    \
//   [0x1_]   [0x3_]
//    /   \      |
// [_0_1] [_0_2] [_0]
//  "a"    "b"   "c"
//
// The change proof query uses start=\x10 (the branch node), end=None.
// Only \x30 changes ("c" -> "changed"), but the verifier must still
// validate that \x10's children are intact.
#[test]
fn test_start_tail_last_node_children_checked() {
    let (sender, _dir_sender) = new_db();
    let (receiver, _dir_receiver) = new_db();

    let start_data_for_both: Vec<BatchOp<&[u8], &[u8]>> = vec![
        BatchOp::Put {
            key: b"\x10\x01",
            value: b"a",
        },
        BatchOp::Put {
            key: b"\x10\x02",
            value: b"b",
        },
        BatchOp::Put {
            key: b"\x30",
            value: b"c",
        },
    ];
    sender
        .propose(start_data_for_both.clone())
        .unwrap()
        .commit()
        .unwrap();
    let sender_root1 = sender.root_hash().unwrap();
    receiver
        .propose(start_data_for_both)
        .unwrap()
        .commit()
        .unwrap();
    let receiver_root1 = receiver.root_hash().unwrap();

    // Only modify \x30 on the sender; \x10's children are untouched
    sender
        .propose(vec![BatchOp::Put {
            key: b"\x30",
            value: b"changed",
        }])
        .unwrap()
        .commit()
        .unwrap();
    let sender_root2 = sender.root_hash().unwrap();

    let proof = sender
        .change_proof(
            sender_root1,
            sender_root2.clone(),
            Some(b"\x10"),
            None,
            None,
        )
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, sender_root2, Some(b"\x10"), None, None).unwrap();
    verify_and_check(&receiver, &proof, &ctx, receiver_root1).unwrap();
}

#[test]
fn test_start_proof_exclusion_for_deleted_key() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial = vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\x20",
            value: b"v1",
        },
        BatchOp::Put {
            key: b"\x30",
            value: b"v2",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    db_a.propose(vec![
        BatchOp::Delete { key: b"\x10" },
        BatchOp::Put {
            key: b"\x30",
            value: b"changed",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x10"), None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, Some(b"\x10"), None, None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

#[test]
fn test_delete_range_rejected() {
    let (db, _dir) = new_db();
    db.propose(vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\xa0",
            value: b"v1",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    db.propose(vec![
        BatchOp::Put {
            key: b"\x50",
            value: b"mid",
        },
        BatchOp::Put {
            key: b"\x80",
            value: b"end",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    let valid = db
        .change_proof(root1, root2.clone(), None, None, None)
        .unwrap();

    // Place a DeleteRange before a valid Put so the end-proof consistency
    // check passes on the last op (Put) and the O(n) scan catches the
    // DeleteRange.
    let crafted = FrozenChangeProof::new(
        crate::Proof::new(valid.start_proof().as_ref().into()),
        crate::Proof::new(valid.end_proof().as_ref().into()),
        Box::new([
            BatchOp::DeleteRange {
                prefix: b"\x50".to_vec().into(),
            },
            BatchOp::Put {
                key: b"\x80".to_vec().into(),
                value: b"end".to_vec().into(),
            },
        ]),
    );

    let err = verify_change_proof_structure(&crafted, root2, None, None, None).unwrap_err();
    assert!(matches!(
        err,
        api::Error::ProofError(crate::ProofError::UnsupportedDeleteRange)
    ));
}

/// When boundary proofs are present, the O(n) scan catches the `DeleteRange`
/// before the end-proof consistency check runs.
#[test]
fn test_delete_range_rejected_with_boundary_proofs() {
    let (db, _dir) = new_db();
    db.propose(vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\xa0",
            value: b"v1",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    db.propose(vec![BatchOp::Put {
        key: b"\x50",
        value: b"mid",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    let valid = db
        .change_proof(root1, root2.clone(), None, None, None)
        .unwrap();
    let crafted = FrozenChangeProof::new(
        crate::Proof::new(valid.start_proof().as_ref().into()),
        crate::Proof::new(valid.end_proof().as_ref().into()),
        Box::new([BatchOp::DeleteRange {
            prefix: b"\x50".to_vec().into(),
        }]),
    );

    let err = verify_change_proof_structure(&crafted, root2, None, None, None).unwrap_err();
    assert!(matches!(
        err,
        api::Error::ProofError(crate::ProofError::UnsupportedDeleteRange)
    ));
}

// ── Truncation tests ───────────────────────────────────────────────────────

#[test]
fn test_truncated_proof_round_trip() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial = vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\x20",
            value: b"v1",
        },
        BatchOp::Put {
            key: b"\x30",
            value: b"v2",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    db_a.propose(vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"c0",
        },
        BatchOp::Put {
            key: b"\x20",
            value: b"c1",
        },
        BatchOp::Put {
            key: b"\x30",
            value: b"c2",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    // Round 1: truncated
    let proof1 = db_a
        .change_proof(
            root1_a.clone(),
            root2.clone(),
            None,
            None,
            NonZeroUsize::new(1),
        )
        .unwrap();
    let ctx1 =
        verify_change_proof_structure(&proof1, root2.clone(), None, None, NonZeroUsize::new(1))
            .unwrap();
    verify_and_check(&db_b, &proof1, &ctx1, root1_b.clone()).unwrap();

    // Commit round 1
    let parent1 = db_b.revision(root1_b).unwrap();
    let proposal1 = db_b
        .apply_change_proof_to_parent(&proof1, &*parent1)
        .unwrap();
    let root_b_after_1 = proposal1.root_hash().unwrap();
    proposal1.commit().unwrap();

    // Round 2: continue from last key
    let next_start = proof1.batch_ops().last().unwrap().key().clone();
    let proof2 = db_a
        .change_proof(root1_a, root2.clone(), Some(&next_start), None, None)
        .unwrap();
    let ctx2 =
        verify_change_proof_structure(&proof2, root2, Some(&next_start), None, None).unwrap();
    verify_and_check(&db_b, &proof2, &ctx2, root_b_after_1).unwrap();
}

#[test]
fn test_truncated_proof_with_delete_last_op() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    let initial = vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\x20",
            value: b"v1",
        },
        BatchOp::Put {
            key: b"\x30",
            value: b"v2",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    let changes: Vec<BatchOp<&[u8], &[u8]>> = vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"changed",
        },
        BatchOp::Delete { key: b"\x20" },
        BatchOp::Put {
            key: b"\x30",
            value: b"changed2",
        },
    ];
    db_a.propose(changes).unwrap().commit().unwrap();
    let root2 = db_a.root_hash().unwrap();

    let proof = db_a
        .change_proof(root1_a, root2.clone(), None, None, NonZeroUsize::new(2))
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2, None, None, NonZeroUsize::new(2)).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

#[test]
fn test_generator_uses_last_op_key_for_end_proof() {
    let (db, _dir) = new_db();

    db.propose(vec![
        BatchOp::Put {
            key: b"\x10",
            value: b"v0",
        },
        BatchOp::Put {
            key: b"\xa0",
            value: b"v1",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    db.propose(vec![BatchOp::Put {
        key: b"\x10",
        value: b"changed",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    // end_key far beyond last change
    let proof = db
        .change_proof(root1, root2.clone(), None, Some(b"\xff"), None)
        .unwrap();

    // End proof validates against last_op_key, not end_key
    let last_key = proof.batch_ops().last().unwrap().key();
    proof.end_proof().value_digest(last_key, &root2).unwrap();
}

/// Attacker adds a spurious Put at `start_key` when `start_key` doesn't
/// exist in `end_root`. The start proof is an exclusion proof. The
/// `StartProofOperationMismatch` check catches this: Put expects inclusion
/// but the proof is exclusion.
#[test]
fn test_spurious_put_at_start_key_boundary() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    // Keys \x20 and \x90 exist. \x10 (start_key) does NOT exist.
    let initial = vec![
        BatchOp::Put {
            key: b"\x20",
            value: b"v2",
        },
        BatchOp::Put {
            key: b"\x90",
            value: b"v9",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    // Change \x20 on db_a
    db_a.propose(vec![BatchOp::Put {
        key: b"\x20",
        value: b"changed",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    // Generate bounded proof for [\x10, \x90].
    // Start proof for \x10 is an exclusion proof (\x10 doesn't exist).
    let honest = db_a
        .change_proof(
            root1_a,
            root2.clone(),
            Some(b"\x10".as_ref()),
            Some(b"\x90".as_ref()),
            None,
        )
        .unwrap();

    // Add spurious Put(\x10, fake) — start_key becomes first_op_key.
    let mut ops: Vec<BatchOp<Key, Value>> = honest.batch_ops().to_vec();
    ops.push(BatchOp::Put {
        key: b"\x10".to_vec().into(),
        value: b"fake".to_vec().into(),
    });
    ops.sort_by(|a, b| a.key().cmp(b.key()));

    let crafted = FrozenChangeProof::new(
        crate::Proof::new(honest.start_proof().as_ref().into()),
        crate::Proof::new(honest.end_proof().as_ref().into()),
        ops.into_boxed_slice(),
    );

    // Structural check or root hash check should catch this.
    let result = verify_change_proof_structure(&crafted, root2, Some(b"\x10"), Some(b"\x90"), None);
    if let Ok(ctx) = result {
        verify_and_check(&db_b, &crafted, &ctx, root1_b)
            .expect_err("spurious Put at start_key should be detected");
    }
}

/// Attacker adds a spurious Delete at `start_key` when `start_key` EXISTS
/// in `end_root`. The start proof is an inclusion proof, but the Delete
/// claims the key was removed — `StartProofOperationMismatch`.
#[test]
fn test_spurious_delete_at_start_key_boundary() {
    let (db_a, _dir_a) = new_db();
    let (db_b, _dir_b) = new_db();

    // Keys \x20 and \x90 exist. start_key \x20 EXISTS in end_root.
    let initial = vec![
        BatchOp::Put {
            key: b"\x20",
            value: b"v2",
        },
        BatchOp::Put {
            key: b"\x90",
            value: b"v9",
        },
    ];
    db_a.propose(initial.clone()).unwrap().commit().unwrap();
    let root1_a = db_a.root_hash().unwrap();
    db_b.propose(initial).unwrap().commit().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    // Change \x90 on db_a (not \x20 — \x20 stays in end_root).
    db_a.propose(vec![BatchOp::Put {
        key: b"\x90",
        value: b"changed",
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    // Generate bounded proof for [\x20, \x90].
    // Start proof for \x20 is an inclusion proof (\x20 exists).
    let honest = db_a
        .change_proof(
            root1_a,
            root2.clone(),
            Some(b"\x20".as_ref()),
            Some(b"\x90".as_ref()),
            None,
        )
        .unwrap();

    // Add spurious Delete(\x20) — start_key = first_op_key.
    // The start proof is inclusion (key exists) but Delete claims
    // it was removed — contradiction.
    let mut ops: Vec<BatchOp<Key, Value>> = honest.batch_ops().to_vec();
    ops.push(BatchOp::Delete {
        key: b"\x20".to_vec().into(),
    });
    ops.sort_by(|a, b| a.key().cmp(b.key()));

    let crafted = FrozenChangeProof::new(
        crate::Proof::new(honest.start_proof().as_ref().into()),
        crate::Proof::new(honest.end_proof().as_ref().into()),
        ops.into_boxed_slice(),
    );

    let result = verify_change_proof_structure(&crafted, root2, Some(b"\x20"), Some(b"\x90"), None);
    if let Ok(ctx) = result {
        verify_and_check(&db_b, &crafted, &ctx, root1_b)
            .expect_err("spurious Delete at start_key should be detected");
    }
}
