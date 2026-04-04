// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Adversarial tests for change proof verification.
//!
//! These tests demonstrate that an attacker can craft change proofs containing
//! in-range key/value mutations that pass both structural and root hash
//! verification. The core issue is that `verify_change_proof_root_hash` borrows
//! subtree hashes from boundary proof nodes for "out-of-range" children, but
//! the boundary between in-range and out-of-range is determined by proof path
//! structure — not by the lexicographic key range. Keys that are lexicographically
//! within `[start_key, end_key]` can still fall in subtrees whose hashes are
//! borrowed from proof nodes rather than recomputed from the proposal.

use super::init_merkle;
use crate::api::{self, BatchOp, Db as DbTrait, DbView, FrozenChangeProof, Proposal as _};
use crate::db::{Db, DbConfig};
use crate::merkle::verify_change_proof_root_hash;
use crate::{ChangeProofVerificationContext, verify_change_proof_structure};

type OwnedBatchOps = Vec<BatchOp<Box<[u8]>, Box<[u8]>>>;

fn new_db() -> (Db, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::new(dir.path(), DbConfig::builder().build()).unwrap();
    (db, dir)
}

fn verify_and_check(
    db: &Db,
    proof: &FrozenChangeProof,
    verification: &ChangeProofVerificationContext,
    start_root: api::HashKey,
) -> Result<(), api::Error> {
    let parent = db.revision(start_root)?;
    let proposal = db.apply_change_proof_to_parent(proof, &*parent)?;
    verify_change_proof_root_hash(proof, verification, &proposal)
}

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

// ── Bug demonstrations ────────────────────────────────────────────────────

