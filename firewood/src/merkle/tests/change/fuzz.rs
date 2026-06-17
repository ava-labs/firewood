// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Comprehensive change-proof fuzz test.
//!
//! Each outer iteration generates a random start trie with a per-key shape
//! mix, builds an end trie by applying random Put/Delete operations, then
//! runs many inner iterations. Each inner iteration generates a change proof
//! under one of five boundary scenarios, verifies that the valid proof
//! passes, then applies one of 31 named mutations (counting start/end
//! variants separately) and asserts the mutated proof is rejected.
//!
//! ## KV shape mix (per key, independent)
//!
//! - 50% — 32-byte key, 1-31 byte value (`ValueDigest::Value` path)
//! - 30% — 32-byte key, 32-128 byte value (`ValueDigest::Hash` path after
//!   serialization in merkledb mode)
//! - 20% — variable-length key (1-4096 bytes), variable-length value
//!   (1-4096 bytes) — exercises shallow tries, prefix-key relationships,
//!   and divergent-child exclusion proofs
//!
//! ## Boundary scenarios (weighted)
//!
//! 1. (`0..36`,   36%) Both boundary keys are existing end-state keys
//! 2. (`36..56`,  20%) Start boundary is a non-existent (decreased) key
//! 3. (`56..76`,  20%) End boundary is a non-existent (increased) key
//! 4. (`76..96`,  20%) Both boundaries are non-existent keys
//! 5. (`96..100`,  4%) No bounds (complete proof)
//!
//! ## Mutation groups (weighted, every iteration)
//!
//! - 30% — batch op mutations (omit/add/swap keys and values)
//! - 25% — exclusion proof attacks (truncate, clear child, graft, swap)
//! - 15% — out-of-range structural (corrupt sibling hash, remove intermediate)
//! - 10% — replay/cross-revision (wrong root, reversed, different DB)
//! -  5% — boundary proof structural (truncate, swap, empty)
//! - 15% — combined: force double-exclusion + exclusion/structural mutation
//!
//! Runs 25 iterations in debug builds and 250 in release builds, each with a
//! freshly seeded RNG. On failure, the printed seed can be passed via
//! `FIREWOOD_TEST_SEED` to reproduce.
//!
//! # Reproducing failures
//!
//! ```sh
//! FIREWOOD_TEST_SEED=<seed> cargo nextest run -p firewood --features logger \
//!   -E 'test(test_slow_change_proof_fuzz)' --profile ci
//! ```
//!
//! For detailed diagnostics, enable the `logger` feature and set `RUST_LOG`:
//!
//! - `RUST_LOG=DEBUG` — shows scenario selection, boundary keys, mutation
//!   applied, and whether each mutation was rejected (and by which phase).
//! - `RUST_LOG=TRACE` — additionally dumps the full start/end tries (DOT
//!   format), all proof nodes, and batch ops before and after mutation.

use super::super::*;
use super::verify_and_check;
use crate::api::{BatchOp, Db as DbTrait, FrozenChangeProof, HashKey, Proposal as _};
use crate::db::{Db, DbConfig};
use crate::{ChangeProof, Proof, verify_change_proof_structure};
use firewood_storage::logger::{debug, trace};

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

/// Generate a single key-value pair with a per-key shape mix:
/// - 50% — 32-byte key, 1-31 byte value (`ValueDigest::Value`)
/// - 30% — 32-byte key, 32-128 byte value (`ValueDigest::Hash` after serialization)
/// - 20% — variable-length key (1-4096 bytes), variable-length value (1-4096 bytes)
fn generate_kv(rng: &firewood_storage::SeededRng) -> (Vec<u8>, Vec<u8>) {
    let shape = rng.random_range(0..10_u32);
    match shape {
        0..5 => {
            let key: [u8; 32] = rng.random();
            let val_len = rng.random_range(1..=31_usize);
            let val: Vec<u8> = (0..val_len).map(|_| rng.random()).collect();
            (key.to_vec(), val)
        }
        5..8 => {
            let key: [u8; 32] = rng.random();
            let val_len = rng.random_range(32..=128_usize);
            let val: Vec<u8> = (0..val_len).map(|_| rng.random()).collect();
            (key.to_vec(), val)
        }
        _ => {
            let key_len = rng.random_range(1..=4096_usize);
            let val_len = rng.random_range(1..=4096_usize);
            let key: Vec<u8> = (0..key_len).map(|_| rng.random()).collect();
            let val: Vec<u8> = (0..val_len).map(|_| rng.random()).collect();
            (key, val)
        }
    }
}

/// Pick a random index of a `BatchOp` matching the given variant pattern.
///
/// Used inside the mutation inner loop. `continue`s to the next iteration
/// when no op matches (e.g. picking a Put index when only Deletes remain).
/// Must be invoked directly inside the inner `for`/`loop`. A closure or
/// helper function would intercept the `continue`.
macro_rules! pick_op_index {
    ($rng:expr, $ops:expr, $variant:pat) => {{
        let indices: Vec<usize> = $ops
            .iter()
            .enumerate()
            .filter(|(_, op)| matches!(op, $variant))
            .map(|(i, _)| i)
            .collect();
        if indices.is_empty() {
            continue;
        }
        indices[$rng.random_range(0..indices.len())]
    }};
}

