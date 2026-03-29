// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::num::NonZeroUsize;
use std::sync::Arc;

use super::*;
use crate::{
    api::{self, BatchOp},
    merkle::changes::ChangeProof,
    proofs::{ProofError, change::verify_change_proof},
    Proof,
};
use firewood_storage::{ImmutableProposal, MemStore, NodeStore};

/// Apply the `batch_ops` from a change proof to a forked copy of `start`,
/// returning the resulting finalized (hashed) immutable proposal merkle.
#[expect(clippy::type_complexity)]
fn apply_batch_ops(
    start: &Merkle<NodeStore<Committed, MemStore>>,
    batch_ops: &[BatchOp<Box<[u8]>, Box<[u8]>>],
) -> Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> {
    let mut proposal = start.fork().expect("fork should succeed");
    for op in batch_ops {
        match op {
            BatchOp::Put { key, value } => {
                proposal
                    .insert(key.as_ref(), value.clone())
                    .expect("insert should succeed");
            }
            BatchOp::Delete { key } => {
                proposal
                    .remove(key.as_ref())
                    .expect("remove should succeed");
            }
            BatchOp::DeleteRange { .. } => {
                panic!("unexpected DeleteRange in change proof batch ops");
            }
        }
    }
    proposal.hash()
}

#[test]
#[expect(clippy::too_many_lines)]
/// Fuzz-style test: generates a random start trie, applies random changes to
/// produce an end trie, generates a change proof between them, then verifies
/// the proof under five scenarios:
///
/// 1. Both boundary keys are existing keys
/// 2. Start boundary is a non-existent (decreased) key
/// 3. End boundary is a non-existent (increased) key
/// 4. Both boundaries are non-existent
/// 5. No bounds (complete proof)
///
/// Runs 100 independent iterations, each with a freshly seeded RNG. On failure,
/// the printed seed can be passed via `FIREWOOD_TEST_SEED` to reproduce.
fn test_change_proof_fuzz() {
    // Outer rng seeds each of the 100 independent runs.
    let outer_rng = firewood_storage::SeededRng::from_env_or_random();

    for run in 0..100 {
        let seed = outer_rng.next_u64();
        eprintln!("run {run}: seed={seed} (export FIREWOOD_TEST_SEED={seed} to reproduce)");
        let rng = firewood_storage::SeededRng::new(seed);

        // Build the start trie from 64–2048 random keys.
        let key_count = rng.random_range(64..=2048_u32);
        let start_data = fixed_and_pseudorandom_data(&rng, key_count);
        let mut start_items: Vec<_> = start_data.iter().collect();
        start_items.sort_unstable();
        let start_merkle = init_merkle(start_items.clone());

        // Build the end trie: keep ~70% of start keys, add some new ones, delete others.
        let mut end_data = start_data.clone();

        // Delete ~15% of existing keys.
        let delete_count = (start_items.len() / 7).max(1);
        for i in (0..start_items.len()).step_by(start_items.len() / delete_count + 1) {
            end_data.remove(start_items[i].0);
        }

        // Insert ~15% new keys.
        let insert_count = rng.random_range(10..=50_u32);
        for _ in 0..insert_count {
            end_data.insert(rng.random::<[u8; 32]>(), rng.random::<[u8; 20]>());
        }

        let mut end_items: Vec<_> = end_data.iter().collect();
        end_items.sort_unstable();
        let end_merkle = init_merkle(end_items.clone());
        let end_root = end_merkle
            .nodestore()
            .root_hash()
            .expect("end trie must have a root");

        // Run 50 random verification scenarios per outer iteration.
        for _ in 0..50 {
            let scenario = rng.random_range(0..5_u32);
            match scenario {
                // Scenario 1: both boundary keys are existing end-state keys.
                0 => {
                    if end_items.len() < 2 {
                        continue;
                    }
                    let si = rng.random_range(0..end_items.len() - 1);
                    let ei = rng.random_range(si + 1..end_items.len());
                    let start_key: &[u8] = end_items[si].0;
                    let end_key: &[u8] = end_items[ei].0;

                    let proof = end_merkle
                        .change_proof(
                            Some(start_key),
                            Some(end_key),
                            start_merkle.nodestore(),
                            None,
                        )
                        .expect("change_proof should succeed");

                    let applied = apply_batch_ops(&start_merkle, proof.batch_ops());
                    verify_change_proof(
                        &proof,
                        applied.nodestore(),
                        &end_root,
                        Some(start_key),
                        Some(end_key),
                        None,
                    )
                    .expect("verify_change_proof should pass (scenario 1)");
                }

                // Scenario 2: start boundary is a non-existent (decreased) key.
                1 => {
                    if end_items.len() < 2 {
                        continue;
                    }
                    let si = rng.random_range(1..end_items.len() - 1);
                    let ei = rng.random_range(si..end_items.len());
                    let decreased = decrease_key(end_items[si].0);
                    // Skip if decreased collides with the previous key.
                    if &decreased >= end_items[si].0
                        || (si > 0
                            && decreased.as_ref() == end_items[si - 1].0.as_ref())
                    {
                        continue;
                    }
                    let end_key: &[u8] = end_items[ei].0;

                    let proof = end_merkle
                        .change_proof(
                            Some(&decreased),
                            Some(end_key),
                            start_merkle.nodestore(),
                            None,
                        )
                        .expect("change_proof should succeed");

                    let applied = apply_batch_ops(&start_merkle, proof.batch_ops());
                    verify_change_proof(
                        &proof,
                        applied.nodestore(),
                        &end_root,
                        Some(&decreased),
                        Some(end_key),
                        None,
                    )
                    .expect("verify_change_proof should pass (scenario 2)");
                }

                // Scenario 3: end boundary is a non-existent (increased) key.
                2 => {
                    if end_items.len() < 2 {
                        continue;
                    }
                    let si = rng.random_range(0..end_items.len() - 1);
                    let ei = rng.random_range(si..end_items.len() - 1);
                    let increased = increase_key(end_items[ei].0);
                    // Skip if increased collides with the next key or overflows.
                    if &increased <= end_items[ei].0
                        || (ei + 1 < end_items.len()
                            && increased.as_ref() == end_items[ei + 1].0.as_ref())
                    {
                        continue;
                    }
                    let start_key: &[u8] = end_items[si].0;

                    let proof = end_merkle
                        .change_proof(
                            Some(start_key),
                            Some(&increased),
                            start_merkle.nodestore(),
                            None,
                        )
                        .expect("change_proof should succeed");

                    let applied = apply_batch_ops(&start_merkle, proof.batch_ops());
                    verify_change_proof(
                        &proof,
                        applied.nodestore(),
                        &end_root,
                        Some(start_key),
                        Some(&increased),
                        None,
                    )
                    .expect("verify_change_proof should pass (scenario 3)");
                }

                // Scenario 4: both boundaries are non-existent keys.
                3 => {
                    if end_items.len() < 2 {
                        continue;
                    }
                    let si = rng.random_range(1..end_items.len() - 1);
                    let ei = rng.random_range(si..end_items.len() - 1);
                    let decreased = decrease_key(end_items[si].0);
                    let increased = increase_key(end_items[ei].0);
                    if &decreased >= end_items[si].0
                        || (si > 0
                            && decreased.as_ref() == end_items[si - 1].0.as_ref())
                        || &increased <= end_items[ei].0
                        || (ei + 1 < end_items.len()
                            && increased.as_ref() == end_items[ei + 1].0.as_ref())
                    {
                        continue;
                    }

                    let proof = end_merkle
                        .change_proof(
                            Some(&decreased),
                            Some(&increased),
                            start_merkle.nodestore(),
                            None,
                        )
                        .expect("change_proof should succeed");

                    let applied = apply_batch_ops(&start_merkle, proof.batch_ops());
                    verify_change_proof(
                        &proof,
                        applied.nodestore(),
                        &end_root,
                        Some(&decreased),
                        Some(&increased),
                        None,
                    )
                    .expect("verify_change_proof should pass (scenario 4)");
                }

                // Scenario 5: no bounds — complete proof.
                _ => {
                    let proof = end_merkle
                        .change_proof(
                            None::<&[u8]>,
                            None::<&[u8]>,
                            start_merkle.nodestore(),
                            None,
                        )
                        .expect("change_proof should succeed");

                    let applied = apply_batch_ops(&start_merkle, proof.batch_ops());
                    verify_change_proof(&proof, applied.nodestore(), &end_root, None, None, None)
                        .expect("verify_change_proof should pass (scenario 5)");
                }
            }
        }
    }
}

