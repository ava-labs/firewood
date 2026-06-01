// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Empty start trie tests.
//!
//! When the start revision is empty, every key in the end revision is a new
//! insertion. These tests exercise the interaction between empty start tries
//! and the divergent child / value skip logic.

use super::*;
use test_case::test_case;

/// Empty start trie, single key inserted. Complete proof (no bounds).
#[test]
fn test_empty_start_trie_single_key_no_bounds() {
    let (db, _dir) = setup_db![];
    let (empty_root, root2) = setup_2nd_commit!(db, [(b"\x50", b"hello")]);

    let proof = db
        .change_proof(empty_root, root2.clone(), None, None, None)
        .unwrap();
    let ctx = verify_change_proof_structure(&proof, root2.clone(), None, None, None).unwrap();

    let (target, _dir_target) = setup_db![];
    let empty_root_target = target.root_hash().unwrap();

    verify_and_check(&target, &proof, &ctx, empty_root_target).unwrap();
}

/// Empty start trie, multiple keys inserted, bounded range with inclusion
/// start proof (`start_key` exists in end trie).
#[test]
fn test_empty_start_trie_bounded_inclusion() {
    let (db, _dir) = setup_db![];
    let (empty_root, root2) = setup_2nd_commit!(
        db,
        [
            (b"\x10", b"a"),
            (b"\x20", b"b"),
            (b"\x30", b"c"),
            (b"\x40", b"d")
        ]
    );

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

    let (target, _dir_target) = setup_db![];
    let empty_root_target = target.root_hash().unwrap();

    verify_and_check(&target, &proof, &ctx, empty_root_target).unwrap();
}

/// Empty start trie, bounded range with exclusion start proof
/// (`start_key` does NOT exist in end trie).
#[test]
fn test_empty_start_trie_bounded_exclusion() {
    let (db, _dir) = setup_db![];
    let (empty_root, root2) =
        setup_2nd_commit!(db, [(b"\x10", b"a"), (b"\x20", b"b"), (b"\x30", b"c")]);

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

    let (target, _dir_target) = setup_db![];
    let empty_root_target = target.root_hash().unwrap();

    verify_and_check(&target, &proof, &ctx, empty_root_target).unwrap();
}

/// Empty start trie with prefix keys (key is a prefix of another key).
/// Tests that branch nodes with values at prefix keys are handled
/// correctly when the start trie is empty. Includes an empty key (`b""`)
/// which places a value on the root node itself.
#[test]
fn test_empty_start_trie_prefix_keys() {
    let (db, _dir) = setup_db![];
    let (empty_root, root2) = setup_2nd_commit!(
        db,
        [
            (b"", b"root_val"),
            (b"\x10", b"prefix"),
            (b"\x10\x50", b"child1"),
            (b"\x10\x50\xaa", b"grandchild"),
            (b"\x20", b"other"),
        ]
    );

    let (target, _dir_target) = setup_db![];
    let empty_root_target = target.root_hash().unwrap();

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
    verify_and_check(&target, &proof, &ctx, empty_root_target.clone()).unwrap();

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
    verify_and_check(&target, &proof, &ctx, empty_root_target).unwrap();
}

use super::super::ethhash::{empty_code_hash, rlp_encode_account, rlp_encode_storage};

/// Build a 64-byte key whose first 32 bytes are `account_key` and whose
/// 33rd byte is `byte_32` (the rest zero). Used to construct storage keys
/// and range boundaries under a fixed account.
fn key_under_account(account_key: &[u8; 32], byte_32: u8) -> [u8; 64] {
    std::array::from_fn(|i| match i {
        0..=31 => account_key[i],
        32 => byte_32,
        _ => 0,
    })
}

