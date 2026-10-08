// Copyright (C) 2024, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Unit tests for the read-only nibble-path descent.

use std::sync::Arc;

use crate::api;
use crate::merkle::Merkle;
use crate::merkle::descend::{ProbeOutcome, descend_to_prefix, subtree_hash};

use super::{init_merkle_in, init_merkle_with_header_in};
use firewood_macros::hash_mode;
use firewood_storage::{
    Child, Committed, DeletedNodeTracking, EthHash, HashMode, HashType, HashedNodeReader as _,
    LeafNode, MemStore, MerkleDbHash, NibblesIterator, Node, NodeStore, Path, PathComponent,
    Reconstructed, RootReader as _,
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
fn fixture<H: HashMode>() -> Merkle<NodeStore<Committed, MemStore, H>> {
    init_merkle_in::<H, _, _, _>(vec![
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
    // Assert the payload, not just the variant: a caller forming this
    // position's subtree commitment uses this hash verbatim, so "it is the
    // parent's stored hash for that slot" is the actual contract.
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
    let merkle = init_merkle_in::<H, _, _, _>(vec![(vec![0xA7, 0x11], b"one".to_vec())]);
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
    // Read `node`, not just `consumed`: the assertion below is what pins the
    // node's partial path for this outcome.
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
    // that edge with two components unconsumed — the genuinely mid-edge
    // case, where a caller must re-encode with the adjusted split. This test
    // is the sole guard against swapping the two `if` checks in the descent
    // loop, so assert the node's full partial path rather than just
    // `consumed`, which the end-of-edge test asserts identically.
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
    // Child A's branch has partial [7]; both probes below diverge inside it,
    // but they pin down different things.

    // [A,8] diverges into slot 8, which is absent regardless of the
    // divergence check: even without it, falling through to child selection
    // on the diverged nibble would hit `None` and return `Empty` anyway.
    // This probe alone would stay green if the divergence check were
    // deleted.
    let outcome =
        descend_to_prefix(merkle.nodestore(), &components(&[0xA, 0x8])).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::Empty));

    // [A,1] diverges into slot 1, which is occupied (the fixture holds key
    // 0xA711): without the divergence check, falling through would treat the
    // diverged nibble as a child selector, find slot 1 populated, and return
    // `EdgeExact` — a hash for a position that holds no keys under the
    // probed prefix. This is the case that actually pins the check.
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
    let merkle = init_merkle_in::<H, _, _, _>(Vec::<(Vec<u8>, Vec<u8>)>::new());
    let outcome =
        descend_to_prefix(merkle.nodestore(), &components(&[0xA])).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::Empty));
}

#[hash_mode]
#[test]
fn probe_through_an_unhashed_child_reports_unhashed<H: HashMode>() {
    // Before `root_hash()` is first called the descent must report
    // UnhashedChild — not Empty, which a caller would read as "no local keys"
    // and turn into a deletion order.
    let reconstructed = unhashed_recon_fixture::<H>();

    let outcome = descend_to_prefix(&reconstructed, &components(&[0xA])).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::UnhashedChild));

    // The `Child::Node` arm returns `UnhashedChild` before checking whether
    // `rest` is empty, so a probe that runs through the unhashed child
    // rather than ending on it takes the same path. Exercise that case too.
    //
    // The `0x6` is load-bearing, not arbitrary: nibble iteration yields the
    // high nibble first, so the leaf's partial path from b"abc" (0x61 0x62
    // 0x63) begins with 0x6. This probe therefore names a position under
    // which local keys genuinely exist — the case where collapsing
    // `UnhashedChild` into `Empty` would order the deletion of correct data.
    // Substituting any other nibble here still reaches the same arm but
    // silently drops that property.
    let outcome =
        descend_to_prefix(&reconstructed, &components(&[0xA, 0x6])).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::UnhashedChild));

    // After forcing the hash, Child::Node is swapped for MaybePersisted and
    // the same probe resolves normally. `rest` is empty and the slot now
    // carries a hash, so the outcome is deterministically EdgeExact.
    assert!(reconstructed.root_hash().is_some());
    let outcome = descend_to_prefix(&reconstructed, &components(&[0xA])).expect("descent succeeds");
    assert!(matches!(outcome, ProbeOutcome::EdgeExact(_)));
}