/// Destructure a proof into the three owned parts that the mutation
/// cases work with: start-proof nodes, end-proof nodes, and batch ops.
macro_rules! proof_parts {
    ($proof:expr) => {{
        let start_nodes: Vec<ProofNode> = $proof.start_proof().as_ref().to_vec();
        let end_nodes: Vec<ProofNode> = $proof.end_proof().as_ref().to_vec();
        let ops: Vec<BatchOp<Key, Value>> = $proof.batch_ops().to_vec();
        (start_nodes, end_nodes, ops)
    }};
}

/// Clear the first present child-hash slot in `$node`, or `continue` the
/// inner loop if no child hashes are present. Must be invoked directly
/// inside the inner `for`/`loop`. See [`pick_op_index`] for the caveat.
macro_rules! clear_first_child_hash {
    ($node:expr) => {{
        let mut found = false;
        for pc in PathComponent::ALL {
            if $node.child_hashes[pc].is_some() {
                $node.child_hashes.replace(pc, None);
                found = true;
                break;
            }
        }
        if !found {
            continue;
        }
    }};
}

/// Apply `$action` to a present child hash in `$node`, scanning from a
/// random starting offset so the choice is uniformly distributed across
/// the present slots. `continue`s the inner loop if no child hashes are
/// present. Must be invoked directly inside the inner `for`/`loop`.
/// See [`pick_op_index`] for the caveat.
macro_rules! mutate_random_child_hash {
    ($rng:expr, $node:expr, |$h:ident| $action:expr) => {{
        let mut found = false;
        let start_pc = $rng.random_range(0..16_u8);
        for offset in 0..16_u8 {
            // `% 16` confines the index to the 16-slot child array.
            // `wrapping_add` quiets `clippy::arithmetic_side_effects`; the
            // sum can never actually wrap because both operands are < 16.
            let pc_idx = start_pc.wrapping_add(offset) % 16;
            let pc = PathComponent::ALL[pc_idx as usize];
            if let Some(ref mut $h) = $node.child_hashes[pc] {
                $action;
                found = true;
                break;
            }
        }
        if !found {
            continue;
        }
    }};
}

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

/// 10% of proofs are serialized and deserialized before verification.
/// In merkledb mode, values >= 32 bytes are converted to `ValueDigest::Hash`
/// during serialization, exercising the Hash reconciliation path in
/// `reconcile_branch_proof_node` and the value digest fallback in
/// `compute_root_hash_with_proofs`.
///
/// See `test_range_proof_with_hashed_value` and related tests in `range.rs`
/// for deterministic coverage of the Hash path.
fn maybe_round_trip(
    rng: &firewood_storage::SeededRng,
    proof: FrozenChangeProof,
) -> FrozenChangeProof {
    if rng.random_range(0..10_u32) == 0 {
        let mut buf = Vec::new();
        proof.write_to_vec(&mut buf);
        FrozenChangeProof::from_slice(&buf).expect("round-trip deserialization should succeed")
    } else {
        proof
    }
}

/// Loop-local state threaded into panic messages so a CI failure
/// carries enough context to reproduce the iteration and identify which
/// mutation slipped through.
struct TestLocator<'a> {
    seed: u64,
    run: usize,
    /// The 0..100 boundary-scenario draw — see [`pick_proof_for_scenario`].
    scenario: u32,
    /// The 0..100 mutation-group draw.
    group: u32,
    /// The specific mutation that was applied (e.g. `"M7_spurious_delete"`).
    mutation: &'a str,
}

impl std::fmt::Display for TestLocator<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "seed={}, run={}, scenario={}, group={}, mutation={}",
            self.seed, self.run, self.scenario, self.group, self.mutation,
        )
    }
}

/// A change proof generated for a boundary scenario, together with the
/// boundary keys that produced it. `scenario_desc` carries a short
/// label for debug logging and panic messages; `start_key` and `end_key`
/// are `None` only for the no-bounds scenario.
struct ProofWithBounds {
    proof: FrozenChangeProof,
    start_key: Option<Vec<u8>>,
    end_key: Option<Vec<u8>>,
    scenario_desc: &'static str,
}

/// The primary trie under test. Holds the `start_root`/`end_root` roots
/// every change proof is generated for, plus a 3rd revision
/// (`replay_root`) used as a wrong end-root in M29.
struct PrimaryTrie {
    db: Db,
    /// Held to keep the temp directory alive while `db` is in use.
    _dir: tempfile::TempDir,
    start_root: HashKey,
    end_root: HashKey,
    replay_root: HashKey,
    /// Sorted, deduplicated keys committed to `start_root`. Used as
    /// the source for M7 spurious-delete candidates.
    start_keys: Vec<Vec<u8>>,
    /// Sorted, deduplicated keys present in `end_root`. Used as the
    /// source for boundary-key selection in every scenario.
    end_keys: Vec<Vec<u8>>,
}

