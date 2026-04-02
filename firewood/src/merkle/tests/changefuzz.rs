// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Fuzz-style test: generates a random start trie, applies random changes to
//! produce an end trie, generates a change proof between them, then verifies
//! the proof under five scenarios:
//!
//! 1. Both boundary keys are existing keys
//! 2. Start boundary is a non-existent (decreased) key
//! 3. End boundary is a non-existent (increased) key
//! 4. Both boundaries are non-existent
//! 5. No bounds (complete proof)
//!
//! Runs 100 independent iterations, each with a freshly seeded RNG. On failure,
//! the printed seed can be passed via `FIREWOOD_TEST_SEED` to reproduce.

use super::*;
use crate::api::{self, BatchOp, Db as DbTrait, Proposal as _};
use crate::db::{Db, DbConfig};
use crate::merkle::verify_change_proof_root_hash;
use crate::{ChangeProofVerificationContext, verify_change_proof_structure};

/// Verify a change proof end-to-end: structural check + root hash check.
fn verify_and_check(
    db: &Db,
    proof: &api::FrozenChangeProof,
    verification: &ChangeProofVerificationContext,
    start_root: api::HashKey,
) -> Result<(), api::Error> {
    let parent = db.revision(start_root)?;
    let proposal = db.apply_change_proof_to_parent(proof, &*parent)?;
    verify_change_proof_root_hash(proof, verification, &proposal)
}