fn assert_unhashed_view<T: std::fmt::Debug>(result: Result<T, api::Error>) {
    match result {
        Err(api::Error::UnhashedView { .. }) => {}
        other => panic!("expected UnhashedView, got {other:?}"),
    }
}

fn assert_file_io<T: std::fmt::Debug>(result: Result<T, api::Error>) {
    match result {
        Err(api::Error::FileIO(_)) => {}
        other => panic!("expected FileIO, got {other:?}"),
    }
}

/// A reconstruction store whose root branch holds one `Child::Node` leaf at
/// slot A (partial path from `b"abc"`), so a probe through A meets an
/// unhashed child until `root_hash()` is first called. Mirrors the swap-back
/// test `reconstructed_root_hash_rewrites_root_children` in
/// storage/src/nodestore/mod.rs. `new_empty_recon` is gated on
/// `cfg(any(test, feature = "test_utils"))`, which firewood's dev-dependency
/// on firewood-storage enables.
fn unhashed_recon_fixture<H: HashMode>() -> NodeStore<Reconstructed<MemStore, H>, MemStore, H> {
    super::holes::recon_with_child_at_a::<H>(Node::Leaf(LeafNode {
        partial_path: Path::from_nibbles_iterator(NibblesIterator::new(b"abc")),
        value: b"v0".to_vec().into_boxed_slice(),
    }))
}

/// The fixture's keys plus 0xB155, so that [B] becomes a branch with
/// children 0 and 1 and the leaf holding 0xB055 hangs off the edge at
/// [B,0] with partial path [5,5].
fn fixture_with_edge_at_b0<H: HashMode>() -> Merkle<NodeStore<Committed, MemStore, H>> {
    init_merkle_in::<H, _, _, _>(vec![
        (vec![0xA7, 0x11], b"one".to_vec()),
        (vec![0xA7, 0x77], b"two".to_vec()),
        (vec![0xB0, 0x55], b"three".to_vec()),
        (vec![0xB1, 0x55], b"four".to_vec()),
    ])
}

#[hash_mode]
#[test]
fn subtree_hash_at_a_child_edge_is_the_stored_hash<H: HashMode>() {
    let merkle = fixture::<H>();
    let hash = subtree_hash::<H, _>(merkle.nodestore(), &components(&[0xA]))
        .expect("descent reads no disk in this fixture");
    assert_eq!(hash, Some(stored_child_hash(&merkle, 0xA)));
}

#[hash_mode]
#[test]
fn subtree_hash_mid_edge_matches_a_trie_with_an_edge_there<H: HashMode>() {
    // In `fixture` the leaf for 0xB055 hangs off [B] with partial path
    // [0,5,5]; probing [B,0] lands mid-edge. In `fixture_with_edge_at_b0` the
    // same leaf hangs off a real edge at [B,0] with partial path [5,5], so
    // the parent's stored hash there is exactly what a sealed stub for the
    // position would hold.
    let merkle = fixture::<H>();
    let hash = subtree_hash::<H, _>(merkle.nodestore(), &components(&[0xB, 0x0]))
        .expect("descent succeeds")
        .expect("keys exist under [B,0]");

    let target = fixture_with_edge_at_b0::<H>();
    let ProbeOutcome::EdgeExact(stub) =
        descend_to_prefix(target.nodestore(), &components(&[0xB, 0x0])).expect("descent succeeds")
    else {
        panic!("[B,0] is a child edge in the target fixture");
    };
    assert_eq!(hash, stub);

    // MerkleDB hashes the full path, so the adjusted split reproduces the
    // stored hash of the whole [B] subtree; Ethereum hashes the partial path
    // and so cannot. A broken adjustment would pass the merkledb half alone.
    let stored = stored_child_hash(&merkle, 0xB);
    if H::ALGORITHM.is_ethereum() {
        assert_ne!(hash, stored);
    } else {
        assert_eq!(hash, stored);
    }
}