/// An independent trie used by the M31 cross-DB replay mutation to
/// produce a valid-looking proof against an unrelated state.
struct CrossDbTrie {
    db: Db,
    /// Held to keep the temp directory alive while `db` is in use.
    _dir: tempfile::TempDir,
    start_root: HashKey,
    end_root: HashKey,
}

/// State produced once per outer iteration: a `PrimaryTrie` whose
/// `start_root`/`end_root` pair the change proofs are generated for,
/// and an unrelated `CrossDbTrie` used only by the M31 mutation.
struct TrieFixture {
    primary: PrimaryTrie,
    cross_db: CrossDbTrie,
}

/// Construct the per-outer-iteration state: a primary trie (the proof's
/// subject) plus an independent cross-DB trie used only by M31.
fn build_trie_fixture(rng: &firewood_storage::SeededRng) -> TrieFixture {
    TrieFixture {
        primary: build_primary_trie(rng),
        cross_db: build_cross_db_trie(&rng.seeded_fork()),
    }
}

/// Build the primary trie: commit a random start state, apply random
/// deletes and inserts to produce `end_root`, and commit a 3rd revision
/// (`replay_root`) used by M29.
fn build_primary_trie(rng: &firewood_storage::SeededRng) -> PrimaryTrie {
    // ── Build start trie ──────────────────────────────────────────
    // The start trie is the initial committed state. Its root hash is
    // `start_root` and every change proof in this iteration proves the
    // transition `start_root` → `end_root`.
    //
    // `generate_kv` yields a mix of fixed-32-byte and variable-length
    // keys; varlen keys can collide so a dedup is required before we
    // use `start_keys` as the boundary-key source for the M7 mutation.
    let key_count = rng.random_range(16..=2048_u32) as usize;
    let kvs: Vec<(Vec<u8>, Vec<u8>)> = (0..key_count).map(|_| generate_kv(rng)).collect();
    let mut start_keys: Vec<Vec<u8>> = kvs.iter().map(|(k, _)| k.clone()).collect();
    start_keys.sort_unstable();
    start_keys.dedup();

    let dir = tempfile::tempdir().unwrap();
    let db = Db::new(dir.path(), DbConfig::builder().build()).unwrap();

    let start_batch: Vec<BatchOp<&[u8], &[u8]>> = kvs
        .iter()
        .map(|(k, v)| BatchOp::Put {
            key: k.as_ref(),
            value: v.as_ref(),
        })
        .collect();
    db.propose(start_batch).unwrap().commit().unwrap();
    let start_root = db.root_hash().unwrap();
    debug!(
        "start trie: {key_count} input keys, {} unique, start_root={start_root:?}",
        start_keys.len()
    );
    trace!("start trie dump:\n{}", db.dump_to_string().unwrap());

    // ── Build end trie ────────────────────────────────────────────
    // The end trie is the state after applying a random batch of
    // deletes and inserts to the start trie. Its root hash is
    // `end_root`, the target every change proof commits to.
    //
    // Each start key has a ~1-in-7 chance of being deleted (expected
    // ~14%), and 10-50 fresh entries are inserted on top.
    let mut end_batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> = Vec::new();
    let mut deleted_indices = Vec::new();
    for (i, key) in start_keys.iter().enumerate() {
        if rng.random_range(0..7_u32) == 0 {
            end_batch.push(BatchOp::Delete { key: key.clone() });
            deleted_indices.push(i);
        }
    }

    let insert_count = rng.random_range(10..=50_u32) as usize;
    let new_kvs: Vec<(Vec<u8>, Vec<u8>)> = (0..insert_count).map(|_| generate_kv(rng)).collect();
    let new_keys: Vec<Vec<u8>> = new_kvs.iter().map(|(k, _)| k.clone()).collect();
    for (key, val) in &new_kvs {
        end_batch.push(BatchOp::Put {
            key: key.clone(),
            value: val.clone(),
        });
    }

    db.propose(end_batch).unwrap().commit().unwrap();
    let end_root = db.root_hash().unwrap();
    debug!(
        "end trie: deleted={}, inserted={insert_count}, end_root={end_root:?}",
        deleted_indices.len()
    );
    trace!("end trie dump:\n{}", db.dump_to_string().unwrap());

    // ── Build end-state key list ──────────────────────────────────
    // The end-state key list is the sorted, deduplicated set of keys
    // present in the end trie (start keys minus deletions, plus the
    // fresh inserts). The inner loop draws boundary keys from this
    // list to define the range each change proof covers.
    let mut end_keys: Vec<Vec<u8>> = start_keys
        .iter()
        .enumerate()
        .filter(|(i, _)| !deleted_indices.contains(i))
        .map(|(_, k)| k.clone())
        .chain(new_keys.iter().cloned())
        .collect();
    end_keys.sort_unstable();
    end_keys.dedup();

    // ── 3rd revision for replay tests ─────────────────────────────
    // A further commit on top of `end_root`, producing a new root
    // `replay_root`. The M29 mutation substitutes `replay_root` in
    // place of `end_root` during verification; the proof should be
    // rejected because it doesn't commit to this alternate root.
    let extra_kvs: Vec<(Vec<u8>, Vec<u8>)> = (0..10).map(|_| generate_kv(rng)).collect();
    let extra_batch: Vec<BatchOp<&[u8], &[u8]>> = extra_kvs
        .iter()
        .map(|(k, v)| BatchOp::Put {
            key: k.as_ref(),
            value: v.as_ref(),
        })
        .collect();
    db.propose(extra_batch).unwrap().commit().unwrap();
    let replay_root = db.root_hash().unwrap();
    debug!("3rd revision replay_root={replay_root:?}");

    PrimaryTrie {
        db,
        _dir: dir,
        start_root,
        end_root,
        replay_root,
        start_keys,
        end_keys,
    }
}

