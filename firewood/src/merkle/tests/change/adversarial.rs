// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Adversarial tests for change proof verification.
//!
//! These tests verify that crafted change proofs containing spurious in-range
//! mutations are correctly **rejected** by both structural and root hash
//! verification. Each test constructs an attack proof with an unauthorized
//! operation and asserts that verification catches it.

use super::super::init_merkle;
use super::*;

type OwnedBatchOps = Vec<BatchOp<Box<[u8]>, Box<[u8]>>>;

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

/// 5 well-separated 32-byte keys at nibbles 0x10, 0x30, 0x50, 0x70, 0x90.
fn five_keys() -> [[u8; 32]; 5] {
    let mut arr = [[0u8; 32]; 5];
    arr[0][0] = 0x10; // A
    arr[1][0] = 0x30; // B
    arr[2][0] = 0x50; // C
    arr[3][0] = 0x70; // D
    arr[4][0] = 0x90; // E
    arr
}

/// Non-existent gap boundaries between A..B and D..E.
fn gap_boundaries() -> ([u8; 32], [u8; 32]) {
    let mut start = [0u8; 32];
    start[0] = 0x20;
    let mut end = [0u8; 32];
    end[0] = 0x80;
    (start, end)
}

/// Helper for the common adversarial pattern: generate a valid bounded proof
/// for the 5-key setup, inject a spurious operation, and assert rejection.
#[allow(clippy::too_many_arguments)]
fn inject_and_assert_rejected(
    db: &Db,
    root1: api::HashKey,
    root2: api::HashKey,
    valid_proof: &FrozenChangeProof,
    start_key: &[u8],
    end_key: &[u8],
    spurious_op: BatchOp<Box<[u8]>, Box<[u8]>>,
    msg: &str,
) {
    let mut ops: OwnedBatchOps = valid_proof.batch_ops().to_vec();
    let pos = ops
        .binary_search_by(|op| op.key().as_ref().cmp(spurious_op.key().as_ref()))
        .unwrap_or_else(|i| i);
    ops.insert(pos, spurious_op);

    let attack_proof = crate::ChangeProof::new(
        crate::Proof::new(valid_proof.start_proof().as_ref().into()),
        crate::Proof::new(valid_proof.end_proof().as_ref().into()),
        ops.into_boxed_slice(),
    );

    assert!(
        is_rejected(
            db,
            &attack_proof,
            root2,
            Some(start_key),
            Some(end_key),
            root1
        ),
        "{msg}"
    );
}

// ── Bug demonstrations ────────────────────────────────────────────────────

/// Verifies that a spurious Delete for an in-range key that exists in the end
/// trie (but is unchanged between revisions) is correctly rejected.
///
/// Setup:
///   - Start trie and end trie both contain keys A, B, C, D, E (with some
///     actual changes at other keys so `batch_ops` is non-empty).
///   - The change proof is bounded by non-existent gap keys around B..D.
///   - Key C is unchanged between revisions and is NOT in `batch_ops`.
///   - An attacker adds `Delete { key: C }` to `batch_ops`.
#[test]
fn test_spurious_delete_in_range_is_rejected() {
    let keys = five_keys();
    let (db, _dir) = setup_db![
        (&keys[0], &[0xAAu8; 20]),
        (&keys[1], &[0xBBu8; 20]),
        (&keys[2], &[0xCCu8; 20]),
        (&keys[3], &[0xDDu8; 20]),
        (&keys[4], &[0xEEu8; 20])
    ];
    let (root1, root2) = setup_2nd_commit!(db, [(&keys[1], &[0xB2u8; 20])]);

    let (start_key, end_key) = gap_boundaries();
    let valid_proof = db
        .change_proof(
            root1.clone(),
            root2.clone(),
            Some(&start_key),
            Some(&end_key),
            None,
        )
        .unwrap();

    // Sanity: the valid proof should pass verification.
    assert!(
        !is_rejected(
            &db,
            &valid_proof,
            root2.clone(),
            Some(&start_key),
            Some(&end_key),
            root1.clone(),
        ),
        "valid proof should pass"
    );

    // Attack: add a spurious Delete for key C (0x50...).
    // C is unchanged between revisions, within [start_key, end_key],
    // and exists in both tries.
    inject_and_assert_rejected(
        &db,
        root1,
        root2,
        &valid_proof,
        &start_key,
        &end_key,
        BatchOp::Delete {
            key: keys[2].to_vec().into_boxed_slice(),
        },
        "spurious Delete of in-range key C should be rejected",
    );
}

