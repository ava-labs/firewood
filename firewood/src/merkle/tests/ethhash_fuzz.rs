// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Ethhash-shaped proof fuzzer.
//!
//! Generic proof fuzzers only hit the depth-64 account and per-account-storage
//! structures incidentally. This fuzzer *deliberately* generates Ethereum-shaped
//! state (accounts at depth 64 with RLP `[nonce, balance, storageRoot, codeHash]`
//! values and per-account storage tries), so the storage-trie-root fold and the
//! partial-storage reconcile are exercised on every iteration, across both range
//! and change proofs.
//!
//! Coverage: random shaped state, a mix of K=1 (fold) and K>=2 (partial-storage)
//! accounts, verified across several boundary scenarios for both range and
//! change proofs. Each scenario also tampers the proof two ways (forged value,
//! dropped interior entry) and asserts the verifier rejects both.

use std::collections::{BTreeMap, HashSet};

use super::change::verify_and_check;
use super::ethhash::{
    account_storage_key, empty_code_hash, rlp_encode_account, rlp_encode_storage,
};
use crate::api::{
    BatchOp, Db as DbTrait, DbView, FrozenChangeProof, FrozenRangeProof, HashKey, Proposal as _,
};
use crate::db::{Db, DbConfig};
use crate::merkle::{verify_change_proof_root_hash, verify_range_proof};
use crate::{ChangeProof, Proof, verify_change_proof_structure};
use firewood_storage::SeededRng;
use rand::seq::SliceRandom;

/// Placeholder storageRoot written into every account value; live hashing
/// recomputes the real one from the account's storage children.
const DUMMY_STORAGE_ROOT: [u8; 32] = [0u8; 32];

/// An account's key paired with its sorted storage-slot bytes (byte 32 of each
/// of its 64-byte storage keys).
type AccountSlots = ([u8; 32], Box<[u8]>);

/// A randomly generated Ethereum-shaped fixture: a committed start state, a
/// committed end state, and the sorted key set of the end state (for boundary
/// selection).
struct ShapedFixture {
    db: Db,
    /// Keeps the temp dir alive while `db` is in use.
    _dir: tempfile::TempDir,
    start_root: HashKey,
    end_root: HashKey,
    /// Sorted, deduplicated keys present in the end state.
    end_keys: Box<[Box<[u8]>]>,
    /// For each account in the end state, its sorted storage-slot bytes.
    storage_layout: Box<[AccountSlots]>,
}

/// Pick `k` distinct storage-child high nibbles (0..16): shuffle all 16 and
/// take the first `k`. Each lands in a distinct account-branch child slot, so
/// `k` is the storage-child count the fold keys off.
fn pick_distinct_nibbles(rng: &SeededRng, k: usize) -> Vec<u8> {
    let mut nibbles: Vec<u8> = (0u8..16).collect();
    nibbles.shuffle(&mut &*rng);
    nibbles.truncate(k);
    nibbles
}

/// RLP-encode an account value with random nonce/balance and the placeholder
/// storageRoot.
fn random_account_value(rng: &SeededRng, code_hash: &[u8; 32]) -> Vec<u8> {
    let nonce = u64::from(rng.random_range(0..1_000_000_u32));
    let balance = u64::from(rng.random_range(0..1_000_000_u32));
    rlp_encode_account(nonce, balance, &DUMMY_STORAGE_ROOT, code_hash).to_vec()
}

