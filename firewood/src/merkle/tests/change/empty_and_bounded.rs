// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Empty start trie and bounded range verification.

use super::*;

//
// When the start revision is empty, every key in the end revision is a new
// insertion. The start proofs are always exclusion proofs (nothing exists
// in the empty trie). These tests exercise the interaction between empty
// start tries and the divergent child / value skip logic.

/// Empty start trie, single key inserted. Complete proof (no bounds).
#[cfg(feature = "ethhash")]
#[test]
fn test_empty_start_trie_single_key_no_bounds() {
    let (db, _dir) = setup_db![];
    let (empty_root, root2) = setup_2nd_commit!(db, [(b"\x50", b"hello")]);

    // Complete proof — no bounds.
    let proof = db
        .change_proof(empty_root, root2.clone(), None, None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2.clone(), None, None, None).unwrap();

    // Receiver also starts empty.
    let (db_b, _dir_b) = setup_db![];
    let empty_root_b = db_b.root_hash().unwrap();

    verify_and_check(&db_b, &proof, &ctx, empty_root_b).unwrap();
}

/// Empty start trie, multiple keys inserted, bounded range with inclusion
/// start proof (`start_key` exists in end trie).
#[cfg(feature = "ethhash")]
#[test]
fn test_empty_start_trie_bounded_inclusion() {
    let (db, _dir) = setup_db![];

    // Insert several keys.
    let (empty_root, root2) = setup_2nd_commit!(
        db,
        [
            (b"\x10", b"a"),
            (b"\x20", b"b"),
            (b"\x30", b"c"),
            (b"\x40", b"d")
        ]
    );

    // Bounded range [\x10, \x30] — start_key \x10 exists (inclusion).
    let proof = db
        .change_proof(
            empty_root.clone(),
            root2.clone(),
            Some(b"\x10"),
            Some(b"\x30"),
            None,
        )
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2.clone(), Some(b"\x10"), Some(b"\x30"), None)
            .unwrap();

    let (db_b, _dir_b) = setup_db![];
    let empty_root_b = db_b.root_hash().unwrap();

    verify_and_check(&db_b, &proof, &ctx, empty_root_b).unwrap();
}

/// Empty start trie, bounded range with exclusion start proof
/// (`start_key` does NOT exist in end trie).
#[cfg(feature = "ethhash")]
#[test]
fn test_empty_start_trie_bounded_exclusion() {
    let (db, _dir) = setup_db![];
    let (empty_root, root2) =
        setup_2nd_commit!(db, [(b"\x10", b"a"), (b"\x20", b"b"), (b"\x30", b"c")]);

    // start_key \x05 does NOT exist — exclusion proof.
    let proof = db
        .change_proof(
            empty_root.clone(),
            root2.clone(),
            Some(b"\x05"),
            Some(b"\x30"),
            None,
        )
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2.clone(), Some(b"\x05"), Some(b"\x30"), None)
            .unwrap();

    let (db_b, _dir_b) = setup_db![];
    let empty_root_b = db_b.root_hash().unwrap();

    verify_and_check(&db_b, &proof, &ctx, empty_root_b).unwrap();
}