/// Build an unrelated trie used only as a source of valid-looking
/// change proofs for the M31 cross-DB mutation.
fn build_cross_db_trie(rng: &firewood_storage::SeededRng) -> CrossDbTrie {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::new(dir.path(), DbConfig::builder().build()).unwrap();

    // ── Build start state ─────────────────────────────────────────
    let key_count = rng.random_range(16..=2048_u32) as usize;
    let kvs: Vec<(Vec<u8>, Vec<u8>)> = (0..key_count).map(|_| generate_kv(rng)).collect();
    let batch: Vec<BatchOp<&[u8], &[u8]>> = kvs
        .iter()
        .map(|(k, v)| BatchOp::Put {
            key: k.as_ref(),
            value: v.as_ref(),
        })
        .collect();
    db.propose(batch).unwrap().commit().unwrap();
    let start_root = db.root_hash().unwrap();
    debug!("cross-db start: {key_count} keys, start_root={start_root:?}");

    // ── Build end state ───────────────────────────────────────────
    // 20 fresh entries — enough to ensure non-empty `batch_ops` in the
    // proofs M31 builds, which is its only requirement.
    let extra_kvs: Vec<(Vec<u8>, Vec<u8>)> = (0..20).map(|_| generate_kv(rng)).collect();
    let extra_batch: Vec<BatchOp<&[u8], &[u8]>> = extra_kvs
        .iter()
        .map(|(k, v)| BatchOp::Put {
            key: k.as_ref(),
            value: v.as_ref(),
        })
        .collect();
    db.propose(extra_batch).unwrap().commit().unwrap();
    let end_root = db.root_hash().unwrap();
    debug!("cross-db end: end_root={end_root:?}");

    CrossDbTrie {
        db,
        _dir: dir,
        start_root,
        end_root,
    }
}