/// Demonstrates that adding a spurious Delete for an in-range key that exists
/// in the end trie (but is unchanged between revisions) is not rejected by
/// change proof verification.
///
/// Setup:
///   - Start trie and end trie both contain keys A, B, C, D, E (with some
///     actual changes at other keys so `batch_ops` is non-empty).
///   - The change proof is bounded by non-existent gap keys around B..D.
///   - Key C is unchanged between revisions and is NOT in `batch_ops`.
///   - An attacker adds `Delete { key: C }` to `batch_ops`.
///   - Structural check passes (C is between `start_key` and `end_key`, and the
///     end proof was generated for a key after C).
///   - Root hash check passes because C's subtree hash comes from the boundary
///     proof nodes (out-of-range) rather than being recomputed from the proposal.
///
/// This means an attacker can claim a key was deleted when it wasn't.
#[test]
#[allow(clippy::too_many_lines)]
fn test_spurious_delete_in_range_not_rejected() {
    let (db, _dir) = new_db();

    // 5 well-separated keys so they occupy distinct trie subtrees.
    let keys: [[u8; 32]; 5] = {
        let mut arr = [[0u8; 32]; 5];
        arr[0][0] = 0x10; // A
        arr[1][0] = 0x30; // B
        arr[2][0] = 0x50; // C — the target
        arr[3][0] = 0x70; // D
        arr[4][0] = 0x90; // E
        arr
    };

    let val_a = [0xAAu8; 20];
    let val_b = [0xBBu8; 20];
    let val_c = [0xCCu8; 20];
    let val_d = [0xDDu8; 20];
    let val_e = [0xEEu8; 20];

    // Commit revision 1 (start trie): all 5 keys.
    db.propose(vec![
        BatchOp::Put {
            key: &keys[0],
            value: &val_a,
        },
        BatchOp::Put {
            key: &keys[1],
            value: &val_b,
        },
        BatchOp::Put {
            key: &keys[2],
            value: &val_c,
        },
        BatchOp::Put {
            key: &keys[3],
            value: &val_d,
        },
        BatchOp::Put {
            key: &keys[4],
            value: &val_e,
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    // Commit revision 2 (end trie): change B's value, keep everything else.
    let val_b2 = [0xB2u8; 20];
    db.propose(vec![BatchOp::Put {
        key: &keys[1],
        value: &val_b2,
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    // Generate a bounded change proof with non-existent boundaries.
    // start_key = 0x20... (gap between A and B)
    // end_key   = 0x80... (gap between D and E)
    let mut start_key = [0u8; 32];
    start_key[0] = 0x20;
    let mut end_key = [0u8; 32];
    end_key[0] = 0x80;

    let valid_proof = db
        .change_proof(
            root1.clone(),
            root2.clone(),
            Some(&start_key),
            Some(&end_key),
            None,
        )
        .unwrap();

    // Sanity: the valid proof should contain only the B value change.
    assert!(
        !valid_proof.batch_ops().is_empty(),
        "expected non-empty batch_ops for the B value change"
    );

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

    // ── Attack: add a spurious Delete for key C (0x50...) ─────────────
    // C is unchanged between revisions, within [start_key, end_key],
    // and exists in both tries.
    let mut ops: OwnedBatchOps = valid_proof.batch_ops().to_vec();
    let target_key: Box<[u8]> = keys[2].to_vec().into_boxed_slice();
    let pos = ops
        .binary_search_by(|op| op.key().as_ref().cmp(target_key.as_ref()))
        .unwrap_or_else(|i| i);
    ops.insert(pos, BatchOp::Delete { key: target_key });

    let attack_proof = crate::ChangeProof::new(
        crate::Proof::new(valid_proof.start_proof().as_ref().into()),
        crate::Proof::new(valid_proof.end_proof().as_ref().into()),
        ops.into_boxed_slice(),
    );

    // Verify that applying the attack proof produces a DIFFERENT root hash
    // than root2, proving the attack actually changed the trie state.
    let parent = db.revision(root1.clone()).unwrap();
    let attack_proposal = db
        .apply_change_proof_to_parent(&attack_proof, &*parent)
        .unwrap();
    let attack_root = attack_proposal.root_hash().unwrap();
    assert_ne!(
        attack_root, root2,
        "attack proposal root hash should differ from root2 \
         (the spurious Delete removed key C)"
    );

    // Yet the verifier does NOT reject it — this is the bug.
    assert!(
        is_rejected(
            &db,
            &attack_proof,
            root2,
            Some(&start_key),
            Some(&end_key),
            root1,
        ),
        "spurious Delete of in-range key C was NOT rejected, \
         even though proposal root {attack_root:?} != end root"
    );
}

/// Variant: the spurious delete target shares a first nibble with a key that
/// IS in `batch_ops`, forcing them into the same top-level subtree. The subtree
/// must be recomputed because it contains a changed key, so the delete of the
/// unchanged sibling should be visible.
#[test]
fn test_spurious_delete_same_subtree_as_changed_key() {
    let (db, _dir) = new_db();

    // B and C share first nibble 0x3_.
    let keys: [[u8; 32]; 5] = {
        let mut arr = [[0u8; 32]; 5];
        arr[0][0] = 0x10; // A
        arr[1][0] = 0x30; // B — will be changed
        arr[2][0] = 0x38; // C — target, same first nibble as B
        arr[3][0] = 0x70; // D
        arr[4][0] = 0x90; // E
        arr
    };

    db.propose(vec![
        BatchOp::Put {
            key: &keys[0],
            value: &[0xAA; 20],
        },
        BatchOp::Put {
            key: &keys[1],
            value: &[0xBB; 20],
        },
        BatchOp::Put {
            key: &keys[2],
            value: &[0xCC; 20],
        },
        BatchOp::Put {
            key: &keys[3],
            value: &[0xDD; 20],
        },
        BatchOp::Put {
            key: &keys[4],
            value: &[0xEE; 20],
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    db.propose(vec![BatchOp::Put {
        key: &keys[1],
        value: &[0xB2; 20],
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    let mut start_key = [0u8; 32];
    start_key[0] = 0x20;
    let mut end_key = [0u8; 32];
    end_key[0] = 0x80;

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
    let mut ops: OwnedBatchOps = valid_proof.batch_ops().to_vec();
    let target_key: Box<[u8]> = keys[2].to_vec().into_boxed_slice();
    let pos = ops
        .binary_search_by(|op| op.key().as_ref().cmp(target_key.as_ref()))
        .unwrap_or_else(|i| i);
    ops.insert(pos, BatchOp::Delete { key: target_key });

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
        "spurious Delete of C (same subtree as changed B) was NOT rejected"
    );
}

/// Same bug with EXISTING boundary keys (inclusion proofs).
/// The issue isn't exclusion vs inclusion — it's that the end proof traces
/// to the last changed key (B), and `compute_outside_children` marks
/// everything above B's nibble as out-of-range, including C.
#[test]
fn test_spurious_delete_with_existing_boundaries() {
    let (db, _dir) = new_db();

    let keys: [[u8; 32]; 5] = {
        let mut arr = [[0u8; 32]; 5];
        arr[0][0] = 0x10; // A — start_key (exists)
        arr[1][0] = 0x30; // B — changed
        arr[2][0] = 0x50; // C — target (unchanged)
        arr[3][0] = 0x70; // D
        arr[4][0] = 0x90; // E — end_key (exists)
        arr
    };

    db.propose(vec![
        BatchOp::Put {
            key: &keys[0],
            value: &[0xAA; 20],
        },
        BatchOp::Put {
            key: &keys[1],
            value: &[0xBB; 20],
        },
        BatchOp::Put {
            key: &keys[2],
            value: &[0xCC; 20],
        },
        BatchOp::Put {
            key: &keys[3],
            value: &[0xDD; 20],
        },
        BatchOp::Put {
            key: &keys[4],
            value: &[0xEE; 20],
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    // Only B changes.
    db.propose(vec![BatchOp::Put {
        key: &keys[1],
        value: &[0xB2; 20],
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

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
    let mut ops: OwnedBatchOps = valid_proof.batch_ops().to_vec();
    let target_key: Box<[u8]> = keys[2].to_vec().into_boxed_slice();
    let pos = ops
        .binary_search_by(|op| op.key().as_ref().cmp(target_key.as_ref()))
        .unwrap_or_else(|i| i);
    ops.insert(pos, BatchOp::Delete { key: target_key });

    let attack_proof = crate::ChangeProof::new(
        crate::Proof::new(valid_proof.start_proof().as_ref().into()),
        crate::Proof::new(valid_proof.end_proof().as_ref().into()),
        ops.into_boxed_slice(),
    );

    // Confirm the proposal root hash actually differs.
    let parent = db.revision(root1.clone()).unwrap();
    let attack_proposal = db
        .apply_change_proof_to_parent(&attack_proof, &*parent)
        .unwrap();
    let attack_root = attack_proposal.root_hash().unwrap();
    assert_ne!(attack_root, root2, "attack should change the root hash");

    assert!(
        is_rejected(
            &db,
            &attack_proof,
            root2,
            Some(&keys[0]),
            Some(&keys[4]),
            root1,
        ),
        "spurious Delete with existing boundaries was NOT rejected"
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
        (keys[0], [0xAA; 20]),
        (keys[1], [0xBB; 20]),
        (keys[2], [0xCC; 20]),
    ]);
    let root_hash = firewood_storage::HashedNodeReader::root_hash(merkle.nodestore()).unwrap();

    // Generate a proof for B.
    let proof_for_b = merkle.prove(&keys[1]).unwrap();

    // The proof should verify as an inclusion proof for B.
    proof_for_b
        .verify(keys[1], Some(&[0xBB; 20]), &root_hash)
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
    let (db, _dir) = new_db();

    let keys: [[u8; 32]; 5] = {
        let mut arr = [[0u8; 32]; 5];
        arr[0][0] = 0x10;
        arr[1][0] = 0x30;
        arr[2][0] = 0x50;
        arr[3][0] = 0x70;
        arr[4][0] = 0x90;
        arr
    };

    // Revision 1.
    db.propose(vec![
        BatchOp::Put {
            key: &keys[0],
            value: &[0xAA; 20],
        },
        BatchOp::Put {
            key: &keys[1],
            value: &[0xBB; 20],
        },
        BatchOp::Put {
            key: &keys[2],
            value: &[0xCC; 20],
        },
        BatchOp::Put {
            key: &keys[3],
            value: &[0xDD; 20],
        },
        BatchOp::Put {
            key: &keys[4],
            value: &[0xEE; 20],
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    // Revision 2: change B and D.
    db.propose(vec![
        BatchOp::Put {
            key: &keys[1],
            value: &[0xB2; 20],
        },
        BatchOp::Put {
            key: &keys[3],
            value: &[0xD2; 20],
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    // Bounded change proof with non-existent boundaries.
    let mut start_key = [0u8; 32];
    start_key[0] = 0x20;
    let mut end_key = [0u8; 32];
    end_key[0] = 0x80;

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
        value: [0xFF; 20].to_vec().into_boxed_slice(),
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
    let (db, _dir) = new_db();

    let keys: [[u8; 32]; 5] = {
        let mut arr = [[0u8; 32]; 5];
        arr[0][0] = 0x10;
        arr[1][0] = 0x30;
        arr[2][0] = 0x50;
        arr[3][0] = 0x70;
        arr[4][0] = 0x90;
        arr
    };

    db.propose(vec![
        BatchOp::Put {
            key: &keys[0],
            value: &[0xAA; 20],
        },
        BatchOp::Put {
            key: &keys[1],
            value: &[0xBB; 20],
        },
        BatchOp::Put {
            key: &keys[2],
            value: &[0xCC; 20],
        },
        BatchOp::Put {
            key: &keys[3],
            value: &[0xDD; 20],
        },
        BatchOp::Put {
            key: &keys[4],
            value: &[0xEE; 20],
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root1 = db.root_hash().unwrap();

    // Revision 2: change B.
    db.propose(vec![BatchOp::Put {
        key: &keys[1],
        value: &[0xB2; 20],
    }])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db.root_hash().unwrap();

    let mut start_key = [0u8; 32];
    start_key[0] = 0x20;
    let mut end_key = [0u8; 32];
    end_key[0] = 0x80;

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
    let mut ops: OwnedBatchOps = valid_proof.batch_ops().to_vec();
    let mut fake_key = [0u8; 32];
    fake_key[0] = 0x60;
    let pos = ops
        .binary_search_by(|op| op.key().as_ref().cmp(fake_key.as_ref()))
        .unwrap_or_else(|i| i);
    ops.insert(
        pos,
        BatchOp::Put {
            key: fake_key.to_vec().into_boxed_slice(),
            value: [0xFF; 20].to_vec().into_boxed_slice(),
        },
    );

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
        "spurious Put of in-range key 0x60 was NOT rejected"
    );
}
