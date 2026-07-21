// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Deterministic regression tests for change-proof verification at range
//! boundaries, where a boundary key shares a trie-descent path with a
//! neighbouring key.
//!
//! Soundness — a tampered or forged op must be rejected:
//! - `test_tampered_right_edge_delete_to_put_is_rejected` (#2091): in a
//!   no-bounds proof, whose end proof anchors on the highest changed key
//!   (`merkle/mod.rs` `change_proof`: `end_key.or(batch_ops.last())`), a deleted
//!   key sharing the anchor's branch but sorting below it is flipped from
//!   Delete to Put.
//! - `test_forged_in_range_delete_to_put_is_rejected` (#2138): an in-range op
//!   whose key is a prefix of the end bound is flipped from Delete to Put.
//! - `test_forged_in_range_op_under_on_path_child_is_rejected`: an in-range op
//!   under the child the boundary descends into is flipped from Delete to Put
//!   (that child is rebuilt from the proposal, so the forgery is caught).
//!
//! Completeness — an honest out-of-range deletion just past a boundary must
//! still verify:
//! - `test_out_of_range_delete_past_end_bound_verifies` (#2136): end bound,
//!   where the deleted key extends the bound.
//! - `test_out_of_range_delete_below_start_bound_verifies`: start bound, where
//!   the bound extends the deleted key.

use super::*;
use crate::{ChangeProof, Proof};

fn is_rejected(
    db: &Db,
    proof: &FrozenChangeProof,
    start_root: api::HashKey,
    end_root: api::HashKey,
) -> bool {
    match verify_change_proof_structure(proof, end_root, None, None, None) {
        Err(_) => true,
        Ok(ctx) => verify_and_check(db, proof, &ctx, start_root).is_err(),
    }
}

/// Whether a proof verifies (is accepted) against the given inclusive bounds.
fn verifies(
    db: &Db,
    proof: &FrozenChangeProof,
    start_root: api::HashKey,
    end_root: api::HashKey,
    start_key: Option<&[u8]>,
    end_key: Option<&[u8]>,
) -> bool {
    match verify_change_proof_structure(proof, end_root, start_key, end_key, None) {
        Err(_) => false,
        Ok(ctx) => verify_and_check(db, proof, &ctx, start_root).is_ok(),
    }
}

#[test]
fn test_tampered_right_edge_delete_to_put_is_rejected() {
    // `0xf0` and `0xfa` keep the `f` branch (nibble path `[f]`) real in the END
    // trie. `0xf51c` (victim) and `0xf5cd` (the max changed key) both live under
    // that branch's child `5`, and both are deleted — so in the end trie child
    // `5` of the `f` branch is absent, and `prove(0xf5cd)` is an exclusion proof
    // terminating at the `f` branch that marks child `5` out-of-range. The
    // victim `0xf51c` sorts below the anchor `0xf5cd` but in that same on-path
    // child.
    let (db, _dir) = setup_db![
        (b"\x10".as_slice(), b"low".as_slice()),
        (b"\xf0".as_slice(), b"fz".as_slice()),
        (b"\xfa".as_slice(), b"fa".as_slice()),
        (b"\xf5\x1c".as_slice(), b"victim".as_slice()), // <- 0xf51c (victim)
        (b"\xf5\xcd".as_slice(), b"anchor".as_slice())  // <- 0xf5cd (anchor)
    ];
    let start_root = db.root_hash().unwrap();
    let end_batch: Vec<BatchOp<&[u8], &[u8]>> = vec![
        BatchOp::Delete { key: b"\xf5\x1c" },
        BatchOp::Delete { key: b"\xf5\xcd" },
    ];
    db.propose(end_batch).unwrap().commit().unwrap();
    let end_root = db.root_hash().unwrap();

    // No-bounds change proof; its end proof anchors on the max op key 0xf5cd.
    let proof = db
        .change_proof(start_root.clone(), end_root.clone(), None, None, None)
        .unwrap();
    assert!(
        !is_rejected(&db, &proof, start_root.clone(), end_root.clone()),
        "honest proof should verify"
    );

    // Tamper: Delete{0xf51c} -> Put{0xf51c, "forged"}.
    let mutated_ops = proof
        .batch_ops()
        .iter()
        .map(|op| match op {
            BatchOp::Delete { key } if key.as_ref() == b"\xf5\x1c" => BatchOp::Put {
                key: key.clone(),
                value: Box::from(&b"forged"[..]),
            },
            other => other.clone(),
        })
        .collect::<Vec<_>>();
    let mutated = ChangeProof::new(
        Proof::new(proof.start_proof().as_ref().into()),
        Proof::new(proof.end_proof().as_ref().into()),
        mutated_ops.into_boxed_slice(),
    );

    assert!(
        is_rejected(&db, &mutated, start_root, end_root),
        "SOUNDNESS BUG: change-proof verification accepted a proof whose batch op for \
         0xf51c was forged from Delete to Put (the key shares the right-edge's 0xf5 \
         branch and sorts below the deleted anchor 0xf5cd, so it is wrongly treated \
         as out-of-range and validated against the proof node instead of the proposal)"
    );
}