/// Pick a boundary scenario and generate a valid change proof for it.
///
/// Returns `None` when the scenario's constraints can't be satisfied for
/// this trie shape (e.g. too few keys to pick distinct indices, or a
/// neighbor key cannot be constructed). The caller should treat `None` as
/// a skip and move to the next iteration.
///
/// `scenario` is a value in `0..100` whose weight bands map to:
/// - `0..36`  — both boundaries existing keys
/// - `36..56` — start boundary is a non-existent neighbor (decreased)
/// - `56..76` — end boundary is a non-existent neighbor (increased)
/// - `76..96` — both boundaries non-existent neighbors
/// - `96..100` — no bounds (whole-trie proof)
#[expect(clippy::arithmetic_side_effects, clippy::too_many_lines)]
fn pick_proof_for_scenario(
    rng: &firewood_storage::SeededRng,
    db: &Db,
    start_root: &HashKey,
    end_root: &HashKey,
    end_keys: &[Vec<u8>],
    scenario: u32,
) -> Option<ProofWithBounds> {
    match scenario {
        // Both boundaries are existing end-state keys. Pick two
        // distinct indices `si < ei` from `end_keys` and use those
        // keys directly as the inclusive bounds.
        0..36 => {
            if end_keys.len() < 2 {
                return None;
            }
            let si = rng.random_range(0..end_keys.len() - 1);
            let ei = rng.random_range(si + 1..end_keys.len());
            let sk = end_keys[si].clone();
            let ek = end_keys[ei].clone();
            let p = db
                .change_proof(
                    start_root.clone(),
                    end_root.clone(),
                    Some(&sk),
                    Some(&ek),
                    None,
                )
                .expect("change_proof should succeed");
            Some(ProofWithBounds {
                proof: p,
                start_key: Some(sk),
                end_key: Some(ek),
                scenario_desc: "both_exist",
            })
        }
        // Start boundary is a non-existent key just below an existing
        // one. Pick an interior `si` (1..len-1) and decrement
        // `end_keys[si]` so the result lands in the gap between
        // `end_keys[si-1]` and `end_keys[si]` — exclusive of both.
        // This forces an exclusion proof at the left boundary.
        36..56 => {
            // `random_range(1..len-1)` requires len >= 3.
            if end_keys.len() < 3 {
                return None;
            }
            let si = rng.random_range(1..end_keys.len() - 1);
            let ei = rng.random_range(si..end_keys.len());
            let decreased = decrease_key_vec(&end_keys[si])?;
            if decreased >= end_keys[si] || (si > 0 && decreased <= end_keys[si - 1]) {
                return None;
            }
            let ek = end_keys[ei].clone();
            let p = db
                .change_proof(
                    start_root.clone(),
                    end_root.clone(),
                    Some(&decreased),
                    Some(&ek),
                    None,
                )
                .expect("change_proof should succeed");
            Some(ProofWithBounds {
                proof: p,
                start_key: Some(decreased),
                end_key: Some(ek),
                scenario_desc: "start_nonexistent",
            })
        }
        // End boundary is a non-existent key just above an existing
        // one. Symmetric to scenario 1: pick `ei` such that
        // incrementing `end_keys[ei]` lands in the gap before
        // `end_keys[ei+1]`. Forces an exclusion proof at the right
        // boundary.
        56..76 => {
            if end_keys.len() < 2 {
                return None;
            }
            let si = rng.random_range(0..end_keys.len() - 1);
            let ei = rng.random_range(si..end_keys.len() - 1);
            // Skip when no usable neighbor above `end_keys[ei]` exists.
            // `?` returns None on an all-0xFF key. The `<=` clause is
            // defensive. The `==` clause skips collision with the in-trie
            // successor.
            let increased = increase_key_vec(&end_keys[ei])?;
            if increased <= end_keys[ei]
                || (ei + 1 < end_keys.len() && increased == end_keys[ei + 1])
            {
                return None;
            }
            let sk = end_keys[si].clone();
            let p = db
                .change_proof(
                    start_root.clone(),
                    end_root.clone(),
                    Some(&sk),
                    Some(&increased),
                    None,
                )
                .expect("change_proof should succeed");
            Some(ProofWithBounds {
                proof: p,
                start_key: Some(sk),
                end_key: Some(increased),
                scenario_desc: "end_nonexistent",
            })
        }
        // Both boundaries are non-existent neighbor keys. Combines the
        // constraints of scenarios 1 and 2: forces exclusion proofs at
        // both ends simultaneously.
        76..96 => {
            // `random_range(1..len-1)` requires len >= 3.
            if end_keys.len() < 3 {
                return None;
            }
            let si = rng.random_range(1..end_keys.len() - 1);
            let ei = rng.random_range(si..end_keys.len() - 1);
            // Skip when no usable neighbor exists on either side.
            // `?` returns None on all-zero or all-0xFF keys. The `>=`/`<=`
            // clauses are defensive. The `==` clauses skip collisions with
            // the in-trie predecessor and successor.
            let decreased = decrease_key_vec(&end_keys[si])?;
            let increased = increase_key_vec(&end_keys[ei])?;
            if decreased >= end_keys[si]
                || (si > 0 && decreased == end_keys[si - 1])
                || increased <= end_keys[ei]
                || (ei + 1 < end_keys.len() && increased == end_keys[ei + 1])
            {
                return None;
            }
            let p = db
                .change_proof(
                    start_root.clone(),
                    end_root.clone(),
                    Some(&decreased),
                    Some(&increased),
                    None,
                )
                .expect("change_proof should succeed");
            Some(ProofWithBounds {
                proof: p,
                start_key: Some(decreased),
                end_key: Some(increased),
                scenario_desc: "both_nonexistent",
            })
        }
        // No bounds — proof covers the entire key space.
        _ => {
            let p = db
                .change_proof(start_root.clone(), end_root.clone(), None, None, None)
                .expect("change_proof should succeed");
            Some(ProofWithBounds {
                proof: p,
                start_key: None,
                end_key: None,
                scenario_desc: "no_bounds",
            })
        }
    }
}

