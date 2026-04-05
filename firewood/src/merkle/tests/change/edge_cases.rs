// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Edge cases: odd depths, divergence, exclusion, deletion, children.

use super::*;

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