/// An honest change proof is rejected with `EndRootMismatch` when an
/// out-of-range key just past the end bound was deleted. `0xfb00` sorts after
/// the bound `0xfb` (a longer key extending a shorter one comes later), so
/// its deletion is out of range and correctly not in `batch_ops`. But the
/// verifier's rebuilt trie retains it under the `f` branch's `b` child, so the
/// computed root does not match `end_root`.
#[test]
fn test_out_of_range_delete_past_end_bound_verifies() {
    let (db, _dir) = setup_db![(b"\xfb\x00".as_slice(), b"\x00".as_slice())];
    let start_root = db.root_hash().unwrap();
    let end_batch: Vec<BatchOp<&[u8], &[u8]>> = vec![
        BatchOp::Delete { key: b"\xfb\x00" },
        BatchOp::Put {
            key: b"\xf7",
            value: b"\x00",
        },
        BatchOp::Put {
            key: b"\xf1",
            value: b"\x00",
        },
    ];
    db.propose(end_batch).unwrap().commit().unwrap();
    let end_root = db.root_hash().unwrap();

    let (sk, ek) = (b"\x00".as_slice(), b"\xfb".as_slice());
    let proof = db
        .change_proof(
            start_root.clone(),
            end_root.clone(),
            Some(sk),
            Some(ek),
            None,
        )
        .unwrap();
    assert!(
        verifies(&db, &proof, start_root, end_root, Some(sk), Some(ek)),
        "honest change proof over [0x00, 0xfb] must verify. The deletion of \
         the out-of-range 0xfb00 (past the end bound 0xfb, which is its \
         prefix) must not cause an EndRootMismatch"
    );
}

/// A forged in-range Delete-to-Put is accepted. `0x56` is in range
/// (`0x56 < 0x5600`) and a prefix of the non-existent end bound `0x5600`.
/// `0x5601` is out of range (`> 0x5600`) sharing the `0x56` path, so the
/// verifier takes `0x56`'s subtree from the proof instead of validating it
/// against the batch, letting a forged value through.
#[test]
fn test_forged_in_range_delete_to_put_is_rejected() {
    let (db, _dir) = setup_db![
        (b"\x56".as_slice(), b"\x01".as_slice()),
        (b"\x56\x01".as_slice(), b"\x01".as_slice())
    ];
    let start_root = db.root_hash().unwrap();
    let end_batch: Vec<BatchOp<&[u8], &[u8]>> = vec![BatchOp::Delete { key: b"\x56" }];
    db.propose(end_batch).unwrap().commit().unwrap();
    let end_root = db.root_hash().unwrap();

    let (sk, ek) = (b"\x00".as_slice(), b"\x56\x00".as_slice());
    let proof = db
        .change_proof(
            start_root.clone(),
            end_root.clone(),
            Some(sk),
            Some(ek),
            None,
        )
        .unwrap();
    let forged_ops = proof
        .batch_ops()
        .iter()
        .map(|op| match op {
            BatchOp::Delete { key } if key.as_ref() == b"\x56" => BatchOp::Put {
                key: key.clone(),
                value: Box::from(&b"forged"[..]),
            },
            other => other.clone(),
        })
        .collect::<Vec<_>>();
    let forged = ChangeProof::new(
        Proof::new(proof.start_proof().as_ref().into()),
        Proof::new(proof.end_proof().as_ref().into()),
        forged_ops.into_boxed_slice(),
    );
    assert!(
        !verifies(&db, &forged, start_root, end_root, Some(sk), Some(ek)),
        "SOUNDNESS BUG: a forged in-range Delete{{0x56}}->Put was accepted. \
         The in-range 0x56 must be validated against the batch, not taken \
         from the proof"
    );
}

