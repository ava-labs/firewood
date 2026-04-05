// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Truncation, generator, boundary operations, and disjoint proofs.

use super::*;

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