#[test]
#[expect(clippy::too_many_lines)]
fn test_slow_change_proof_fuzz() {
    let outer_rng = firewood_storage::SeededRng::from_env_or_random();

    for run in 0..25 {
        let seed = outer_rng.next_u64();
        eprintln!("run {run}: seed={seed} (export FIREWOOD_TEST_SEED={seed} to reproduce)");
        let rng = firewood_storage::SeededRng::new(seed);

        // Build the start trie from 64-2048 random keys.
        let key_count = rng.random_range(64..=2048_u32);
        let start_data = fixed_and_pseudorandom_data(&rng, key_count);
        let mut start_keys: Vec<[u8; 32]> = start_data.keys().copied().collect();
        start_keys.sort_unstable();

        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path(), DbConfig::builder().build()).unwrap();

        // Commit the start revision.
        let start_batch: Vec<BatchOp<&[u8], &[u8]>> = start_data
            .iter()
            .map(|(k, v)| BatchOp::Put {
                key: k.as_ref(),
                value: v.as_ref(),
            })
            .collect();
        db.propose(start_batch).unwrap().commit().unwrap();
        let root1 = db.root_hash().unwrap();

        // Build the end trie: delete ~15% of keys, insert ~15% new keys.
        let delete_step = (start_keys.len() / 7).max(1);
        let mut end_batch: Vec<BatchOp<&[u8], &[u8]>> = Vec::new();

        // Delete every delete_step-th key.
        let mut deleted_indices = Vec::new();
        for i in (0..start_keys.len()).step_by(delete_step + 1) {
            end_batch.push(BatchOp::Delete {
                key: start_keys[i].as_ref(),
            });
            deleted_indices.push(i);
        }

        // Generate new random key-value pairs (store owned so borrows live long enough).
        let insert_count = rng.random_range(10..=50_u32);
        let new_kvs: Vec<([u8; 32], [u8; 20])> = (0..insert_count)
            .map(|_| (rng.random::<[u8; 32]>(), rng.random::<[u8; 20]>()))
            .collect();
        let new_keys: Vec<[u8; 32]> = new_kvs.iter().map(|(k, _)| *k).collect();
        for (key, val) in &new_kvs {
            end_batch.push(BatchOp::Put {
                key: key.as_ref(),
                value: val.as_ref(),
            });
        }

        db.propose(end_batch).unwrap().commit().unwrap();
        let root2 = db.root_hash().unwrap();

        // Build the list of keys that exist in the end state.
        let mut end_keys: Vec<[u8; 32]> = start_keys
            .iter()
            .enumerate()
            .filter(|(i, _)| !deleted_indices.contains(i))
            .map(|(_, k)| *k)
            .chain(new_keys.iter().copied())
            .collect();
        end_keys.sort_unstable();
        end_keys.dedup();

        // Run 50 random verification scenarios with weighted selection.
        // Scenarios 0-3 pick random boundary keys, so they benefit from
        // repetition. Scenario 4 (no bounds) is deterministic for a given
        // trie pair, so we give it low weight.
        for _ in 0..50 {
            let scenario = rng.random_range(0..100_u32);
            match scenario {
                // 36% — both boundaries are existing end-state keys.
                0..36 => {
                    if end_keys.len() < 2 {
                        continue;
                    }
                    let si = rng.random_range(0..end_keys.len() - 1);
                    let ei = rng.random_range(si + 1..end_keys.len());
                    let start_key = &end_keys[si];
                    let end_key = &end_keys[ei];

                    let proof = db
                        .change_proof(
                            root1.clone(),
                            root2.clone(),
                            Some(start_key.as_ref()),
                            Some(end_key.as_ref()),
                            None,
                        )
                        .expect("change_proof should succeed");

                    let ctx = verify_change_proof_structure(
                        &proof,
                        root2.clone(),
                        Some(start_key.as_ref()),
                        Some(end_key.as_ref()),
                        None,
                    )
                    .expect("structural check should pass (scenario 0)");

                    verify_and_check(&db, &proof, &ctx, root1.clone())
                        .expect("verify should pass (scenario 0)");
                }

                // 20% — start boundary is a non-existent (decreased) key.
                36..56 => {
                    if end_keys.len() < 2 {
                        continue;
                    }
                    let si = rng.random_range(1..end_keys.len() - 1);
                    let ei = rng.random_range(si..end_keys.len());
                    let decreased = decrease_key(&end_keys[si]);
                    // Skip if decreased collides with the previous key.
                    if decreased >= end_keys[si] || (si > 0 && decreased == end_keys[si - 1]) {
                        continue;
                    }
                    let end_key = &end_keys[ei];

                    let proof = db
                        .change_proof(
                            root1.clone(),
                            root2.clone(),
                            Some(decreased.as_ref()),
                            Some(end_key.as_ref()),
                            None,
                        )
                        .expect("change_proof should succeed");

                    let ctx = verify_change_proof_structure(
                        &proof,
                        root2.clone(),
                        Some(decreased.as_ref()),
                        Some(end_key.as_ref()),
                        None,
                    )
                    .expect("structural check should pass (scenario 1)");

                    verify_and_check(&db, &proof, &ctx, root1.clone())
                        .expect("verify should pass (scenario 1)");
                }

                // 20% — end boundary is a non-existent (increased) key.
                56..76 => {
                    if end_keys.len() < 2 {
                        continue;
                    }
                    let si = rng.random_range(0..end_keys.len() - 1);
                    let ei = rng.random_range(si..end_keys.len() - 1);
                    let increased = increase_key(&end_keys[ei]);
                    // Skip if increased collides with the next key or overflows.
                    if increased <= end_keys[ei]
                        || (ei + 1 < end_keys.len() && increased == end_keys[ei + 1])
                    {
                        continue;
                    }
                    let start_key = &end_keys[si];

                    let proof = db
                        .change_proof(
                            root1.clone(),
                            root2.clone(),
                            Some(start_key.as_ref()),
                            Some(increased.as_ref()),
                            None,
                        )
                        .expect("change_proof should succeed");

                    let ctx = verify_change_proof_structure(
                        &proof,
                        root2.clone(),
                        Some(start_key.as_ref()),
                        Some(increased.as_ref()),
                        None,
                    )
                    .expect("structural check should pass (scenario 2)");

                    verify_and_check(&db, &proof, &ctx, root1.clone())
                        .expect("verify should pass (scenario 2)");
                }

                // 20% — both boundaries are non-existent keys.
                76..96 => {
                    if end_keys.len() < 2 {
                        continue;
                    }
                    let si = rng.random_range(1..end_keys.len() - 1);
                    let ei = rng.random_range(si..end_keys.len() - 1);
                    let decreased = decrease_key(&end_keys[si]);
                    let increased = increase_key(&end_keys[ei]);
                    if decreased >= end_keys[si]
                        || (si > 0 && decreased == end_keys[si - 1])
                        || increased <= end_keys[ei]
                        || (ei + 1 < end_keys.len() && increased == end_keys[ei + 1])
                    {
                        continue;
                    }

                    let proof = db
                        .change_proof(
                            root1.clone(),
                            root2.clone(),
                            Some(decreased.as_ref()),
                            Some(increased.as_ref()),
                            None,
                        )
                        .expect("change_proof should succeed");

                    let ctx = verify_change_proof_structure(
                        &proof,
                        root2.clone(),
                        Some(decreased.as_ref()),
                        Some(increased.as_ref()),
                        None,
                    )
                    .expect("structural check should pass (scenario 3)");

                    verify_and_check(&db, &proof, &ctx, root1.clone())
                        .expect("verify should pass (scenario 3)");
                }

                // 4% — no bounds, complete proof.
                _ => {
                    let proof = db
                        .change_proof(root1.clone(), root2.clone(), None, None, None)
                        .expect("change_proof should succeed");

                    let ctx =
                        verify_change_proof_structure(&proof, root2.clone(), None, None, None)
                            .expect("structural check should pass (scenario 4)");

                    verify_and_check(&db, &proof, &ctx, root1.clone())
                        .expect("verify should pass (scenario 4)");
                }
            }
        }
    }
}