// ── Structural (verify_proof_structure) negative tests ──────────────────────
//
// These tests verify that malformed change proofs are rejected before the
// root-hash step.  They use a dummy empty trie and a zero `end_root` because
// `verify_proof_structure` runs first and the root-hash check is never reached.

fn empty_committed() -> Merkle<NodeStore<Committed, MemStore>> {
    init_merkle::<_, &[u8], &[u8]>([])
}

fn empty_proof() -> Proof<Box<[ProofNode]>> {
    Proof::new(Box::new([]))
}

fn bkey(k: &[u8]) -> Box<[u8]> {
    k.into()
}

fn bval(v: &[u8]) -> Box<[u8]> {
    v.into()
}

#[test]
fn test_reject_inverted_range() {
    let dummy = empty_committed();
    let end_root = TrieHash::empty();
    let proof = ChangeProof::new(empty_proof(), empty_proof(), Box::new([]));
    let err = verify_change_proof(
        &proof,
        dummy.nodestore(),
        &end_root,
        Some(b"\x90"),
        Some(b"\x10"), // start > end
        None,
    )
    .unwrap_err();
    assert!(matches!(err, api::Error::InvalidRange { .. }), "got {err:?}");
}

#[test]
fn test_reject_delete_range_op() {
    let dummy = empty_committed();
    let end_root = TrieHash::empty();
    let batch: Box<[BatchOp<Key, Value>]> =
        Box::new([BatchOp::DeleteRange { prefix: bkey(b"\x10") }]);
    let proof = ChangeProof::new(empty_proof(), empty_proof(), batch);
    let err =
        verify_change_proof(&proof, dummy.nodestore(), &end_root, None, None, None).unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::UnsupportedDeleteRange)),
        "got {err:?}"
    );
}

#[test]
fn test_reject_unsorted_keys() {
    let dummy = empty_committed();
    let end_root = TrieHash::empty();
    // Keys are in descending order — must be ascending.
    let batch: Box<[BatchOp<Key, Value>]> = Box::new([
        BatchOp::Put { key: bkey(b"\x90"), value: bval(b"\x09") },
        BatchOp::Put { key: bkey(b"\x10"), value: bval(b"\x01") },
    ]);
    let proof = ChangeProof::new(empty_proof(), empty_proof(), batch);
    let err =
        verify_change_proof(&proof, dummy.nodestore(), &end_root, None, None, None).unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::ChangeProofKeysNotSorted)),
        "got {err:?}"
    );
}

#[test]
fn test_reject_duplicate_keys() {
    let dummy = empty_committed();
    let end_root = TrieHash::empty();
    let batch: Box<[BatchOp<Key, Value>]> = Box::new([
        BatchOp::Put { key: bkey(b"\x50"), value: bval(b"\x05") },
        BatchOp::Put { key: bkey(b"\x50"), value: bval(b"\x06") }, // duplicate
    ]);
    let proof = ChangeProof::new(empty_proof(), empty_proof(), batch);
    let err =
        verify_change_proof(&proof, dummy.nodestore(), &end_root, None, None, None).unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::ChangeProofKeysNotSorted)),
        "got {err:?}"
    );
}

#[test]
fn test_reject_start_key_exceeds_first_op() {
    let dummy = empty_committed();
    let end_root = TrieHash::empty();
    let batch: Box<[BatchOp<Key, Value>]> =
        Box::new([BatchOp::Put { key: bkey(b"\x50"), value: bval(b"\x05") }]);
    let proof = ChangeProof::new(empty_proof(), empty_proof(), batch);
    // start_key (\x90) > first op key (\x50)
    let err =
        verify_change_proof(&proof, dummy.nodestore(), &end_root, Some(b"\x90"), None, None)
            .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::StartKeyLargerThanFirstKey)),
        "got {err:?}"
    );
}

#[test]
fn test_reject_end_key_below_last_op() {
    let dummy = empty_committed();
    let end_root = TrieHash::empty();
    let batch: Box<[BatchOp<Key, Value>]> =
        Box::new([BatchOp::Put { key: bkey(b"\x50"), value: bval(b"\x05") }]);
    let proof = ChangeProof::new(empty_proof(), empty_proof(), batch);
    // end_key (\x10) < last op key (\x50)
    let err =
        verify_change_proof(&proof, dummy.nodestore(), &end_root, None, Some(b"\x10"), None)
            .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::EndKeyLessThanLastKey)),
        "got {err:?}"
    );
}

#[test]
fn test_reject_missing_boundary_proof() {
    let dummy = empty_committed();
    let end_root = TrieHash::empty();
    // Non-empty batch ops with a start bound but no boundary proofs.
    let batch: Box<[BatchOp<Key, Value>]> =
        Box::new([BatchOp::Put { key: bkey(b"\x50"), value: bval(b"\x05") }]);
    let proof = ChangeProof::new(empty_proof(), empty_proof(), batch);
    let err =
        verify_change_proof(&proof, dummy.nodestore(), &end_root, Some(b"\x10"), None, None)
            .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::MissingBoundaryProof)),
        "got {err:?}"
    );
}