/// Build a random Ethereum-shaped DB: several accounts, each with a random
/// storage-child count, *guaranteeing* at least one K=1 account (forces the
/// storage-trie-root fold) and at least one K≥2 account (enables partial-storage
/// reconcile). Then apply a random batch of storage/account mutations to produce
/// the end state.
#[expect(
    clippy::too_many_lines,
    reason = "sequential fixture builder; splitting would only thread state through single-use helpers"
)]
fn build_shaped_db(rng: &SeededRng) -> ShapedFixture {
    let code_hash = empty_code_hash();

    // ── Distinct, sorted account keys ─────────────────────────────
    let n_accounts = rng.random_range(4..=10);
    // Distinct keys in (random) generation order, so the index-0 and index-1
    // anchors picked below sit at random positions in the key space.
    let mut accounts: Vec<[u8; 32]> = Vec::with_capacity(n_accounts);
    while accounts.len() < n_accounts {
        let key: [u8; 32] = rng.random();
        if !accounts.contains(&key) {
            accounts.push(key);
        }
    }

    // ── Start state (account + storage children per account) ──────
    // `state` is the source of truth; the start batch is committed from it and
    // the end key set is read back from it after mutation.
    let mut state: BTreeMap<Vec<u8>, Vec<u8>> = BTreeMap::new();
    // The targeted accounts whose child counts the end-state mutations must
    // preserve, recorded as (account key, storage-slot bytes).
    let mut k1_account: Option<([u8; 32], u8)> = None;
    let mut k2_account: Option<([u8; 32], Vec<u8>)> = None;
    for (i, account) in accounts.iter().enumerate() {
        state.insert(account.to_vec(), random_account_value(rng, &code_hash));
        // Account 0 gets exactly one storage child (fold), account 1 at least
        // two (partial-storage); the rest are random.
        let k = match i {
            0 => 1,
            1 => rng.random_range(2..=6),
            _ => rng.random_range(0..=6),
        };
        let slots: Vec<u8> = pick_distinct_nibbles(rng, k)
            .iter()
            .map(|&n| n << 4)
            .collect();
        for &slot in &slots {
            let storage_key = account_storage_key(account, slot).to_vec();
            state.insert(storage_key, rlp_encode_storage(&rng.random::<[u8; 32]>()));
        }
        match i {
            0 => k1_account = Some((*account, slots[0])),
            1 => k2_account = Some((*account, slots)),
            _ => {}
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let db = Db::new(dir.path(), DbConfig::builder().build()).unwrap();
    let start_batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> = state
        .iter()
        .map(|(k, v)| BatchOp::Put {
            key: k.clone(),
            value: v.clone(),
        })
        .collect();
    db.propose(start_batch).unwrap().commit().unwrap();
    let start_root = db.root_hash().unwrap();

    // ── End state: a deduped batch of random mutations ────────────
    // A BTreeMap keyed by the trie key gives a sorted, one-op-per-key delta
    // (`None` = delete), which is what `propose` and the proofs expect. The
    // mutations preserve the targeted accounts' K=1 / K≥2 child counts so the
    // fold and storage-cut scenarios always have an eligible account.

    // Account/storage keys whose deletion would change a protected count.
    let mut protected: HashSet<Vec<u8>> = HashSet::new();
    if let Some((account, slot)) = &k1_account {
        protected.insert(account.to_vec());
        protected.insert(account_storage_key(account, *slot).to_vec());
    }
    if let Some((account, slots)) = &k2_account {
        protected.insert(account.to_vec());
        for &slot in slots {
            protected.insert(account_storage_key(account, slot).to_vec());
        }
    }
    let k1_acct = k1_account.as_ref().map(|(account, _)| *account);

    let n_mut: usize = rng.random_range(3..=12);
    let mut delta: BTreeMap<Vec<u8>, Option<Vec<u8>>> = BTreeMap::new();
    let existing_keys: Vec<Vec<u8>> = state.keys().cloned().collect();
    for _ in 0..n_mut {
        match rng.random_range(0..3_u32) {
            // Add (or overwrite) a storage slot under a random account, but not
            // the K=1 account (a new child would break its count) and not an
            // account this delta already deleted (no orphaned storage).
            0 => {
                let account = accounts[rng.random_range(0..accounts.len())];
                if Some(account) == k1_acct || matches!(delta.get(account.as_slice()), Some(None)) {
                    continue;
                }
                let nibble: u8 = rng.random_range(0..16);
                let storage_key = account_storage_key(&account, nibble << 4).to_vec();
                delta.insert(
                    storage_key,
                    Some(rlp_encode_storage(&rng.random::<[u8; 32]>())),
                );
            }
            // Modify a random account value (no child-count change).
            1 => {
                let account = accounts[rng.random_range(0..accounts.len())];
                delta.insert(
                    account.to_vec(),
                    Some(random_account_value(rng, &code_hash)),
                );
            }
            // Delete a random existing key, skipping the protected ones.
            // Deleting an account also deletes its storage children (both
            // pre-existing and ones added earlier in this delta), so the end
            // state never holds storage under a missing account.
            _ => {
                let key = existing_keys[rng.random_range(0..existing_keys.len())].clone();
                if protected.contains(&key) {
                    continue;
                }
                if key.len() == 32 {
                    let children: Vec<Vec<u8>> = existing_keys
                        .iter()
                        .chain(delta.keys())
                        .filter(|k| k.len() == 64 && k.starts_with(&key))
                        .cloned()
                        .collect();
                    for child in children {
                        delta.insert(child, None);
                    }
                }
                delta.insert(key, None);
            }
        }
    }

    // Keep the targeted accounts in the change set so the change-proof
    // fold/reconcile paths fire, without altering their counts: re-value the
    // K=1 account's child and one of the K≥2 account's children.
    if let Some((account, slot)) = &k1_account {
        delta.insert(
            account_storage_key(account, *slot).to_vec(),
            Some(rlp_encode_storage(&rng.random::<[u8; 32]>())),
        );
    }
    if let Some((account, slots)) = &k2_account {
        let slot = slots[rng.random_range(0..slots.len())];
        delta.insert(
            account_storage_key(account, slot).to_vec(),
            Some(rlp_encode_storage(&rng.random::<[u8; 32]>())),
        );
    }

    let mut end_batch: Vec<BatchOp<Vec<u8>, Vec<u8>>> = Vec::new();
    for (key, op) in &delta {
        if let Some(value) = op {
            state.insert(key.clone(), value.clone());
            end_batch.push(BatchOp::Put {
                key: key.clone(),
                value: value.clone(),
            });
        } else {
            state.remove(key);
            end_batch.push(BatchOp::Delete { key: key.clone() });
        }
    }
    // `delta` is never empty: the anchor re-values above always add a Put.
    db.propose(end_batch).unwrap().commit().unwrap();
    let end_root = db.root_hash().unwrap();

    // Record each account's storage slots so the fold and storage-cut scenarios
    // can aim a boundary inside a chosen account's storage trie (the whole point
    // of a shaped fuzzer). A storage key is `account(32) ++ [slot, 0...]`, so
    // byte 32 is its slot; iterating the BTreeMap yields each account's slots
    // already sorted.
    let mut layout: BTreeMap<[u8; 32], Vec<u8>> = BTreeMap::new();
    for key in state.keys() {
        if key.len() == 64 {
            let account: [u8; 32] = key[0..32].try_into().unwrap();
            layout.entry(account).or_default().push(key[32]);
        }
    }

    ShapedFixture {
        db,
        _dir: dir,
        start_root,
        end_root,
        end_keys: state.keys().map(|k| k.clone().into_boxed_slice()).collect(),
        storage_layout: layout
            .into_iter()
            .map(|(account, slots)| (account, slots.into_boxed_slice()))
            .collect(),
    }
}

/// Bounds that cut strictly inside some account's storage trie, between two
/// adjacent present storage slots, so a proof over `[start, end]` holds a
/// proper subset of that account's storage children (the partial-storage
/// trigger). `None` if no account has >= 2 storage children.
fn storage_cut_bounds(rng: &SeededRng, layout: &[AccountSlots]) -> Option<(Vec<u8>, Vec<u8>)> {
    let multi: Box<[&AccountSlots]> = layout.iter().filter(|e| e.1.len() >= 2).collect();
    if multi.is_empty() {
        return None;
    }
    let entry = multi[rng.random_range(0..multi.len())];
    // Drop the largest slot, then cut just above one of the rest. Slots are
    // multiples of 0x10, so `lo + 1` is below every higher slot: children at
    // slots <= lo stay in range while at least the largest one is excluded.
    let (_, lower_slots) = entry.1.split_last()?;
    let lo = lower_slots[rng.random_range(0..lower_slots.len())];
    let cut = lo.saturating_add(1);
    Some((
        entry.0.to_vec(),
        account_storage_key(&entry.0, cut).to_vec(),
    ))
}

/// Bounds spanning a single-storage-child account from its account key through
/// its lone child, so the child is in range and the verifier must fold it as a
/// standalone storage-trie root. `None` if no account has exactly one child.
fn fold_bounds(rng: &SeededRng, layout: &[AccountSlots]) -> Option<(Vec<u8>, Vec<u8>)> {
    let singles: Box<[&AccountSlots]> = layout.iter().filter(|e| e.1.len() == 1).collect();
    if singles.is_empty() {
        return None;
    }
    let entry = singles[rng.random_range(0..singles.len())];
    Some((
        entry.0.to_vec(),
        account_storage_key(&entry.0, entry.1[0]).to_vec(),
    ))
}

/// Generate and verify a valid range proof over `root` for the given bounds,
/// returning it for tampering.
fn check_valid_range_proof(
    db: &Db,
    root: &HashKey,
    first: Option<&[u8]>,
    last: Option<&[u8]>,
    locator: &str,
) -> FrozenRangeProof {
    let view = db.revision(root.clone()).unwrap();
    let range_proof = view
        .range_proof(first, last, None)
        .unwrap_or_else(|e| panic!("range_proof should succeed ({locator}): {e}"));
    verify_range_proof(first, last, root, &range_proof)
        .unwrap_or_else(|e| panic!("valid range proof should verify ({locator}): {e}"));
    range_proof
}

/// Generate and verify a valid change proof from `start_root` to `end_root` for
/// the given bounds, returning it for tampering.
fn check_valid_change_proof(
    db: &Db,
    start_root: &HashKey,
    end_root: &HashKey,
    first: Option<&[u8]>,
    last: Option<&[u8]>,
    locator: &str,
) -> FrozenChangeProof {
    let change_proof = db
        .change_proof(start_root.clone(), end_root.clone(), first, last, None)
        .unwrap_or_else(|e| panic!("change_proof should succeed ({locator}): {e}"));
    let ctx = verify_change_proof_structure(&change_proof, end_root.clone(), first, last, None)
        .unwrap_or_else(|e| panic!("valid change proof structure should verify ({locator}): {e}"));
    let parent = db.revision(start_root.clone()).unwrap();
    let proposal = db
        .apply_change_proof_to_parent(&change_proof, &*parent)
        .unwrap_or_else(|e| panic!("apply_change_proof_to_parent should succeed ({locator}): {e}"));
    verify_change_proof_root_hash(&change_proof, &ctx, &proposal)
        .unwrap_or_else(|e| panic!("valid change proof root hash should verify ({locator}): {e}"));
    change_proof
}

/// Whether a tampered change proof is rejected by the structural check or,
/// failing that, the root-hash check (its proposal won't match `end_root`).
fn change_proof_rejected(
    db: &Db,
    tampered: &FrozenChangeProof,
    start_root: &HashKey,
    end_root: &HashKey,
    first: Option<&[u8]>,
    last: Option<&[u8]>,
) -> bool {
    match verify_change_proof_structure(tampered, end_root.clone(), first, last, None) {
        Err(_) => true,
        Ok(ctx) => verify_and_check(db, tampered, &ctx, start_root.clone()).is_err(),
    }
}

/// Tamper a change proof by flipping one `Put`'s value, so the verifier must
/// reject it. `None` if the proof has no `Put`.
///
/// `^= 0xFF` on byte 0 (the RLP prefix) keeps the must-reject assertion sound on
/// two fronts. The non-zero mask guarantees the value actually changes (an
/// unchanged value would still verify). And byte 0 is never an account's
/// `storageRoot` field, which the verifier recomputes from children and accepts
/// even when tampered.
fn forge_a_put_value(proof: &FrozenChangeProof, rng: &SeededRng) -> Option<FrozenChangeProof> {
    let mut ops = proof.batch_ops().to_vec();
    let put_indices: Vec<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, op)| matches!(op, BatchOp::Put { .. }))
        .map(|(i, _)| i)
        .collect();
    if put_indices.is_empty() {
        return None;
    }
    let idx = put_indices[rng.random_range(0..put_indices.len())];
    if let BatchOp::Put { key, value } = &ops[idx] {
        let key = key.clone();
        let mut new_value = value.to_vec();
        if new_value.is_empty() {
            new_value.push(0);
        }
        new_value[0] ^= 0xFF;
        ops[idx] = BatchOp::Put {
            key,
            value: new_value.into(),
        };
    }
    Some(ChangeProof::new(
        Proof::new(proof.start_proof().as_ref().into()),
        Proof::new(proof.end_proof().as_ref().into()),
        ops.into_boxed_slice(),
    ))
}