/// Run structural + root-hash verification on a mutated proof and panic
/// if either accepts it.
///
/// A mutation may be rejected by the structural check (the proof shape
/// itself is invalid) or by the root-hash check (the proof shape is
/// valid but doesn't commit to `end_root`). Either rejection counts.
///
/// `locator` is interpolated into the panic message so a CI failure
/// carries enough state to reproduce.
fn assert_proof_rejected(
    db: &Db,
    mutated_proof: &FrozenChangeProof,
    start_root: HashKey,
    end_root: HashKey,
    start_key: Option<&[u8]>,
    end_key: Option<&[u8]>,
    locator: &TestLocator<'_>,
) {
    debug!(
        "mutation={} \
         mutated_batch_ops={} mutated_start_proof={} mutated_end_proof={}",
        locator.mutation,
        mutated_proof.batch_ops().len(),
        mutated_proof.start_proof().as_ref().len(),
        mutated_proof.end_proof().as_ref().len(),
    );
    trace!("mutated batch_ops: {:?}", mutated_proof.batch_ops());
    trace!(
        "mutated start_proof: {:?}",
        mutated_proof.start_proof().as_ref()
    );
    trace!(
        "mutated end_proof: {:?}",
        mutated_proof.end_proof().as_ref()
    );

    let structural_result =
        verify_change_proof_structure(mutated_proof, end_root.clone(), start_key, end_key, None);
    let rejected = match &structural_result {
        Err(e) => {
            debug!("rejected by structural check: {e}");
            true
        }
        Ok(ctx) => {
            let root_result = verify_and_check(db, mutated_proof, ctx, start_root);
            if let Err(e) = &root_result {
                debug!("rejected by root hash check: {e}");
                true
            } else {
                debug!(
                    "NOT REJECTED — mutation {} passed verification!",
                    locator.mutation
                );
                false
            }
        }
    };

    assert!(
        rejected,
        "proof was NOT rejected! {locator}, \
         batch_ops={}, start_proof_len={}, end_proof_len={}, \
         structural_ok={}, end_root={end_root:?}",
        mutated_proof.batch_ops().len(),
        mutated_proof.start_proof().as_ref().len(),
        mutated_proof.end_proof().as_ref().len(),
        structural_result.is_ok(),
    );
}