#[test]
fn test_reject_unexpected_end_proof() {
    // Build a real end proof from a known trie, then reuse it in a proof that
    // has neither an end_key nor any batch ops — the honest generator never
    // produces this combination.
    let trie = init_merkle([
        (b"\x10".as_ref(), b"\x01".as_ref()),
        (b"\x90".as_ref(), b"\x09".as_ref()),
    ]);
    let end_root = trie.nodestore().root_hash().unwrap();

    let real = trie
        .change_proof(Some(b"\x10"), Some(b"\x90"), trie.nodestore(), None)
        .unwrap();

    // Tamper: strip start proof and batch ops but keep the non-empty end proof.
    let tampered =
        ChangeProof::new(empty_proof(), real.end_proof().clone(), Box::new([]));

    let err = verify_change_proof(&tampered, trie.nodestore(), &end_root, None, None, None)
        .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::UnexpectedEndProof)),
        "got {err:?}"
    );
}

#[test]
fn test_reject_proof_exceeds_max_length() {
    let dummy = empty_committed();
    let end_root = TrieHash::empty();
    let batch: Box<[BatchOp<Key, Value>]> = Box::new([
        BatchOp::Put { key: bkey(b"\x10"), value: bval(b"\x01") },
        BatchOp::Put { key: bkey(b"\x50"), value: bval(b"\x05") },
        BatchOp::Put { key: bkey(b"\x90"), value: bval(b"\x09") },
    ]);
    let proof = ChangeProof::new(empty_proof(), empty_proof(), batch);
    let err = verify_change_proof(
        &proof,
        dummy.nodestore(),
        &end_root,
        None,
        None,
        NonZeroUsize::new(2), // max 2, but proof has 3
    )
    .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::ProofIsLargerThanMaxLength)),
        "got {err:?}"
    );
}

// ── Step-3 (verify_root_hash) negative tests ────────────────────────────────
//
// Uses three single-byte keys in distinct nibble subtrees of the root branch:
//
//   key_a = [0x10]  (nibble path [1,0])
//   key_b = [0x50]  (nibble path [5,0])
//   key_c = [0x90]  (nibble path [9,0])
//
// A change proof for range [key_a, key_c] produces both a start proof and end
// proof (Case 2c) with key_b sitting as an in-range child between them.

/// Case 1 (complete proof, no boundary keys): the applied trie's root hash must
/// equal `end_root`.  Verify that passing the unmodified start trie (i.e., not
/// applying the batch ops) causes `EndRootMismatch`.
#[cfg(not(feature = "ethhash"))]
#[test]
fn test_reject_wrong_root_complete_proof() {
    let start = init_merkle([(b"\x10".as_ref(), b"\x01".as_ref())]);
    let end = init_merkle([(b"\x10".as_ref(), b"\x02".as_ref())]); // value changed
    let end_root = end.nodestore().root_hash().unwrap();

    let proof = end
        .change_proof(None::<&[u8]>, None::<&[u8]>, start.nodestore(), None)
        .unwrap();

    // Correct: apply batch ops → root matches end_root.
    let correct = apply_batch_ops(&start, proof.batch_ops());
    verify_change_proof(&proof, correct.nodestore(), &end_root, None, None, None)
        .expect("should pass with correct applied trie");

    // Wrong: skip the batch ops → root still equals start root ≠ end_root.
    let wrong = apply_batch_ops(&start, &[]);
    let err = verify_change_proof(&proof, wrong.nodestore(), &end_root, None, None, None)
        .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::EndRootMismatch)),
        "got {err:?}"
    );
}

/// Case 2c (dual proofs): a batch op that adds key_b is omitted when building
/// the applied trie.  The proof's root node stores a non-None child hash for
/// nibble 5 (key_b's direction), but the applied trie has None there →
/// `InRangeChildMismatch`.
#[cfg(not(feature = "ethhash"))]
#[test]
fn test_reject_omitted_batch_op_in_range() {
    let start = init_merkle([
        (b"\x10".as_ref(), b"\x01".as_ref()),
        (b"\x90".as_ref(), b"\x09".as_ref()),
    ]);
    let end = init_merkle([
        (b"\x10".as_ref(), b"\x01".as_ref()),
        (b"\x50".as_ref(), b"\x05".as_ref()), // added key_b
        (b"\x90".as_ref(), b"\x09".as_ref()),
    ]);
    let end_root = end.nodestore().root_hash().unwrap();

    let proof = end
        .change_proof(Some(b"\x10"), Some(b"\x90"), start.nodestore(), None)
        .expect("change_proof should succeed");

    // Correct: apply the Put for key_b.
    let correct = apply_batch_ops(&start, proof.batch_ops());
    verify_change_proof(
        &proof,
        correct.nodestore(),
        &end_root,
        Some(b"\x10"),
        Some(b"\x90"),
        None,
    )
    .expect("should pass with correct applied trie");

    // Wrong: don't apply any batch ops → key_b is absent from applied trie.
    // The root's child at nibble 5 is None, but the proof shows a hash there.
    let wrong = apply_batch_ops(&start, &[]);
    let err = verify_change_proof(
        &proof,
        wrong.nodestore(),
        &end_root,
        Some(b"\x10"),
        Some(b"\x90"),
        None,
    )
    .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::InRangeChildMismatch { depth: 0 })),
        "got {err:?}"
    );
}