/// Drop one storage-leaf `Put` (64-byte key) that is neither the first nor the
/// last batch op, so the verifier must reject the now-incomplete change proof.
/// `None` if there is no such op.
///
/// A dropped in-range `Put` must not be silently accepted. The dropped key must
/// be a storage leaf, not an account: an account `Put` that only restates its
/// `storageRoot` is non-authoritative (the verifier recomputes that field from
/// the storage children), so omitting it leaves the root unchanged and is
/// correctly accepted. A storage leaf is authoritative, so omitting it changes
/// the account's recomputed `storageRoot` and the root no longer matches.
/// Interior-only keeps the omission inside the boundary proofs.
fn drop_an_interior_put(proof: &FrozenChangeProof, rng: &SeededRng) -> Option<FrozenChangeProof> {
    let ops = proof.batch_ops();
    if ops.len() < 3 {
        return None;
    }
    let last_idx = ops.len().saturating_sub(1);
    let storage_puts: Box<[usize]> = (1..last_idx)
        .filter(|&i| matches!(&ops[i], BatchOp::Put { key, .. } if key.len() == 64))
        .collect();
    if storage_puts.is_empty() {
        return None;
    }
    let drop_idx = storage_puts[rng.random_range(0..storage_puts.len())];
    let mut kept = ops.to_vec();
    kept.remove(drop_idx);
    Some(ChangeProof::new(
        Proof::new(proof.start_proof().as_ref().into()),
        Proof::new(proof.end_proof().as_ref().into()),
        kept.into_boxed_slice(),
    ))
}

