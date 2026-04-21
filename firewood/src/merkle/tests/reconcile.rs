// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::*;

#[test]
fn test_reconcile_branch_proof_node_creates_missing_branch_without_value() {
    let mut merkle = create_in_memory_merkle();

    let proof_node = test_branch_proof_node(&[0xa, 0xb, 0xc], None);

    merkle
        .reconcile_branch_proof_node(&proof_node, |_| unreachable!())
        .unwrap();

    let node = merkle
        .get_node_from_nibbles(&[0xa, 0xb, 0xc])
        .unwrap()
        .unwrap();
    let branch = node.as_branch().expect("expected branch node");
    assert!(branch.value.is_none());
}

#[test]
fn test_reconcile_branch_proof_node_sets_missing_value_via_callback() {
    let mut merkle = create_in_memory_merkle();
    merkle.insert_branch_from_nibbles(&[0xa, 0xb]).unwrap();

    let proof_node =
        test_branch_proof_node(&[0xa, 0xb], Some(ValueDigest::Value(Box::from([7u8]))));

    // Proof has a value but trie doesn't — callback resolves it.
    merkle
        .reconcile_branch_proof_node(&proof_node, |pn| match &pn.value_digest {
            Some(ValueDigest::Value(v)) => Ok(Some(v.clone())),
            _ => Ok(None),
        })
        .unwrap();

    let node = merkle.get_node_from_nibbles(&[0xa, 0xb]).unwrap().unwrap();
    let branch = node.as_branch().expect("expected branch node");
    assert_eq!(branch.value.as_deref(), Some([7u8].as_slice()));
}

#[test]
fn test_reconcile_branch_proof_node_clears_value_via_callback() {
    let mut merkle = create_in_memory_merkle();
    merkle.insert(&[0xab], Box::from([3u8])).unwrap();
    merkle.insert_branch_from_nibbles(&[0xa, 0xb]).unwrap();

    let proof_node = test_branch_proof_node(&[0xa, 0xb], None);

    // Proof says no value, trie has one — callback returns None to clear it.
    merkle
        .reconcile_branch_proof_node(&proof_node, |_| Ok(None))
        .unwrap();

    let node = merkle.get_node_from_nibbles(&[0xa, 0xb]).unwrap().unwrap();
    let branch = node.as_branch().expect("expected branch node");
    assert!(branch.value.is_none());
}

#[test]
fn test_reconcile_branch_proof_node_rejects_conflict_via_callback() {
    let mut merkle = create_in_memory_merkle();
    merkle.insert(&[0xab], Box::from([1u8])).unwrap();
    merkle.insert_branch_from_nibbles(&[0xa, 0xb]).unwrap();

    let proof_node =
        test_branch_proof_node(&[0xa, 0xb], Some(ValueDigest::Value(Box::from([2u8]))));

    // Callback rejects the conflict.
    let err = merkle
        .reconcile_branch_proof_node(&proof_node, |_| Err(ProofError::UnexpectedValue))
        .unwrap_err();
    assert!(matches!(err, ProofError::UnexpectedValue));

    // Value is unchanged.
    let node = merkle.get_node_from_nibbles(&[0xa, 0xb]).unwrap().unwrap();
    let branch = node.as_branch().expect("expected branch node");
    assert_eq!(branch.value.as_deref(), Some([1u8].as_slice()));
}