/// A forged in-range Delete-to-Put must be rejected when the changed key sits
/// under the child the boundary descends into. `0xf51c` and `0xf5cd` are both
/// deleted in range, so child `5` of the `f` branch is empty in the end trie.
/// The end proof for `0xf5cd` stops at the `f` branch, descending toward the
/// bound through that empty child `5`, and both deleted keys are in range
/// under it. Because an in-range key there is rebuilt from the proposal rather
/// than taken from the proof, flipping `0xf51c`'s Delete to a Put no longer
/// matches `end_root`.
#[test]
fn test_forged_in_range_op_under_on_path_child_is_rejected() {
    let (db, _dir) = setup_db![
        (b"\x10".as_slice(), b"low".as_slice()),
        (b"\xf0".as_slice(), b"fz".as_slice()),
        (b"\xfa".as_slice(), b"fa".as_slice()),
        (b"\xf5\x1c".as_slice(), b"victim".as_slice()),
        (b"\xf5\xcd".as_slice(), b"anchor".as_slice())
    ];
    let start_root = db.root_hash().unwrap();
    let end_batch: Vec<BatchOp<&[u8], &[u8]>> = vec![
        BatchOp::Delete { key: b"\xf5\x1c" },
        BatchOp::Delete { key: b"\xf5\xcd" },
    ];
    db.propose(end_batch).unwrap().commit().unwrap();
    let end_root = db.root_hash().unwrap();

    let (sk, ek) = (b"\x00".as_slice(), b"\xf5\xcd".as_slice());
    let proof = db
        .change_proof(
            start_root.clone(),
            end_root.clone(),
            Some(sk),
            Some(ek),
            None,
        )
        .unwrap();
    assert!(
        verifies(
            &db,
            &proof,
            start_root.clone(),
            end_root.clone(),
            Some(sk),
            Some(ek)
        ),
        "honest proof must verify"
    );

    // Forge the in-range Delete{0xf51c} -> Put. 0xf51c is in range and sits
    // under child 5, which the boundary descends into, so that child is
    // rebuilt from the proposal, catching the forged value.
    let forged_ops = proof
        .batch_ops()
        .iter()
        .map(|op| match op {
            BatchOp::Delete { key } if key.as_ref() == b"\xf5\x1c" => BatchOp::Put {
                key: key.clone(),
                value: Box::from(&b"forged"[..]),
            },
            other => other.clone(),
        })
        .collect::<Vec<_>>();
    let forged = ChangeProof::new(
        Proof::new(proof.start_proof().as_ref().into()),
        Proof::new(proof.end_proof().as_ref().into()),
        forged_ops.into_boxed_slice(),
    );
    assert!(
        !verifies(&db, &forged, start_root, end_root, Some(sk), Some(ek)),
        "SOUNDNESS BUG: a forged in-range Delete{{0xf51c}}->Put was accepted. \
         An in-range key under the child the boundary descends into must be \
         validated against the batch, not taken from the proof"
    );
}

/// An honest change proof is rejected with `EndRootMismatch` when an
/// out-of-range key just below the start bound was deleted, where the start
/// bound extends that deleted key (`0xd44f` extends `0xd4`). The same defect
/// can occur at either boundary: at the end bound the deleted key extends the
/// bound, at the start bound the bound extends the deleted key. The fix handles
/// both edges with the same on-path-child correction.
///
/// start trie:  `{ 0xd4: 0x00, 0xdb: 0x00 }`
/// end trie:    `{ 0xd5: 0x00, 0xdb: 0x00 }`  (`0xd4` deleted, `0xd5` added)
/// proof range: `[0xd44f, 0xf9]`
///
/// `0xd4 < sk = 0xd44f`, so deleting it is out of range and correctly not in
/// `batch_ops`. But the verifier's rebuilt trie retains `0xd4` under the `d`
/// branch, so the computed root does not match `end_root`. `0xd5` and `0xdb`
/// are in range (`0xdb` keeps the `d` branch non-trivial). Minimized from
/// change-proof fuzz seed 8534711138888643184 (`start_nonexistent` scenario).
#[test]
fn test_out_of_range_delete_below_start_bound_verifies() {
    let (db, _dir) = setup_db![
        (b"\xd4".as_slice(), b"\x00".as_slice()),
        (b"\xdb".as_slice(), b"\x00".as_slice())
    ];
    let start_root = db.root_hash().unwrap();
    let end_batch: Vec<BatchOp<&[u8], &[u8]>> = vec![
        BatchOp::Delete { key: b"\xd4" },
        BatchOp::Put {
            key: b"\xd5",
            value: b"\x00",
        },
    ];
    db.propose(end_batch).unwrap().commit().unwrap();
    let end_root = db.root_hash().unwrap();

    let (sk, ek) = (b"\xd4\x4f".as_slice(), b"\xf9".as_slice());
    let proof = db
        .change_proof(
            start_root.clone(),
            end_root.clone(),
            Some(sk),
            Some(ek),
            None,
        )
        .unwrap();
    assert!(
        verifies(&db, &proof, start_root, end_root, Some(sk), Some(ek)),
        "honest change proof over [0xd44f, 0xf9] must verify. The deletion of \
         the out-of-range 0xd4 (below sk, and a prefix of it) must not cause \
         an EndRootMismatch"
    );
}
