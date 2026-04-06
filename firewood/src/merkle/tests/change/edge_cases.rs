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

/// The `prove()` fix includes divergent children in start proofs. This
/// exercises the fixed path — a Delete near `start_key` under a shared
/// branch produces a valid proof. Before the fix, the proof generator
/// omitted the divergent child, leaving a verification gap.
#[test]
fn test_boundary_child_gap_closed_for_start_key() {
    // \x10\x50 and \x10\x58 share a branch at nibble path [1,0].
    let (source, _dir_source) = setup_db![
        (b"\x10\x50", b"a"),
        (b"\x10\x58", b"b"),
        (b"\x30\x00", b"c")
    ];
    let root1_source = source.root_hash().unwrap();
    let (target, _dir_target) = setup_db![
        (b"\x10\x50", b"a"),
        (b"\x10\x58", b"b"),
        (b"\x30\x00", b"c")
    ];
    let root1_target = target.root_hash().unwrap();

    // Delete \x10\x50, change \x30\x00. \x10\x58 is unchanged.
    source
        .propose(vec![
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
    let root2 = source.root_hash().unwrap();

    let honest = source
        .change_proof(
            root1_source,
            root2.clone(),
            Some(b"\x10\x50"),
            Some(b"\x30\x00"),
            None,
        )
        .unwrap();

    let ctx =
        verify_change_proof_structure(&honest, root2, Some(b"\x10\x50"), Some(b"\x30\x00"), None)
            .unwrap();
    verify_and_check(&target, &honest, &ctx, root1_target).unwrap();
}

/// Case A of `compute_outside_children`: boundary key diverges within
/// the terminal node's partial path.
///
/// Keys `\x10\x50` and `\x10\x58` share a branch with partial path containing
/// nibble 5. `start_key` `\x10\x40` diverges at nibble 4 < 5, so the start
/// boundary is "before" the terminal — no children are marked outside.
#[test]
fn test_terminal_divergence_within_partial_path() {
    let (source, _dir_source) =
        setup_db![(b"\x10\x50", b"a"), (b"\x10\x58", b"b"), (b"\x30", b"c")];
    let (target, _dir_target) =
        setup_db![(b"\x10\x50", b"a"), (b"\x10\x58", b"b"), (b"\x30", b"c")];
    let root1_target = target.root_hash().unwrap();

    let (root1_source, root2) = setup_2nd_commit!(source, [(b"\x30", b"changed")]);

    let proof = source
        .change_proof(root1_source, root2.clone(), Some(b"\x10\x40"), None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, Some(b"\x10\x40"), None, None).unwrap();
    verify_and_check(&target, &proof, &ctx, root1_target).unwrap();
}

/// Case B of `compute_outside_children`: terminal is an ancestor of the
/// boundary key. The on-path child must be marked as outside so its hash
/// comes from the proof, not the proving trie.
///
/// Keys at `\x10\x01` and `\x10\x02` create a branch at nibble path `[1,0]`.
/// `start_key` `\x10` is exhausted at depth 2 — the terminal node at `[1,0]`
/// is an ancestor of `start_key`'s nibble path. The on-path child (nibble 0)
/// is marked outside.
#[test]
fn test_terminal_ancestor_marks_on_path_child() {
    let (source, _dir_source) = setup_db![
        (b"\x10\x01", b"a"),
        (b"\x10\x02", b"b"),
        (b"\x10\x03", b"c"),
        (b"\x30", b"d")
    ];
    let (target, _dir_target) = setup_db![
        (b"\x10\x01", b"a"),
        (b"\x10\x02", b"b"),
        (b"\x10\x03", b"c"),
        (b"\x30", b"d")
    ];
    let root1_target = target.root_hash().unwrap();

    let (root1_source, root2) = setup_2nd_commit!(source, [(b"\x30", b"changed")]);

    let proof = source
        .change_proof(root1_source, root2.clone(), Some(b"\x10"), None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, Some(b"\x10"), None, None).unwrap();
    verify_and_check(&target, &proof, &ctx, root1_target).unwrap();
}

/// Case C of `compute_outside_children` (right edge): boundary is a
/// prefix of the terminal node's key. All children extend beyond
/// `end_key`, so all are marked outside.
///
/// Key `\x20` is a prefix of `\x20\xab`. `end_key` = `\x20` — children of
/// the `\x20` node extend beyond `end_key` and must use proof hashes.
#[test]
fn test_right_edge_boundary_prefix_of_terminal() {
    let (source, _dir_source) = setup_db![(b"\x10", b"a"), (b"\x20", b"b"), (b"\x20\xab", b"c")];
    let (target, _dir_target) = setup_db![(b"\x10", b"a"), (b"\x20", b"b"), (b"\x20\xab", b"c")];
    let root1_target = target.root_hash().unwrap();

    let (root1_source, root2) = setup_2nd_commit!(source, [(b"\x10", b"changed")]);

    let proof = source
        .change_proof(root1_source, root2.clone(), None, Some(b"\x20"), None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, None, Some(b"\x20"), None).unwrap();
    verify_and_check(&target, &proof, &ctx, root1_target).unwrap();
}

/// An out-of-range prefix node's value changes between root1 and root2.
/// The proof's value must be adopted during `reconcile_branch_proof_node`
/// so the hybrid hash matches `end_root`.
///
/// `\x10` has a value that changes from `"old_pfx"` to `"new_pfx"`. The query
/// range `[\x10\x50, \x30]` excludes `\x10` (it's a proper prefix of `start_key`),
/// so `\x10` is out-of-range. The start proof path passes through `\x10`, and
/// reconciliation must adopt the proof's value (`"new_pfx"`) to match `end_root`.
#[test]
fn test_out_of_range_value_change_adopted() {
    let (source, _dir_source) = setup_db![
        (b"\x10", b"old_pfx"),
        (b"\x10\x50", b"val0000"),
        (b"\x30", b"val1000")
    ];
    let (target, _dir_target) = setup_db![
        (b"\x10", b"old_pfx"),
        (b"\x10\x50", b"val0000"),
        (b"\x30", b"val1000")
    ];
    let root1_target = target.root_hash().unwrap();

    let (root1_source, root2) =
        setup_2nd_commit!(source, [(b"\x10", b"new_pfx"), (b"\x30", b"changed")]);

    let proof = source
        .change_proof(
            root1_source,
            root2.clone(),
            Some(b"\x10\x50"),
            Some(b"\x30"),
            None,
        )
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, Some(b"\x10\x50"), Some(b"\x30"), None)
        .unwrap();
    verify_and_check(&target, &proof, &ctx, root1_target).unwrap();
}

/// A key that is a proper prefix of `start_key` has a value in root1 that
/// is deleted in root2. Because the prefix key is outside the query range
/// (shorter key < `start_key`), it is not in `batch_ops`. The proving trie
/// must clear this stale value so the computed hash matches `end_root`.
///
/// Without the fix, `reconcile_branch_proof_node` ignored the trie's value
/// when the proof node had no value, silently leaving the stale value in the
/// proving trie and causing `EndRootMismatch`.
#[test]
fn test_change_proof_prefix_key_deleted_in_end_root() {
    let (db, _dir) = setup_db![
        (b"\xab", b"prefix_value"),
        (b"\xab\xcd", b"full_key"),
        (b"\xab\xef", b"sibling"),
        (b"\xff", b"high"),
    ];
    let root1 = db.root_hash().unwrap();

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

    // b"\xab" < b"\xab\x00", so the deleted prefix key is outside the range.
    // The start proof passes through [a,b] where root1 has a value and root2
    // does not — reconciliation must clear the stale value.
    let proof = db
        .change_proof(
            root1.clone(),
            root2.clone(),
            Some(b"\xab\x00".as_slice()),
            Some(b"\xff".as_slice()),
            None,
        )
        .unwrap();

    let ctx = verify_change_proof_structure(
        &proof,
        root2.clone(),
        Some(b"\xab\x00".as_slice()),
        Some(b"\xff".as_slice()),
        None,
    )
    .unwrap();

    // This failed with EndRootMismatch before the fix.
    verify_and_check(&db, &proof, &ctx, root1).unwrap();
}
