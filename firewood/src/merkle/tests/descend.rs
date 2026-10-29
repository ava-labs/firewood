// Copyright (C) 2024, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Unit tests for the read-only nibble-path descent.

use firewood_macros::hash_mode;
use firewood_storage::Child;
use firewood_storage::Committed;
use firewood_storage::EthHash;
use firewood_storage::HashMode;
use firewood_storage::HashType;
use firewood_storage::MemStore;
use firewood_storage::MerkleDbHash;
use firewood_storage::Node;
use firewood_storage::NodeStore;
use firewood_storage::PathComponent;
use firewood_storage::RootReader as _;

use super::init_merkle;
use crate::merkle::Merkle;
use crate::merkle::descend::ProbeOutcome;
use crate::merkle::descend::descend_to_prefix;

fn components(nibbles: &[u8]) -> Vec<PathComponent> {
    nibbles
        .iter()
        .map(|&n| PathComponent::try_new(n).expect("test nibble in range"))
        .collect()
}

/// Keys 0xA711, 0xA777, 0xB055: root branch -> child A (branch, partial [7])
/// -> children 1 and 7 (leaves with partial [1] / [7]); child B (leaf,
/// partial [0,5,5] — long enough to land probes mid-edge).
fn fixture<H: HashMode>() -> Merkle<NodeStore<Committed, MemStore, H>> {
    init_merkle::<H, _, _, _>(vec![
        (vec![0xA7, 0x11], b"one".to_vec()),
        (vec![0xA7, 0x77], b"two".to_vec()),
        (vec![0xB0, 0x55], b"three".to_vec()),
    ])
}

/// The hash the fixture's root branch stores for one of its child slots.
/// Committed stores hold no `Child::Node`, so both hashed variants are
/// accepted and the unhashed one is a test-fixture bug.
fn stored_child_hash<H: HashMode>(
    merkle: &Merkle<NodeStore<Committed, MemStore, H>>,
    nibble: u8,
) -> HashType {
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

#[hash_mode]
#[test]
fn probe_at_child_edge_returns_stored_hash<H: HashMode>() {
    let merkle = fixture::<H>();
    let outcome = descend_to_prefix(merkle.nodestore(), &components(&[0xA]))
        .expect("descent reads no disk in this fixture");
    let ProbeOutcome::EdgeExact(hash) = outcome else {
        panic!("a probe ending on a child edge yields EdgeExact");
    };
    assert_eq!(hash, stored_child_hash(&merkle, 0xA));
}

#[hash_mode]
#[test]
fn single_key_trie_probes_against_the_root_partial_path<H: HashMode>() {
    // With one key the root is a leaf whose partial path is the whole key,
    // so the root itself exercises the mid-path, divergence, and past-a-leaf
    // arms that the multi-key fixture only reaches below the root.
    let merkle = init_merkle::<H, _, _, _>(vec![(vec![0xA7, 0x11], b"one".to_vec())]);
    let probe = |nibbles: &[u8]| {
        descend_to_prefix(merkle.nodestore(), &components(nibbles)).expect("descent succeeds")
    };

    let ProbeOutcome::AtNode { consumed, .. } = probe(&[0xA, 0x7]) else {
        panic!("a probe ending inside the root's partial path yields AtNode");
    };
    assert_eq!(consumed, 2);
    assert!(matches!(probe(&[0xB]), ProbeOutcome::Empty));
    assert!(matches!(
        probe(&[0xA, 0x7, 0x1, 0x1, 0x5]),
        ProbeOutcome::Empty
    ));
}

#[hash_mode]
#[test]
fn probe_at_end_of_partial_path_lands_on_the_node<H: HashMode>() {
    let merkle = fixture::<H>();
    let outcome =
        descend_to_prefix(merkle.nodestore(), &components(&[0xA, 0x7])).expect("descent succeeds");
    let ProbeOutcome::AtNode { node, consumed } = outcome else {
        panic!("a probe ending at the end of a partial path yields AtNode");
    };
    assert_eq!(consumed, 1);
    assert_eq!(node.partial_path().as_components(), &components(&[0x7])[..]);
}

#[hash_mode]
#[test]
fn probe_mid_edge_lands_on_the_node_with_partial_consumption<H: HashMode>() {
    let merkle = fixture::<H>();
    // The leaf under B has partial path [0,5,5]; probing [B,0] ends inside
    // that edge, so the outcome is the leaf with one component consumed.
    let outcome =
        descend_to_prefix(merkle.nodestore(), &components(&[0xB, 0x0])).expect("descent succeeds");
    let ProbeOutcome::AtNode { node, consumed } = outcome else {
        panic!("a probe ending mid-edge yields AtNode");
    };
    assert_eq!(consumed, 1);
    assert_eq!(
        node.partial_path().as_components(),
        &components(&[0x0, 0x5, 0x5])[..]
    );
}

#[hash_mode]
#[test]
fn probe_diverging_inside_a_compressed_path_is_empty<H: HashMode>() {
    let merkle = fixture::<H>();
    // Child A's branch has partial path [7]. [A,8] and [A,1] both diverge
    // against that partial path, so neither prefix holds keys. A descent that
    // skipped the partial path would land [A,1] on key 0xA711 instead.
    let outcome =
        descend_to_prefix(merkle.nodestore(), &components(&[0xA, 0x8])).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::Empty));

    let outcome =
        descend_to_prefix(merkle.nodestore(), &components(&[0xA, 0x1])).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::Empty));
}