/// Case 2c (dual proofs): the attacker submits the legitimate boundary proofs
/// (which still hash correctly to `end_root`) but forges the batch ops so that
/// key_a is written with a wrong value.  The proof's start_proof leaf claims
/// `value_digest = Value(0x02)` (from the end trie), but the applied trie has
/// `key_a = 0xff` → `ProofNodeValueMismatch`.
///
/// This exercises the step-3 check that `verify_proof_node_value` catches value
/// discrepancies that the boundary-proof hash chain alone cannot detect.
///
/// Gated to non-ethhash: ethhash interprets values at account depths as
/// RLP-encoded structs; arbitrary byte values like 0xff may cause different
/// errors at that layer.
#[cfg(not(feature = "ethhash"))]
#[test]
fn test_reject_wrong_value_in_applied_trie() {
    // start: {key_a: 0x01, key_c: 0x09}
    // end:   {key_a: 0x02, key_c: 0x09}  (key_a value changed)
    let start = init_merkle([
        (b"\x10".as_ref(), b"\x01".as_ref()),
        (b"\x90".as_ref(), b"\x09".as_ref()),
    ]);
    let end = init_merkle([
        (b"\x10".as_ref(), b"\x02".as_ref()),
        (b"\x90".as_ref(), b"\x09".as_ref()),
    ]);
    let end_root = end.nodestore().root_hash().unwrap();

    // Legitimate proof: batch_ops = [Put key_a: 0x02].
    // The start_proof leaf at key_a records value_digest = Value(0x02),
    // consistent with end_root.
    let proof = end
        .change_proof(Some(b"\x10"), Some(b"\x90"), start.nodestore(), None)
        .expect("change_proof should succeed");

    // Verify the legitimate proof passes.
    let correct = apply_batch_ops(&start, proof.batch_ops());
    verify_change_proof(&proof, correct.nodestore(), &end_root, Some(b"\x10"), Some(b"\x90"), None)
        .expect("should pass with correct applied trie");

    // Attack: keep the boundary proofs unchanged (still hash to end_root) but
    // swap the batch op to write 0xff instead of 0x02.  The applied trie now has
    // key_a = 0xff, which doesn't match the proof's value_digest = Value(0x02).
    let forged_ops: Box<[BatchOp<Key, Value>]> =
        Box::new([BatchOp::Put { key: bkey(b"\x10"), value: bval(b"\xff") }]);
    let forged_proof = ChangeProof::new(
        proof.start_proof().clone(),
        proof.end_proof().clone(),
        forged_ops,
    );
    let wrong_applied = apply_batch_ops(&start, forged_proof.batch_ops());

    let err = verify_change_proof(
        &forged_proof,
        wrong_applied.nodestore(),
        &end_root,
        Some(b"\x10"),
        Some(b"\x90"),
        None,
    )
    .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::ProofNodeValueMismatch { .. })),
        "got {err:?}"
    );
}

/// Case 2c (dual proofs): XOR the least-significant byte of an interior batch
/// op key.  The first and last keys are untouched so the boundary proofs still
/// validate against `end_root`, but the tampered intermediate key places a leaf
/// at a different trie position → `InRangeChildMismatch`.
///
/// This validates the hash correctness of the in-range child check: even a
/// one-bit difference in an interior key is caught because the affected subtree
/// hash no longer matches what the proof recorded from the end trie.
#[cfg(not(feature = "ethhash"))]
#[test]
fn test_reject_tampered_interior_key() {
    // start: {key_a, key_e} — only the boundary keys from start's perspective.
    // end:   {key_a, key_b, key_c, key_d, key_e} — three interior keys added.
    // Proof range [key_a, key_e]; batch_ops = [Put key_b, Put key_c, Put key_d].
    let start = init_merkle([
        (b"\x10".as_ref(), b"\x01".as_ref()),
        (b"\x90".as_ref(), b"\x09".as_ref()),
    ]);
    let end = init_merkle([
        (b"\x10".as_ref(), b"\x01".as_ref()),
        (b"\x30".as_ref(), b"\x03".as_ref()),
        (b"\x50".as_ref(), b"\x05".as_ref()),
        (b"\x70".as_ref(), b"\x07".as_ref()),
        (b"\x90".as_ref(), b"\x09".as_ref()),
    ]);
    let end_root = end.nodestore().root_hash().unwrap();

    let proof = end
        .change_proof(Some(b"\x10"), Some(b"\x90"), start.nodestore(), None)
        .expect("change_proof should succeed");

    // Correct applied trie should pass.
    let correct = apply_batch_ops(&start, proof.batch_ops());
    verify_change_proof(&proof, correct.nodestore(), &end_root, Some(b"\x10"), Some(b"\x90"), None)
        .expect("should pass with correct applied trie");

    // Tamper: XOR the last byte of the middle key (0x50 → 0x51).
    // The first (0x30) and last (0x70) interior batch op keys are untouched,
    // so the proof's boundary proofs remain valid against end_root.  But the
    // applied trie now places a leaf at nibble path [5,1] instead of [5,0],
    // which produces a different child hash at root nibble 5.
    let mut tampered_ops: Vec<BatchOp<Key, Value>> = proof.batch_ops().to_vec();
    let mid = tampered_ops.len() / 2;
    if let BatchOp::Put { key, .. } = &mut tampered_ops[mid] {
        let last = key.len() - 1;
        let mut new_key = key.to_vec();
        new_key[last] ^= 0x01;
        *key = new_key.into_boxed_slice();
    }

    let wrong_applied = apply_batch_ops(&start, &tampered_ops);

    let err = verify_change_proof(
        &proof, // unchanged proof: hash chain still validates to end_root
        wrong_applied.nodestore(),
        &end_root,
        Some(b"\x10"),
        Some(b"\x90"),
        None,
    )
    .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::InRangeChildMismatch { depth: 0 })),
        "got {err:?}"
    );
}

#[cfg(not(feature = "ethhash"))]
#[test]
fn test_reject_removed_key_branch_trie() {
    // Build a 676-key trie with keys "aa".."zz" (4-level trie with branch nodes).
    let letters: Vec<u8> = (b'a'..=b'z').collect();
    let start_kvs: Vec<(Vec<u8>, Vec<u8>)> = letters
        .iter()
        .flat_map(|&c1| letters.iter().map(move |&c2| (vec![c1, c2], vec![1u8])))
        .collect();
    let start = init_merkle(start_kvs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())));

    // Alter all values from 1 to 2 to produce the end trie.
    let end_kvs: Vec<(Vec<u8>, Vec<u8>)> = start_kvs
        .iter()
        .map(|(k, _)| (k.clone(), vec![2u8]))
        .collect();
    let end = init_merkle(end_kvs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())));
    let end_root = end.nodestore().root_hash().unwrap();

    let proof = end
        .change_proof(Some(b"ba"), Some(b"yz"), start.nodestore(), None)
        .expect("change_proof should succeed");

    // Correct applied trie should verify.
    let correct = apply_batch_ops(&start, proof.batch_ops());
    verify_change_proof(&proof, correct.nodestore(), &end_root, Some(b"ba"), Some(b"yz"), None)
        .expect("should pass with correct applied trie");

    // Tamper: remove "bp" from the batch ops before applying.
    //
    // Why "bp" and not "mm"?  The start boundary is "ba" = nibbles [6,2,6,1].
    // At depth 1 (the [6] node) the boundary nibble is 2 ('b'), so in-range
    // children are those with nibble > 2.  "b?" keys sit at nibble 2 —
    // exactly on the boundary — so they are NOT in-range at depth 1, and a
    // missing "b?" key cannot fire a mismatch there.
    //
    // At depth 2 (the [6,2] node for all "b?" keys) the boundary nibble is 6
    // (the high nibble of 'a' = 0x61), so in-range children are those with
    // nibble > 6.  "bp" = [0x62, 0x70]: its third nibble is 7 > 6, putting it
    // firmly in-range at depth 2.  Removing "bp" therefore fires at depth 2.
    let tampered_ops: Vec<BatchOp<Key, Value>> = proof
        .batch_ops()
        .iter()
        .filter(|op| op.key().as_ref() != b"bp")
        .cloned()
        .collect();
    assert!(
        tampered_ops.len() < proof.batch_ops().len(),
        "key \"bp\" should be in the proof's batch ops for range [ba, yz]"
    );

    let wrong_applied = apply_batch_ops(&start, &tampered_ops);
    let err = verify_change_proof(
        &proof,
        wrong_applied.nodestore(),
        &end_root,
        Some(b"ba"),
        Some(b"yz"),
        None,
    )
    .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::InRangeChildMismatch { depth: 2 })),
        "got {err:?}"
    );
}

