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

use super::super::*;
use super::verify_and_check;
use crate::api::{BatchOp, Db as DbTrait, FrozenChangeProof, Proposal as _};
use crate::db::{Db, DbConfig};
use crate::{ChangeProof, Proof, verify_change_proof_structure};
use firewood_storage::logger::{debug, trace};

#[test]
#[expect(clippy::too_many_lines)]
fn test_slow_change_proof_fuzz() {
    // When FIREWOOD_TEST_SEED is set, run only that single seed (for
    // reproducing CI failures). Otherwise, run random iterations.
    // Debug assertions significantly slow down each iteration; use fewer
    // iterations in debug builds so the test finishes in reasonable time.
    let iterations = if cfg!(debug_assertions) { 25 } else { 250 };
    let seeds: Vec<u64> = if let Ok(s) = std::env::var("FIREWOOD_TEST_SEED") {
        vec![s.parse().expect("FIREWOOD_TEST_SEED must be a u64")]
    } else {
        let outer_rng = firewood_storage::SeededRng::from_random();
        (0..iterations).map(|_| outer_rng.next_u64()).collect()
    };

    for (run, &seed) in seeds.iter().enumerate() {
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

// ── Adversarial fuzz test helpers ──────────────────────────────────────────

type Key = Box<[u8]>;
type Value = Box<[u8]>;

/// Build a `FrozenChangeProof` from mutable parts.
fn build_change_proof(
    start: Vec<ProofNode>,
    end: Vec<ProofNode>,
    ops: Vec<BatchOp<Key, Value>>,
) -> FrozenChangeProof {
    ChangeProof::new(
        Proof::new(start.into_boxed_slice()),
        Proof::new(end.into_boxed_slice()),
        ops.into_boxed_slice(),
    )
}

/// Flip a random bit in a `HashType`.
fn flip_hash_bit(rng: &firewood_storage::SeededRng, hash: &mut firewood_storage::HashType) {
    // Convert to TrieHash (always 32 bytes), flip a bit, convert back.
    let trie_hash = hash.clone().into_triehash();
    let bytes: [u8; 32] = trie_hash.into();
    let mut new_bytes = bytes;
    let idx = rng.random_range(0..32_usize);
    let bit = rng.random_range(0..8_u32);
    new_bytes[idx] ^= 1 << bit;
    *hash = firewood_storage::TrieHash::from(new_bytes).into_hash_type();
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
    // Debug assertions significantly slow down each iteration; use fewer
    // iterations in debug builds so the test finishes in reasonable time.
    let iterations = if cfg!(debug_assertions) { 25 } else { 250 };
    let seeds: Vec<u64> = if let Ok(s) = std::env::var("FIREWOOD_TEST_SEED") {
        vec![s.parse().expect("FIREWOOD_TEST_SEED must be a u64")]
    } else {
        let outer_rng = firewood_storage::SeededRng::from_random();
        (0..iterations).map(|_| outer_rng.next_u64()).collect()
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

// ── Adversarial change proof fuzz test ────────────────────────────────────

/// Adversarial fuzz test: generates valid change proofs, then applies random
/// mutations (corruptions) and asserts that verification always rejects them.
///
/// # Reproducing failures
///
/// Each iteration prints a seed to stderr. To reproduce a specific failure:
///
/// ```sh
/// FIREWOOD_TEST_SEED=<seed> cargo nextest run -p firewood --features logger \
///   -E 'test(adversarial_change_proof_fuzz)' --profile ci
/// ```
///
/// For detailed diagnostics, enable the `logger` feature and set `RUST_LOG`:
///
/// - `RUST_LOG=DEBUG` — shows scenario selection, boundary keys, mutation
///   applied, and whether each mutation was rejected (and by which phase).
/// - `RUST_LOG=TRACE` — additionally dumps the full start/end tries (DOT
///   format), all proof nodes, and batch ops before and after mutation.
///
/// The failure message includes the seed, scenario, group, mutation name,
/// and proof dimensions to help narrow down the issue.
///
/// # Mutation groups (by weight)
///
///   30% — batch op mutations (omit/add/swap keys and values)
///   25% — exclusion proof attacks (truncate, clear child, graft, swap)
///   15% — out-of-range structural (corrupt sibling hash, remove intermediate)
///   10% — replay/cross-revision (wrong root, reversed, different DB)
///    5% — boundary proof structural (truncate, swap, empty)
///   15% — combined: force double-exclusion + group 4/5 mutation
#[test]
#[expect(clippy::too_many_lines)]
fn test_slow_adversarial_change_proof_fuzz() {
    // When FIREWOOD_TEST_SEED is set, run only that single seed (for
    // reproducing CI failures). Otherwise, run random iterations.
    let iterations = if cfg!(debug_assertions) { 10 } else { 100 };
    let seeds: Vec<u64> = if let Ok(s) = std::env::var("FIREWOOD_TEST_SEED") {
        vec![s.parse().expect("FIREWOOD_TEST_SEED must be a u64")]
    } else {
        let outer_rng = firewood_storage::SeededRng::from_random();
        (0..iterations).map(|_| outer_rng.next_u64()).collect()
    };

    for (run, &seed) in seeds.iter().enumerate() {
        eprintln!(
            "adversarial run {run}: seed={seed} (export FIREWOOD_TEST_SEED={seed} to reproduce)"
        );
        let rng = firewood_storage::SeededRng::new(seed);

        // ── Build start trie ──────────────────────────────────────────
        let key_count = rng.random_range(64..=2048_u32);
        let start_data = fixed_and_pseudorandom_data(&rng, key_count);
        let mut start_keys: Vec<[u8; 32]> = start_data.keys().copied().collect();
        start_keys.sort_unstable();

        let dir = tempfile::tempdir().unwrap();
        let db = Db::new(dir.path(), DbConfig::builder().build()).unwrap();

        let start_batch: Vec<BatchOp<&[u8], &[u8]>> = start_data
            .iter()
            .map(|(k, v)| BatchOp::Put {
                key: k.as_ref(),
                value: v.as_ref(),
            })
            .collect();
        db.propose(start_batch).unwrap().commit().unwrap();
        let root1 = db.root_hash().unwrap();
        debug!("start trie: {key_count} keys, root1={root1:?}");
        trace!("start trie dump:\n{}", db.dump_to_string().unwrap());

        // ── Build end trie ────────────────────────────────────────────
        let delete_step = (start_keys.len() / 7).max(1);
        let mut end_batch: Vec<BatchOp<&[u8], &[u8]>> = Vec::new();

        let mut deleted_indices = Vec::new();
        for i in (0..start_keys.len()).step_by(delete_step + 1) {
            end_batch.push(BatchOp::Delete {
                key: start_keys[i].as_ref(),
            });
            deleted_indices.push(i);
        }

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
        debug!(
            "end trie: deleted={}, inserted={insert_count}, root2={root2:?}",
            deleted_indices.len()
        );
        trace!("end trie dump:\n{}", db.dump_to_string().unwrap());

        // ── Build end-state key list ──────────────────────────────────
        let mut end_keys: Vec<[u8; 32]> = start_keys
            .iter()
            .enumerate()
            .filter(|(i, _)| !deleted_indices.contains(i))
            .map(|(_, k)| *k)
            .chain(new_keys.iter().copied())
            .collect();
        end_keys.sort_unstable();
        end_keys.dedup();

        // ── 3rd revision for replay tests ─────────────────────────────
        let extra_kvs: Vec<([u8; 32], [u8; 20])> = (0..10)
            .map(|_| (rng.random::<[u8; 32]>(), rng.random::<[u8; 20]>()))
            .collect();
        let extra_batch: Vec<BatchOp<&[u8], &[u8]>> = extra_kvs
            .iter()
            .map(|(k, v)| BatchOp::Put {
                key: k.as_ref(),
                value: v.as_ref(),
            })
            .collect();
        db.propose(extra_batch).unwrap().commit().unwrap();
        let root3 = db.root_hash().unwrap();
        debug!("3rd revision root3={root3:?}");

        // ── 2nd independent DB for replay tests ──────────────────────
        let dir2 = tempfile::tempdir().unwrap();
        let db2 = Db::new(dir2.path(), DbConfig::builder().build()).unwrap();
        let rng2 = rng.seeded_fork();
        let start_data2 = fixed_and_pseudorandom_data(&rng2, key_count);
        let batch2: Vec<BatchOp<&[u8], &[u8]>> = start_data2
            .iter()
            .map(|(k, v)| BatchOp::Put {
                key: k.as_ref(),
                value: v.as_ref(),
            })
            .collect();
        db2.propose(batch2).unwrap().commit().unwrap();
        let root1_b = db2.root_hash().unwrap();
        let extra2: Vec<([u8; 32], [u8; 20])> = (0..20)
            .map(|_| (rng2.random::<[u8; 32]>(), rng2.random::<[u8; 20]>()))
            .collect();
        let batch2b: Vec<BatchOp<&[u8], &[u8]>> = extra2
            .iter()
            .map(|(k, v)| BatchOp::Put {
                key: k.as_ref(),
                value: v.as_ref(),
            })
            .collect();
        db2.propose(batch2b).unwrap().commit().unwrap();
        let root2_b = db2.root_hash().unwrap();

        // ── Inner loop: generate valid proof, mutate, assert rejection ─
        for _ in 0..50 {
            let scenario = rng.random_range(0..100_u32);

            let (proof, start_key, end_key): (
                FrozenChangeProof,
                Option<[u8; 32]>,
                Option<[u8; 32]>,
            ) = match scenario {
                // Both boundaries exist.
                0..36 => {
                    if end_keys.len() < 2 {
                        continue;
                    }
                    let si = rng.random_range(0..end_keys.len() - 1);
                    let ei = rng.random_range(si + 1..end_keys.len());
                    let sk = end_keys[si];
                    let ek = end_keys[ei];
                    let p = db
                        .change_proof(
                            root1.clone(),
                            root2.clone(),
                            Some(sk.as_ref()),
                            Some(ek.as_ref()),
                            None,
                        )
                        .expect("change_proof should succeed");
                    (p, Some(sk), Some(ek))
                }
                // Start boundary non-existent.
                36..56 => {
                    if end_keys.len() < 2 {
                        continue;
                    }
                    let si = rng.random_range(1..end_keys.len() - 1);
                    let ei = rng.random_range(si..end_keys.len());
                    let decreased = decrease_key(&end_keys[si]);
                    if decreased >= end_keys[si] || (si > 0 && decreased == end_keys[si - 1]) {
                        continue;
                    }
                    let ek = end_keys[ei];
                    let p = db
                        .change_proof(
                            root1.clone(),
                            root2.clone(),
                            Some(decreased.as_ref()),
                            Some(ek.as_ref()),
                            None,
                        )
                        .expect("change_proof should succeed");
                    (p, Some(decreased), Some(ek))
                }
                // End boundary non-existent.
                56..76 => {
                    if end_keys.len() < 2 {
                        continue;
                    }
                    let si = rng.random_range(0..end_keys.len() - 1);
                    let ei = rng.random_range(si..end_keys.len() - 1);
                    let increased = increase_key(&end_keys[ei]);
                    if increased <= end_keys[ei]
                        || (ei + 1 < end_keys.len() && increased == end_keys[ei + 1])
                    {
                        continue;
                    }
                    let sk = end_keys[si];
                    let p = db
                        .change_proof(
                            root1.clone(),
                            root2.clone(),
                            Some(sk.as_ref()),
                            Some(increased.as_ref()),
                            None,
                        )
                        .expect("change_proof should succeed");
                    (p, Some(sk), Some(increased))
                }
                // Both boundaries non-existent.
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
                    let p = db
                        .change_proof(
                            root1.clone(),
                            root2.clone(),
                            Some(decreased.as_ref()),
                            Some(increased.as_ref()),
                            None,
                        )
                        .expect("change_proof should succeed");
                    (p, Some(decreased), Some(increased))
                }
                // No bounds.
                _ => {
                    let p = db
                        .change_proof(root1.clone(), root2.clone(), None, None, None)
                        .expect("change_proof should succeed");
                    (p, None, None)
                }
            };

            let scenario_desc = match scenario {
                0..36 => "both_exist",
                36..56 => "start_nonexistent",
                56..76 => "end_nonexistent",
                76..96 => "both_nonexistent",
                _ => "no_bounds",
            };
            debug!(
                "scenario={scenario}({scenario_desc}) start_key={} end_key={} \
                 batch_ops={} start_proof={} end_proof={}",
                start_key.as_ref().map_or("None".into(), hex::encode),
                end_key.as_ref().map_or("None".into(), hex::encode),
                proof.batch_ops().len(),
                proof.start_proof().as_ref().len(),
                proof.end_proof().as_ref().len(),
            );
            trace!("batch_ops: {:#?}", proof.batch_ops());
            debug!("start_proof: {:?}", proof.start_proof().as_ref());
            debug!("end_proof: {:?}", proof.end_proof().as_ref());

            // Pick mutation group.
            let group = rng.random_range(0..100_u32);

            let mutation_name: &str;
            let mutated_proof: FrozenChangeProof;
            let mut use_start_key: Option<Vec<u8>> = start_key.as_ref().map(|k| k.to_vec());
            let mut use_end_key: Option<Vec<u8>> = end_key.as_ref().map(|k| k.to_vec());
            let mut use_end_root = root2.clone();
            let mut use_start_root = root1.clone();
            let use_db: &Db;

            match group {
                // ── Group 1: Batch op mutations (30%) ─────────────────
                0..30 => {
                    use_db = &db;
                    let mut ops: Vec<BatchOp<Key, Value>> = proof.batch_ops().to_vec();
                    if ops.is_empty() {
                        continue;
                    }

                    let sub = rng.random_range(0..7_u32);
                    match sub {
                        // M1: Omit a Put
                        0 => {
                            let put_indices: Vec<usize> = ops
                                .iter()
                                .enumerate()
                                .filter(|(_, op)| matches!(op, BatchOp::Put { .. }))
                                .map(|(i, _)| i)
                                .collect();
                            if put_indices.is_empty() {
                                continue;
                            }
                            let idx = put_indices[rng.random_range(0..put_indices.len())];
                            ops.remove(idx);
                            mutation_name = "M1_omit_put";
                        }
                        // M2: Omit a Delete
                        1 => {
                            let del_indices: Vec<usize> = ops
                                .iter()
                                .enumerate()
                                .filter(|(_, op)| matches!(op, BatchOp::Delete { .. }))
                                .map(|(i, _)| i)
                                .collect();
                            if del_indices.is_empty() {
                                continue;
                            }
                            let idx = del_indices[rng.random_range(0..del_indices.len())];
                            ops.remove(idx);
                            mutation_name = "M2_omit_delete";
                        }
                        // M3: Swap a value
                        2 => {
                            let put_indices: Vec<usize> = ops
                                .iter()
                                .enumerate()
                                .filter(|(_, op)| matches!(op, BatchOp::Put { .. }))
                                .map(|(i, _)| i)
                                .collect();
                            if put_indices.is_empty() {
                                continue;
                            }
                            let idx = put_indices[rng.random_range(0..put_indices.len())];
                            let key = ops[idx].key().clone();
                            let old_val = match &ops[idx] {
                                BatchOp::Put { value, .. } => value.clone(),
                                _ => unreachable!(),
                            };
                            let mask: [u8; 20] = loop {
                                let m: [u8; 20] = rng.random();
                                if m != [0u8; 20] {
                                    break m;
                                }
                            };
                            let new_val: Vec<u8> = old_val
                                .iter()
                                .zip(mask.iter())
                                .map(|(a, b)| a ^ b)
                                .collect();
                            debug!(
                                "M3: key={} old_val={} new_val={}",
                                hex::encode(&*key),
                                hex::encode(&*old_val),
                                hex::encode(&new_val),
                            );
                            ops[idx] = BatchOp::Put {
                                key,
                                value: new_val.into_boxed_slice(),
                            };
                            mutation_name = "M3_swap_value";
                        }
                        // M4: Put → Delete
                        3 => {
                            let put_indices: Vec<usize> = ops
                                .iter()
                                .enumerate()
                                .filter(|(_, op)| matches!(op, BatchOp::Put { .. }))
                                .map(|(i, _)| i)
                                .collect();
                            if put_indices.is_empty() {
                                continue;
                            }
                            let idx = put_indices[rng.random_range(0..put_indices.len())];
                            let key = ops[idx].key().clone();
                            ops[idx] = BatchOp::Delete { key };
                            mutation_name = "M4_put_to_delete";
                        }
                        // M5: Delete → Put
                        4 => {
                            let del_indices: Vec<usize> = ops
                                .iter()
                                .enumerate()
                                .filter(|(_, op)| matches!(op, BatchOp::Delete { .. }))
                                .map(|(i, _)| i)
                                .collect();
                            if del_indices.is_empty() {
                                continue;
                            }
                            let idx = del_indices[rng.random_range(0..del_indices.len())];
                            let key = ops[idx].key().clone();
                            let new_val: [u8; 20] = rng.random();
                            debug!(
                                "M5: key={} new_val={}",
                                hex::encode(&*key),
                                hex::encode(new_val),
                            );
                            ops[idx] = BatchOp::Put {
                                key,
                                value: new_val.to_vec().into_boxed_slice(),
                            };
                            mutation_name = "M5_delete_to_put";
                        }
                        // M6: Add spurious Put
                        5 => {
                            let new_key: [u8; 32] = rng.random();
                            let new_val: [u8; 20] = rng.random();
                            let key_box: Key = new_key.to_vec().into_boxed_slice();
                            if ops.iter().any(|op| op.key().as_ref() == new_key) {
                                continue;
                            }
                            let pos = ops
                                .binary_search_by(|op| op.key().as_ref().cmp(&new_key[..]))
                                .unwrap_or_else(|i| i);
                            ops.insert(
                                pos,
                                BatchOp::Put {
                                    key: key_box,
                                    value: new_val.to_vec().into_boxed_slice(),
                                },
                            );
                            mutation_name = "M6_spurious_put";
                        }
                        // M7: Add spurious Delete for a key that exists in
                        // both the start and end tries within the proven range.
                        _ => {
                            let sk = start_key.as_ref().map(AsRef::as_ref);
                            let ek = end_key.as_ref().map(AsRef::as_ref);
                            let candidates: Vec<[u8; 32]> = end_keys
                                .iter()
                                .filter(|k| {
                                    let in_range = sk.is_none_or(|s| k.as_slice() >= s)
                                        && ek.is_none_or(|e| k.as_slice() <= e);
                                    in_range
                                        && start_keys.binary_search(k).is_ok()
                                        && !ops.iter().any(|op| op.key().as_ref() == k.as_slice())
                                })
                                .copied()
                                .collect();
                            if candidates.is_empty() {
                                continue;
                            }
                            let new_key = candidates[rng.random_range(0..candidates.len())];
                            let key_box: Key = new_key.to_vec().into_boxed_slice();
                            let pos = ops
                                .binary_search_by(|op| op.key().as_ref().cmp(&new_key[..]))
                                .unwrap_or_else(|i| i);
                            ops.insert(pos, BatchOp::Delete { key: key_box });
                            mutation_name = "M7_spurious_delete";
                        }
                    }

                    let start_nodes: Vec<ProofNode> = proof.start_proof().as_ref().to_vec();
                    let end_nodes: Vec<ProofNode> = proof.end_proof().as_ref().to_vec();
                    mutated_proof = build_change_proof(start_nodes, end_nodes, ops);
                }

                // ── Group 4: Exclusion proof attacks (25%) ────────────
                30..55 => {
                    use_db = &db;
                    let start_nodes: Vec<ProofNode> = proof.start_proof().as_ref().to_vec();
                    let end_nodes: Vec<ProofNode> = proof.end_proof().as_ref().to_vec();
                    let ops: Vec<BatchOp<Key, Value>> = proof.batch_ops().to_vec();

                    let has_start = !start_nodes.is_empty();
                    let has_end = !end_nodes.is_empty();
                    if !has_start && !has_end {
                        continue;
                    }

                    let sub = rng.random_range(0..6_u32);
                    match sub {
                        // M20: Truncate exclusion proof to ancestor
                        0 => {
                            if has_start && start_nodes.len() >= 2 {
                                let mut nodes = start_nodes.clone();
                                nodes.pop();
                                mutated_proof = build_change_proof(nodes, end_nodes, ops);
                                mutation_name = "M20_truncate_exclusion_start";
                            } else if has_end && end_nodes.len() >= 2 {
                                let mut nodes = end_nodes.clone();
                                nodes.pop();
                                mutated_proof = build_change_proof(start_nodes, nodes, ops);
                                mutation_name = "M20_truncate_exclusion_end";
                            } else {
                                continue;
                            }
                        }
                        // M21: Clear child hash at last proof node
                        1 => {
                            if has_start {
                                let mut nodes = start_nodes.clone();
                                let last = nodes.last_mut().unwrap();
                                let mut found = false;
                                for pc in PathComponent::ALL {
                                    if last.child_hashes[pc].is_some() {
                                        last.child_hashes.replace(pc, None);
                                        found = true;
                                        break;
                                    }
                                }
                                if !found {
                                    continue;
                                }
                                mutated_proof = build_change_proof(nodes, end_nodes, ops);
                                mutation_name = "M21_clear_child_start";
                            } else {
                                let mut nodes = end_nodes.clone();
                                let last = nodes.last_mut().unwrap();
                                let mut found = false;
                                for pc in PathComponent::ALL {
                                    if last.child_hashes[pc].is_some() {
                                        last.child_hashes.replace(pc, None);
                                        found = true;
                                        break;
                                    }
                                }
                                if !found {
                                    continue;
                                }
                                mutated_proof = build_change_proof(start_nodes, nodes, ops);
                                mutation_name = "M21_clear_child_end";
                            }
                        }
                        // M22: Inject fake value into a valueless proof node
                        2 => {
                            let target_start =
                                has_start && start_nodes.iter().any(|n| n.value_digest.is_none());
                            if target_start {
                                let mut nodes = start_nodes.clone();
                                let idx =
                                    nodes.iter().position(|n| n.value_digest.is_none()).unwrap();
                                let fake_val: [u8; 20] = rng.random();
                                nodes[idx].value_digest =
                                    Some(ValueDigest::Value(fake_val.to_vec().into()));
                                mutated_proof = build_change_proof(nodes, end_nodes, ops);
                                mutation_name = "M22_inject_value_start";
                            } else if has_end && end_nodes.iter().any(|n| n.value_digest.is_none())
                            {
                                let mut nodes = end_nodes.clone();
                                let idx =
                                    nodes.iter().position(|n| n.value_digest.is_none()).unwrap();
                                let fake_val: [u8; 20] = rng.random();
                                nodes[idx].value_digest =
                                    Some(ValueDigest::Value(fake_val.to_vec().into()));
                                mutated_proof = build_change_proof(start_nodes, nodes, ops);
                                mutation_name = "M22_inject_value_end";
                            } else {
                                continue;
                            }
                        }
                        // M23: Remove value from a proof path node
                        3 => {
                            let target_start =
                                has_start && start_nodes.iter().any(|n| n.value_digest.is_some());
                            if target_start {
                                let mut nodes = start_nodes.clone();
                                let idx =
                                    nodes.iter().position(|n| n.value_digest.is_some()).unwrap();
                                nodes[idx].value_digest = None;
                                mutated_proof = build_change_proof(nodes, end_nodes, ops);
                                mutation_name = "M23_remove_value_start";
                            } else if has_end && end_nodes.iter().any(|n| n.value_digest.is_some())
                            {
                                let mut nodes = end_nodes.clone();
                                let idx =
                                    nodes.iter().position(|n| n.value_digest.is_some()).unwrap();
                                nodes[idx].value_digest = None;
                                mutated_proof = build_change_proof(start_nodes, nodes, ops);
                                mutation_name = "M23_remove_value_end";
                            } else {
                                continue;
                            }
                        }
                        // M24: Flip a child hash in the end proof's last node
                        4 => {
                            if !has_end {
                                continue;
                            }
                            let mut nodes = end_nodes.clone();
                            let last = nodes.last_mut().unwrap();
                            let mut found = false;
                            for pc in PathComponent::ALL {
                                if let Some(ref mut h) = last.child_hashes[pc] {
                                    flip_hash_bit(&rng, h);
                                    found = true;
                                    break;
                                }
                            }
                            if !found {
                                continue;
                            }
                            mutated_proof = build_change_proof(start_nodes, nodes, ops);
                            mutation_name = "M24_corrupt_end_last";
                        }
                        // M25: Corrupt shared prefix node's child hash
                        _ => {
                            if !has_start || !has_end {
                                continue;
                            }
                            let shared_idx = start_nodes.iter().enumerate().find_map(|(i, sn)| {
                                end_nodes
                                    .get(i)
                                    .and_then(|en| if sn.key == en.key { Some(i) } else { None })
                            });
                            let Some(idx) = shared_idx else {
                                continue;
                            };
                            let mut nodes = start_nodes.clone();
                            let node = &mut nodes[idx];
                            let mut found = false;
                            for pc in PathComponent::ALL {
                                if let Some(ref mut h) = node.child_hashes[pc] {
                                    flip_hash_bit(&rng, h);
                                    found = true;
                                    break;
                                }
                            }
                            if !found {
                                continue;
                            }
                            mutated_proof = build_change_proof(nodes, end_nodes, ops);
                            mutation_name = "M25_corrupt_shared_node";
                        }
                    }
                }

                // ── Group 5: Out-of-range structural (15%) ────────────
                55..70 => {
                    use_db = &db;
                    let start_nodes: Vec<ProofNode> = proof.start_proof().as_ref().to_vec();
                    let end_nodes: Vec<ProofNode> = proof.end_proof().as_ref().to_vec();
                    let ops: Vec<BatchOp<Key, Value>> = proof.batch_ops().to_vec();

                    let sub = rng.random_range(0..2_u32);
                    match sub {
                        // M27: Corrupt out-of-range sibling hash
                        0 => {
                            if !start_nodes.is_empty() {
                                let mut nodes = start_nodes.clone();
                                let idx = rng.random_range(0..nodes.len());
                                let node = &mut nodes[idx];
                                let mut found = false;
                                let start_pc = rng.random_range(0..16_u8);
                                for offset in 0..16_u8 {
                                    let pc_idx = (start_pc + offset) % 16;
                                    let pc = PathComponent::ALL[pc_idx as usize];
                                    if let Some(ref mut h) = node.child_hashes[pc] {
                                        flip_hash_bit(&rng, h);
                                        found = true;
                                        break;
                                    }
                                }
                                if !found {
                                    continue;
                                }
                                mutated_proof = build_change_proof(nodes, end_nodes, ops);
                                mutation_name = "M27_corrupt_sibling_start";
                            } else if !end_nodes.is_empty() {
                                let mut nodes = end_nodes.clone();
                                let idx = rng.random_range(0..nodes.len());
                                let node = &mut nodes[idx];
                                let mut found = false;
                                let start_pc = rng.random_range(0..16_u8);
                                for offset in 0..16_u8 {
                                    let pc_idx = (start_pc + offset) % 16;
                                    let pc = PathComponent::ALL[pc_idx as usize];
                                    if let Some(ref mut h) = node.child_hashes[pc] {
                                        flip_hash_bit(&rng, h);
                                        found = true;
                                        break;
                                    }
                                }
                                if !found {
                                    continue;
                                }
                                mutated_proof = build_change_proof(start_nodes, nodes, ops);
                                mutation_name = "M27_corrupt_sibling_end";
                            } else {
                                continue;
                            }
                        }
                        // M28: Remove intermediate proof node
                        _ => {
                            if start_nodes.len() >= 3 {
                                let mut nodes = start_nodes;
                                let idx = rng.random_range(1..nodes.len() - 1);
                                nodes.remove(idx);
                                mutated_proof = build_change_proof(nodes, end_nodes, ops);
                                mutation_name = "M28_remove_intermediate_start";
                            } else if end_nodes.len() >= 3 {
                                let mut nodes = end_nodes;
                                let idx = rng.random_range(1..nodes.len() - 1);
                                nodes.remove(idx);
                                mutated_proof = build_change_proof(start_nodes, nodes, ops);
                                mutation_name = "M28_remove_intermediate_end";
                            } else {
                                continue;
                            }
                        }
                    }
                }

                // ── Group 6: Replay / cross-revision (10%) ────────────
                70..80 => {
                    // Need non-empty batch_ops for meaningful cross-revision checks.
                    if proof.batch_ops().len() < 2 {
                        continue;
                    }
                    let start_nodes: Vec<ProofNode> = proof.start_proof().as_ref().to_vec();
                    let end_nodes: Vec<ProofNode> = proof.end_proof().as_ref().to_vec();
                    let ops: Vec<BatchOp<Key, Value>> = proof.batch_ops().to_vec();

                    let sub = rng.random_range(0..3_u32);
                    match sub {
                        // M29: Wrong end_root (use root3)
                        0 => {
                            use_db = &db;
                            mutated_proof = build_change_proof(start_nodes, end_nodes, ops);
                            use_end_root = root3.clone();
                            mutation_name = "M29_wrong_end_root";
                        }
                        // M30: Reversed roots — use root2 as start and root1 as end
                        1 => {
                            use_db = &db;
                            mutated_proof = build_change_proof(start_nodes, end_nodes, ops);
                            use_start_root = root2.clone();
                            use_end_root = root1.clone();
                            mutation_name = "M30_reversed_roots";
                        }
                        // M31: Proof from different DB
                        _ => {
                            use_db = &db;
                            if let Ok(other_proof) = db2.change_proof(
                                root1_b.clone(),
                                root2_b.clone(),
                                start_key.as_ref().map(AsRef::as_ref),
                                end_key.as_ref().map(AsRef::as_ref),
                                None,
                            ) {
                                if other_proof.batch_ops().len() < 2 {
                                    continue;
                                }
                                let s: Vec<ProofNode> = other_proof.start_proof().as_ref().to_vec();
                                let e: Vec<ProofNode> = other_proof.end_proof().as_ref().to_vec();
                                let o: Vec<BatchOp<Key, Value>> = other_proof.batch_ops().to_vec();
                                mutated_proof = build_change_proof(s, e, o);
                                // Keep use_end_root = root2 (the default).
                                // The requester asked for a proof to root2,
                                // not root2_b, so verification must reject.
                                mutation_name = "M31_wrong_db";
                            } else {
                                continue;
                            }
                        }
                    }
                }

                // ── Group 2: Boundary proof structural (5%) ───────────
                80..85 => {
                    use_db = &db;
                    // Empty batch_ops means no in-range state to verify,
                    // so proof mutations are harmless.
                    if proof.batch_ops().is_empty() {
                        continue;
                    }
                    let start_nodes: Vec<ProofNode> = proof.start_proof().as_ref().to_vec();
                    let end_nodes: Vec<ProofNode> = proof.end_proof().as_ref().to_vec();
                    let ops: Vec<BatchOp<Key, Value>> = proof.batch_ops().to_vec();

                    let sub = rng.random_range(0..4_u32);
                    match sub {
                        // M8: Truncate start proof
                        0 => {
                            if start_nodes.is_empty() {
                                continue;
                            }
                            let mut nodes = start_nodes;
                            nodes.pop();
                            mutated_proof = build_change_proof(nodes, end_nodes, ops);
                            mutation_name = "M8_truncate_start";
                        }
                        // M9: Truncate end proof
                        1 => {
                            if end_nodes.is_empty() {
                                continue;
                            }
                            let mut nodes = end_nodes;
                            nodes.pop();
                            mutated_proof = build_change_proof(start_nodes, nodes, ops);
                            mutation_name = "M9_truncate_end";
                        }
                        // M10: Swap proofs
                        2 => {
                            if start_nodes == end_nodes {
                                continue;
                            }
                            mutated_proof = build_change_proof(end_nodes, start_nodes, ops);
                            mutation_name = "M10_swap_proofs";
                        }
                        // M11: Empty the end proof
                        _ => {
                            if end_nodes.is_empty() {
                                continue;
                            }
                            mutated_proof = build_change_proof(start_nodes, Vec::new(), ops);
                            mutation_name = "M11_empty_end_proof";
                        }
                    }
                }

                // ── Combined: force double-exclusion + group 4/5 (15%) ─
                _ => {
                    use_db = &db;
                    if end_keys.len() < 3 {
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
                    let double_proof = db
                        .change_proof(
                            root1.clone(),
                            root2.clone(),
                            Some(decreased.as_ref()),
                            Some(increased.as_ref()),
                            None,
                        )
                        .expect("change_proof should succeed");

                    use_start_key = Some(decreased.to_vec());
                    use_end_key = Some(increased.to_vec());

                    let start_nodes: Vec<ProofNode> = double_proof.start_proof().as_ref().to_vec();
                    let end_nodes: Vec<ProofNode> = double_proof.end_proof().as_ref().to_vec();
                    let ops: Vec<BatchOp<Key, Value>> = double_proof.batch_ops().to_vec();

                    if start_nodes.is_empty() || end_nodes.is_empty() {
                        continue;
                    }

                    let sub = rng.random_range(0..3_u32);
                    match sub {
                        // Truncate start exclusion proof
                        0 => {
                            if start_nodes.len() < 2 {
                                continue;
                            }
                            let mut nodes = start_nodes;
                            nodes.pop();
                            mutated_proof = build_change_proof(nodes, end_nodes, ops);
                            mutation_name = "combined_truncate_start";
                        }
                        // Corrupt a child hash in end proof
                        1 => {
                            let mut nodes = end_nodes;
                            let idx = rng.random_range(0..nodes.len());
                            let node = &mut nodes[idx];
                            let mut found = false;
                            for pc in PathComponent::ALL {
                                if let Some(ref mut h) = node.child_hashes[pc] {
                                    flip_hash_bit(&rng, h);
                                    found = true;
                                    break;
                                }
                            }
                            if !found {
                                continue;
                            }
                            mutated_proof = build_change_proof(start_nodes, nodes, ops);
                            mutation_name = "combined_corrupt_end_child";
                        }
                        // Corrupt start proof value
                        _ => {
                            // Remove a value from a start proof node (if any).
                            let idx = start_nodes.iter().position(|n| n.value_digest.is_some());
                            if let Some(idx) = idx {
                                let mut nodes = start_nodes;
                                nodes[idx].value_digest = None;
                                mutated_proof = build_change_proof(nodes, end_nodes, ops);
                                mutation_name = "combined_remove_value";
                            } else {
                                continue;
                            }
                        }
                    }
                }
            }

            // ── Assert rejection ──────────────────────────────────────
            debug!(
                "group={group} mutation={mutation_name} \
                 mutated_batch_ops={} mutated_start_proof={} mutated_end_proof={}",
                mutated_proof.batch_ops().len(),
                mutated_proof.start_proof().as_ref().len(),
                mutated_proof.end_proof().as_ref().len(),
            );
            trace!("mutated batch_ops: {:#?}", mutated_proof.batch_ops());
            trace!(
                "mutated start_proof: {:#?}",
                mutated_proof.start_proof().as_ref()
            );
            trace!(
                "mutated end_proof: {:#?}",
                mutated_proof.end_proof().as_ref()
            );

            let structural_result = verify_change_proof_structure(
                &mutated_proof,
                use_end_root.clone(),
                use_start_key.as_deref(),
                use_end_key.as_deref(),
                None,
            );
            let rejected = match &structural_result {
                Err(e) => {
                    debug!("rejected by structural check: {e}");
                    true
                }
                Ok(ctx) => {
                    let root_result =
                        verify_and_check(use_db, &mutated_proof, ctx, use_start_root.clone());
                    if let Err(e) = &root_result {
                        debug!("rejected by root hash check: {e}");
                        true
                    } else {
                        debug!("NOT REJECTED — mutation {mutation_name} passed verification!");
                        false
                    }
                }
            };

            assert!(
                rejected,
                "mutation {mutation_name} was NOT rejected! \
                 seed={seed}, run={run}, scenario={scenario}, group={group}, \
                 batch_ops={}, start_proof_len={}, end_proof_len={}, \
                 structural_ok={}, end_root={use_end_root:?}",
                mutated_proof.batch_ops().len(),
                mutated_proof.start_proof().as_ref().len(),
                mutated_proof.end_proof().as_ref().len(),
                structural_result.is_ok(),
            );
        }
    }
}