/// Variant: the spurious delete target shares a first nibble with a key that
/// IS in `batch_ops`, forcing them into the same top-level subtree. The subtree
/// must be recomputed because it contains a changed key, so the delete of the
/// unchanged sibling should be visible.
#[test]
fn test_spurious_delete_same_subtree_as_changed_key() {
    // B and C share first nibble 0x3_.
    let mut keys = five_keys();
    keys[2][0] = 0x38; // C — target, same first nibble as B (0x30)

    let (db, _dir) = setup_db![
        (&keys[0], &[0xAAu8; 20]),
        (&keys[1], &[0xBBu8; 20]),
        (&keys[2], &[0xCCu8; 20]),
        (&keys[3], &[0xDDu8; 20]),
        (&keys[4], &[0xEEu8; 20])
    ];
    let (root1, root2) = setup_2nd_commit!(db, [(&keys[1], &[0xB2u8; 20])]);

    let (start_key, end_key) = gap_boundaries();
    let valid_proof = db
        .change_proof(
            root1.clone(),
            root2.clone(),
            Some(&start_key),
            Some(&end_key),
            None,
        )
        .unwrap();

    // Attack: spurious Delete for C (0x38), same top-nibble subtree as B (0x30).
    inject_and_assert_rejected(
        &db,
        root1,
        root2,
        &valid_proof,
        &start_key,
        &end_key,
        BatchOp::Delete {
            key: keys[2].to_vec().into_boxed_slice(),
        },
        "spurious Delete of C (same subtree as changed B) was NOT rejected",
    );
}

/// Same bug with EXISTING boundary keys (inclusion proofs).
/// The issue isn't exclusion vs inclusion — it's that the end proof traces
/// to the last changed key (B), and `compute_outside_children` marks
/// everything above B's nibble as out-of-range, including C.
#[test]
fn test_spurious_delete_with_existing_boundaries() {
    let keys = five_keys();
    let (db, _dir) = setup_db![
        (&keys[0], &[0xAAu8; 20]),
        (&keys[1], &[0xBBu8; 20]),
        (&keys[2], &[0xCCu8; 20]),
        (&keys[3], &[0xDDu8; 20]),
        (&keys[4], &[0xEEu8; 20])
    ];
    let (root1, root2) = setup_2nd_commit!(db, [(&keys[1], &[0xB2u8; 20])]);

    // Boundaries are EXISTING keys: A (0x10) and E (0x90).
    // Both boundary proofs will be inclusion proofs.
    let valid_proof = db
        .change_proof(
            root1.clone(),
            root2.clone(),
            Some(&keys[0]),
            Some(&keys[4]),
            None,
        )
        .unwrap();

    assert!(
        !is_rejected(
            &db,
            &valid_proof,
            root2.clone(),
            Some(&keys[0]),
            Some(&keys[4]),
            root1.clone(),
        ),
        "valid proof should pass"
    );

    // Attack: spurious Delete for C (0x50).
    inject_and_assert_rejected(
        &db,
        root1,
        root2,
        &valid_proof,
        &keys[0],
        &keys[4],
        BatchOp::Delete {
            key: keys[2].to_vec().into_boxed_slice(),
        },
        "spurious Delete with existing boundaries was NOT rejected",
    );
}

// ── value_digest bug demonstration ────────────────────────────────────────

/// Demonstrates the root cause: `value_digest` accepts a proof for key B
/// as a valid exclusion proof for a completely different key C.
///
/// The proof `[ROOT, leaf_B]` traces root -> `child[3]` -> `leaf_B`. When asked
/// "does key C (nibble 5) exist?", `value_digest` should reject the proof
/// because the path follows child 3 (toward B), not child 5 (toward C).
/// Instead, it accepts `leaf_B` as a "divergent child" exclusion for C.
#[test]
fn test_value_digest_accepts_wrong_path_as_exclusion() {
    let keys: [[u8; 32]; 3] = {
        let mut arr = [[0u8; 32]; 3];
        arr[0][0] = 0x10; // A
        arr[1][0] = 0x30; // B
        arr[2][0] = 0x50; // C
        arr
    };

    let merkle = init_merkle([
        (keys[0], [0xAAu8; 20]),
        (keys[1], [0xBBu8; 20]),
        (keys[2], [0xCCu8; 20]),
    ]);
    let root_hash = firewood_storage::HashedNodeReader::root_hash(merkle.nodestore()).unwrap();

    // Generate a proof for B.
    let proof_for_b = merkle.prove(&keys[1]).unwrap();

    // The proof should verify as an inclusion proof for B.
    proof_for_b
        .verify(keys[1], Some(&[0xBBu8; 20]), &root_hash)
        .expect("proof for B should verify B");

    // The proof should NOT verify as an exclusion proof for C.
    // C exists in the trie, but even setting that aside, the proof path
    // traces toward B (nibble 3), not toward C (nibble 5). The proof
    // has no information about C's subtree.
    let result = proof_for_b.value_digest(keys[2], &root_hash);
    assert!(
        result.is_err(),
        "proof for B should NOT be accepted when queried for C, \
         but value_digest returned {result:?}"
    );
}