// ── Single-sided proof cases (Cases 2a and 2b) ───────────────────────────────
//
// These exercise code paths that the dual-proof tests above never reach:
//   Case 2a: start_key=None → only an end proof; `verify_in_range_children`
//            called directly with `Ordering::Less`.
//   Case 2b: end_key=None  → only a start proof; `verify_in_range_children`
//            called directly with `Ordering::Greater`.

/// Case 2a: start_key=None, only an end proof present.  Omitting the batch op
/// that adds `\x50` leaves the applied trie without a child at nibble 5.  The
/// end proof's root node expects that child (nibble 5 < end boundary nibble 9)
/// → `InRangeChildMismatch { depth: 0 }`.
#[cfg(not(feature = "ethhash"))]
#[test]
fn test_reject_case2a_omitted_op_end_proof_only() {
    let start = init_merkle([
        (b"\x10".as_ref(), b"\x01".as_ref()),
        (b"\x90".as_ref(), b"\x09".as_ref()),
    ]);
    let end = init_merkle([
        (b"\x10".as_ref(), b"\x01".as_ref()),
        (b"\x50".as_ref(), b"\x05".as_ref()),
        (b"\x90".as_ref(), b"\x09".as_ref()),
    ]);
    let end_root = end.nodestore().root_hash().unwrap();

    let proof = end
        .change_proof(None::<&[u8]>, Some(b"\x90"), start.nodestore(), None)
        .expect("change_proof should succeed");

    let correct = apply_batch_ops(&start, proof.batch_ops());
    verify_change_proof(&proof, correct.nodestore(), &end_root, None, Some(b"\x90"), None)
        .expect("should pass with correct applied trie");

    let wrong = apply_batch_ops(&start, &[]);
    let err =
        verify_change_proof(&proof, wrong.nodestore(), &end_root, None, Some(b"\x90"), None)
            .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::InRangeChildMismatch { depth: 0 })),
        "got {err:?}"
    );
}

/// Case 2b: end_key=None, only a start proof present.  Omitting the batch op
/// that adds `\x50` leaves the applied trie without a child at nibble 5.  The
/// start proof's root node expects that child (nibble 5 > start boundary nibble 1)
/// → `InRangeChildMismatch { depth: 0 }`.
#[cfg(not(feature = "ethhash"))]
#[test]
fn test_reject_case2b_omitted_op_start_proof_only() {
    let start = init_merkle([
        (b"\x10".as_ref(), b"\x01".as_ref()),
        (b"\x90".as_ref(), b"\x09".as_ref()),
    ]);
    let end = init_merkle([
        (b"\x10".as_ref(), b"\x01".as_ref()),
        (b"\x50".as_ref(), b"\x05".as_ref()),
        (b"\x90".as_ref(), b"\x09".as_ref()),
    ]);
    let end_root = end.nodestore().root_hash().unwrap();

    let proof = end
        .change_proof(Some(b"\x10"), None::<&[u8]>, start.nodestore(), None)
        .expect("change_proof should succeed");

    let correct = apply_batch_ops(&start, proof.batch_ops());
    verify_change_proof(&proof, correct.nodestore(), &end_root, Some(b"\x10"), None, None)
        .expect("should pass with correct applied trie");

    let wrong = apply_batch_ops(&start, &[]);
    let err =
        verify_change_proof(&proof, wrong.nodestore(), &end_root, Some(b"\x10"), None, None)
            .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::InRangeChildMismatch { depth: 0 })),
        "got {err:?}"
    );
}

/// Case 2c: attacker injects a spurious Put into the batch ops for a key that
/// was never in the end trie.  The boundary proof nodes reflect an end trie
/// with no child at nibble 5, but the applied trie gains one →
/// `InRangeChildMismatch { depth: 0 }`.
#[cfg(not(feature = "ethhash"))]
#[test]
fn test_reject_spurious_batch_op_in_range() {
    let start = init_merkle([
        (b"\x10".as_ref(), b"\x01".as_ref()),
        (b"\x90".as_ref(), b"\x09".as_ref()),
    ]);
    let end = init_merkle([
        (b"\x10".as_ref(), b"\x02".as_ref()), // \x10 changed
        (b"\x90".as_ref(), b"\x09".as_ref()),
    ]);
    let end_root = end.nodestore().root_hash().unwrap();

    let proof = end
        .change_proof(Some(b"\x10"), Some(b"\x90"), start.nodestore(), None)
        .expect("change_proof should succeed");

    let correct = apply_batch_ops(&start, proof.batch_ops());
    verify_change_proof(&proof, correct.nodestore(), &end_root, Some(b"\x10"), Some(b"\x90"), None)
        .expect("should pass with correct applied trie");

    // Inject a spurious Put for \x50 (absent from the end trie).
    // \x10 < \x50 so appending preserves sorted order.
    let mut spurious_ops: Vec<BatchOp<Key, Value>> = proof.batch_ops().to_vec();
    spurious_ops.push(BatchOp::Put { key: bkey(b"\x50"), value: bval(b"\x05") });

    let wrong = apply_batch_ops(&start, &spurious_ops);
    let err = verify_change_proof(
        &proof,
        wrong.nodestore(),
        &end_root,
        Some(b"\x10"),
        Some(b"\x90"),
        None,
    )
    .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::InRangeChildMismatch { depth: 0 })),
        "got {err:?}"
    );
}

