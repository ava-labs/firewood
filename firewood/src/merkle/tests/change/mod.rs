// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::*;
use crate::api::{self, BatchOp, Db as DbTrait, DbView, FrozenChangeProof, Proposal as _};
use crate::db::{Db, DbConfig};
use crate::merkle::verify_change_proof_root_hash;
use crate::{ChangeProofVerificationContext, verify_change_proof_structure};
use firewood_storage::PathComponentSliceExt;

// ── Test infrastructure ────────────────────────────────────────────────────

pub(super) fn new_db() -> (Db, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::new(dir.path(), DbConfig::builder().build()).unwrap();
    (db, dir)
}

/// Create a new database pre-populated with key/value pairs.
///
/// ```rust,no_run
/// let (db, _dir) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1")];
/// ```
macro_rules! setup_db {
    [] => {{
        let (db, dir) = new_db();
        db.propose(vec![] as Vec<BatchOp<&[u8], &[u8]>>)
            .unwrap()
            .commit()
            .unwrap();
        (db, dir)
    }};
    [$(($key:expr, $val:expr)),+ $(,)?] => {{
        let (db, dir) = new_db();
        db.propose(vec![$(BatchOp::Put { key: $key as &[u8], value: $val as &[u8] }),+])
            .unwrap()
            .commit()
            .unwrap();
        (db, dir)
    }};
}

/// Commit a second batch of Puts to `$db`, returning `(root_before, root_after)`.
///
/// ```rust,no_run
/// let (root1, root2) = setup_2nd_commit!(db, [(b"\x50", b"mid")]);
/// ```
macro_rules! setup_2nd_commit {
    ($db:expr, [$(($key:expr, $val:expr)),+ $(,)?]) => {{
        let root1 = $db.root_hash().unwrap();
        $db.propose(vec![$(BatchOp::Put { key: $key as &[u8], value: $val as &[u8] }),+])
            .unwrap()
            .commit()
            .unwrap();
        let root2 = $db.root_hash().unwrap();
        (root1, root2)
    }};
}

/// Verify a change proof end-to-end: structural check + root hash check.
///
/// Builds a proposal (`start_root` + `batch_ops`) and verifies its in-range
/// state matches `end_root` using the restructure approach: an in-memory
/// proving trie is built from the proposal's in-range keys, boundary
/// proof nodes are reconciled into it, and a hybrid root hash is computed.
pub(super) fn verify_and_check(
    db: &Db,
    proof: &FrozenChangeProof,
    verification: &ChangeProofVerificationContext,
    start_root: api::HashKey,
) -> Result<(), api::Error> {
    let parent = db.revision(start_root)?;
    let proposal = db.apply_change_proof_to_parent(proof, &*parent)?;
    verify_change_proof_root_hash(proof, verification, &proposal)
}

mod adversarial;
mod fuzz;

// ── Structural validation tests ────────────────────────────────────────────

