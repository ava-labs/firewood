// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Attack detection: forged, omitted, or spurious batch ops.

use super::*;

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