/// Case 2c: attacker omits a Delete from the batch ops.  `\x50` was deleted in
/// the end trie; without the Delete the applied trie still has `\x50`, giving
/// it a child at nibble 5 that the proof's boundary nodes don't expect →
/// `InRangeChildMismatch { depth: 0 }`.
#[cfg(not(feature = "ethhash"))]
#[test]
fn test_reject_missing_delete_in_range() {
    let start = init_merkle([
        (b"\x10".as_ref(), b"\x01".as_ref()),
        (b"\x50".as_ref(), b"\x05".as_ref()),
        (b"\x90".as_ref(), b"\x09".as_ref()),
    ]);
    let end = init_merkle([
        (b"\x10".as_ref(), b"\x01".as_ref()),
        (b"\x90".as_ref(), b"\x09".as_ref()), // \x50 deleted
    ]);
    let end_root = end.nodestore().root_hash().unwrap();

    let proof = end
        .change_proof(Some(b"\x10"), Some(b"\x90"), start.nodestore(), None)
        .expect("change_proof should succeed");

    let correct = apply_batch_ops(&start, proof.batch_ops());
    verify_change_proof(&proof, correct.nodestore(), &end_root, Some(b"\x10"), Some(b"\x90"), None)
        .expect("should pass with correct applied trie");

    // Strip the Delete op — applied trie retains \x50.
    let tampered_ops: Vec<BatchOp<Key, Value>> = proof
        .batch_ops()
        .iter()
        .filter(|op| !matches!(op, BatchOp::Delete { .. }))
        .cloned()
        .collect();
    assert!(
        tampered_ops.len() < proof.batch_ops().len(),
        "Delete for \\x50 should be in batch ops"
    );

    let wrong = apply_batch_ops(&start, &tampered_ops);
    let err = verify_change_proof(
        &proof,
        wrong.nodestore(),
        &end_root,
        Some(b"\x10"),
        Some(b"\x90"),
        None,
    )
    .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::InRangeChildMismatch { depth: 0 })),
        "got {err:?}"
    );
}

/// Case 2c with 676-key "aa".."zz" trie: exercises the end-proof tail
/// (`Ordering::Less` path in `verify_dual_proofs`).
///
/// "xr" = [0x78, 0x72], nibbles [7,8,7,2].  The end proof's tail starts at
/// the [7] node (depth 1).  The end boundary nibble at depth 1 is 9
/// (from "yz" = [7,9,7,a]).  Nibble 8 < 9 → in-range for `Less` →
/// `InRangeChildMismatch { depth: 1 }`.
#[cfg(not(feature = "ethhash"))]
#[test]
fn test_reject_removed_key_end_proof_tail() {
    let letters: Vec<u8> = (b'a'..=b'z').collect();
    let start_kvs: Vec<(Vec<u8>, Vec<u8>)> = letters
        .iter()
        .flat_map(|&c1| letters.iter().map(move |&c2| (vec![c1, c2], vec![1u8])))
        .collect();
    let start = init_merkle(start_kvs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())));

    let end_kvs: Vec<(Vec<u8>, Vec<u8>)> =
        start_kvs.iter().map(|(k, _)| (k.clone(), vec![2u8])).collect();
    let end = init_merkle(end_kvs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())));
    let end_root = end.nodestore().root_hash().unwrap();

    let proof = end
        .change_proof(Some(b"ba"), Some(b"yz"), start.nodestore(), None)
        .expect("change_proof should succeed");

    let correct = apply_batch_ops(&start, proof.batch_ops());
    verify_change_proof(&proof, correct.nodestore(), &end_root, Some(b"ba"), Some(b"yz"), None)
        .expect("should pass with correct applied trie");

    let tampered_ops: Vec<BatchOp<Key, Value>> = proof
        .batch_ops()
        .iter()
        .filter(|op| op.key().as_ref() != b"xr")
        .cloned()
        .collect();
    assert!(
        tampered_ops.len() < proof.batch_ops().len(),
        "key \"xr\" should be in the proof's batch ops for range [ba, yz]"
    );

    let wrong = apply_batch_ops(&start, &tampered_ops);
    let err = verify_change_proof(
        &proof,
        wrong.nodestore(),
        &end_root,
        Some(b"ba"),
        Some(b"yz"),
        None,
    )
    .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::InRangeChildMismatch { depth: 1 })),
        "got {err:?}"
    );
}

/// Case 2b: attacker provides a valid start proof (genuinely hashes to
/// `end_root`) but fabricates the batch ops — injecting a key with a wrong
/// value.  The applied trie's in-range child hashes must match those recorded
/// in the start proof's nodes; a bogus value changes the subtree hash all the
/// way up → `InRangeChildMismatch`.
#[cfg(not(feature = "ethhash"))]
#[test]
fn test_reject_case2b_bogus_batch_ops() {
    let start = init_merkle([
        (b"\x10".as_ref(), b"\x01".as_ref()),
        (b"\x90".as_ref(), b"\x09".as_ref()),
    ]);
    let end = init_merkle([
        (b"\x10".as_ref(), b"\x01".as_ref()),
        (b"\x50".as_ref(), b"\x05".as_ref()),
        (b"\x90".as_ref(), b"\x09".as_ref()),
    ]);
    let end_root = end.nodestore().root_hash().unwrap();

    // Legitimate proof (start_key=Some, end_key=None → Case 2b, empty end proof).
    let proof = end
        .change_proof(Some(b"\x10"), None::<&[u8]>, start.nodestore(), None)
        .expect("change_proof should succeed");

    // Sanity: correct application passes.
    let correct = apply_batch_ops(&start, proof.batch_ops());
    verify_change_proof(&proof, correct.nodestore(), &end_root, Some(b"\x10"), None, None)
        .expect("should pass with correct applied trie");

    // Attack: keep the valid start proof but replace the batch op value for
    // \x50 with a fabricated 0xff.  The start proof's child hash at nibble 5
    // reflects the end trie (value 0x05); the applied trie hashes to something
    // different → InRangeChildMismatch.
    let bogus_ops: Vec<BatchOp<Key, Value>> = proof
        .batch_ops()
        .iter()
        .map(|op| {
            if op.key().as_ref() == b"\x50" {
                BatchOp::Put { key: bkey(b"\x50"), value: bval(b"\xff") }
            } else {
                op.clone()
            }
        })
        .collect();
    let wrong = apply_batch_ops(&start, &bogus_ops);
    let err =
        verify_change_proof(&proof, wrong.nodestore(), &end_root, Some(b"\x10"), None, None)
            .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::InRangeChildMismatch { depth: 0 })),
        "got {err:?}"
    );
}