#[hash_mode]
#[test]
fn probe_at_an_absent_child_slot_is_empty<H: HashMode>() {
    let merkle = fixture::<H>();
    let outcome =
        descend_to_prefix(merkle.nodestore(), &components(&[0xC])).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::Empty));
}

#[hash_mode]
#[test]
fn probe_past_a_leaf_is_empty<H: HashMode>() {
    let merkle = fixture::<H>();
    let outcome = descend_to_prefix(merkle.nodestore(), &components(&[0xA, 0x7, 0x1, 0x1, 0x5]))
        .expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::Empty));
}

#[hash_mode]
#[test]
fn probe_with_empty_prefix_lands_on_the_root<H: HashMode>() {
    let merkle = fixture::<H>();
    let outcome = descend_to_prefix(merkle.nodestore(), &[]).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::AtNode { consumed: 0, .. }));
}

#[hash_mode]
#[test]
fn probe_on_empty_trie_is_empty<H: HashMode>() {
    let merkle = init_merkle::<H, _, _, _>(Vec::<(Vec<u8>, Vec<u8>)>::new());
    let outcome =
        descend_to_prefix(merkle.nodestore(), &components(&[0xA])).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::Empty));
}

/// The key bytes of the unhashed leaf under slot A in the reconstruction
/// fixture.
const UNHASHED_LEAF_PATH: &[u8] = b"abc";

#[hash_mode]
#[test]
fn probe_through_an_unhashed_child_reports_unhashed<H: HashMode>() {
    // Construction mirrors the swap-back test
    // `reconstructed_root_hash_rewrites_root_children` in
    // storage/src/nodestore/mod.rs: a reconstruction store built via
    // new_empty_recon holds Child::Node children until root_hash() is first
    // called. Before that call the descent must report UnhashedChild — not
    // Empty, which a caller would read as "no local keys" and turn into a
    // deletion order. new_empty_recon is gated on
    // cfg(any(test, feature = "test_utils")), which firewood's dev-dependency
    // on firewood-storage enables.
    use std::sync::Arc;

    use firewood_storage::BranchNode;
    use firewood_storage::Children;
    use firewood_storage::HashedNodeReader as _;
    use firewood_storage::LeafNode;
    use firewood_storage::NibblesIterator;
    use firewood_storage::Path;
    use firewood_storage::Reconstructed;

    let storage = Arc::new(MemStore::new(Vec::new()));
    let mut recon = NodeStore::new_empty_recon(Arc::clone(&storage));

    let mut children = Children::new();
    children[PathComponent::ALL[0xA]] = Some(Child::Node(Node::Leaf(LeafNode {
        partial_path: Path::from_nibbles_iterator(NibblesIterator::new(UNHASHED_LEAF_PATH)),
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

    // A probe through the unhashed child into keys it holds is also
    // UnhashedChild.
    let first_leaf_nibble = UNHASHED_LEAF_PATH[0] >> 4;
    let outcome = descend_to_prefix(&reconstructed, &components(&[0xA, first_leaf_nibble]))
        .expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::UnhashedChild));

    // After forcing the hash, Child::Node is swapped for MaybePersisted and
    // the same probe resolves normally. `rest` is empty and the slot now
    // carries a hash, so the outcome is deterministically EdgeExact.
    assert!(reconstructed.root_hash().is_some());
    let outcome = descend_to_prefix(&reconstructed, &components(&[0xA])).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::EdgeExact(_)));
}
