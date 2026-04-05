// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! `prove()` divergent child inclusion in exclusion proofs.

use super::*;

/// When a key is deleted and the branch compresses (`partial_path` absorbs
/// the remaining child), the exclusion proof must include the divergent
/// child node. This test verifies that `prove()` appends the child.
#[test]
fn test_exclusion_proof_includes_divergent_child() {
    // Create two keys that share a branch: \x10\x50 and \x10\x58.
    // Deleting \x10\x50 causes the branch to compress — the remaining
    // child \x10\x58 absorbs the branch's partial_path.
    let (db, _dir) = setup_db![(b"\x10\x50", b"a"), (b"\x10\x58", b"b")];
    let root1 = db.root_hash().unwrap();

    // Delete \x10\x50 — branch at nibble depth 5 compresses.
    db.propose(vec![BatchOp::Delete::<_, &[u8]> { key: b"\x10\x50" }])
        .unwrap()
        .commit()
        .unwrap();

    // Generate exclusion proof for the deleted key.
    let rev = db.revision(db.root_hash().unwrap()).unwrap();
    let merkle = crate::merkle::Merkle::from(&*rev);
    let proof = merkle.prove(b"\x10\x50").unwrap();
    let nodes = proof.as_ref();

    // The proof must have more than just the ancestor nodes — it must
    // include the divergent child whose key extends past \x10\x50.
    let last = nodes.last().unwrap();
    let last_key_nibbles: Vec<u8> = last.key.as_byte_slice().to_vec();
    let target_nibbles: Vec<u8> = firewood_storage::NibblesIterator::new(b"\x10\x50").collect();

    // The last node's key must NOT equal the target key (exclusion).
    assert_ne!(
        last_key_nibbles, target_nibbles,
        "should be exclusion proof"
    );

    // The last node's key must extend past the target key — it's the
    // divergent child, not just an ancestor.
    assert!(
        last_key_nibbles.len() > target_nibbles.len()
            || last_key_nibbles
                .iter()
                .zip(target_nibbles.iter())
                .any(|(a, b)| a != b),
        "last node should be the divergent child, not an ancestor"
    );

    // The proof should still validate as a valid exclusion proof.
    let root2 = db.root_hash().unwrap();
    let hash = root2.clone();
    assert!(
        proof.value_digest(b"\x10\x50", &hash).unwrap().is_none(),
        "should be a valid exclusion proof"
    );

    // Verify the full change proof round-trip works.
    let (db_b, _dir_b) = setup_db![(b"\x10\x50", b"a"), (b"\x10\x58", b"b")];
    let root1_b = db_b.root_hash().unwrap();

    let change = db
        .change_proof(root1, root2.clone(), Some(b"\x10\x50"), None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&change, root2, Some(b"\x10\x50"), None, None).unwrap();
    verify_and_check(&db_b, &change, &ctx, root1_b).unwrap();
}

/// When the child at the next nibble doesn't exist (None), the exclusion
/// proof should NOT append a divergent child. The proof should still be
/// a valid exclusion proof.
#[test]
fn test_exclusion_proof_no_child_at_next_nibble() {
    // Create keys at \x20 and \x30. Proving \x10: the root has children
    // at nibble 2 and 3, but NO child at nibble 1. The proof stops at
    // the root with no divergent child to append.
    let (db, _dir) = setup_db![(b"\x20", b"a"), (b"\x30", b"b")];

    let rev = db.revision(db.root_hash().unwrap()).unwrap();
    let merkle = crate::merkle::Merkle::from(&*rev);
    let proof = merkle.prove(b"\x10").unwrap();

    // The proof should be a valid exclusion proof.
    let root_hash = db.root_hash().unwrap();
    let hash = root_hash;
    assert!(
        proof.value_digest(b"\x10", &hash).unwrap().is_none(),
        "should be a valid exclusion proof without divergent child"
    );
}

/// An incomplete exclusion proof (missing the divergent child when one
/// exists) must be rejected by `value_digest` with `ExclusionProofMissingChild`.
#[test]
fn test_incomplete_exclusion_proof_rejected() {
    // Create a trie with enough keys to ensure the proof has multiple
    // nodes. \x10\x50 and \x10\x58 share a branch; \x30 ensures the
    // root is a branch with children at nibbles 1 and 3.
    let (db, _dir) = setup_db![
        (b"\x10\x50", b"a"),
        (b"\x10\x58", b"b"),
        (b"\x30\x00", b"c")
    ];

    // Delete \x10\x50. The branch at nibble depth 5 compresses, and
    // \x10\x58 becomes the divergent child for exclusion proofs of
    // \x10\x50.
    db.propose(vec![BatchOp::Delete::<_, &[u8]> { key: b"\x10\x50" }])
        .unwrap()
        .commit()
        .unwrap();
    let root2 = db.root_hash().unwrap();

    // Generate a correct exclusion proof (includes divergent child).
    let rev2 = db.revision(root2.clone()).unwrap();
    let merkle = crate::merkle::Merkle::from(&*rev2);
    let full_proof = merkle.prove(b"\x10\x50").unwrap();

    // The full proof should have at least 2 nodes: ancestor + divergent child.
    let nodes = full_proof.as_ref();
    assert!(
        nodes.len() >= 2,
        "expected at least 2 nodes (ancestor + divergent child), got {}",
        nodes.len()
    );

    // Craft an incomplete proof by removing the last node (the divergent child).
    let truncated: Vec<crate::ProofNode> = nodes[..nodes.len() - 1].to_vec();
    let incomplete = crate::Proof::new(truncated.into_boxed_slice());

    // The incomplete proof should be rejected because the ancestor has
    // a child at the next nibble but the proof doesn't include it.
    let hash = root2;
    let err = incomplete.value_digest(b"\x10\x50", &hash).unwrap_err();
    assert!(
        matches!(err, crate::ProofError::ExclusionProofMissingChild),
        "expected ExclusionProofMissingChild, got {err:?}"
    );
}