/// Demonstrates that Case 2a (end proof only, no start proof, `start_key=None`)
/// catches tampered batch op values even without a left-edge proof.
///
/// The end boundary "zaa" is a 3-byte key that does not exist in the trie
/// (all keys are 2 bytes), so the end proof is an exclusion proof.
///
/// Setup: 52-key trie ("aa".."az" and "za".."zz").  In R2, only "ac" and "za"
/// change from V1 to V2; all other keys remain V1.  A dual-proof (Case 2c) for
/// the range ["abc", "zaa"] is generated and the start proof is stripped → Case 2a.
///
/// R2 is dropped after proof generation so the verifier works from R1 + batch_ops.
///
/// The end proof path to "zaa" has end_bn = 7 at depth 0.
/// In-range for `Less` = nibble < 7, which covers nibble 6 — the entire "a?"
/// subtree.  Tampering "ac" (nibble path [6,1,6,3]) changes the hash at nibble 6
/// of the root → `InRangeChildMismatch { depth: 0 }`.
#[cfg(not(feature = "ethhash"))]
#[test]
fn test_case2a_no_start_proof_tamper_detects() {
    let letters: Vec<u8> = (b'a'..=b'z').collect();

    // R1: keys "aa".."az" and "za".."zz", all value "V1".
    let r1_kvs: Vec<(Vec<u8>, Vec<u8>)> = [b'a', b'z']
        .iter()
        .flat_map(|&c1| letters.iter().map(move |&c2| (vec![c1, c2], b"V1".to_vec())))
        .collect();
    let r1 = init_merkle(r1_kvs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())));

    // R2: only "ac" and "za" change to V2; all others remain V1.
    // Leaving the rest at V1 ensures the end proof's in-range hashes (nibble 6
    // at depth 0 = entire "a?" subtree) match the applied trie for the valid case.
    let r2_kvs: Vec<(Vec<u8>, Vec<u8>)> = r1_kvs
        .iter()
        .map(|(k, _)| {
            let v = if k.as_slice() == b"ac" || k.as_slice() == b"za" {
                b"V2".to_vec()
            } else {
                b"V1".to_vec()
            };
            (k.clone(), v)
        })
        .collect();
    let r2 = init_merkle(r2_kvs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())));
    let end_root = r2.nodestore().root_hash().unwrap();

    // Get the legitimate dual-proof (Case 2c) for the range ["abc", "zaa"].
    // Both boundaries are 3-byte keys absent from the 2-byte-key trie → exclusion proofs.
    let proof = r2
        .change_proof(Some(b"abc"), Some(b"zaa"), r1.nodestore(), None)
        .expect("change_proof should succeed");
    drop(r2); // Drop R2 — verifier must not inspect it.

    // Strip the start proof → Case 2a (end proof only, start_key=None).
    let no_start = ChangeProof::new(
        empty_proof(),
        proof.end_proof().clone(),
        proof.batch_ops().to_vec().into_boxed_slice(),
    );

    // First verification: correct batch ops → must pass.
    let correct = apply_batch_ops(&r1, no_start.batch_ops());
    verify_change_proof(
        &no_start,
        correct.nodestore(),
        &end_root,
        None,
        Some(b"zaa"),
        None,
    )
    .expect("first verification should pass with correct batch ops");

    // Second verification: tamper "ac" value from V2 to V3.
    // "ac" = [0x61, 0x63] sits at root nibble 6, which is strictly less than
    // end boundary nibble 7 ("zaa" high nibble), so the end proof's in-range
    // check covers it → InRangeChildMismatch.
    let tampered_ops: Box<[BatchOp<Key, Value>]> = proof
        .batch_ops()
        .iter()
        .map(|op| {
            if op.key().as_ref() == b"ac" {
                BatchOp::Put { key: bkey(b"ac"), value: bval(b"V3") }
            } else {
                op.clone()
            }
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    assert!(
        tampered_ops.len() == proof.batch_ops().len(),
        "\"ac\" should be in batch ops for range [abc, zaa]"
    );
    let tampered = ChangeProof::new(
        empty_proof(),
        proof.end_proof().clone(),
        tampered_ops,
    );
    let wrong = apply_batch_ops(&r1, tampered.batch_ops());
    let err = verify_change_proof(
        &tampered,
        wrong.nodestore(),
        &end_root,
        None,
        Some(b"zaa"),
        None,
    )
    .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::InRangeChildMismatch { .. })),
        "got {err:?}"
    );
}