/// Empty start trie, large end trie (100 keys), multiple rounds with
/// different boundary positions.
#[cfg(feature = "ethhash")]
#[test]
fn test_empty_start_trie_large_end_trie_multi_round() {
    let (db, _dir) = setup_db![];
    let empty_root = db.root_hash().unwrap();

    // Insert 100 keys: \x00\x00 through \x00\x63.
    let keys: Vec<[u8; 2]> = (0..100).map(|i| [0x00, i]).collect();
    let ops: Vec<BatchOp<&[u8], &[u8]>> = keys
        .iter()
        .map(|k| BatchOp::Put {
            key: k.as_ref(),
            value: b"val" as &[u8],
        })
        .collect();
    db.propose(ops).unwrap().commit().unwrap();
    let root2 = db.root_hash().unwrap();

    let (db_b, _dir_b) = setup_db![];
    let empty_root_b = db_b.root_hash().unwrap();

    // Simulate iterative sync with 4 rounds of ~25 keys each.
    let boundaries: &[Option<&[u8]>] = &[
        None,
        Some(b"\x00\x19"),
        Some(b"\x00\x32"),
        Some(b"\x00\x4b"),
    ];

    for window in boundaries.windows(2) {
        let start = window[0];
        let end = window[1];

        let proof = db
            .change_proof(empty_root.clone(), root2.clone(), start, end, None)
            .unwrap();
        let ctx = verify_change_proof_structure(&proof, root2.clone(), start, end, None).unwrap();
        verify_and_check(&db_b, &proof, &ctx, empty_root_b.clone()).unwrap();
    }

    // Final round: from last boundary to end.
    let proof = db
        .change_proof(
            empty_root.clone(),
            root2.clone(),
            Some(b"\x00\x4b"),
            None,
            None,
        )
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2.clone(), Some(b"\x00\x4b"), None, None)
        .unwrap();
    verify_and_check(&db_b, &proof, &ctx, empty_root_b).unwrap();
}

/// Empty start trie with prefix keys (key is a prefix of another key).
/// Tests that branch nodes with values at prefix keys are handled
/// correctly when the start trie is empty.
#[cfg(feature = "ethhash")]
#[test]
fn test_empty_start_trie_prefix_keys() {
    let (db, _dir) = setup_db![];

    // Insert keys where some are prefixes of others. This creates
    // branch nodes with values — the scenario that triggers the
    // out-of-range value check bug.
    let (empty_root, root2) = setup_2nd_commit!(
        db,
        [
            (b"\x10", b"prefix"),
            (b"\x10\x50", b"child1"),
            (b"\x10\x50\xaa", b"grandchild"),
            (b"\x20", b"other"),
        ]
    );

    let (db_b, _dir_b) = setup_db![];
    let empty_root_b = db_b.root_hash().unwrap();

    // Range [\x10\x50, \x20]: start_key \x10\x50 exists (inclusion).
    // The node at \x10 is on the start proof path with a value, but
    // its byte key < start_key — value check must be skipped.
    let proof = db
        .change_proof(
            empty_root.clone(),
            root2.clone(),
            Some(b"\x10\x50"),
            Some(b"\x20"),
            None,
        )
        .unwrap();
    let ctx = verify_change_proof_structure(
        &proof,
        root2.clone(),
        Some(b"\x10\x50"),
        Some(b"\x20"),
        None,
    )
    .unwrap();
    verify_and_check(&db_b, &proof, &ctx, empty_root_b.clone()).unwrap();

    // Range [\x10\x50\x00, \x20]: start_key doesn't exist (exclusion).
    // Exercises the divergent child path with an empty start trie.
    let proof = db
        .change_proof(
            empty_root.clone(),
            root2.clone(),
            Some(b"\x10\x50\x00"),
            Some(b"\x20"),
            None,
        )
        .unwrap();
    let ctx = verify_change_proof_structure(
        &proof,
        root2.clone(),
        Some(b"\x10\x50\x00"),
        Some(b"\x20"),
        None,
    )
    .unwrap();
    verify_and_check(&db_b, &proof, &ctx, empty_root_b).unwrap();
}

/// Empty start trie, end trie with a single key at the root (empty key
/// prefix). Tests the degenerate case where the root itself has a value.
#[cfg(feature = "ethhash")]
#[test]
fn test_empty_start_trie_start_only_proof() {
    let (db, _dir) = setup_db![];
    let (empty_root, root2) = setup_2nd_commit!(db, [(b"\x10", b"a"), (b"\x20", b"b")]);

    let (db_b, _dir_b) = setup_db![];
    let empty_root_b = db_b.root_hash().unwrap();

    // Start-only proof (end_key = None). Range is [\x10, +inf).
    let proof = db
        .change_proof(empty_root.clone(), root2.clone(), Some(b"\x10"), None, None)
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2.clone(), Some(b"\x10"), None, None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, empty_root_b.clone()).unwrap();

    // End-only proof (start_key = None). Range is (-inf, \x20].
    let proof = db
        .change_proof(empty_root, root2.clone(), None, Some(b"\x20"), None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2, None, Some(b"\x20"), None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, empty_root_b).unwrap();
}

