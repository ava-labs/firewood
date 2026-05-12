// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Attack detection: forged, omitted, or spurious batch ops.

use super::*;

/// Helper: returns true if the (possibly corrupted) proof is rejected by
/// either the structural check or the root hash check.
fn is_rejected(
    db: &Db,
    proof: &FrozenChangeProof,
    end_root: api::HashKey,
    start_key: Option<&[u8]>,
    end_key: Option<&[u8]>,
    start_root: api::HashKey,
) -> bool {
    match verify_change_proof_structure(proof, end_root, start_key, end_key, None) {
        Err(_) => true,
        Ok(ctx) => verify_and_check(db, proof, &ctx, start_root).is_err(),
    }
}

/// Security test: an attacker removes `Delete(\x05)` from `batch_ops`.
///
/// `\x05` is in-range (>= `start_key` `\x01`) and shares nibble 0 with the
/// out-of-range `\x00`. If the collapse step unconditionally strips all
/// non-on-path children (without checking for in-range keys), the proving
/// trie is flattened to match `end_root` and the tampered proof is accepted.
///
/// The verifier would then commit the incomplete `batch_ops`, leaving `\x05`
/// in their state when `end_root` says it shouldn't exist.
#[test]
fn test_crafted_omitted_delete_at_straddling_nibble() {
    // start_root: \x00, \x05, \x10
    let (db, _dir) = setup_db![(b"\x00", b"a"), (b"\x05", b"e"), (b"\x10", b"b")];
    let root1 = db.root_hash().unwrap();

    // end_root: only \x10 (both \x00 and \x05 deleted)
    db.propose(vec![
        BatchOp::Delete {
            key: b"\x00" as &[u8],
        },
        BatchOp::Delete { key: b"\x05" },
        BatchOp::Put {
            key: b"\x10",
            value: b"changed",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    // Generate the honest proof (start_key=\x01: \x00 delete is out of range)
    let honest = db
        .change_proof(root1.clone(), root2.clone(), Some(b"\x01"), None, None)
        .unwrap();

    // The honest proof should have Delete(\x05) and Put(\x10, "changed")
    assert_eq!(honest.batch_ops().len(), 2);

    // Craft a tampered proof: remove Delete(\x05), keep only Put(\x10)
    let tampered = FrozenChangeProof::new(
        crate::Proof::new(honest.start_proof().as_ref().into()),
        crate::Proof::new(honest.end_proof().as_ref().into()),
        Box::new([BatchOp::Put {
            key: b"\x10".to_vec().into(),
            value: b"changed".to_vec().into(),
        }]),
    );

    assert!(
        is_rejected(&db, &tampered, root2, Some(b"\x01"), None, root1),
        "tampered proof with omitted Delete at straddling nibble should be rejected"
    );
}
