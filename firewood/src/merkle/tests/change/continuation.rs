// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Real-proof integration tests for `find_next_key_after_change_proof` —
//! post-verification continuation logic for the syncer's next-range cursor.

use super::*;
use crate::find_next_key_after_change_proof;

#[test]
// Real-proof test for `find_next_key_after_change_proof`'s Case 5
// (Put + terminal at last_key's path with value).
//
// Trie at the end revision contains a single key `0x05`. A Put at
// `0x05` (updating its value from "old" to "new") is the only diff.
// Requesting range `[0x00, 0x10]` produces `batch_ops = [Put 0x05 "new"]`
// and an `end_proof` whose terminal lies at `0x05`'s leaf carrying a
// value — reached because the walk for `0x10` diverges within `0x05`'s
// compressed path.
//
// The function cannot distinguish this Case 5 shape from a truncated
// inclusion of `0x05`, so it conservatively returns
// `Some(lex_successor(0x05), 0x10)`. The follow-up fetch returns no
// diffs (handled by the two-round termination test below).
fn test_find_next_key_change_put_case5() {
    let (db, _dir) = setup_db![(b"\x05", b"old")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x05", b"new")]);

    let proof = db
        .change_proof(root1, root2.clone(), Some(b"\x00"), Some(b"\x10"), None)
        .unwrap();
    assert_eq!(proof.batch_ops().len(), 1);
    assert!(matches!(
        proof.batch_ops()[0],
        BatchOp::Put { ref key, .. } if key.as_ref() == b"\x05"
    ));

    // Structural verification (mirrors the range-proof Layer B test).
    verify_change_proof_structure(&proof, root2, Some(b"\x00"), Some(b"\x10"), None).unwrap();

    let result = find_next_key_after_change_proof(&proof, Some(b"\x10")).unwrap();
    let (cursor, returned_end) = result.expect("Case 5 must return Some");

    // Liveness: cursor strictly greater than last_op's key.
    assert!(
        cursor.as_ref() > b"\x05".as_slice(),
        "cursor {:?} must be > 0x05",
        cursor.as_ref()
    );
    // End key echoed back unchanged.
    assert_eq!(returned_end.as_deref(), Some(b"\x10".as_slice()));
}

#[test]
// Real-proof test for two-round termination.
//
// Syncer-style flow:
//   Round 1: change_proof([0x00, 0x10]) yields the Case 5 shape from
//            `test_find_next_key_change_put_case5`. find_next_key
//            returns `Some(lex_successor(0x05), 0x10)`.
//   Round 2: change_proof([lex_successor(0x05), 0x10]) yields no diffs
//            because no key past `0x05` exists in either revision
//            within the requested range. find_next_key returns `None`
//            via Case 1 (empty `batch_ops`).
//
// Termination in exactly two rounds confirms forward progress.
fn test_find_next_key_change_two_round_termination() {
    let (db, _dir) = setup_db![(b"\x05", b"old")];
    let (root1, root2) = setup_2nd_commit!(db, [(b"\x05", b"new")]);

    // Round 1.
    let proof1 = db
        .change_proof(
            root1.clone(),
            root2.clone(),
            Some(b"\x00"),
            Some(b"\x10"),
            None,
        )
        .unwrap();
    verify_change_proof_structure(&proof1, root2.clone(), Some(b"\x00"), Some(b"\x10"), None)
        .unwrap();
    let (next_start, next_end) = find_next_key_after_change_proof(&proof1, Some(b"\x10"))
        .unwrap()
        .expect("round 1 must return Some for the Case 5 ambiguous shape");

    // Round 2: feed the cursor back as the new range.
    let proof2 = db
        .change_proof(
            root1,
            root2.clone(),
            Some(next_start.as_ref()),
            next_end.as_deref(),
            None,
        )
        .unwrap();
    assert!(
        proof2.batch_ops().is_empty(),
        "no diffs past 0x05 should exist in the requested range"
    );
    verify_change_proof_structure(
        &proof2,
        root2,
        Some(next_start.as_ref()),
        next_end.as_deref(),
        None,
    )
    .unwrap();
    assert_eq!(
        find_next_key_after_change_proof(&proof2, next_end.as_deref()).unwrap(),
        None,
        "round 2 must return None — Case 1 (empty batch_ops)"
    );
}

#[test]
// Real-proof test for the Delete conservative-continuation path.
//
// The start revision contains `{0x05, 0x80}`; the end revision
// deletes `0x05`, leaving `{0x80}`. Requesting range `[0x00, 0x10]`
// produces `batch_ops = [Delete 0x05]` and an `end_proof` whose
// terminal lies at the (single remaining) `0x80` leaf with value —
// terminal_key `0x80` differs from `last_key 0x05`, so this is the
// "Delete + terminal at different key" shape.
//
// The algorithm cannot tell whether the proof was truncated (an
// exclusion of `0x05` that landed on `0x80`'s sibling leaf) or
// exhaustive (an exclusion of `0x10` that landed on the same node).
// It conservatively returns `Some(lex_successor(0x05), 0x10)`.
fn test_find_next_key_change_delete_conservative() {
    let (db, _dir) = setup_db![(b"\x05", b"v0"), (b"\x80", b"v1")];
    let root1 = db.root_hash().unwrap();
    let delete_batch: Vec<BatchOp<&[u8], &[u8]>> = vec![BatchOp::Delete { key: b"\x05" }];
    db.propose(delete_batch).unwrap().commit().unwrap();
    let root2 = db.root_hash().unwrap();

    let proof = db
        .change_proof(root1, root2.clone(), Some(b"\x00"), Some(b"\x10"), None)
        .unwrap();
    assert_eq!(proof.batch_ops().len(), 1);
    assert!(matches!(
        proof.batch_ops()[0],
        BatchOp::Delete { ref key } if key.as_ref() == b"\x05"
    ));

    verify_change_proof_structure(&proof, root2, Some(b"\x00"), Some(b"\x10"), None).unwrap();

    let result = find_next_key_after_change_proof(&proof, Some(b"\x10")).unwrap();
    let (cursor, returned_end) = result.expect("Delete shape must return Some");
    assert!(
        cursor.as_ref() > b"\x05".as_slice(),
        "cursor {:?} must be > 0x05",
        cursor.as_ref()
    );
    assert_eq!(returned_end.as_deref(), Some(b"\x10".as_slice()));
}