// ── Control tests (correctly rejected) ────────────────────────────────────

/// Control test: swapping the value of a Put that IS in `batch_ops` is correctly
/// rejected. This confirms the hybrid hash works for keys that were changed
/// between revisions (their subtree is computed from the proposal, not borrowed
/// from proof nodes).
#[test]
fn test_swapped_value_in_range_is_rejected() {
    let keys = five_keys();
    let (db, _dir) = setup_db![
        (&keys[0], &[0xAAu8; 20]),
        (&keys[1], &[0xBBu8; 20]),
        (&keys[2], &[0xCCu8; 20]),
        (&keys[3], &[0xDDu8; 20]),
        (&keys[4], &[0xEEu8; 20])
    ];
    let (root1, root2) =
        setup_2nd_commit!(db, [(&keys[1], &[0xB2u8; 20]), (&keys[3], &[0xD2u8; 20])]);

    let (start_key, end_key) = gap_boundaries();
    let valid_proof = db
        .change_proof(
            root1.clone(),
            root2.clone(),
            Some(&start_key),
            Some(&end_key),
            None,
        )
        .unwrap();

    assert!(
        valid_proof.batch_ops().len() >= 2,
        "expected at least 2 batch_ops"
    );

    // Attack: change the value of the first Put (B) to garbage.
    let mut ops: OwnedBatchOps = valid_proof.batch_ops().to_vec();
    let first_put_idx = ops
        .iter()
        .position(|op| matches!(op, BatchOp::Put { .. }))
        .expect("should have at least one Put");
    let key = ops[first_put_idx].key().clone();
    ops[first_put_idx] = BatchOp::Put {
        key,
        value: [0xFFu8; 20].to_vec().into_boxed_slice(),
    };

    let attack_proof = crate::ChangeProof::new(
        crate::Proof::new(valid_proof.start_proof().as_ref().into()),
        crate::Proof::new(valid_proof.end_proof().as_ref().into()),
        ops.into_boxed_slice(),
    );

    assert!(
        is_rejected(
            &db,
            &attack_proof,
            root2,
            Some(&start_key),
            Some(&end_key),
            root1,
        ),
        "swapped value of in-range key B was NOT rejected"
    );
}

/// Control test: a spurious Put for a non-existent in-range key is correctly
/// rejected when the key falls in a subtree that is recomputed (not borrowed).
#[test]
fn test_spurious_put_in_range_is_rejected() {
    let keys = five_keys();
    let (db, _dir) = setup_db![
        (&keys[0], &[0xAAu8; 20]),
        (&keys[1], &[0xBBu8; 20]),
        (&keys[2], &[0xCCu8; 20]),
        (&keys[3], &[0xDDu8; 20]),
        (&keys[4], &[0xEEu8; 20])
    ];
    let (root1, root2) = setup_2nd_commit!(db, [(&keys[1], &[0xB2u8; 20])]);

    let (start_key, end_key) = gap_boundaries();
    let valid_proof = db
        .change_proof(
            root1.clone(),
            root2.clone(),
            Some(&start_key),
            Some(&end_key),
            None,
        )
        .unwrap();

    // Attack: add a Put for key 0x60... (between C and D, doesn't exist).
    let mut fake_key = [0u8; 32];
    fake_key[0] = 0x60;
    inject_and_assert_rejected(
        &db,
        root1,
        root2,
        &valid_proof,
        &start_key,
        &end_key,
        BatchOp::Put {
            key: fake_key.to_vec().into_boxed_slice(),
            value: [0xFFu8; 20].to_vec().into_boxed_slice(),
        },
        "spurious Put of in-range key 0x60 was NOT rejected",
    );
}

// ── Root structure incompatibility (found by TLA+ model checking) ─────