/// Demonstrates that Case 2b (start proof only, no end proof, `end_key=None`)
/// catches tampered batch op values even without a right-edge proof.
///
/// The start boundary "abc" is a 3-byte key that does not exist in the trie
/// (all keys are 2 bytes), so the start proof is an exclusion proof.
///
/// Setup: 52-key trie ("aa".."az" and "za".."zz").
/// A change proof for the range ["abc", "zaa"] is generated (Case 2c, dual proofs),
/// then the end proof is stripped to convert it to Case 2b.
///
/// The R2 trie is dropped after proof generation so the verifier cannot inspect
/// R2 state directly — it must work from R1 + batch_ops alone.
///
/// First verification (correct batch ops) must pass.
/// Second verification ("za" value changed from V2 to V3) must fail with
/// `InRangeChildMismatch`: "za" = [0x7a, 0x61] sits at root nibble 7, which is
/// strictly greater than the start boundary nibble 6 ("abc"[0] high nibble), so
/// the start proof's in-range child check covers it.
#[cfg(not(feature = "ethhash"))]
#[test]
fn test_case2b_no_end_proof_tamper_detects() {
    let letters: Vec<u8> = (b'a'..=b'z').collect();

    // R1: keys "aa".."az" and "za".."zz", all value "V1".
    let r1_kvs: Vec<(Vec<u8>, Vec<u8>)> = [b'a', b'z']
        .iter()
        .flat_map(|&c1| letters.iter().map(move |&c2| (vec![c1, c2], b"V1".to_vec())))
        .collect();
    let r1 = init_merkle(r1_kvs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())));

    // R2: change "ac".."az" to V2 and "za" to V2; leave everything else at V1.
    // - "aa" and "ab" are below the range start "abc" and appear on the start
    //   proof path; they must stay at V1 so verify_proof_node_value passes.
    // - "zb".."zz" are above the range end "zaa" and covered by the start
    //   proof's Greater check; they must stay at V1 so the subtree hash matches
    //   the applied trie after stripping the end proof.
    let r2_kvs: Vec<(Vec<u8>, Vec<u8>)> = r1_kvs
        .iter()
        .map(|(k, _)| {
            let v = if (k.as_slice() >= b"ac".as_ref() && k.as_slice() <= b"az".as_ref())
                || k.as_slice() == b"za"
            {
                b"V2".to_vec()
            } else {
                b"V1".to_vec()
            };
            (k.clone(), v)
        })
        .collect();
    let r2 = init_merkle(r2_kvs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())));
    let end_root = r2.nodestore().root_hash().unwrap();

    // Get the legitimate dual-proof (Case 2c) for the range ["abc", "zaa"].
    // Both boundaries are 3-byte keys absent from the 2-byte-key trie → exclusion proofs.
    let proof = r2
        .change_proof(Some(b"abc"), Some(b"zaa"), r1.nodestore(), None)
        .expect("change_proof should succeed");
    drop(r2); // Drop R2 — verifier must not inspect it.

    // Strip the end proof → Case 2b (start proof only, end_key=None).
    let no_end = ChangeProof::new(
        proof.start_proof().clone(),
        empty_proof(),
        proof.batch_ops().to_vec().into_boxed_slice(),
    );

    // First verification: correct batch ops → must pass.
    let correct = apply_batch_ops(&r1, no_end.batch_ops());
    verify_change_proof(
        &no_end,
        correct.nodestore(),
        &end_root,
        Some(b"abc"),
        None,
        None,
    )
    .expect("first verification should pass with correct batch ops");

    // Second verification: tamper "za" value from V2 to V3.
    // Changing the value alters the hash propagating up to root nibble 7,
    // which is in-range (> boundary nibble 6) → InRangeChildMismatch.
    let tampered_ops: Box<[BatchOp<Key, Value>]> = proof
        .batch_ops()
        .iter()
        .map(|op| {
            if op.key().as_ref() == b"za" {
                BatchOp::Put { key: bkey(b"za"), value: bval(b"V3") }
            } else {
                op.clone()
            }
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    assert!(
        tampered_ops.len() == proof.batch_ops().len(),
        "\"za\" should be in batch ops for range [abc, zaa]"
    );
    let tampered = ChangeProof::new(
        proof.start_proof().clone(),
        empty_proof(),
        tampered_ops,
    );
    let wrong = apply_batch_ops(&r1, tampered.batch_ops());
    let err = verify_change_proof(
        &tampered,
        wrong.nodestore(),
        &end_root,
        Some(b"abc"),
        None,
        None,
    )
    .unwrap_err();
    assert!(
        matches!(err, api::Error::ProofError(ProofError::InRangeChildMismatch { .. })),
        "got {err:?}"
    );
}

/// Simulates an iterative sync from R1 to R2 using random-range change proofs,
/// verifying that proofs remain valid when applied to a partially-synced state.
///
/// The trie contains 52 two-byte keys: "aa".."az" (26 keys) and "za".."zz"
/// (26 keys).  R1 has all values set to "V1"; R2 has all values set to "V2".
///
/// Each round:
/// 1. Pick a random [start_key, end_key] from the 52-key set.
/// 2. Get a change proof from R2, always using R1 as the declared start state.
/// 3. Verify the proof twice:
///    - Applied to R1: baseline check that the proof is well-formed.
///    - Applied to `current`: checks that the same proof is also valid when
///      applied on top of a partially-synced intermediate state.
/// 4. `current` is updated to the result of applying the proof to `current`.
///
/// Because a hashed `Merkle` is immutable after finalisation, each round
/// rebuilds `current` from scratch: all existing key-value pairs are collected
/// via iteration, the batch ops are applied (puts update, deletes remove), and
/// a fresh trie is constructed from the resulting sorted list.
///
/// Repeat until `current`'s root hash equals R2's root hash.
#[cfg(not(feature = "ethhash"))]
#[test]
fn test_iterative_sync_converges() {
    let rng = firewood_storage::SeededRng::from_env_or_random();

    // 52 two-byte keys: "aa".."az" and "za".."zz".
    let letters: Vec<u8> = (b'a'..=b'z').collect();
    let all_keys: Vec<Vec<u8>> = [b'a', b'z']
        .iter()
        .flat_map(|&c1| letters.iter().map(move |&c2| vec![c1, c2]))
        .collect();

    let r1_kvs: Vec<(Vec<u8>, Vec<u8>)> =
        all_keys.iter().map(|k| (k.clone(), b"V1".to_vec())).collect();
    let r2_kvs: Vec<(Vec<u8>, Vec<u8>)> =
        all_keys.iter().map(|k| (k.clone(), b"V2".to_vec())).collect();

    let r1 = init_merkle(r1_kvs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())));
    let mut current = init_merkle(r1_kvs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())));
    let r2 = init_merkle(r2_kvs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())));
    let end_root = r2.nodestore().root_hash().expect("R2 must have a root");

    for round in 0.. {
        if current.nodestore().root_hash().as_ref() == Some(&end_root) {
            break;
        }
        // 10,000 rounds makes non-convergence ~10^-85 likely (vs. ~10^-77 for a
        // 256-bit hash collision), so this fuse should never fire in practice.
        assert!(round < 10_000, "sync did not converge after 10,000 rounds");

        // Pick random start/end keys (distinct, sorted).
        let si = rng.random_range(0..all_keys.len() - 1);
        let ei = rng.random_range(si + 1..all_keys.len());
        let start_key = &all_keys[si];
        let end_key = &all_keys[ei];

        // Always generate the proof using R1 as the declared start state.
        // This is what a real sync peer would provide.
        let proof = r2
            .change_proof(
                Some(start_key.as_slice()),
                Some(end_key.as_slice()),
                r1.nodestore(),
                None,
            )
            .expect("change_proof should succeed");

        // Build a new trie by applying batch_ops on top of `base`.
        // Because a finalised Merkle is immutable, we reconstruct by iterating
        // all existing keys, dropping those overridden by the ops, then
        // inserting the op results.
        let apply = |base: &Merkle<NodeStore<Committed, MemStore>>| {
            let batch_map: std::collections::HashMap<Vec<u8>, Option<Vec<u8>>> = proof
                .batch_ops()
                .iter()
                .map(|op| match op {
                    BatchOp::Put { key, value } => {
                        (key.as_ref().to_vec(), Some(value.as_ref().to_vec()))
                    }
                    BatchOp::Delete { key } => (key.as_ref().to_vec(), None),
                    BatchOp::DeleteRange { .. } => panic!("unexpected DeleteRange"),
                })
                .collect();
            let mut kvs: Vec<(Vec<u8>, Vec<u8>)> = base
                .key_value_iter_from_key(b"")
                .map(|r| r.expect("iteration should succeed"))
                .filter_map(|(k, v)| {
                    if batch_map.contains_key(k.as_ref()) {
                        None // overridden or deleted below
                    } else {
                        Some((k.as_ref().to_vec(), v.as_ref().to_vec()))
                    }
                })
                .collect();
            for (k, maybe_v) in &batch_map {
                if let Some(v) = maybe_v {
                    kvs.push((k.clone(), v.clone()));
                }
            }
            kvs.sort_unstable();
            init_merkle(kvs.iter().map(|(k, v)| (k.as_slice(), v.as_slice())))
        };

        // Verification 1: batch_ops applied to R1 — baseline correctness check.
        let applied_r1 = apply(&r1);
        verify_change_proof(
            &proof,
            applied_r1.nodestore(),
            &end_root,
            Some(start_key.as_slice()),
            Some(end_key.as_slice()),
            None,
        )
        .expect("verify against R2 (R1-based) should pass");

        // Verification 2: same proof applied to `current` (partially synced state).
        // Keys already updated to V2 in prior rounds are idempotently re-applied;
        // keys outside this range retain whatever value `current` holds, which
        // must be consistent with the proof's out-of-range child hashes from R1.
        let r3 = apply(&current);
        verify_change_proof(
            &proof,
            r3.nodestore(),
            &end_root,
            Some(start_key.as_slice()),
            Some(end_key.as_slice()),
            None,
        )
        .expect("verify against R2 (current-based) should pass");

        current = r3;
    }
}
