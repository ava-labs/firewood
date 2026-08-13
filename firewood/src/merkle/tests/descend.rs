// Copyright (C) 2024, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Unit tests for the read-only nibble-path descent.

use crate::merkle::Merkle;
use crate::merkle::descend::{ProbeOutcome, descend_to_prefix};

use super::init_merkle;
use firewood_storage::{
    Child, Committed, DefaultHashMode as H, HashType, MemStore, Node, NodeStore, PathComponent,
    RootReader as _,
};

fn components(nibbles: &[u8]) -> Vec<PathComponent> {
    nibbles
        .iter()
        .map(|&n| PathComponent::try_new(n).expect("test nibble in range"))
        .collect()
}

/// Keys 0xA711, 0xA777, 0xB055: root branch -> child A (branch, partial [7])
/// -> children 1 and 7 (leaves with partial [1] / [7]); child B (leaf,
/// partial [0,5,5] — long enough to land probes mid-edge).
fn fixture() -> Merkle<NodeStore<Committed, MemStore, H>> {
    init_merkle(vec![
        (vec![0xA7, 0x11], b"one".to_vec()),
        (vec![0xA7, 0x77], b"two".to_vec()),
        (vec![0xB0, 0x55], b"three".to_vec()),
    ])
}

/// The hash the fixture's root branch stores for one of its child slots.
/// Committed stores hold no `Child::Node`, so both hashed variants are
/// accepted and the unhashed one is a test-fixture bug.
fn stored_child_hash(merkle: &Merkle<NodeStore<Committed, MemStore, H>>, nibble: u8) -> HashType {
    let root = merkle
        .nodestore()
        .root_node()
        .expect("the fixture is non-empty");
    let Node::Branch(branch) = &*root else {
        panic!("the fixture's root is a branch");
    };
    let slot = PathComponent::try_new(nibble).expect("test nibble in range");
    match branch.children[slot]
        .as_ref()
        .expect("the slot is occupied")
    {
        Child::AddressWithHash(_, hash) | Child::MaybePersisted(_, hash) => hash.clone(),
        Child::Node(_) => panic!("a committed store holds no unhashed children"),
    }
}

#[test]
fn probe_at_child_edge_returns_stored_hash() {
    let merkle = fixture();
    let outcome = descend_to_prefix(merkle.nodestore(), &components(&[0xA]))
        .expect("descent reads no disk in this fixture");
    // Assert the payload, not just the variant: PR2 uses this hash verbatim
    // as the subtree commitment for the probed position, so "it is the
    // parent's stored hash for that slot" is the actual contract.
    let ProbeOutcome::EdgeExact(hash) = outcome else {
        panic!("a probe ending on a child edge yields EdgeExact");
    };
    assert_eq!(hash, stored_child_hash(&merkle, 0xA));
}

#[test]
fn probe_at_end_of_partial_path_lands_on_the_node() {
    let merkle = fixture();
    let outcome =
        descend_to_prefix(merkle.nodestore(), &components(&[0xA, 0x7])).expect("descent succeeds");
    // Read `node`, not just `consumed`: nothing else in this file does, and an
    // unread field fails `-D warnings` on dead_code even under cfg(test).
    let ProbeOutcome::AtNode { node, consumed } = outcome else {
        panic!("a probe ending at the end of a partial path yields AtNode");
    };
    assert_eq!(consumed, 1);
    assert_eq!(node.partial_path().as_components(), &components(&[0x7])[..]);
}

#[test]
fn probe_mid_edge_lands_on_the_node_with_partial_consumption() {
    let merkle = fixture();
    // The leaf under B has partial path [0,5,5]; probing [B,0] ends inside
    // that edge with two components unconsumed — the genuinely mid-edge
    // case, where a caller must re-encode with the adjusted split.
    let outcome =
        descend_to_prefix(merkle.nodestore(), &components(&[0xB, 0x0])).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::AtNode { consumed: 1, .. }));
}

#[test]
fn probe_diverging_inside_a_compressed_path_is_empty() {
    let merkle = fixture();
    // Child A's branch has partial [7]; probing [A,8] diverges inside it.
    let outcome =
        descend_to_prefix(merkle.nodestore(), &components(&[0xA, 0x8])).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::Empty));
}

#[test]
fn probe_at_an_absent_child_slot_is_empty() {
    let merkle = fixture();
    let outcome =
        descend_to_prefix(merkle.nodestore(), &components(&[0xC])).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::Empty));
}

#[test]
fn probe_past_a_leaf_is_empty() {
    let merkle = fixture();
    let outcome = descend_to_prefix(merkle.nodestore(), &components(&[0xA, 0x7, 0x1, 0x1, 0x5]))
        .expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::Empty));
}

#[test]
fn probe_with_empty_prefix_lands_on_the_root() {
    let merkle = fixture();
    let outcome = descend_to_prefix(merkle.nodestore(), &[]).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::AtNode { consumed: 0, .. }));
}

#[test]
fn probe_on_empty_trie_is_empty() {
    let merkle = init_merkle(Vec::<(Vec<u8>, Vec<u8>)>::new());
    let outcome =
        descend_to_prefix(merkle.nodestore(), &components(&[0xA])).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::Empty));
}

#[test]
fn probe_through_an_unhashed_child_reports_unhashed() {
    // Construction mirrors the swap-back test at
    // storage/src/nodestore/mod.rs:1884-1930: a reconstruction store built
    // via new_empty_recon holds Child::Node children until root_hash() is
    // first called. Before that call the descent must report UnhashedChild —
    // not Empty, which a caller would read as "no local keys" and turn into
    // a deletion order. new_empty_recon is gated on
    // cfg(any(test, feature = "test_utils")) and firewood's dev-dependency
    // on firewood-storage enables test_utils (firewood/Cargo.toml:63).
    use firewood_storage::{
        BranchNode, Child, Children, DefaultHashMode as H, HashedNodeReader as _, LeafNode,
        NibblesIterator, Node, Path, Reconstructed,
    };
    use std::sync::Arc;

    let storage = Arc::new(MemStore::default::<H>());
    let mut recon = NodeStore::new_empty_recon(Arc::clone(&storage));

    let mut children = Children::new();
    children[PathComponent::ALL[0xA]] = Some(Child::Node(Node::Leaf(LeafNode {
        partial_path: Path::from_nibbles_iterator(NibblesIterator::new(b"abc")),
        value: b"v0".to_vec().into_boxed_slice(),
    })));
    recon.root_mut().replace(Node::Branch(Box::new(BranchNode {
        partial_path: Path::new(),
        value: None,
        children,
    })));
    let reconstructed: NodeStore<Reconstructed<_, H>, _, H> = recon.into();

    let outcome = descend_to_prefix(&reconstructed, &components(&[0xA])).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::UnhashedChild));

    // After forcing the hash, Child::Node is swapped for MaybePersisted and
    // the same probe resolves normally.
    assert!(reconstructed.root_hash().is_some());
    let outcome = descend_to_prefix(&reconstructed, &components(&[0xA])).expect("descent succeeds");
    assert!(matches!(
        outcome,
        ProbeOutcome::EdgeExact(_) | ProbeOutcome::AtNode { .. }
    ));
}