#[hash_mode]
#[test]
fn subtree_hash_at_a_node_matches_a_trie_with_an_edge_there<H: HashMode>() {
    // Probing [A,7] ends exactly at the end of child A's partial path [7]:
    // the `AtNode` outcome with everything consumed. The re-encoding then has
    // an empty partial path, which under Ethereum differs from the stored
    // hash of the [A] edge just as the mid-edge case does.
    let merkle = fixture::<H>();
    let hash = subtree_hash::<H, _>(merkle.nodestore(), &components(&[0xA, 0x7]))
        .expect("descent succeeds")
        .expect("keys exist under [A,7]");
    let target = init_merkle_in::<H, _, _, _>(vec![
        (vec![0xA7, 0x11], b"one".to_vec()),
        (vec![0xA7, 0x77], b"two".to_vec()),
        (vec![0xA8, 0x00], b"split".to_vec()),
        (vec![0xB0, 0x55], b"three".to_vec()),
    ]);
    let ProbeOutcome::EdgeExact(stub) =
        descend_to_prefix(target.nodestore(), &components(&[0xA, 0x7])).expect("descent succeeds")
    else {
        panic!("[A,7] is a child edge in the target fixture");
    };
    assert_eq!(hash, stub);
}

#[hash_mode]
#[test]
fn subtree_hash_is_none_where_no_keys_exist<H: HashMode>() {
    let merkle = fixture::<H>();
    for probe in [&[0xA, 0x8][..], &[0xC], &[0xA, 0x7, 0x1, 0x1, 0x5]] {
        let hash =
            subtree_hash::<H, _>(merkle.nodestore(), &components(probe)).expect("descent succeeds");
        assert_eq!(hash, None, "probe {probe:x?}");
    }
    let empty = init_merkle_in::<H, _, _, _>(Vec::<(Vec<u8>, Vec<u8>)>::new());
    let hash =
        subtree_hash::<H, _>(empty.nodestore(), &components(&[0xA])).expect("no root to read");
    assert_eq!(hash, None);
}

#[hash_mode]
#[test]
fn subtree_hash_errors_on_an_unhashed_child<H: HashMode>() {
    let reconstructed = unhashed_recon_fixture::<H>();

    // Ending on the unhashed child, running through it, and landing on the
    // root whose slot holds it are all errors, never `None`.
    assert_unhashed_view(subtree_hash::<H, _>(&reconstructed, &components(&[0xA])));
    assert_unhashed_view(subtree_hash::<H, _>(
        &reconstructed,
        &components(&[0xA, 0x6]),
    ));
    assert_unhashed_view(subtree_hash::<H, _>(&reconstructed, &[]));

    // Forcing the root hash swaps the child for a hashed one; the same
    // probes then resolve.
    assert!(reconstructed.root_hash().is_some());
    assert!(
        subtree_hash::<H, _>(&reconstructed, &components(&[0xA]))
            .expect("descent succeeds")
            .is_some()
    );
    assert!(
        subtree_hash::<H, _>(&reconstructed, &[])
            .expect("descent succeeds")
            .is_some()
    );
}

#[hash_mode]
#[test]
fn subtree_hash_errors_when_an_addressed_root_does_not_read<H: HashMode>() {
    // A committed store opened from a header that names a root, over storage
    // holding no nodes: the root is addressed but every read of it fails.
    // The failure must surface as the read error, never as an empty trie.
    let (_merkle, header) =
        init_merkle_with_header_in::<H, _, _, _>(vec![(vec![0xA7, 0x11], b"one".to_vec())]);
    let broken: NodeStore<Committed, MemStore, H> = NodeStore::open(
        &header,
        Arc::new(MemStore::new(Vec::new())),
        DeletedNodeTracking::Enabled,
    )
    .expect("the header carries the root hash, so opening reads nothing");
    assert!(broken.root_address().is_some());
    assert!(broken.root_node().is_none());

    assert_file_io(subtree_hash::<H, _>(&broken, &[]));
    assert_file_io(subtree_hash::<H, _>(&broken, &components(&[0xA])));
}