/// Tamper a range proof by flipping one key-value pair's value, so the verifier
/// must reject it. `None` if the proof has no pairs. As in [`forge_a_put_value`],
/// `^= 0xFF` on byte 0 guarantees the value really changes and never touches an
/// account's `storageRoot` field (which the verifier recomputes from children).
fn forge_a_range_value(proof: &FrozenRangeProof, rng: &SeededRng) -> Option<FrozenRangeProof> {
    let mut kvs = proof.key_values().to_vec();
    if kvs.is_empty() {
        return None;
    }
    let idx = rng.random_range(0..kvs.len());
    let mut new_value = kvs[idx].1.to_vec();
    if new_value.is_empty() {
        new_value.push(0);
    }
    new_value[0] ^= 0xFF;
    kvs[idx].1 = new_value.into();
    Some(FrozenRangeProof::new(
        Proof::new(proof.start_proof().as_ref().into()),
        Proof::new(proof.end_proof().as_ref().into()),
        kvs.into_boxed_slice(),
    ))
}

/// Drop one storage-leaf pair (64-byte key) that is neither the first nor the
/// last, so the reconstructed trie is missing an interior leaf and no longer
/// hashes to the root. `None` if there is no such pair. Restricting to a storage
/// leaf keeps the dropped value authoritative: omitting it changes the account's
/// recomputed `storageRoot`, so the root always mismatches. Interior-only keeps
/// the omission inside the boundary proofs.
fn drop_an_interior_kv(proof: &FrozenRangeProof, rng: &SeededRng) -> Option<FrozenRangeProof> {
    let kvs = proof.key_values();
    if kvs.len() < 3 {
        return None;
    }
    let last_idx = kvs.len().saturating_sub(1);
    let storage_kvs: Box<[usize]> = (1..last_idx).filter(|&i| kvs[i].0.len() == 64).collect();
    if storage_kvs.is_empty() {
        return None;
    }
    let drop_idx = storage_kvs[rng.random_range(0..storage_kvs.len())];
    let mut kept = kvs.to_vec();
    kept.remove(drop_idx);
    Some(FrozenRangeProof::new(
        Proof::new(proof.start_proof().as_ref().into()),
        Proof::new(proof.end_proof().as_ref().into()),
        kept.into_boxed_slice(),
    ))
}

