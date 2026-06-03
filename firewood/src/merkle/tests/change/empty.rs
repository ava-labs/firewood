// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Empty start trie tests.
//!
//! When the start revision is empty, every key in the end revision is a new
//! insertion. These tests exercise the interaction between empty start tries
//! and the divergent child / value skip logic.

use super::*;

#[cfg(feature = "ethhash")]
fn rlp_encode_account(
    nonce: u64,
    balance: u64,
    storage_root: &[u8; 32],
    code_hash: &[u8; 32],
) -> Box<[u8]> {
    use rlp::RlpStream;

    let mut rlp = RlpStream::new_list(4);
    rlp.append(&nonce);
    rlp.append(&balance);
    rlp.append(&storage_root.as_slice());
    rlp.append(&code_hash.as_slice());
    rlp.out().to_vec().into_boxed_slice()
}

#[cfg(feature = "ethhash")]
fn rlp_encode_storage(value: &[u8; 32]) -> Box<[u8]> {
    use rlp::RlpStream;

    let mut rlp = RlpStream::new();
    rlp.append(&value.as_slice());
    rlp.out().to_vec().into_boxed_slice()
}

#[cfg(feature = "ethhash")]
fn empty_code_hash() -> [u8; 32] {
    use sha3::{Digest, Keccak256};

    Keccak256::digest([]).into()
}

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

#[test]
fn test_change_proof_partial_storage_children_against_empty() {
    let account_key: Box<[u8]> = [0x10u8; 32].into();
    let storage_key_a: Box<[u8]> = [account_key.as_ref(), &[0x10u8; 32]].concat().into();
    let storage_key_b: Box<[u8]> = [account_key.as_ref(), &[0x20u8; 32]].concat().into();
    let end_between_storage_children: Box<[u8]> =
        [account_key.as_ref(), &[0x18u8; 32]].concat().into();

    let dummy_storage_root = [0u8; 32];
    let account_value = rlp_encode_account(1, 100, &dummy_storage_root, &empty_code_hash());
    let storage_value_a = rlp_encode_storage(&[0xaa; 32]);
    let storage_value_b = rlp_encode_storage(&[0xbb; 32]);

    let (source, _source_dir) = setup_db![];
    let empty_root = source.root_hash().unwrap();
    source
        .propose(vec![
            BatchOp::Put {
                key: account_key.as_ref(),
                value: account_value.as_ref(),
            },
            BatchOp::Put {
                key: storage_key_a.as_ref(),
                value: storage_value_a.as_ref(),
            },
            BatchOp::Put {
                key: storage_key_b.as_ref(),
                value: storage_value_b.as_ref(),
            },
        ])
        .unwrap()
        .commit()
        .unwrap();
    let root2 = source.root_hash().unwrap();

    let cases = [
        (None, Some(end_between_storage_children.as_ref())),
        (
            Some(account_key.as_ref()),
            Some(end_between_storage_children.as_ref()),
        ),
    ];

    for (start_key, end_key) in cases {
        let proof = source
            .change_proof(empty_root.clone(), root2.clone(), start_key, end_key, None)
            .unwrap();
        let ctx =
            verify_change_proof_structure(&proof, root2.clone(), start_key, end_key, None).unwrap();

        let (target, _target_dir) = setup_db![];
        let empty_root_target = target.root_hash().unwrap();
        verify_and_check(&target, &proof, &ctx, empty_root_target).unwrap();
    }
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