#[test]
#[expect(clippy::arithmetic_side_effects, clippy::too_many_lines)]
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

    // Each outer iteration tests one randomly-generated pair of
    // committed tries (start_root, end_root): it builds the fixture
    // once, then runs 50 inner iterations. Each inner iteration picks
    // a boundary scenario, generates a change proof, verifies the
    // proof is accepted, and then verifies a mutated copy is rejected.
    for (run, &seed) in seeds.iter().enumerate() {
        eprintln!("run {run}: seed={seed} (export FIREWOOD_TEST_SEED={seed} to reproduce)");
        let rng = firewood_storage::SeededRng::new(seed);

        let fixture = build_trie_fixture(&rng);
        let TrieFixture { primary, cross_db } = &fixture;
        let PrimaryTrie {
            db,
            start_root,
            end_root,
            replay_root,
            start_keys,
            end_keys,
            ..
        } = primary;
        let CrossDbTrie {
            db: db2,
            start_root: db2_start_root,
            end_root: db2_end_root,
            ..
        } = cross_db;

        // ── Inner loop: positive verification + mutation rejection ────
        for _ in 0..50 {
            let scenario = rng.random_range(0..100_u32);

            let Some(ProofWithBounds {
                proof,
                start_key,
                end_key,
                scenario_desc,
            }) = pick_proof_for_scenario(&rng, db, start_root, end_root, end_keys, scenario)
            else {
                continue;
            };

            debug!(
                "scenario={scenario}({scenario_desc}) start_key={} end_key={} \
                 batch_ops={} start_proof={} end_proof={}",
                start_key.as_deref().map_or("None".into(), hex::encode),
                end_key.as_deref().map_or("None".into(), hex::encode),
                proof.batch_ops().len(),
                proof.start_proof().as_ref().len(),
                proof.end_proof().as_ref().len(),
            );
            trace!("batch_ops: {:?}", proof.batch_ops());
            trace!("start_proof: {:?}", proof.start_proof().as_ref());
            trace!("end_proof: {:?}", proof.end_proof().as_ref());

            // ── Positive verification ─────────────────────────────────
            // Apply optional serialize round-trip (10% of proofs) before
            // positive verification. In merkledb mode, values >= 32 bytes
            // get converted to ValueDigest::Hash during serialization,
            // exercising the Hash reconciliation path.
            let proof = maybe_round_trip(&rng, proof);
            let positive_ctx = verify_change_proof_structure(
                &proof,
                end_root.clone(),
                start_key.as_deref(),
                end_key.as_deref(),
                None,
            )
            .unwrap_or_else(|e| {
                panic!(
                    "positive structural check should pass: \
                     seed={seed}, run={run}, scenario={scenario}({scenario_desc}): {e}"
                )
            });
            verify_and_check(db, &proof, &positive_ctx, start_root.clone()).unwrap_or_else(|e| {
                panic!(
                    "positive root hash check should pass: \
                     seed={seed}, run={run}, scenario={scenario}({scenario_desc}): {e}"
                )
            });

            let group = rng.random_range(0..100_u32);

            // The verification context defaults to the proof's own bounds
            // and roots. Specific mutations override:
            //   - `use_start_root`: M30 only.
            //   - `use_end_root`:   M29 and M30.
            //   - `use_start_key`/`use_end_key`: the combined block only.
            // All other arms leave these at their defaults.
            let mutation_name: &'static str;
            let mutated_proof: FrozenChangeProof;
            let mut use_start_key: Option<Vec<u8>> = start_key.clone();
            let mut use_end_key: Option<Vec<u8>> = end_key.clone();
            let mut use_end_root = end_root.clone();
            let mut use_start_root = start_root.clone();

            match group {
                // ── Batch op mutations (30%) ──────────────────────────────────────────
                0..30 => {
                    let (start_nodes, end_nodes, mut ops) = proof_parts!(proof);
                    if ops.is_empty() {
                        continue;
                    }

                    let sub = rng.random_range(0..7_u32);
                    match sub {
                        // M1: Omit a Put
                        0 => {
                            let idx = pick_op_index!(rng, ops, BatchOp::Put { .. });
                            ops.remove(idx);
                            mutation_name = "M1_omit_put";
                        }
                        // M2: Omit a Delete
                        1 => {
                            let idx = pick_op_index!(rng, ops, BatchOp::Delete { .. });
                            ops.remove(idx);
                            mutation_name = "M2_omit_delete";
                        }
                        // M3: Swap a value
                        2 => {
                            let idx = pick_op_index!(rng, ops, BatchOp::Put { .. });
                            let key = ops[idx].key().clone();
                            let old_val = match &ops[idx] {
                                BatchOp::Put { value, .. } => value.clone(),
                                _ => unreachable!(),
                            };
                            // XOR the first 20 bytes of `old_val` with a
                            // non-zero mask. If `old_val` is longer than
                            // 20 bytes, the result is truncated to 20 —
                            // still a "different value", so the mutation
                            // remains valid.
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
                            trace!(
                                "M3: key={} old_val={} new_val={}",
                                hex::encode(&key),
                                hex::encode(&old_val),
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
                            let idx = pick_op_index!(rng, ops, BatchOp::Put { .. });
                            let key = ops[idx].key().clone();
                            ops[idx] = BatchOp::Delete { key };
                            mutation_name = "M4_put_to_delete";
                        }
                        // M5: Delete → Put
                        4 => {
                            let idx = pick_op_index!(rng, ops, BatchOp::Delete { .. });
                            let key = ops[idx].key().clone();
                            let new_val: [u8; 20] = rng.random();
                            trace!(
                                "M5: key={} new_val={}",
                                hex::encode(&key),
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
                            let sk = start_key.as_deref();
                            let ek = end_key.as_deref();
                            let candidates: Vec<&Vec<u8>> = end_keys
                                .iter()
                                .filter(|k| {
                                    let in_range = sk.is_none_or(|s| k.as_slice() >= s)
                                        && ek.is_none_or(|e| k.as_slice() <= e);
                                    in_range
                                        && start_keys.binary_search(*k).is_ok()
                                        && !ops.iter().any(|op| op.key().as_ref() == k.as_slice())
                                })
                                .collect();
                            if candidates.is_empty() {
                                continue;
                            }
                            let new_key = candidates[rng.random_range(0..candidates.len())].clone();
                            let pos = ops
                                .binary_search_by(|op| op.key().as_ref().cmp(new_key.as_slice()))
                                .unwrap_or_else(|i| i);
                            ops.insert(
                                pos,
                                BatchOp::Delete {
                                    key: new_key.into_boxed_slice(),
                                },
                            );
                            mutation_name = "M7_spurious_delete";
                        }
                    }

                    mutated_proof = build_change_proof(start_nodes, end_nodes, ops);
                }

                // ── Exclusion proof attacks (25%) ───────────────────────────────
                30..55 => {
                    let (start_nodes, end_nodes, ops) = proof_parts!(proof);

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
                                clear_first_child_hash!(last);
                                mutated_proof = build_change_proof(nodes, end_nodes, ops);
                                mutation_name = "M21_clear_child_start";
                            } else {
                                let mut nodes = end_nodes.clone();
                                let last = nodes.last_mut().unwrap();
                                clear_first_child_hash!(last);
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
                            mutate_random_child_hash!(rng, last, |h| flip_hash_bit(&rng, h));
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
                            mutate_random_child_hash!(rng, node, |h| flip_hash_bit(&rng, h));
                            mutated_proof = build_change_proof(nodes, end_nodes, ops);
                            mutation_name = "M25_corrupt_shared_node";
                        }
                    }
                }

                // ── Out-of-range structural (15%) ───────────────────────────────
                55..70 => {
                    let (start_nodes, end_nodes, ops) = proof_parts!(proof);

                    let sub = rng.random_range(0..2_u32);
                    match sub {
                        // M27: Corrupt out-of-range sibling hash
                        0 => {
                            if !start_nodes.is_empty() {
                                let mut nodes = start_nodes.clone();
                                let idx = rng.random_range(0..nodes.len());
                                let node = &mut nodes[idx];
                                mutate_random_child_hash!(rng, node, |h| flip_hash_bit(&rng, h));
                                mutated_proof = build_change_proof(nodes, end_nodes, ops);
                                mutation_name = "M27_corrupt_sibling_start";
                            } else if !end_nodes.is_empty() {
                                let mut nodes = end_nodes.clone();
                                let idx = rng.random_range(0..nodes.len());
                                let node = &mut nodes[idx];
                                mutate_random_child_hash!(rng, node, |h| flip_hash_bit(&rng, h));
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

                // ── Replay / cross-revision (10%) ───────────────────────────────
                70..80 => {
                    // Need non-empty batch_ops for meaningful cross-revision checks.
                    if proof.batch_ops().len() < 2 {
                        continue;
                    }
                    let (start_nodes, end_nodes, ops) = proof_parts!(proof);

                    let sub = rng.random_range(0..3_u32);
                    match sub {
                        // M29: Verify the proof against `replay_root` (a
                        // later revision of `db`) instead of `end_root`.
                        // The proof commits to `end_root`, so the mismatch
                        // is the attack.
                        0 => {
                            mutated_proof = build_change_proof(start_nodes, end_nodes, ops);
                            use_end_root = replay_root.clone();
                            mutation_name = "M29_wrong_end_root";
                        }
                        // M30: Swap start_root and end_root for
                        // verification. The proof commits to the original
                        // (start_root, end_root) direction; the reversed
                        // pair must be detected as inconsistent.
                        1 => {
                            mutated_proof = build_change_proof(start_nodes, end_nodes, ops);
                            use_start_root = end_root.clone();
                            use_end_root = start_root.clone();
                            mutation_name = "M30_reversed_roots";
                        }
                        // M31: Build a proof against `db2` (an unrelated
                        // trie) committing to `db2_end_root`. Leaving
                        // `use_end_root` at the default `end_root` makes
                        // the verifier check the proof against the wrong
                        // root; the mismatch is the attack.
                        _ => {
                            if let Ok(other_proof) = db2.change_proof(
                                db2_start_root.clone(),
                                db2_end_root.clone(),
                                start_key.as_deref(),
                                end_key.as_deref(),
                                None,
                            ) {
                                if other_proof.batch_ops().len() < 2 {
                                    continue;
                                }
                                let (s, e, o) = proof_parts!(other_proof);
                                mutated_proof = build_change_proof(s, e, o);
                                mutation_name = "M31_wrong_db";
                            } else {
                                continue;
                            }
                        }
                    }
                }

                // ── Boundary proof structural (5%) ──────────────────────────────
                80..85 => {
                    // Empty batch_ops means no in-range state to verify,
                    // so proof mutations are harmless.
                    if proof.batch_ops().is_empty() {
                        continue;
                    }
                    let (start_nodes, end_nodes, ops) = proof_parts!(proof);

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

                // ── Combined: force double-exclusion + corruption (15%) ─
                _ => {
                    if end_keys.len() < 3 {
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
                    let double_proof = db
                        .change_proof(
                            start_root.clone(),
                            end_root.clone(),
                            Some(&decreased),
                            Some(&increased),
                            None,
                        )
                        .expect("change_proof should succeed");

                    use_start_key = Some(decreased);
                    use_end_key = Some(increased);

                    let (start_nodes, end_nodes, ops) = proof_parts!(double_proof);

                    if start_nodes.is_empty() || end_nodes.is_empty() {
                        continue;
                    }

                    let sub = rng.random_range(0..3_u32);
                    match sub {
                        // M32: Truncate the double-exclusion start proof
                        0 => {
                            if start_nodes.len() < 2 {
                                continue;
                            }
                            let mut nodes = start_nodes;
                            nodes.pop();
                            mutated_proof = build_change_proof(nodes, end_nodes, ops);
                            mutation_name = "M32_combined_truncate_start";
                        }
                        // M33: Corrupt a child hash in the end proof
                        1 => {
                            let mut nodes = end_nodes;
                            let idx = rng.random_range(0..nodes.len());
                            let node = &mut nodes[idx];
                            mutate_random_child_hash!(rng, node, |h| flip_hash_bit(&rng, h));
                            mutated_proof = build_change_proof(start_nodes, nodes, ops);
                            mutation_name = "M33_combined_corrupt_end_child";
                        }
                        // M34: Remove a value from a start-proof node
                        _ => {
                            let idx = start_nodes.iter().position(|n| n.value_digest.is_some());
                            if let Some(idx) = idx {
                                let mut nodes = start_nodes;
                                nodes[idx].value_digest = None;
                                mutated_proof = build_change_proof(nodes, end_nodes, ops);
                                mutation_name = "M34_combined_remove_value";
                            } else {
                                continue;
                            }
                        }
                    }
                }
            }

            // ── Assert rejection ──────────────────────────────────────
            let locator = TestLocator {
                seed,
                run,
                scenario,
                group,
                mutation: mutation_name,
            };
            assert_proof_rejected(
                db,
                &mutated_proof,
                use_start_root,
                use_end_root,
                use_start_key.as_deref(),
                use_end_key.as_deref(),
                &locator,
            );
        }
    }
}