#[test]
fn test_slow_ethhash_proof_fuzz() {
    // One rng drives every iteration; `from_env_or_random` prints the seed (set
    // FIREWOOD_TEST_SEED to that value to replay the whole run deterministically).
    let rng = SeededRng::from_env_or_random();

    for run in 0..100 {
        let fixture = build_shaped_db(&rng);
        let ShapedFixture {
            db,
            start_root,
            end_root,
            end_keys,
            storage_layout,
            ..
        } = &fixture;

        let check = |first: Option<&[u8]>, last: Option<&[u8]>, label: &str| {
            let locator = format!("run={run}, scenario={label}");
            // Positive: valid range and change proofs verify.
            let range = check_valid_range_proof(db, end_root, first, last, &locator);
            let change = check_valid_change_proof(db, start_root, end_root, first, last, &locator);

            // Negative: every tamper of the valid change proof must be rejected.
            for (kind, tampered) in [
                ("forge", forge_a_put_value(&change, &rng)),
                ("drop", drop_an_interior_put(&change, &rng)),
            ] {
                if let Some(tampered) = tampered {
                    assert!(
                        change_proof_rejected(db, &tampered, start_root, end_root, first, last),
                        "tampered change proof must be rejected ({kind}, {locator})"
                    );
                }
            }

            // Negative: every tamper of the valid range proof must be rejected.
            for (kind, tampered) in [
                ("forge", forge_a_range_value(&range, &rng)),
                ("drop", drop_an_interior_kv(&range, &rng)),
            ] {
                if let Some(tampered) = tampered {
                    assert!(
                        verify_range_proof(first, last, end_root, &tampered).is_err(),
                        "tampered range proof must be rejected ({kind}, {locator})"
                    );
                }
            }
        };

        // Full range.
        check(None, None, "full");

        // Targeted: a bound cutting inside a multi-child account's storage
        // (partial-storage reconcile), and a range spanning a single-child
        // account's lone child (fold). The protected anchor accounts make both
        // bounds always available; `None` means the fixture builder regressed.
        let (start, end) = storage_cut_bounds(&rng, storage_layout)
            .expect("fixture guarantees an account with at least two storage children");
        check(Some(&start), Some(&end), "storage_cut");
        let (start, end) = fold_bounds(&rng, storage_layout)
            .expect("fixture guarantees an account with exactly one storage child");
        check(Some(&start), Some(&end), "fold");

        // Breadth: the outer existing bounds and a few random existing-key pairs.
        if end_keys.len() >= 2 {
            let first = end_keys.first().unwrap().as_ref();
            let last = end_keys.last().unwrap().as_ref();
            check(Some(first), Some(last), "outer");

            // A random number of random existing-key bound pairs, on top of the
            // targeted (full / outer / storage-cut / fold) scenarios.
            for probe in 0..rng.random_range(1..=4_u32) {
                let i = rng.random_range(0..end_keys.len());
                let j = rng.random_range(0..end_keys.len());
                let (lo, hi) = if i <= j { (i, j) } else { (j, i) };
                check(
                    Some(end_keys[lo].as_ref()),
                    Some(end_keys[hi].as_ref()),
                    &format!("random_bounds_{probe}"),
                );
            }
        }
    }
}
