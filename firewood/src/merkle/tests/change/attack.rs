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