/// Demonstrates that an honest change proof succeeds even when the start and
/// end tries differ outside the proven range in a way that changes the
/// root-level trie structure.
///
/// `collapse_root_to_path` reshapes the proving trie's root to match
/// `end_root`'s compressed structure by stripping out-of-range children
/// and flattening the root's `partial_path`.
///
/// Found by `ChangeProofVerification.tla` `HonestProofAccepted` invariant.
#[test]
fn test_out_of_range_root_structure_change() {
    let (db, _dir) = setup_db![
        (b"\x01", b"alpha"),
        (b"\x10", b"betax"),
        (b"\x11", b"gamma")
    ];
    let root1 = db.root_hash().unwrap();

    // Revision 2: delete \x01 (out of range), change \x10 (in range).
    // Only nibble-1 keys remain → root compresses past nibble 0.
    db.propose(vec![
        BatchOp::Delete { key: b"\x01" },
        BatchOp::Put {
            key: b"\x10",
            value: b"beta2",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    let proof = db
        .change_proof(
            root1.clone(),
            root2.clone(),
            Some(b"\x10".as_slice()),
            None,
            None,
        )
        .unwrap();

    assert!(!proof.batch_ops().is_empty());

    let ctx =
        verify_change_proof_structure(&proof, root2.clone(), Some(b"\x10"), None, None).unwrap();
    let result = verify_and_check(&db, &proof, &ctx, root1);
    assert!(
        result.is_ok(),
        "honest change proof rejected due to out-of-range root structure \
         change: {result:?}"
    );
}

/// Bug 3 regression: Verify that `collapse_root_to_path` correctly handles
/// root reshaping when out-of-range key deletions compress the `endTrie` root.
///
/// Found by `ChangeProofVerification.tla` `HonestProofAccepted` invariant.
#[test]
fn test_root_shape_mismatch_low_range() {
    let (db, _dir) = setup_db![(b"\x01", b"low"), (b"\x90", b"hig")];
    let root1 = db.root_hash().unwrap();

    // Revision 2: delete the low key. Only \x90 remains.
    // endTrie root compresses to start at nibble 9.
    db.propose(vec![
        BatchOp::Delete::<_, &[u8]> { key: b"\x01" },
        BatchOp::Put {
            key: b"\x90",
            value: b"HIGH",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    let proof = db
        .change_proof(
            root1.clone(),
            root2.clone(),
            Some(b"\x80".as_slice()),
            Some(b"\x90".as_slice()),
            None,
        )
        .unwrap();

    let ctx =
        verify_change_proof_structure(&proof, root2.clone(), Some(b"\x80"), Some(b"\x90"), None)
            .unwrap();
    let result = verify_and_check(&db, &proof, &ctx, root1);
    assert!(
        result.is_ok(),
        "honest change proof rejected due to root shape mismatch: {result:?}"
    );
}

/// State injection via `collapse_root_to_path` (found by TLA+ model).
///
/// An attacker injects a spurious key at a different first nibble than the
/// proof path. The collapse strips the injected key's nibble (it's
/// "non-on-path"), so the hash computation never sees it.
///
/// Found by `AdversarialProof.tla` `OnlyCorrectDiffAccepted` invariant.
#[test]
fn test_collapse_root_hides_spurious_key() {
    type BoxBatchOp = BatchOp<Box<[u8]>, Box<[u8]>>;
    let (db, _dir) = setup_db![(b"\x90", b"orig")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x90", b"new!")]);

    let valid_proof = db
        .change_proof(root1.clone(), root2.clone(), None, None, None)
        .unwrap();

    assert_eq!(valid_proof.batch_ops().len(), 1);

    // Attack: inject Put(\x10, "evil") at a different first nibble.
    let mut ops: Vec<BoxBatchOp> = valid_proof.batch_ops().to_vec();
    ops.insert(
        0,
        BatchOp::Put {
            key: b"\x10".to_vec().into_boxed_slice(),
            value: b"evil".to_vec().into_boxed_slice(),
        },
    );

    let attack_proof = crate::ChangeProof::new(
        crate::Proof::new(valid_proof.start_proof().as_ref().into()),
        crate::Proof::new(valid_proof.end_proof().as_ref().into()),
        ops.into_boxed_slice(),
    );

    assert!(
        is_rejected(&db, &attack_proof, root2, None, None, root1),
        "spurious Put at \\x10 was NOT rejected — \
         collapse_root_to_path stripped nibble 1 (non-on-path \
         relative to the end proof through nibble 9), hiding \
         the injected key from the hash computation"
    );
}
