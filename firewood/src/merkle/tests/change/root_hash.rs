// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Root hash verification happy paths (one per proof configuration).

use super::*;

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