/// Bound choice for the parameterized partial-storage-children test below.
/// Resolved to an `Option<&[u8]>` against the test's fixed `account_key`
/// and a fixed mid-storage probe (`0x40`, between `storage_keys[1]` at
/// 0x30 and `storage_keys[2]` at 0x60).
#[derive(Debug, Clone, Copy)]
enum Bound {
    /// No bound on this side of the range.
    None,
    /// Bound exactly at `account_key` (32 bytes; the start-proof
    /// terminates at the account branch itself).
    AtAccount,
    /// Bound in the middle of the account's storage children, between
    /// storage[1] and storage[2] (64 bytes; the proof reconciliation
    /// runs through the storage trie).
    MidStorage,
}

/// Change proof from empty to a single account with four storage children
/// (at distinct first nibbles 0x10, 0x30, 0x60, 0xC0), bounded so the
/// `batch_ops` cover only a subset of the storage children. Verified against
/// an empty target.
///
/// Each case exercises a different reconciliation path:
/// - `left_half_end_bound_only`: end-proof in-range path.
/// - `left_edge_both_bounds`: start-proof in-range path at the account
///   itself + end-proof in-range path in storage.
/// - `right_half_start_bound_only`: start-proof reconciliation but the
///   account is out-of-range (proof carries it as a boundary node).
///
/// All three triggered `UnexpectedValue` before the
/// `account_values_equal_except_storage_root` relaxation in
/// `reconcile_branch_proof_node`, because the proposal's account value
/// has a partial-state storageRoot while the proof carries the
/// full-state value.
#[test_case(Bound::None, Bound::MidStorage ; "left_half_end_bound_only")]
#[test_case(Bound::AtAccount, Bound::MidStorage ; "left_edge_both_bounds")]
#[test_case(Bound::MidStorage, Bound::None ; "right_half_start_bound_only")]
fn test_change_proof_partial_storage_children_against_empty(start_bound: Bound, end_bound: Bound) {
    let dummy_storage_root = [0u8; 32];
    let account_key: [u8; 32] = [0x10u8; 32];

    let storage_keys: Vec<[u8; 64]> = [0x10u8, 0x30, 0x60, 0xC0]
        .iter()
        .map(|&p| key_under_account(&account_key, p))
        .collect();
    let storage_values: Vec<Box<[u8]>> = (1u8..=4)
        .map(|i| rlp_encode_storage(&[i; 32]).into_boxed_slice())
        .collect();
    let account_value = rlp_encode_account(1, 100, &dummy_storage_root, &empty_code_hash());

    let (source, _dir_source) = setup_db![];
    let (empty_root, root2) = setup_2nd_commit!(
        source,
        [
            (account_key.as_ref(), account_value.as_ref()),
            (storage_keys[0].as_ref(), storage_values[0].as_ref()),
            (storage_keys[1].as_ref(), storage_values[1].as_ref()),
            (storage_keys[2].as_ref(), storage_values[2].as_ref()),
            (storage_keys[3].as_ref(), storage_values[3].as_ref()),
        ]
    );

    // Mid-storage probe — between storage_keys[1] (suffix 0x30) and
    // storage_keys[2] (suffix 0x60).
    let mid_storage = key_under_account(&account_key, 0x40);
    let resolve = |bound: Bound| -> Option<&[u8]> {
        match bound {
            Bound::None => None,
            Bound::AtAccount => Some(account_key.as_ref()),
            Bound::MidStorage => Some(mid_storage.as_ref()),
        }
    };
    let start_key = resolve(start_bound);
    let end_key = resolve(end_bound);

    let proof = source
        .change_proof(empty_root.clone(), root2.clone(), start_key, end_key, None)
        .unwrap();
    let ctx =
        verify_change_proof_structure(&proof, root2.clone(), start_key, end_key, None).unwrap();

    let (target, _dir_target) = setup_db![];
    let empty_root_target = target.root_hash().unwrap();
    verify_and_check(&target, &proof, &ctx, empty_root_target).unwrap();
}

