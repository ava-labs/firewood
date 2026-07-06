// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Regression: a no-bounds change proof anchors its end proof on the highest
//! changed key (`merkle/mod.rs` `change_proof`: `end_key.or(batch_ops.last())`).
//! Tampering the right-edge op must still be rejected. Reported in
//! ava-labs/firewood#2091; the change-proof verification fuzzer that originally
//! surfaced it (seed 6348968561142317239) is not on `main`, so this
//! deterministic case guards the fix in its place.

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
