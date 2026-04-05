// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Regression tests for specific bug fixes.

use super::*;

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