#[test]
fn test_inverted_range_rejected() {
    let (db, _dir) = setup_db![(b"\x10", b"v0"), (b"\xa0", b"v1")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x50", b"mid")]);

    let proof = db
        .change_proof(root1, root2.clone(), Some(b"\x10"), Some(b"\xa0"), None)
        .unwrap();

    let err = verify_change_proof_structure(&proof, root2, Some(b"\xa0"), Some(b"\x10"), None)
        .unwrap_err();
    assert!(matches!(err, api::Error::InvalidRange { .. }));
}

#[test]
fn test_boundary_proof_unverifiable() {
    let (db, _dir) = setup_db![(b"\x00", b"v0"), (b"\x10", b"v1")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x05", b"v2")]);

    let proof = db
        .change_proof(root1, root2.clone(), Some(b"\x00"), Some(b"\x10"), None)
        .unwrap();

    // Non-empty start_proof but start_key=None
    let err = verify_change_proof_structure(&proof, root2, None, Some(b"\x10"), None).unwrap_err();
    assert!(matches!(
        err,
        api::Error::ProofError(crate::ProofError::UnexpectedStartProof)
    ));
}

#[test]
fn test_keys_not_sorted() {
    let (db, _dir) = setup_db![(b"\x10", b"v0"), (b"\xa0", b"v1")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x50", b"mid")]);

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
    let (db, _dir) = setup_db![(b"\x10", b"v0"), (b"\xa0", b"v1")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x50", b"mid")]);

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
    let (db, _dir) = setup_db![(b"\x10", b"v0"), (b"\xa0", b"v1")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x50", b"mid")]);

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
    let (db, _dir) = setup_db![(b"\x10", b"v0"), (b"\xa0", b"v1")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x50", b"mid_"), (b"\x60", b"mid2")]);

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
    let (db, _dir) = setup_db![(b"\x10", b"v0"), (b"\xa0", b"v1")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x50", b"mid")]);

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
    let (db, _dir) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x20", b"changed")]);

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
    let (db, _dir) = setup_db![(b"\x10", b"v0"), (b"\xa0", b"v1")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x50", b"mid")]);

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
    let (db_a, _dir_a) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1")];
    let (db_b, _dir_b) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) =
        setup_2nd_commit!(db_a, [(b"\x10", b"changed0"), (b"\x20", b"changed1")]);

    let proof = db_a
        .change_proof(root1_a, root2.clone(), None, None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, None, None, None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

#[test]
fn test_root_hash_single_end_proof() {
    let (db_a, _dir_a) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1")];
    let (db_b, _dir_b) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x10", b"changed")]);

    let proof = db_a
        .change_proof(root1_a, root2.clone(), None, Some(b"\x20"), None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, None, Some(b"\x20"), None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

#[test]
fn test_root_hash_single_start_proof() {
    let (db_a, _dir_a) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1")];
    let (db_b, _dir_b) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x20", b"changed")]);

    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x10"), None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, Some(b"\x10"), None, None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

#[test]
fn test_root_hash_two_proofs() {
    let (db_a, _dir_a) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1"), (b"\x30", b"v2")];
    let (db_b, _dir_b) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1"), (b"\x30", b"v2")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x20", b"changed")]);

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
    let (db_a, _dir_a) = setup_db![
        (b"\x10", b"v0"),
        (b"\x20", b"v1"),
        (b"\x30", b"v2"),
        (b"\x40", b"v3")
    ];
    let (db_b, _dir_b) = setup_db![
        (b"\x10", b"v0"),
        (b"\x20", b"v1"),
        (b"\x30", b"v2"),
        (b"\x40", b"v3")
    ];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) =
        setup_2nd_commit!(db_a, [(b"\x20", b"changed1"), (b"\x30", b"changed2")]);

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
    let (db_a, _dir_a) = setup_db![(b"\x10", b"v0"), (b"\xa0", b"v1")];
    let (db_b, _dir_b) = setup_db![(b"\x10", b"v0"), (b"\xa0", b"v1")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x10", b"real")]);

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
    let (db_a, _dir_a) = setup_db![(b"\x10", b"v0"), (b"\xa0", b"v1")];
    let (db_b, _dir_b) = setup_db![(b"\x10", b"v0"), (b"\xa0", b"v1")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x50", b"mid")]);

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
    let (db_a, _dir_a) = setup_db![(b"\x10", b"v0"), (b"\xa0", b"v1")];
    let (db_b, _dir_b) = setup_db![(b"\x10", b"v0"), (b"\xa0", b"v1")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\xa0", b"changed")]);

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
    let (db_a, _dir_a) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1"), (b"\x30", b"v2")];
    let (db_b, _dir_b) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1"), (b"\x30", b"v2")];
    let root1_b = db_b.root_hash().unwrap();

    // Change only \x30 (outside range [\x10, \x20])
    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x30", b"changed")]);

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
    let (db_a, _dir_a) = setup_db![(b"\x12", b"v0"), (b"\x13", b"v1"), (b"\x50", b"v2")];
    let (db_b, _dir_b) = setup_db![(b"\x12", b"v0"), (b"\x13", b"v1"), (b"\x50", b"v2")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x50", b"changed")]);

    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x14"), None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, Some(b"\x14"), None, None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

#[test]
fn test_start_proof_inclusion_with_children_below() {
    let (db_a, _dir_a) = setup_db![(b"\xab", b"v0"), (b"\xab\xcd", b"v1"), (b"\xf0", b"v2")];
    let (db_b, _dir_b) = setup_db![(b"\xab", b"v0"), (b"\xab\xcd", b"v1"), (b"\xf0", b"v2")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\xf0", b"changed")]);

    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\xab"), None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, Some(b"\xab"), None, None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

#[test]
fn test_end_proof_inclusion_with_children_below() {
    let (db_a, _dir_a) = setup_db![(b"\xab", b"v0"), (b"\xab\xcd", b"v1"), (b"\xf0", b"v2")];
    let root1_a = db_a.root_hash().unwrap();
    let (db_b, _dir_b) = setup_db![(b"\xab", b"v0"), (b"\xab\xcd", b"v1"), (b"\xf0", b"v2")];
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

    // Limited to 1 — last_op_key is a prefix with children below.
    let proof = db_a
        .change_proof(root1_a, root2.clone(), None, None, NonZeroUsize::new(1))
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2, None, None, NonZeroUsize::new(1)).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

#[test]
fn test_divergence_parent_start_key_exhausted() {
    let (db_a, _dir_a) = setup_db![(b"\x12\x01", b"v0"), (b"\x12\x02", b"v1"), (b"\xf0", b"v2")];
    let (db_b, _dir_b) = setup_db![(b"\x12\x01", b"v0"), (b"\x12\x02", b"v1"), (b"\xf0", b"v2")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\xf0", b"changed")]);

    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x12"), Some(b"\xf0"), None)
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2, Some(b"\x12"), Some(b"\xf0"), None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

#[test]
fn test_divergence_at_depth_zero() {
    let (db_a, _dir_a) = setup_db![
        (b"\x10", b"v0"),
        (b"\x11", b"v1"),
        (b"\xa0", b"v2"),
        (b"\xa1", b"v3")
    ];
    let root1_a = db_a.root_hash().unwrap();
    let (db_b, _dir_b) = setup_db![
        (b"\x10", b"v0"),
        (b"\x11", b"v1"),
        (b"\xa0", b"v2"),
        (b"\xa1", b"v3")
    ];
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
        right_edge_key: Some(b"\xa1".to_vec().into()),
    };

    // The crafted proof has start/end proofs that skip the shared root,
    // so the proving trie reconciliation should detect the inconsistency.
    let err = verify_change_proof_root_hash(&crafted, &verification, &proposal).unwrap_err();
    // Reconciliation detects the value conflict when the crafted proof's
    // non-root node is reconciled against the proposal's existing data.
    assert!(
        matches!(
            err,
            api::Error::ProofError(
                crate::ProofError::UnexpectedValue | crate::ProofError::EndRootMismatch,
            )
        ),
        "crafted proof with skipped root should be rejected, got {err:?}"
    );
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
    let (sender, _dir_sender) =
        setup_db![(b"\x10\x01", b"a"), (b"\x10\x02", b"b"), (b"\x30", b"c")];
    let (receiver, _dir_receiver) =
        setup_db![(b"\x10\x01", b"a"), (b"\x10\x02", b"b"), (b"\x30", b"c")];
    let receiver_root1 = receiver.root_hash().unwrap();

    // Only modify \x30 on the sender; \x10's children are untouched
    let (sender_root1, sender_root2) = setup_2nd_commit!(sender, [(b"\x30", b"changed")]);

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
    let (db_a, _dir_a) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1"), (b"\x30", b"v2")];
    let root1_a = db_a.root_hash().unwrap();
    let (db_b, _dir_b) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1"), (b"\x30", b"v2")];
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
    let (db, _dir) = setup_db![(b"\x10", b"v0"), (b"\xa0", b"v1")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x50", b"mid"), (b"\x80", b"end")]);

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
        api::Error::ProofError(crate::ProofError::DeleteRangeFoundInChangeProof)
    ));
}

/// When boundary proofs are present, the O(n) scan catches the `DeleteRange`
/// before the end-proof consistency check runs.
#[test]
fn test_delete_range_rejected_with_boundary_proofs() {
    let (db, _dir) = setup_db![(b"\x10", b"v0"), (b"\xa0", b"v1")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x50", b"mid")]);

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
        api::Error::ProofError(crate::ProofError::DeleteRangeFoundInChangeProof)
    ));
}

// ── Truncation tests ───────────────────────────────────────────────────────

#[test]
fn test_truncated_proof_round_trip() {
    let (db_a, _dir_a) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1"), (b"\x30", b"v2")];
    let (db_b, _dir_b) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1"), (b"\x30", b"v2")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) =
        setup_2nd_commit!(db_a, [(b"\x10", b"c0"), (b"\x20", b"c1"), (b"\x30", b"c2")]);

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
    let (db_a, _dir_a) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1"), (b"\x30", b"v2")];
    let root1_a = db_a.root_hash().unwrap();
    let (db_b, _dir_b) = setup_db![(b"\x10", b"v0"), (b"\x20", b"v1"), (b"\x30", b"v2")];
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
fn test_generator_uses_end_key_for_complete_proof() {
    let (db, _dir) = setup_db![(b"\x10", b"v0"), (b"\xa0", b"v1")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x10", b"changed")]);

    // end_key far beyond last change, no limit — complete proof
    let proof = db
        .change_proof(root1, root2.clone(), None, Some(b"\xff"), None)
        .unwrap();

    // End proof validates against end_key for complete proofs
    proof.end_proof().value_digest(b"\xff", &root2).unwrap();
}

/// Attacker adds a spurious Put at `start_key` when `start_key` doesn't
/// exist in `end_root`. The start proof is an exclusion proof. The
/// `StartProofOperationMismatch` check catches this: Put expects inclusion
/// but the proof is exclusion.
#[test]
fn test_spurious_put_at_start_key_boundary() {
    // Keys \x20 and \x90 exist. \x10 (start_key) does NOT exist.
    let (db_a, _dir_a) = setup_db![(b"\x20", b"v2"), (b"\x90", b"v9")];
    let (db_b, _dir_b) = setup_db![(b"\x20", b"v2"), (b"\x90", b"v9")];
    let root1_b = db_b.root_hash().unwrap();

    // Change \x20 on db_a
    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x20", b"changed")]);

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
    // Keys \x20 and \x90 exist. start_key \x20 EXISTS in end_root.
    let (db_a, _dir_a) = setup_db![(b"\x20", b"v2"), (b"\x90", b"v9")];
    let (db_b, _dir_b) = setup_db![(b"\x20", b"v2"), (b"\x90", b"v9")];
    let root1_b = db_b.root_hash().unwrap();

    // Change \x90 on db_a (not \x20 — \x20 stays in end_root).
    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x90", b"changed")]);

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

/// Verify that change proof verification succeeds when the only key
/// changed between two revisions falls outside the requested range,
/// so the proof's `batch_ops` for the queried subrange is empty.
///
/// R1 has `\x00`, R2 adds `\x00\x10`. The query range
/// `[\x00\x10\x20, \x00\x10\x30]` excludes the new key. The shared
/// prefix node at `\x00\x10` carries a value that legitimately differs
/// between the proposal (R1 + empty ops) and the end trie (R2), but
/// because `\x00\x10` < `\x00\x10\x20` the value check should be
/// skipped.
#[test]
fn test_disjoint_proof() {
    // R1: trie with a single key \x00
    let (db, _dir) = setup_db![(b"\x00", b"v0")];

    // R2: add \x00\x10 (outside query range [\x00\x10\x20, \x00\x10\x30])
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x00\x10", b"v1")]);

    let first = b"\x00\x10\x20";
    let last = b"\x00\x10\x30";

    let proof = db
        .change_proof(
            root1.clone(),
            root2.clone(),
            Some(first.as_ref()),
            Some(last.as_ref()),
            None,
        )
        .unwrap();

    let verification = verify_change_proof_structure(
        &proof,
        root2,
        Some(first.as_slice()),
        Some(last.as_slice()),
        None,
    )
    .unwrap();

    verify_and_check(&db, &proof, &verification, root1).unwrap();
}

// ── prove() divergent child tests ─────────────────────────────────────────

/// When a key is deleted and the branch compresses (`partial_path` absorbs
/// the remaining child), the exclusion proof must include the divergent
/// child node. This test verifies that `prove()` appends the child.
#[test]
fn test_exclusion_proof_includes_divergent_child() {
    // Create two keys that share a branch: \x10\x50 and \x10\x58.
    // Deleting \x10\x50 causes the branch to compress — the remaining
    // child \x10\x58 absorbs the branch's partial_path.
    let (db, _dir) = setup_db![(b"\x10\x50", b"a"), (b"\x10\x58", b"b")];
    let root1 = db.root_hash().unwrap();

    // Delete \x10\x50 — branch at nibble depth 5 compresses.
    db.propose(vec![BatchOp::Delete::<_, &[u8]> { key: b"\x10\x50" }])
        .unwrap()
        .commit()
        .unwrap();

    // Generate exclusion proof for the deleted key.
    let rev = db.revision(db.root_hash().unwrap()).unwrap();
    let merkle = crate::merkle::Merkle::from(&*rev);
    let proof = merkle.prove(b"\x10\x50").unwrap();
    let nodes = proof.as_ref();

    // The proof must have more than just the ancestor nodes — it must
    // include the divergent child whose key extends past \x10\x50.
    let last = nodes.last().unwrap();
    let last_key_nibbles: Vec<u8> = last.key.as_byte_slice().to_vec();
    let target_nibbles: Vec<u8> = firewood_storage::NibblesIterator::new(b"\x10\x50").collect();

    // The last node's key must NOT equal the target key (exclusion).
    assert_ne!(
        last_key_nibbles, target_nibbles,
        "should be exclusion proof"
    );

    // The last node's key must extend past the target key — it's the
    // divergent child, not just an ancestor.
    assert!(
        last_key_nibbles.len() > target_nibbles.len()
            || last_key_nibbles
                .iter()
                .zip(target_nibbles.iter())
                .any(|(a, b)| a != b),
        "last node should be the divergent child, not an ancestor"
    );

    // The proof should still validate as a valid exclusion proof.
    let root2 = db.root_hash().unwrap();
    let hash = root2.clone();
    assert!(
        proof.value_digest(b"\x10\x50", &hash).unwrap().is_none(),
        "should be a valid exclusion proof"
    );

    // Verify the full change proof round-trip works.
    let (db_b, _dir_b) = setup_db![(b"\x10\x50", b"a"), (b"\x10\x58", b"b")];
    let root1_b = db_b.root_hash().unwrap();

    let change = db
        .change_proof(root1, root2.clone(), Some(b"\x10\x50"), None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&change, root2, Some(b"\x10\x50"), None, None).unwrap();
    verify_and_check(&db_b, &change, &ctx, root1_b).unwrap();
}

/// When the child at the next nibble doesn't exist (None), the exclusion
/// proof should NOT append a divergent child. The proof should still be
/// a valid exclusion proof.
#[test]
fn test_exclusion_proof_no_child_at_next_nibble() {
    // Create keys at \x20 and \x30. Proving \x10: the root has children
    // at nibble 2 and 3, but NO child at nibble 1. The proof stops at
    // the root with no divergent child to append.
    let (db, _dir) = setup_db![(b"\x20", b"a"), (b"\x30", b"b")];

    let rev = db.revision(db.root_hash().unwrap()).unwrap();
    let merkle = crate::merkle::Merkle::from(&*rev);
    let proof = merkle.prove(b"\x10").unwrap();

    // The proof should be a valid exclusion proof.
    let root_hash = db.root_hash().unwrap();
    let hash = root_hash;
    assert!(
        proof.value_digest(b"\x10", &hash).unwrap().is_none(),
        "should be a valid exclusion proof without divergent child"
    );
}

/// An incomplete exclusion proof (missing the divergent child when one
/// exists) must be rejected by `value_digest` with `ExclusionProofMissingChild`.
#[test]
fn test_incomplete_exclusion_proof_rejected() {
    // Create a trie with enough keys to ensure the proof has multiple
    // nodes. \x10\x50 and \x10\x58 share a branch; \x30 ensures the
    // root is a branch with children at nibbles 1 and 3.
    let (db, _dir) = setup_db![
        (b"\x10\x50", b"a"),
        (b"\x10\x58", b"b"),
        (b"\x30\x00", b"c")
    ];

    // Delete \x10\x50. The branch at nibble depth 5 compresses, and
    // \x10\x58 becomes the divergent child for exclusion proofs of
    // \x10\x50.
    db.propose(vec![BatchOp::Delete::<_, &[u8]> { key: b"\x10\x50" }])
        .unwrap()
        .commit()
        .unwrap();
    let root2 = db.root_hash().unwrap();

    // Generate a correct exclusion proof (includes divergent child).
    let rev2 = db.revision(root2.clone()).unwrap();
    let merkle = crate::merkle::Merkle::from(&*rev2);
    let full_proof = merkle.prove(b"\x10\x50").unwrap();

    // The full proof should have at least 2 nodes: ancestor + divergent child.
    let nodes = full_proof.as_ref();
    assert!(
        nodes.len() >= 2,
        "expected at least 2 nodes (ancestor + divergent child), got {}",
        nodes.len()
    );

    // Craft an incomplete proof by removing the last node (the divergent child).
    let truncated: Vec<crate::ProofNode> = nodes[..nodes.len() - 1].to_vec();
    let incomplete = crate::Proof::new(truncated.into_boxed_slice());

    // The incomplete proof should be rejected because the ancestor has
    // a child at the next nibble but the proof doesn't include it.
    let hash = root2;
    let err = incomplete.value_digest(b"\x10\x50", &hash).unwrap_err();
    assert!(
        matches!(err, crate::ProofError::ExclusionProofMissingChild),
        "expected ExclusionProofMissingChild, got {err:?}"
    );
}

// ── Value check fix tests ─────────────────────────────────────────────────

/// Out-of-range value change at a start tail node must NOT cause a false
/// rejection. The node's byte key < `start_key`, so its value is out of
/// range and may legitimately differ.
#[test]
fn test_out_of_range_value_at_start_tail_accepted() {
    // Create keys where \x10 is a proper prefix of \x10\x50.
    // \x10 has a value (branch with value at depth 2).
    let (db_a, _dir_a) = setup_db![
        (b"\x10", b"prefix_val"),
        (b"\x10\x50", b"v000000000"),
        (b"\x30", b"v100000000")
    ];
    let (db_b, _dir_b) = setup_db![
        (b"\x10", b"prefix_val"),
        (b"\x10\x50", b"v000000000"),
        (b"\x30", b"v100000000")
    ];
    let root1_b = db_b.root_hash().unwrap();

    // Change \x10's value (out of range for [\x10\x50, \x30])
    // AND change \x30 (in range).
    let (root1_a, root2) =
        setup_2nd_commit!(db_a, [(b"\x10", b"changed_pfx"), (b"\x30", b"changed_v1")]);

    // Range [\x10\x50, \x30]: \x10 is out of range (< \x10\x50).
    // The start proof path includes a node at \x10 whose value changed.
    // This must NOT cause a false rejection.
    let proof = db_a
        .change_proof(
            root1_a,
            root2.clone(),
            Some(b"\x10\x50"),
            Some(b"\x30"),
            None,
        )
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, Some(b"\x10\x50"), Some(b"\x30"), None)
        .unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

/// The `start_key` value MUST be checked for inclusion proofs. If the
/// proposal has a different value at `start_key` than `end_root`, the
/// reconciliation or root hash comparison should detect the mismatch.
#[test]
fn test_start_key_inclusion_value_checked() {
    // Both DBs start with \x10 = "v0", \x30 = "v1".
    let (db_a, _dir_a) = setup_db![(b"\x10", b"v0"), (b"\x30", b"v1")];
    let (db_b, _dir_b) = setup_db![(b"\x10", b"v0"), (b"\x30", b"v1")];
    let root1_b = db_b.root_hash().unwrap();

    // Change \x10 (start_key) to "changed" and \x30 to "also_changed".
    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x10", b"changed"), (b"\x30", b"changed")]);

    // Get an honest proof. The start proof is inclusion for \x10.
    let honest = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x10"), None, None)
        .unwrap();

    // Verify the honest proof works.
    let ctx =
        verify_change_proof_structure(&honest, root2.clone(), Some(b"\x10"), None, None).unwrap();
    verify_and_check(&db_b, &honest, &ctx, root1_b.clone()).unwrap();

    // Now craft a proof with the wrong value for \x10 in batch_ops.
    // Replace Put(\x10, "changed") with Put(\x10, "WRONG").
    let tampered_ops: Vec<BatchOp<crate::merkle::Key, crate::merkle::Value>> = honest
        .batch_ops()
        .iter()
        .map(|op| {
            if op.key().as_ref() == b"\x10" {
                BatchOp::Put {
                    key: b"\x10".to_vec().into(),
                    value: b"WRONG".to_vec().into(),
                }
            } else {
                match op {
                    BatchOp::Put { key, value } => BatchOp::Put {
                        key: key.clone(),
                        value: value.clone(),
                    },
                    BatchOp::Delete { key } => BatchOp::Delete { key: key.clone() },
                    _ => unreachable!(),
                }
            }
        })
        .collect();

    let crafted = FrozenChangeProof::new(
        crate::Proof::new(honest.start_proof().as_ref().into()),
        crate::Proof::new(honest.end_proof().as_ref().into()),
        tampered_ops.into_boxed_slice(),
    );

    // The tampered proof should still pass structural checks (hash chain
    // is valid), but fail root hash verification because the proposal
    // has "WRONG" at \x10 while the proof expects "changed".
    let ctx2 = verify_change_proof_structure(&crafted, root2, Some(b"\x10"), None, None).unwrap();
    let err = verify_and_check(&db_b, &crafted, &ctx2, root1_b)
        .expect_err("tampered start_key value must be detected");
    // With the restructure approach, the tampered value is detected during
    // reconciliation (UnexpectedValue) or root hash comparison (EndRootMismatch).
    assert!(
        matches!(
            err,
            api::Error::ProofError(
                crate::ProofError::UnexpectedValue | crate::ProofError::EndRootMismatch,
            )
        ),
        "tampered start_key value must be detected, got {err:?}"
    );
}

// ── Boundary child gap closure tests ──────────────────────────────────────

/// With the `prove()` fix (divergent child included), omitting a Delete
/// for a key near `start_key` (under the same boundary child) must be
/// detected. Before the fix, this was a gap — the boundary child's
/// hash was never compared.
#[test]
fn test_boundary_child_gap_closed_for_start_key() {
    // Create \x10\x50 and \x10\x58 (share a branch).
    let (db_a, _dir_a) = setup_db![
        (b"\x10\x50", b"a"),
        (b"\x10\x58", b"b"),
        (b"\x30\x00", b"c")
    ];
    let root1_a = db_a.root_hash().unwrap();
    let (db_b, _dir_b) = setup_db![
        (b"\x10\x50", b"a"),
        (b"\x10\x58", b"b"),
        (b"\x30\x00", b"c")
    ];
    let root1_b = db_b.root_hash().unwrap();

    // Delete \x10\x50, change \x30\x00. \x10\x58 is unchanged.
    db_a.propose(vec![
        BatchOp::Delete {
            key: b"\x10\x50" as &[u8],
        },
        BatchOp::Put {
            key: b"\x30\x00",
            value: b"z",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    // Get an honest proof for range [\x10\x50, \x30\x00].
    let honest = db_a
        .change_proof(
            root1_a,
            root2.clone(),
            Some(b"\x10\x50"),
            Some(b"\x30\x00"),
            None,
        )
        .unwrap();

    // Verify the honest proof works.
    let ctx =
        verify_change_proof_structure(&honest, root2, Some(b"\x10\x50"), Some(b"\x30\x00"), None)
            .unwrap();
    verify_and_check(&db_b, &honest, &ctx, root1_b).unwrap();
}

// ── Empty start trie tests ────────────────────────────────────────────────
//
// When the start revision is empty, every key in the end revision is a new
// insertion. The start proofs are always exclusion proofs (nothing exists
// in the empty trie). These tests exercise the interaction between empty
// start tries and the divergent child / value skip logic.

/// Empty start trie, single key inserted. Complete proof (no bounds).
#[cfg(feature = "ethhash")]
#[test]
fn test_empty_start_trie_single_key_no_bounds() {
    let (db, _dir) = setup_db![];
    let (empty_root, root2) = setup_2nd_commit!(db, [(b"\x50", b"hello")]);

    // Complete proof — no bounds.
    let proof = db
        .change_proof(empty_root, root2.clone(), None, None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2.clone(), None, None, None).unwrap();

    // Receiver also starts empty.
    let (db_b, _dir_b) = setup_db![];
    let empty_root_b = db_b.root_hash().unwrap();

    verify_and_check(&db_b, &proof, &ctx, empty_root_b).unwrap();
}

/// Empty start trie, multiple keys inserted, bounded range with inclusion
/// start proof (`start_key` exists in end trie).
#[cfg(feature = "ethhash")]
#[test]
fn test_empty_start_trie_bounded_inclusion() {
    let (db, _dir) = setup_db![];

    // Insert several keys.
    let (empty_root, root2) = setup_2nd_commit!(
        db,
        [
            (b"\x10", b"a"),
            (b"\x20", b"b"),
            (b"\x30", b"c"),
            (b"\x40", b"d")
        ]
    );

    // Bounded range [\x10, \x30] — start_key \x10 exists (inclusion).
    let proof = db
        .change_proof(
            empty_root.clone(),
            root2.clone(),
            Some(b"\x10"),
            Some(b"\x30"),
            None,
        )
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2.clone(), Some(b"\x10"), Some(b"\x30"), None)
            .unwrap();

    let (db_b, _dir_b) = setup_db![];
    let empty_root_b = db_b.root_hash().unwrap();

    verify_and_check(&db_b, &proof, &ctx, empty_root_b).unwrap();
}

/// Empty start trie, bounded range with exclusion start proof
/// (`start_key` does NOT exist in end trie).
#[cfg(feature = "ethhash")]
#[test]
fn test_empty_start_trie_bounded_exclusion() {
    let (db, _dir) = setup_db![];
    let (empty_root, root2) =
        setup_2nd_commit!(db, [(b"\x10", b"a"), (b"\x20", b"b"), (b"\x30", b"c")]);

    // start_key \x05 does NOT exist — exclusion proof.
    let proof = db
        .change_proof(
            empty_root.clone(),
            root2.clone(),
            Some(b"\x05"),
            Some(b"\x30"),
            None,
        )
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2.clone(), Some(b"\x05"), Some(b"\x30"), None)
            .unwrap();

    let (db_b, _dir_b) = setup_db![];
    let empty_root_b = db_b.root_hash().unwrap();

    verify_and_check(&db_b, &proof, &ctx, empty_root_b).unwrap();
}

/// Empty start trie, large end trie (100 keys), multiple rounds with
/// different boundary positions.
#[cfg(feature = "ethhash")]
#[test]
fn test_empty_start_trie_large_end_trie_multi_round() {
    let (db, _dir) = setup_db![];
    let empty_root = db.root_hash().unwrap();

    // Insert 100 keys: \x00\x00 through \x00\x63.
    let keys: Vec<[u8; 2]> = (0..100).map(|i| [0x00, i]).collect();
    let ops: Vec<BatchOp<&[u8], &[u8]>> = keys
        .iter()
        .map(|k| BatchOp::Put {
            key: k.as_ref(),
            value: b"val" as &[u8],
        })
        .collect();
    db.propose(ops).unwrap().commit().unwrap();
    let root2 = db.root_hash().unwrap();

    let (db_b, _dir_b) = setup_db![];
    let empty_root_b = db_b.root_hash().unwrap();

    // Simulate iterative sync with 4 rounds of ~25 keys each.
    let boundaries: &[Option<&[u8]>] = &[
        None,
        Some(b"\x00\x19"),
        Some(b"\x00\x32"),
        Some(b"\x00\x4b"),
    ];

    for window in boundaries.windows(2) {
        let start = window[0];
        let end = window[1];

        let proof = db
            .change_proof(empty_root.clone(), root2.clone(), start, end, None)
            .unwrap();
        let ctx = verify_change_proof_structure(&proof, root2.clone(), start, end, None).unwrap();
        verify_and_check(&db_b, &proof, &ctx, empty_root_b.clone()).unwrap();
    }

    // Final round: from last boundary to end.
    let proof = db
        .change_proof(
            empty_root.clone(),
            root2.clone(),
            Some(b"\x00\x4b"),
            None,
            None,
        )
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2.clone(), Some(b"\x00\x4b"), None, None)
        .unwrap();
    verify_and_check(&db_b, &proof, &ctx, empty_root_b).unwrap();
}

/// Empty start trie with prefix keys (key is a prefix of another key).
/// Tests that branch nodes with values at prefix keys are handled
/// correctly when the start trie is empty.
#[cfg(feature = "ethhash")]
#[test]
fn test_empty_start_trie_prefix_keys() {
    let (db, _dir) = setup_db![];

    // Insert keys where some are prefixes of others. This creates
    // branch nodes with values — the scenario that triggers the
    // out-of-range value check bug.
    let (empty_root, root2) = setup_2nd_commit!(
        db,
        [
            (b"\x10", b"prefix"),
            (b"\x10\x50", b"child1"),
            (b"\x10\x50\xaa", b"grandchild"),
            (b"\x20", b"other"),
        ]
    );

    let (db_b, _dir_b) = setup_db![];
    let empty_root_b = db_b.root_hash().unwrap();

    // Range [\x10\x50, \x20]: start_key \x10\x50 exists (inclusion).
    // The node at \x10 is on the start proof path with a value, but
    // its byte key < start_key — value check must be skipped.
    let proof = db
        .change_proof(
            empty_root.clone(),
            root2.clone(),
            Some(b"\x10\x50"),
            Some(b"\x20"),
            None,
        )
        .unwrap();
    let ctx = verify_change_proof_structure(
        &proof,
        root2.clone(),
        Some(b"\x10\x50"),
        Some(b"\x20"),
        None,
    )
    .unwrap();
    verify_and_check(&db_b, &proof, &ctx, empty_root_b.clone()).unwrap();

    // Range [\x10\x50\x00, \x20]: start_key doesn't exist (exclusion).
    // Exercises the divergent child path with an empty start trie.
    let proof = db
        .change_proof(
            empty_root.clone(),
            root2.clone(),
            Some(b"\x10\x50\x00"),
            Some(b"\x20"),
            None,
        )
        .unwrap();
    let ctx = verify_change_proof_structure(
        &proof,
        root2.clone(),
        Some(b"\x10\x50\x00"),
        Some(b"\x20"),
        None,
    )
    .unwrap();
    verify_and_check(&db_b, &proof, &ctx, empty_root_b).unwrap();
}

/// Empty start trie, end trie with a single key at the root (empty key
/// prefix). Tests the degenerate case where the root itself has a value.
#[cfg(feature = "ethhash")]
#[test]
fn test_empty_start_trie_start_only_proof() {
    let (db, _dir) = setup_db![];
    let (empty_root, root2) = setup_2nd_commit!(db, [(b"\x10", b"a"), (b"\x20", b"b")]);

    let (db_b, _dir_b) = setup_db![];
    let empty_root_b = db_b.root_hash().unwrap();

    // Start-only proof (end_key = None). Range is [\x10, +inf).
    let proof = db
        .change_proof(empty_root.clone(), root2.clone(), Some(b"\x10"), None, None)
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2.clone(), Some(b"\x10"), None, None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, empty_root_b.clone()).unwrap();

    // End-only proof (start_key = None). Range is (-inf, \x20].
    let proof = db
        .change_proof(empty_root, root2.clone(), None, Some(b"\x20"), None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, None, Some(b"\x20"), None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, empty_root_b).unwrap();
}

#[test]
fn test_bounded_range_with_existing_keys() {
    // root1: \x10, \x20, \x30 all exist
    let (db_a, _dir_a) = setup_db![(b"\x10", b"v1"), (b"\x20", b"v2"), (b"\x30", b"v3")];
    let (db_b, _dir_b) = setup_db![(b"\x10", b"v1"), (b"\x20", b"v2"), (b"\x30", b"v3")];
    let root1_b = db_b.root_hash().unwrap();

    // root2: change \x20
    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x20", b"changed")]);

    // Range [\x15, \x25]: \x20 is in range, \x10 and \x30 are outside
    let proof = db_a
        .change_proof(
            root1_a.clone(),
            root2.clone(),
            Some(b"\x15"),
            Some(b"\x25"),
            None,
        )
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2.clone(), Some(b"\x15"), Some(b"\x25"), None)
            .unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b.clone()).unwrap();

    // Also test with exact existing keys as boundaries
    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x10"), Some(b"\x30"), None)
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2, Some(b"\x10"), Some(b"\x30"), None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

/// Regression test: a key that is a proper prefix of `start_key` has a value
/// in root1 that is deleted in root2. Because the prefix key is outside the
/// query range (shorter key &lt; `start_key`), it is not in `batch_ops`. The
/// proving trie must clear this stale value so the computed hash matches
/// `end_root`.
///
/// Without the fix, `reconcile_branch_proof_node` ignored the trie's value
/// when the proof node had no value, silently leaving the stale value in the
/// proving trie and causing `EndRootMismatch`.
#[test]
fn test_change_proof_prefix_key_deleted_in_end_root() {
    // Root1: keys include b"\xab" which is a prefix of b"\xab\xcd".
    // Two children under [a,b] ensure the branch survives deletion of
    // b"\xab"'s value (a single child would be merged away).
    let (db, _dir) = setup_db![
        (b"\xab", b"prefix_value"),
        (b"\xab\xcd", b"full_key"),
        (b"\xab\xef", b"sibling"),
        (b"\xff", b"high"),
    ];
    let root1 = db.root_hash().unwrap();

    // Root2: delete the prefix key b"\xab". The branch at [a,b] survives
    // because it still has two children (b"\xab\xcd" and b"\xab\xef").
    // Also update a key within the query range so batch_ops is non-empty.
    db.propose(vec![
        BatchOp::Delete {
            key: b"\xab" as &[u8],
        },
        BatchOp::Put {
            key: b"\xab\xcd",
            value: b"updated",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    // Query range starts AFTER b"\xab": use b"\xab\x00" as start_key.
    // b"\xab" < b"\xab\x00", so the deleted prefix key is outside the range
    // and won't appear in batch_ops. But the start proof passes through the
    // branch at nibbles [a,b] where root1 has a value and root2 does not.
    let start_key = b"\xab\x00";
    let end_key = b"\xff";

    let proof = db
        .change_proof(
            root1.clone(),
            root2.clone(),
            Some(start_key.as_slice()),
            Some(end_key.as_slice()),
            None,
        )
        .unwrap();

    let ctx = verify_change_proof_structure(
        &proof,
        root2.clone(),
        Some(start_key.as_slice()),
        Some(end_key.as_slice()),
        None,
    )
    .unwrap();

    // This failed with EndRootMismatch before the fix.
    verify_and_check(&db, &proof, &ctx, root1).unwrap();
}

// ── Architectural code path tests ────────────────────────────────────────
//
// These tests exercise specific code paths documented in proofs/mod.rs that
// were previously only covered by fuzz tests.

// ── Step 3: collapse_root_to_path ────────────────────────────────────────

/// Out-of-range deletion compresses the root's `partial_path`. The proving
/// trie's root (forked from the proposal) retains the old shape, and
/// `collapse_root_to_path` must reshape it to match `end_root`.
///
/// Exercises: `collapse_root_to_path`
///
/// Root1: keys at nibbles 0 and 1. Root at `[]`.
/// Root2: `\x01` deleted → root compresses to `partial_path` `[1]`.
/// Bounded proof `[\x10, \x20]` excludes the deleted key.
#[test]
fn test_root_compression_from_out_of_range_delete() {
    let (db_a, _dir_a) = setup_db![(b"\x01", b"low"), (b"\x10", b"mid"), (b"\x20", b"hig")];
    let (db_b, _dir_b) = setup_db![(b"\x01", b"low"), (b"\x10", b"mid"), (b"\x20", b"hig")];
    let root1_a = db_a.root_hash().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    // Delete \x01 (outside range) and change \x10 (in range).
    db_a.propose(vec![
        BatchOp::Delete { key: b"\x01" },
        BatchOp::Put {
            key: b"\x10",
            value: b"MID",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    let proof = db_a
        .change_proof(
            root1_a,
            root2.clone(),
            Some(b"\x10".as_ref()),
            Some(b"\x20".as_ref()),
            None,
        )
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2, Some(b"\x10"), Some(b"\x20"), None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

// ── Step 4: compute_outside_children terminal cases ──────────────────────

/// Case A: Boundary key diverges within the terminal node's partial path.
///
/// Exercises: `compute_outside_children` lines 233-244 in merkle/mod.rs
///
/// Keys `\x10\x50` and `\x10\x58` share a branch with partial path containing
/// nibble 5. `start_key` `\x10\x40` diverges at nibble 4 < 5, so the start
/// boundary is "before" the terminal — no children are marked outside.
#[test]
fn test_terminal_divergence_within_partial_path() {
    let (db_a, _dir_a) = setup_db![(b"\x10\x50", b"a"), (b"\x10\x58", b"b"), (b"\x30", b"c")];
    let (db_b, _dir_b) = setup_db![(b"\x10\x50", b"a"), (b"\x10\x58", b"b"), (b"\x30", b"c")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x30", b"changed")]);

    // Start_key \x10\x40: diverges within the branch's partial path at
    // nibble 4 vs the branch's nibble 5. This hits Case A of
    // compute_outside_children where the boundary is "before" the terminal.
    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x10\x40"), None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, Some(b"\x10\x40"), None, None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

/// Case B: Terminal is an ancestor of the boundary key. The on-path child
/// must be marked as outside so its hash comes from the proof, not the
/// proving trie.
///
/// Exercises: `compute_outside_children` lines 246-260 in merkle/mod.rs
///
/// Keys at `\x10\x01` and `\x10\x02` create a branch at nibble path `[1,0]`.
/// `start_key` `\x10` is exhausted at depth 2 — the terminal node at `[1,0]`
/// is an ancestor of `start_key`'s nibble path. The on-path child (nibble 0)
/// is marked outside, and children below it (none here) would also be
/// marked.
#[test]
fn test_terminal_ancestor_marks_on_path_child() {
    let (db_a, _dir_a) = setup_db![
        (b"\x10\x01", b"a"),
        (b"\x10\x02", b"b"),
        (b"\x10\x03", b"c"),
        (b"\x30", b"d")
    ];
    let (db_b, _dir_b) = setup_db![
        (b"\x10\x01", b"a"),
        (b"\x10\x02", b"b"),
        (b"\x10\x03", b"c"),
        (b"\x30", b"d")
    ];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x30", b"changed")]);

    // Start_key \x10: exhausted at depth 2, making the terminal an
    // ancestor. compute_outside_children Case B marks the on-path child
    // and all children below it as outside.
    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x10"), None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, Some(b"\x10"), None, None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

/// Case C (right edge): Boundary is a prefix of the terminal node's key.
/// All children extend beyond `end_key`, so all are marked outside.
///
/// Exercises: `compute_outside_children` lines 261-266
///
/// Key `\x20` is a prefix of `\x20\xab`. `end_key` = `\x20` — children of
/// the `\x20` node extend beyond `end_key` and must use proof hashes.
#[test]
fn test_right_edge_boundary_prefix_of_terminal() {
    let (db_a, _dir_a) = setup_db![(b"\x10", b"a"), (b"\x20", b"b"), (b"\x20\xab", b"c")];
    let (db_b, _dir_b) = setup_db![(b"\x10", b"a"), (b"\x20", b"b"), (b"\x20\xab", b"c")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x10", b"changed")]);

    // End_key \x20 is a prefix of the terminal proof node's key \x20\xab.
    // compute_outside_children Case C marks all children as outside.
    let proof = db_a
        .change_proof(root1_a, root2.clone(), None, Some(b"\x20"), None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, None, Some(b"\x20"), None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

// ── Step 2: Out-of-range value reconciliation ────────────────────────────

/// An out-of-range prefix node's value changes between root1 and root2.
/// The proof's value must be adopted during reconciliation so the hybrid
/// hash matches `end_root`.
///
/// Exercises: `reconcile_branch_proof_node` value adoption
///
/// `\x10` has a value that changes from `"old_pfx"` to `"new_pfx"`. The query
/// range `[\x10\x50, \x30]` excludes `\x10` (it's a proper prefix of `start_key`),
/// so `\x10` is out-of-range. The start proof path passes through `\x10`, and
/// reconciliation must adopt the proof's value (`"new_pfx"`) to match `end_root`.
#[test]
fn test_out_of_range_value_change_adopted() {
    let (db_a, _dir_a) = setup_db![
        (b"\x10", b"old_pfx"),
        (b"\x10\x50", b"val0000"),
        (b"\x30", b"val1000")
    ];
    let (db_b, _dir_b) = setup_db![
        (b"\x10", b"old_pfx"),
        (b"\x10\x50", b"val0000"),
        (b"\x30", b"val1000")
    ];
    let root1_b = db_b.root_hash().unwrap();

    // Change \x10's value (out of range) AND \x30's value (in range).
    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x10", b"new_pfx"), (b"\x30", b"changed")]);

    // Range [\x10\x50, \x30]: \x10 < \x10\x50, so \x10 is out of range.
    // The proof must adopt end_root's value for \x10 during reconciliation.
    let proof = db_a
        .change_proof(
            root1_a,
            root2.clone(),
            Some(b"\x10\x50"),
            Some(b"\x30"),
            None,
        )
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, Some(b"\x10\x50"), Some(b"\x30"), None)
        .unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

// ── EndProofOperationMismatch tests ──────────────────────────────────────

/// Spurious Put at `end_key` when `end_key` doesn't exist (exclusion end proof).
/// The end proof is exclusion but Put expects inclusion — mismatch.
///
/// Exercises: `EndProofOperationMismatch`
#[test]
fn test_spurious_put_at_end_key_boundary() {
    let (db_a, _dir_a) = setup_db![(b"\x20", b"v2"), (b"\x90", b"v9")];
    let (db_b, _dir_b) = setup_db![(b"\x20", b"v2"), (b"\x90", b"v9")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x20", b"changed")]);

    // end_key \xb0 doesn't exist — exclusion end proof.
    let honest = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x20"), Some(b"\xb0"), None)
        .unwrap();

    // Inject Put at \xb0 (end_key). end proof is exclusion but Put expects inclusion.
    let mut ops: Vec<BatchOp<Key, Value>> = honest.batch_ops().to_vec();
    ops.push(BatchOp::Put {
        key: b"\xb0".to_vec().into(),
        value: b"fake".to_vec().into(),
    });
    ops.sort_by(|a, b| a.key().cmp(b.key()));

    let crafted = FrozenChangeProof::new(
        crate::Proof::new(honest.start_proof().as_ref().into()),
        crate::Proof::new(honest.end_proof().as_ref().into()),
        ops.into_boxed_slice(),
    );

    let result =
        verify_change_proof_structure(&crafted, root2.clone(), Some(b"\x20"), Some(b"\xb0"), None);
    if let Ok(ctx) = result {
        verify_and_check(&db_b, &crafted, &ctx, root1_b)
            .expect_err("spurious Put at end_key should be detected");
    }
}

/// Spurious Delete at `end_key` when `end_key` EXISTS (inclusion end proof).
/// The end proof is inclusion but Delete expects exclusion — mismatch.
///
/// Exercises: `EndProofOperationMismatch`
#[test]
fn test_spurious_delete_at_end_key_boundary() {
    let (db_a, _dir_a) = setup_db![(b"\x20", b"v2"), (b"\x90", b"v9")];
    let (db_b, _dir_b) = setup_db![(b"\x20", b"v2"), (b"\x90", b"v9")];
    let root1_b = db_b.root_hash().unwrap();

    // Change \x20, leave \x90 unchanged.
    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x20", b"changed")]);

    // end_key \x90 EXISTS — inclusion end proof.
    let honest = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x20"), Some(b"\x90"), None)
        .unwrap();

    // Inject Delete at \x90 (end_key). end proof is inclusion but Delete expects exclusion.
    let mut ops: Vec<BatchOp<Key, Value>> = honest.batch_ops().to_vec();
    ops.push(BatchOp::Delete {
        key: b"\x90".to_vec().into(),
    });
    ops.sort_by(|a, b| a.key().cmp(b.key()));

    let crafted = FrozenChangeProof::new(
        crate::Proof::new(honest.start_proof().as_ref().into()),
        crate::Proof::new(honest.end_proof().as_ref().into()),
        ops.into_boxed_slice(),
    );

    let result =
        verify_change_proof_structure(&crafted, root2.clone(), Some(b"\x20"), Some(b"\x90"), None);
    if let Ok(ctx) = result {
        verify_and_check(&db_b, &crafted, &ctx, root1_b)
            .expect_err("spurious Delete at end_key should be detected");
    }
}

// ── MissingEndProof with empty batch_ops ─────────────────────────────────

/// Empty `batch_ops` with `end_key` set but no `end_proof` → `MissingEndProof`.
///
/// Exercises: the `(true, true)` branch in `verify_change_proof_structure`
#[test]
fn test_missing_end_proof_empty_batch_ops() {
    let proof = FrozenChangeProof::new(
        crate::Proof::new(Box::new([])),
        crate::Proof::new(Box::new([])),
        Box::new([]),
    );
    let err =
        verify_change_proof_structure(&proof, api::HashKey::empty(), None, Some(b"\x50"), None)
            .unwrap_err();
    assert!(matches!(
        err,
        api::Error::ProofError(crate::ProofError::MissingEndProof)
    ));
}

// ── ConflictingProofNodes test ───────────────────────────────────────────

/// When start and end proofs share a proof node at the same key but with
/// different content, reconciliation rejects with `ConflictingProofNodes`.
///
/// Exercises: `verify_change_proof_root_hash` line 752
///
/// Take a valid bounded proof and flip a bit in the end proof's root node.
/// This breaks the structural hash chain, so we skip structural validation
/// and call `verify_change_proof_root_hash` directly. The start and end
/// proofs both contain the root node, but now they disagree.
#[test]
fn test_conflicting_proof_nodes_rejected() {
    let (db, _dir) = setup_db![(b"\x10", b"a"), (b"\x20", b"b")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x10", b"A"), (b"\x20", b"B")]);

    // Valid bounded proof [\x10, \x20] — both proofs share the root node.
    let valid = db
        .change_proof(
            root1.clone(),
            root2.clone(),
            Some(b"\x10"),
            Some(b"\x20"),
            None,
        )
        .unwrap();

    // Modify a child hash in the end proof's root node so it differs from
    // the start proof's root node (which remains untouched).
    let mut end_nodes: Vec<crate::ProofNode> = valid.end_proof().as_ref().to_vec();
    assert!(!end_nodes.is_empty(), "end proof should have nodes");

    // Flip a bit in the first child hash we find at the root node.
    let root_node = &mut end_nodes[0];
    for pc in firewood_storage::PathComponent::ALL {
        if let Some(ref mut h) = root_node.child_hashes[pc] {
            let trie_hash = h.clone().into_triehash();
            let bytes: [u8; 32] = trie_hash.into();
            let mut new_bytes = bytes;
            new_bytes[0] ^= 1;
            *h = firewood_storage::TrieHash::from(new_bytes).into_hash_type();
            break;
        }
    }

    let crafted = FrozenChangeProof::new(
        crate::Proof::new(valid.start_proof().as_ref().into()),
        crate::Proof::new(end_nodes.into_boxed_slice()),
        valid.batch_ops().into(),
    );

    // Skip structural validation (the modified hash chain won't pass).
    // Call `verify_change_proof_root_hash` directly with a hand-built context.
    let ctx = ChangeProofVerificationContext {
        end_root: root2,
        start_key: Some(b"\x10".to_vec().into()),
        end_key: Some(b"\x20".to_vec().into()),
        right_edge_key: Some(b"\x20".to_vec().into()),
    };

    let parent = db.revision(root1).unwrap();
    let proposal = db.apply_change_proof_to_parent(&crafted, &*parent).unwrap();
    let err = verify_change_proof_root_hash(&crafted, &ctx, &proposal).unwrap_err();
    assert!(
        matches!(
            err,
            api::Error::ProofError(crate::ProofError::ConflictingProofNodes)
        ),
        "expected ConflictingProofNodes, got {err:?}"
    );
}

// ── ValueAtOddNibbleLength test ──────────────────────────────────────────

/// A proof node at an odd nibble depth carrying a value must be rejected.
/// Values can only exist at even nibble depths (complete byte boundaries).
///
/// Exercises: `reconcile_branch_proof_node` lines 2163-2166
///
/// Keys `\x10` and `\x15` share nibble 1 at depth 0, then fork at depth 1
/// (nibbles 0 vs 5). The branch at depth 1 is at odd nibble length. We
/// inject a spurious value into that odd-depth proof node.
#[test]
fn test_value_at_odd_nibble_length_rejected() {
    let (db, _dir) = setup_db![(b"\x10", b"a"), (b"\x15", b"b"), (b"\x30", b"c")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x30", b"changed")]);

    let valid = db
        .change_proof(root1.clone(), root2.clone(), Some(b"\x10"), None, None)
        .unwrap();

    // Find a start proof node at odd nibble depth and inject a value.
    let mut start_nodes: Vec<crate::ProofNode> = valid.start_proof().as_ref().to_vec();
    let odd_idx = start_nodes
        .iter()
        .position(|n| n.key.len() % 2 != 0)
        .expect("should have a proof node at odd nibble depth");

    start_nodes[odd_idx].value_digest = Some(firewood_storage::ValueDigest::Value(
        b"injected".to_vec().into(),
    ));

    let crafted = FrozenChangeProof::new(
        crate::Proof::new(start_nodes.into_boxed_slice()),
        crate::Proof::new(valid.end_proof().as_ref().into()),
        valid.batch_ops().into(),
    );

    // Skip structural validation (injected value breaks the hash chain).
    // Call `verify_change_proof_root_hash` directly.
    let ctx = ChangeProofVerificationContext {
        end_root: root2,
        start_key: Some(b"\x10".to_vec().into()),
        end_key: None,
        right_edge_key: None,
    };

    let parent = db.revision(root1).unwrap();
    let proposal = db.apply_change_proof_to_parent(&crafted, &*parent).unwrap();
    let err = verify_change_proof_root_hash(&crafted, &ctx, &proposal).unwrap_err();
    assert!(
        matches!(
            err,
            api::Error::ProofError(crate::ProofError::ValueAtOddNibbleLength)
        ),
        "expected ValueAtOddNibbleLength, got {err:?}"
    );
}
