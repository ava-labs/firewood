// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Structural validation: one test per error variant in `verify_change_proof_structure`.

use super::*;

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