/// Increment a variable-length key by 1. Returns `None` on overflow (all 0xFF).
fn increase_key_vec(key: &[u8]) -> Option<Vec<u8>> {
    let mut new_key = key.to_vec();
    for ch in new_key.iter_mut().rev() {
        let overflow;
        (*ch, overflow) = ch.overflowing_add(1);
        if !overflow {
            return Some(new_key);
        }
    }
    None
}

/// Decrement a variable-length key by 1. Returns `None` on underflow (all 0x00).
fn decrease_key_vec(key: &[u8]) -> Option<Vec<u8>> {
    let mut new_key = key.to_vec();
    for ch in new_key.iter_mut().rev() {
        let overflow;
        (*ch, overflow) = ch.overflowing_sub(1);
        if !overflow {
            return Some(new_key);
        }
    }
    None
}

/// Fuzz test with variable-length keys (1-4096 bytes). Short keys create
/// shallow trie structures where:
/// - Out-of-range value changes at start proof nodes are more likely
///   (exercises the `packed < sk` value-check skip)
/// - Prefix key relationships exist (key `b"\x10"` coexisting with
///   `b"\x10\x50"`), creating branch nodes with values
/// - Exclusion proofs with divergent children are common (exercises the
///   `PathIterator` divergent child fix and `ExclusionProofMissingChild`
///   check in `value_digest`)
#[test]
#[expect(clippy::too_many_lines)]
fn test_slow_change_proof_fuzz_varlen() {
    // When FIREWOOD_TEST_SEED is set, run only that single seed (for
    // reproducing CI failures). Otherwise, run 100 random iterations.
    let seeds: Vec<u64> = if let Ok(s) = std::env::var("FIREWOOD_TEST_SEED") {
        vec![s.parse().expect("FIREWOOD_TEST_SEED must be a u64")]
    } else {
        let outer_rng = firewood_storage::SeededRng::from_random();
        (0..25).map(|_| outer_rng.next_u64()).collect()
    };

    for (run, &seed) in seeds.iter().enumerate() {
        eprintln!("run {run}: seed={seed} (export FIREWOOD_TEST_SEED={seed} to reproduce)");
        let rng = firewood_storage::SeededRng::new(seed);

        // Build the start trie from variable-length random keys.
        let key_count = rng.random_range(16..=512_u32) as usize;
        let start_kvs = generate_random_kvs(&rng, key_count);
        let mut start_keys: Vec<Vec<u8>> = start_kvs.iter().map(|(k, _)| k.clone()).collect();
        start_keys.sort();
        start_keys.dedup();

        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path(), DbConfig::builder().build()).unwrap();

        // Commit the start revision.
        let start_batch: Vec<BatchOp<&[u8], &[u8]>> = start_kvs
            .iter()
            .map(|(k, v)| BatchOp::Put {
                key: k.as_ref(),
                value: v.as_ref(),
            })
            .collect();
        db.propose(start_batch).unwrap().commit().unwrap();
        let root1 = db.root_hash().unwrap();

        // Build the end trie: delete ~15% of keys, insert ~15% new keys.
        let delete_step = (start_keys.len() / 7).max(1);
        let mut end_batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> = Vec::new();

        let mut deleted_indices = Vec::new();
        for i in (0..start_keys.len()).step_by(delete_step + 1) {
            end_batch.push(BatchOp::Delete {
                key: start_keys[i].clone(),
            });
            deleted_indices.push(i);
        }

        let insert_count = rng.random_range(10..=50_u32) as usize;
        let new_kvs = generate_random_kvs(&rng, insert_count);
        let new_keys: Vec<Vec<u8>> = new_kvs.iter().map(|(k, _)| k.clone()).collect();
        for (key, val) in &new_kvs {
            end_batch.push(BatchOp::Put {
                key: key.clone(),
                value: val.clone(),
            });
        }

        db.propose(end_batch).unwrap().commit().unwrap();
        let root2 = db.root_hash().unwrap();

        // Build the sorted list of keys in the end state.
        let mut end_keys: Vec<Vec<u8>> = start_keys
            .iter()
            .enumerate()
            .filter(|(i, _)| !deleted_indices.contains(i))
            .map(|(_, k)| k.clone())
            .chain(new_keys.iter().cloned())
            .collect();
        end_keys.sort();
        end_keys.dedup();

        for _ in 0..50 {
            let scenario = rng.random_range(0..100_u32);
            match scenario {
                // 36% -- both boundaries are existing end-state keys.
                0..36 => {
                    if end_keys.len() < 2 {
                        continue;
                    }
                    let si = rng.random_range(0..end_keys.len() - 1);
                    let ei = rng.random_range(si + 1..end_keys.len());

                    let proof = db
                        .change_proof(
                            root1.clone(),
                            root2.clone(),
                            Some(&end_keys[si]),
                            Some(&end_keys[ei]),
                            None,
                        )
                        .expect("change_proof should succeed");

                    let ctx = verify_change_proof_structure(
                        &proof,
                        root2.clone(),
                        Some(&end_keys[si]),
                        Some(&end_keys[ei]),
                        None,
                    )
                    .expect("structural check should pass (scenario 0)");

                    verify_and_check(&db, &proof, &ctx, root1.clone())
                        .expect("verify should pass (scenario 0)");
                }

                // 20% -- start boundary is a non-existent (decreased) key.
                36..56 => {
                    if end_keys.len() < 2 {
                        continue;
                    }
                    let si = rng.random_range(1..end_keys.len() - 1);
                    let ei = rng.random_range(si..end_keys.len());
                    let Some(decreased) = decrease_key_vec(&end_keys[si]) else {
                        continue;
                    };
                    if decreased >= end_keys[si] || (si > 0 && decreased == end_keys[si - 1]) {
                        continue;
                    }

                    let proof = db
                        .change_proof(
                            root1.clone(),
                            root2.clone(),
                            Some(&decreased),
                            Some(&end_keys[ei]),
                            None,
                        )
                        .expect("change_proof should succeed");

                    let ctx = verify_change_proof_structure(
                        &proof,
                        root2.clone(),
                        Some(&decreased),
                        Some(&end_keys[ei]),
                        None,
                    )
                    .expect("structural check should pass (scenario 1)");

                    verify_and_check(&db, &proof, &ctx, root1.clone())
                        .expect("verify should pass (scenario 1)");
                }

                // 20% -- end boundary is a non-existent (increased) key.
                56..76 => {
                    if end_keys.len() < 2 {
                        continue;
                    }
                    let si = rng.random_range(0..end_keys.len() - 1);
                    let ei = rng.random_range(si..end_keys.len() - 1);
                    let Some(increased) = increase_key_vec(&end_keys[ei]) else {
                        continue;
                    };
                    if increased <= end_keys[ei]
                        || (ei + 1 < end_keys.len() && increased == end_keys[ei + 1])
                    {
                        continue;
                    }

                    let proof = db
                        .change_proof(
                            root1.clone(),
                            root2.clone(),
                            Some(&end_keys[si]),
                            Some(&increased),
                            None,
                        )
                        .expect("change_proof should succeed");

                    let ctx = verify_change_proof_structure(
                        &proof,
                        root2.clone(),
                        Some(&end_keys[si]),
                        Some(&increased),
                        None,
                    )
                    .expect("structural check should pass (scenario 2)");

                    verify_and_check(&db, &proof, &ctx, root1.clone())
                        .expect("verify should pass (scenario 2)");
                }

                // 20% -- both boundaries are non-existent keys.
                76..96 => {
                    if end_keys.len() < 2 {
                        continue;
                    }
                    let si = rng.random_range(1..end_keys.len() - 1);
                    let ei = rng.random_range(si..end_keys.len() - 1);
                    let Some(decreased) = decrease_key_vec(&end_keys[si]) else {
                        continue;
                    };
                    let Some(increased) = increase_key_vec(&end_keys[ei]) else {
                        continue;
                    };
                    if decreased >= end_keys[si]
                        || (si > 0 && decreased == end_keys[si - 1])
                        || increased <= end_keys[ei]
                        || (ei + 1 < end_keys.len() && increased == end_keys[ei + 1])
                    {
                        continue;
                    }

                    let proof = db
                        .change_proof(
                            root1.clone(),
                            root2.clone(),
                            Some(&decreased),
                            Some(&increased),
                            None,
                        )
                        .expect("change_proof should succeed");

                    let ctx = verify_change_proof_structure(
                        &proof,
                        root2.clone(),
                        Some(&decreased),
                        Some(&increased),
                        None,
                    )
                    .expect("structural check should pass (scenario 3)");

                    verify_and_check(&db, &proof, &ctx, root1.clone())
                        .expect("verify should pass (scenario 3)");
                }

                // 4% -- no bounds, complete proof.
                _ => {
                    let proof = db
                        .change_proof(root1.clone(), root2.clone(), None, None, None)
                        .expect("change_proof should succeed");

                    let ctx =
                        verify_change_proof_structure(&proof, root2.clone(), None, None, None)
                            .expect("structural check should pass (scenario 4)");

                    verify_and_check(&db, &proof, &ctx, root1.clone())
                        .expect("verify should pass (scenario 4)");
                }
            }
        }
    }
}