#[test]
fn test_bounded_range_with_existing_keys() {
    // root1: \x10, \x20, \x30 all exist
    let (db_a, _dir_a) = setup_db![(b"\x10", b"v1"), (b"\x20", b"v2"), (b"\x30", b"v3")];
    let (db_b, _dir_b) = setup_db![(b"\x10", b"v1"), (b"\x20", b"v2"), (b"\x30", b"v3")];
    let root1_b = db_b.root_hash().unwrap();

    // root2: change \x20
    let (root1_a, root2) = setup_2nd_commit!(db_a, [(b"\x20", b"changed")]);

    // Range [\x15, \x25]: \x20 is in range, \x10 and \x30 are outside
    let proof = db_a
        .change_proof(
            root1_a.clone(),
            root2.clone(),
            Some(b"\x15"),
            Some(b"\x25"),
            None,
        )
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2.clone(), Some(b"\x15"), Some(b"\x25"), None)
            .unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b.clone()).unwrap();

    // Also test with exact existing keys as boundaries
    let proof = db_a
        .change_proof(root1_a, root2.clone(), Some(b"\x10"), Some(b"\x30"), None)
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2, Some(b"\x10"), Some(b"\x30"), None).unwrap();
    verify_and_check(&db_b, &proof, &ctx, root1_b).unwrap();
}

/// Regression test: a key that is a proper prefix of `start_key` has a value
/// in root1 that is deleted in root2. Because the prefix key is outside the
/// query range (shorter key &lt; `start_key`), it is not in `batch_ops`. The
/// proving trie must clear this stale value so the computed hash matches
/// `end_root`.
///
/// Without the fix, `reconcile_branch_proof_node` ignored the trie's value
/// when the proof node had no value, silently leaving the stale value in the
/// proving trie and causing `EndRootMismatch`.
#[test]
fn test_change_proof_prefix_key_deleted_in_end_root() {
    // Root1: keys include b"\xab" which is a prefix of b"\xab\xcd".
    // Two children under [a,b] ensure the branch survives deletion of
    // b"\xab"'s value (a single child would be merged away).
    let (db, _dir) = setup_db![
        (b"\xab", b"prefix_value"),
        (b"\xab\xcd", b"full_key"),
        (b"\xab\xef", b"sibling"),
        (b"\xff", b"high"),
    ];
    let root1 = db.root_hash().unwrap();

    // Root2: delete the prefix key b"\xab". The branch at [a,b] survives
    // because it still has two children (b"\xab\xcd" and b"\xab\xef").
    // Also update a key within the query range so batch_ops is non-empty.
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

    // Query range starts AFTER b"\xab": use b"\xab\x00" as start_key.
    // b"\xab" < b"\xab\x00", so the deleted prefix key is outside the range
    // and won't appear in batch_ops. But the start proof passes through the
    // branch at nibbles [a,b] where root1 has a value and root2 does not.
    let start_key = b"\xab\x00";
    let end_key = b"\xff";

    let proof = db
        .change_proof(
            root1.clone(),
            root2.clone(),
            Some(start_key.as_slice()),
            Some(end_key.as_slice()),
            None,
        )
        .unwrap();

    let ctx = verify_change_proof_structure(
        &proof,
        root2.clone(),
        Some(start_key.as_slice()),
        Some(end_key.as_slice()),
        None,
    )
    .unwrap();

    // This failed with EndRootMismatch before the fix.
    verify_and_check(&db, &proof, &ctx, root1).unwrap();
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
