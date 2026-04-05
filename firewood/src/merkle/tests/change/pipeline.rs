// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Architectural code path tests exercising each phase of the verification pipeline.

use super::*;

// These tests exercise specific code paths documented in proofs/mod.rs that
// were previously only covered by fuzz tests.

// ── Step 3: collapse_root_to_path ────────────────────────────────────────

/// Out-of-range deletion compresses the root's `partial_path`. The proving
/// trie's root (forked from the proposal) retains the old shape, and
/// `collapse_root_to_path` must reshape it to match `end_root`.
///
/// Exercises: `collapse_root_to_path`
///
/// Root1: keys at nibbles 0 and 1. Root at `[]`.
/// Root2: `\x01` deleted → root compresses to `partial_path` `[1]`.
/// Bounded proof `[\x10, \x20]` excludes the deleted key.
#[test]
fn test_root_compression_from_out_of_range_delete() {
    let (db_a, _dir_a) = setup_db![(b"\x01", b"low"), (b"\x10", b"mid"), (b"\x20", b"hig")];
    let (db_b, _dir_b) = setup_db![(b"\x01", b"low"), (b"\x10", b"mid"), (b"\x20", b"hig")];
    let root1_a = db_a.root_hash().unwrap();
    let root1_b = db_b.root_hash().unwrap();

    // Delete \x01 (outside range) and change \x10 (in range).
    db_a.propose(vec![
        BatchOp::Delete { key: b"\x01" },
        BatchOp::Put {
            key: b"\x10",
            value: b"MID",
        },
    ])
    .unwrap()
    .commit()
    .unwrap();
    let root2 = db_a.root_hash().unwrap();

    let proof = db_a
        .change_proof(
            root1_a,
            root2.clone(),
            Some(b"\x10".as_ref()),
            Some(b"\x20".as_ref()),
            None,
        )
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2, Some(b"\x10"), Some(b"\x20"), None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

// ── Step 4: compute_outside_children terminal cases ──────────────────────

/// Case A: Boundary key diverges within the terminal node's partial path.
///
/// Exercises: `compute_outside_children` lines 233-244 in merkle/mod.rs
///
/// Keys `\x10\x50` and `\x10\x58` share a branch with partial path containing
/// nibble 5. `start_key` `\x10\x40` diverges at nibble 4 < 5, so the start
/// boundary is "before" the terminal — no children are marked outside.
#[test]
fn test_terminal_divergence_within_partial_path() {
    let (db_a, _dir_a) = setup_db![(b"\x10\x50", b"a"), (b"\x10\x58", b"b"), (b"\x30", b"c")];
    let (db_b, _dir_b) = setup_db![(b"\x10\x50", b"a"), (b"\x10\x58", b"b"), (b"\x30", b"c")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x30", b"changed")]);

    // Start_key \x10\x40: diverges within the branch's partial path at
    // nibble 4 vs the branch's nibble 5. This hits Case A of
    // compute_outside_children where the boundary is "before" the terminal.
    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x10\x40"), None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, Some(b"\x10\x40"), None, None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

/// Case B: Terminal is an ancestor of the boundary key. The on-path child
/// must be marked as outside so its hash comes from the proof, not the
/// proving trie.
///
/// Exercises: `compute_outside_children` lines 246-260 in merkle/mod.rs
///
/// Keys at `\x10\x01` and `\x10\x02` create a branch at nibble path `[1,0]`.
/// `start_key` `\x10` is exhausted at depth 2 — the terminal node at `[1,0]`
/// is an ancestor of `start_key`'s nibble path. The on-path child (nibble 0)
/// is marked outside, and children below it (none here) would also be
/// marked.
#[test]
fn test_terminal_ancestor_marks_on_path_child() {
    let (db_a, _dir_a) = setup_db![
        (b"\x10\x01", b"a"),
        (b"\x10\x02", b"b"),
        (b"\x10\x03", b"c"),
        (b"\x30", b"d")
    ];
    let (db_b, _dir_b) = setup_db![
        (b"\x10\x01", b"a"),
        (b"\x10\x02", b"b"),
        (b"\x10\x03", b"c"),
        (b"\x30", b"d")
    ];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x30", b"changed")]);

    // Start_key \x10: exhausted at depth 2, making the terminal an
    // ancestor. compute_outside_children Case B marks the on-path child
    // and all children below it as outside.
    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x10"), None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, Some(b"\x10"), None, None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

/// Case C (right edge): Boundary is a prefix of the terminal node's key.
/// All children extend beyond `end_key`, so all are marked outside.
///
/// Exercises: `compute_outside_children` lines 261-266
///
/// Key `\x20` is a prefix of `\x20\xab`. `end_key` = `\x20` — children of
/// the `\x20` node extend beyond `end_key` and must use proof hashes.
#[test]
fn test_right_edge_boundary_prefix_of_terminal() {
    let (db_a, _dir_a) = setup_db![(b"\x10", b"a"), (b"\x20", b"b"), (b"\x20\xab", b"c")];
    let (db_b, _dir_b) = setup_db![(b"\x10", b"a"), (b"\x20", b"b"), (b"\x20\xab", b"c")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x10", b"changed")]);

    // End_key \x20 is a prefix of the terminal proof node's key \x20\xab.
    // compute_outside_children Case C marks all children as outside.
    let proof = db_a
        .change_proof(root1_a, root2.clone(), None, Some(b"\x20"), None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, None, Some(b"\x20"), None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

// ── Step 2: Out-of-range value reconciliation ────────────────────────────

/// An out-of-range prefix node's value changes between root1 and root2.
/// The proof's value must be adopted during reconciliation so the hybrid
/// hash matches `end_root`.
///
/// Exercises: `reconcile_branch_proof_node` value adoption
///
/// `\x10` has a value that changes from `"old_pfx"` to `"new_pfx"`. The query
/// range `[\x10\x50, \x30]` excludes `\x10` (it's a proper prefix of `start_key`),
/// so `\x10` is out-of-range. The start proof path passes through `\x10`, and
/// reconciliation must adopt the proof's value (`"new_pfx"`) to match `end_root`.
#[test]
fn test_out_of_range_value_change_adopted() {
    let (db_a, _dir_a) = setup_db![
        (b"\x10", b"old_pfx"),
        (b"\x10\x50", b"val0000"),
        (b"\x30", b"val1000")
    ];
    let (db_b, _dir_b) = setup_db![
        (b"\x10", b"old_pfx"),
        (b"\x10\x50", b"val0000"),
        (b"\x30", b"val1000")
    ];
    let root1_b = db_b.root_hash().unwrap();

    // Change \x10's value (out of range) AND \x30's value (in range).
    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x10", b"new_pfx"), (b"\x30", b"changed")]);

    // Range [\x10\x50, \x30]: \x10 < \x10\x50, so \x10 is out of range.
    // The proof must adopt end_root's value for \x10 during reconciliation.
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

// ── EndProofOperationMismatch tests ──────────────────────────────────────

/// Spurious Put at `end_key` when `end_key` doesn't exist (exclusion end proof).
/// The end proof is exclusion but Put expects inclusion — mismatch.
///
/// Exercises: `EndProofOperationMismatch`
#[test]
fn test_spurious_put_at_end_key_boundary() {
    let (db_a, _dir_a) = setup_db![(b"\x20", b"v2"), (b"\x90", b"v9")];
    let (db_b, _dir_b) = setup_db![(b"\x20", b"v2"), (b"\x90", b"v9")];
    let root1_b = db_b.root_hash().unwrap();

    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x20", b"changed")]);

    // end_key \xb0 doesn't exist — exclusion end proof.
    let honest = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x20"), Some(b"\xb0"), None)
        .unwrap();

    // Inject Put at \xb0 (end_key). end proof is exclusion but Put expects inclusion.
    let mut ops: Vec<BatchOp<Key, Value>> = honest.batch_ops().to_vec();
    ops.push(BatchOp::Put {
        key: b"\xb0".to_vec().into(),
        value: b"fake".to_vec().into(),
    });
    ops.sort_by(|a, b| a.key().cmp(b.key()));

    let crafted = FrozenChangeProof::new(
        crate::Proof::new(honest.start_proof().as_ref().into()),
        crate::Proof::new(honest.end_proof().as_ref().into()),
        ops.into_boxed_slice(),
    );

    let result =
        verify_change_proof_structure(&crafted, root2.clone(), Some(b"\x20"), Some(b"\xb0"), None);
    if let Ok(ctx) = result {
        verify_and_check(&db_b, &crafted, &ctx, root1_b)
            .expect_err("spurious Put at end_key should be detected");
    }
}

/// Spurious Delete at `end_key` when `end_key` EXISTS (inclusion end proof).
/// The end proof is inclusion but Delete expects exclusion — mismatch.
///
/// Exercises: `EndProofOperationMismatch`
#[test]
fn test_spurious_delete_at_end_key_boundary() {
    let (db_a, _dir_a) = setup_db![(b"\x20", b"v2"), (b"\x90", b"v9")];
    let (db_b, _dir_b) = setup_db![(b"\x20", b"v2"), (b"\x90", b"v9")];
    let root1_b = db_b.root_hash().unwrap();

    // Change \x20, leave \x90 unchanged.
    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x20", b"changed")]);

    // end_key \x90 EXISTS — inclusion end proof.
    let honest = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x20"), Some(b"\x90"), None)
        .unwrap();

    // Inject Delete at \x90 (end_key). end proof is inclusion but Delete expects exclusion.
    let mut ops: Vec<BatchOp<Key, Value>> = honest.batch_ops().to_vec();
    ops.push(BatchOp::Delete {
        key: b"\x90".to_vec().into(),
    });
    ops.sort_by(|a, b| a.key().cmp(b.key()));

    let crafted = FrozenChangeProof::new(
        crate::Proof::new(honest.start_proof().as_ref().into()),
        crate::Proof::new(honest.end_proof().as_ref().into()),
        ops.into_boxed_slice(),
    );

    let result =
        verify_change_proof_structure(&crafted, root2.clone(), Some(b"\x20"), Some(b"\x90"), None);
    if let Ok(ctx) = result {
        verify_and_check(&db_b, &crafted, &ctx, root1_b)
            .expect_err("spurious Delete at end_key should be detected");
    }
}

// ── MissingEndProof with empty batch_ops ─────────────────────────────────

/// Empty `batch_ops` with `end_key` set but no `end_proof` → `MissingEndProof`.
///
/// Exercises: the `(true, true)` branch in `verify_change_proof_structure`
#[test]
fn test_missing_end_proof_empty_batch_ops() {
    let proof = FrozenChangeProof::new(
        crate::Proof::new(Box::new([])),
        crate::Proof::new(Box::new([])),
        Box::new([]),
    );
    let err =
        verify_change_proof_structure(&proof, api::HashKey::empty(), None, Some(b"\x50"), None)
            .unwrap_err();
    assert!(matches!(
        err,
        api::Error::ProofError(crate::ProofError::MissingEndProof)
    ));
}

// ── ConflictingProofNodes test ───────────────────────────────────────────

/// When start and end proofs share a proof node at the same key but with
/// different content, reconciliation rejects with `ConflictingProofNodes`.
///
/// Exercises: `verify_change_proof_root_hash` line 752
///
/// Take a valid bounded proof and flip a bit in the end proof's root node.
/// This breaks the structural hash chain, so we skip structural validation
/// and call `verify_change_proof_root_hash` directly. The start and end
/// proofs both contain the root node, but now they disagree.
#[test]
fn test_conflicting_proof_nodes_rejected() {
    let (db, _dir) = setup_db![(b"\x10", b"a"), (b"\x20", b"b")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x10", b"A"), (b"\x20", b"B")]);

    // Valid bounded proof [\x10, \x20] — both proofs share the root node.
    let valid = db
        .change_proof(
            root1.clone(),
            root2.clone(),
            Some(b"\x10"),
            Some(b"\x20"),
            None,
        )
        .unwrap();

    // Modify a child hash in the end proof's root node so it differs from
    // the start proof's root node (which remains untouched).
    let mut end_nodes: Vec<crate::ProofNode> = valid.end_proof().as_ref().to_vec();
    assert!(!end_nodes.is_empty(), "end proof should have nodes");

    // Flip a bit in the first child hash we find at the root node.
    let root_node = &mut end_nodes[0];
    for pc in firewood_storage::PathComponent::ALL {
        if let Some(ref mut h) = root_node.child_hashes[pc] {
            let trie_hash = h.clone().into_triehash();
            let bytes: [u8; 32] = trie_hash.into();
            let mut new_bytes = bytes;
            new_bytes[0] ^= 1;
            *h = firewood_storage::TrieHash::from(new_bytes).into_hash_type();
            break;
        }
    }

    let crafted = FrozenChangeProof::new(
        crate::Proof::new(valid.start_proof().as_ref().into()),
        crate::Proof::new(end_nodes.into_boxed_slice()),
        valid.batch_ops().into(),
    );

    // Skip structural validation (the modified hash chain won't pass).
    // Call `verify_change_proof_root_hash` directly with a hand-built context.
    let ctx = ChangeProofVerificationContext {
        end_root: root2,
        start_key: Some(b"\x10".to_vec().into()),
        end_key: Some(b"\x20".to_vec().into()),
        right_edge_key: Some(b"\x20".to_vec().into()),
    };

    let parent = db.revision(root1).unwrap();
    let proposal = db.apply_change_proof_to_parent(&crafted, &*parent).unwrap();
    let err = verify_change_proof_root_hash(&crafted, &ctx, &proposal).unwrap_err();
    assert!(
        matches!(
            err,
            api::Error::ProofError(crate::ProofError::ConflictingProofNodes)
        ),
        "expected ConflictingProofNodes, got {err:?}"
    );
}

// ── ValueAtOddNibbleLength test ──────────────────────────────────────────

/// A proof node at an odd nibble depth carrying a value must be rejected.
/// Values can only exist at even nibble depths (complete byte boundaries).
///
/// Exercises: `reconcile_branch_proof_node` lines 2163-2166
///
/// Keys `\x10` and `\x15` share nibble 1 at depth 0, then fork at depth 1
/// (nibbles 0 vs 5). The branch at depth 1 is at odd nibble length. We
/// inject a spurious value into that odd-depth proof node.
#[test]
fn test_value_at_odd_nibble_length_rejected() {
    let (db, _dir) = setup_db![(b"\x10", b"a"), (b"\x15", b"b"), (b"\x30", b"c")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x30", b"changed")]);

    let valid = db
        .change_proof(root1.clone(), root2.clone(), Some(b"\x10"), None, None)
        .unwrap();

    // Find a start proof node at odd nibble depth and inject a value.
    let mut start_nodes: Vec<crate::ProofNode> = valid.start_proof().as_ref().to_vec();
    let odd_idx = start_nodes
        .iter()
        .position(|n| n.key.len() % 2 != 0)
        .expect("should have a proof node at odd nibble depth");

    start_nodes[odd_idx].value_digest = Some(firewood_storage::ValueDigest::Value(
        b"injected".to_vec().into(),
    ));

    let crafted = FrozenChangeProof::new(
        crate::Proof::new(start_nodes.into_boxed_slice()),
        crate::Proof::new(valid.end_proof().as_ref().into()),
        valid.batch_ops().into(),
    );

    // Skip structural validation (injected value breaks the hash chain).
    // Call `verify_change_proof_root_hash` directly.
    let ctx = ChangeProofVerificationContext {
        end_root: root2,
        start_key: Some(b"\x10".to_vec().into()),
        end_key: None,
        right_edge_key: None,
    };

    let parent = db.revision(root1).unwrap();
    let proposal = db.apply_change_proof_to_parent(&crafted, &*parent).unwrap();
    let err = verify_change_proof_root_hash(&crafted, &ctx, &proposal).unwrap_err();
    assert!(
        matches!(
            err,
            api::Error::ProofError(crate::ProofError::ValueAtOddNibbleLength)
        ),
        "expected ValueAtOddNibbleLength, got {err:?}"
    );
}