/// Cross-batch ordering: apply the RIGHT half first, then the LEFT half.
/// State sync receives batches in arbitrary order. When the right-side
/// batch lands first, the target's depth-64 branch has storage children
/// but no account value (the account is in the left-side batch). When the
/// left batch arrives later, the account value is added to the existing
/// branch and live hashing recomputes storageRoot from the full child set.
/// Final root must match the source's root.
///
/// Note: this test passes both with and without the reconcile-side fix
/// for partial-vs-full storageRoot conflicts. By the time the LEFT batch
/// is applied, the cumulative children in the local DB are complete, so
/// the proposal's storageRoot matches the proof's. The single-batch
/// `test_change_proof_left_half_*` tests are what actually exercise the
/// reconcile fix. This test guards convergence across batches.
#[test]
fn test_change_proof_arbitrary_order_right_then_left_converges() {
    let dummy_storage_root = [0u8; 32];
    let account_key: [u8; 32] = [0x10u8; 32];

    let storage_keys: Vec<[u8; 64]> = [0x10u8, 0x30, 0x60, 0xC0]
        .iter()
        .map(|&p| key_under_account(&account_key, p))
        .collect();
    let storage_values: Vec<Box<[u8]>> = (1u8..=4)
        .map(|i| rlp_encode_storage(&[i; 32]).into_boxed_slice())
        .collect();
    let account_value = rlp_encode_account(1, 100, &dummy_storage_root, &empty_code_hash());

    // Source: empty → account + 4 storage children.
    let (source, _dir_source) = setup_db![];
    let (empty_root, full_root) = setup_2nd_commit!(
        source,
        [
            (account_key.as_ref(), account_value.as_ref()),
            (storage_keys[0].as_ref(), storage_values[0].as_ref()),
            (storage_keys[1].as_ref(), storage_values[1].as_ref()),
            (storage_keys[2].as_ref(), storage_values[2].as_ref()),
            (storage_keys[3].as_ref(), storage_values[3].as_ref()),
        ]
    );

    // mid_key sits between storage[1] and storage[2].
    let mid_key = key_under_account(&account_key, 0x40);

    let proof_left = source
        .change_proof(
            empty_root.clone(),
            full_root.clone(),
            None,
            Some(mid_key.as_ref()),
            None,
        )
        .unwrap();
    let ctx_left = verify_change_proof_structure(
        &proof_left,
        full_root.clone(),
        None,
        Some(mid_key.as_ref()),
        None,
    )
    .unwrap();

    let proof_right = source
        .change_proof(
            empty_root.clone(),
            full_root.clone(),
            Some(mid_key.as_ref()),
            None,
            None,
        )
        .unwrap();
    let ctx_right = verify_change_proof_structure(
        &proof_right,
        full_root.clone(),
        Some(mid_key.as_ref()),
        None,
        None,
    )
    .unwrap();

    // Apply RIGHT first, then LEFT, to an empty target.
    let (target, _dir_target) = setup_db![];
    let target_empty_root = target.root_hash().unwrap();

    // Step 1: apply RIGHT half. Verifies, then commits.
    let parent = target.revision(target_empty_root.clone()).unwrap();
    let proposal_right = target
        .apply_change_proof_to_parent(&proof_right, &*parent)
        .unwrap();
    crate::merkle::verify_change_proof_root_hash(&proof_right, &ctx_right, &proposal_right)
        .unwrap();
    proposal_right.commit().unwrap();
    let after_right_root = target.root_hash().unwrap();
    assert_ne!(
        after_right_root, full_root,
        "intermediate state should differ from full state"
    );

    // Step 2: apply LEFT half on top. The account value is in this batch's
    // batch_ops — gets added to the existing depth-64 branch (which already
    // has S2/S3/S4 from the right batch), live hashing recomputes storageRoot
    // from the now-full child set.
    let parent = target.revision(after_right_root).unwrap();
    let proposal_left = target
        .apply_change_proof_to_parent(&proof_left, &*parent)
        .unwrap();
    crate::merkle::verify_change_proof_root_hash(&proof_left, &ctx_left, &proposal_left).unwrap();
    proposal_left.commit().unwrap();
    let final_root = target.root_hash().unwrap();

    // Final root must match the source's full-state root.
    assert_eq!(
        final_root, full_root,
        "final root after applying both batches should match source"
    );
}
