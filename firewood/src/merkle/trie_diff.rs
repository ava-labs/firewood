// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Utility for comparing two in-memory trie subtrees and printing divergences.
//!
//! # When to use
//!
//! This is a debugging tool for `EndRootMismatch` failures in change proof
//! verification (`verify_change_proof_root_hash`). Those failures mean the
//! "proving trie" (a fork of the proposal, restructured with proof data)
//! produces a different root hash than `end_root`. This module helps you
//! find *where* the two tries diverge.
//!
//! # Quick start
//!
//! From any test in `firewood/src/merkle/tests/`:
//!
//! ```ignore
//! use crate::merkle::trie_diff;
//! trie_diff::diff_tries(&node_a, &node_b, &[]);
//! ```
//!
//! Output goes to stderr and looks like:
//!
//! ```text
//! DIFF depth=2 path=[a,c]
//!   A: Hash(35bf2b...)
//!   B: Hash(6c6147...)
//!   value: A=3bytes B=None
//!   child 4: hashes differ
//! DIFF depth=3 path=[a,c,4]
//!   ...
//! ```
//!
//! Descent stops when subtree hashes match, so only the divergent paths
//! are printed.
//!
//! # Reproducing fuzz failures
//!
//! The fuzz tests (`test_slow_change_proof_fuzz`, `test_slow_change_proof_fuzz_varlen`)
//! print a seed on failure. Reproduce with:
//!
//! ```bash
//! FIREWOOD_TEST_SEED=<seed> cargo nextest run --workspace \
//!   --features ethhash,logger --all-targets --cargo-profile release \
//!   --profile ci -E 'test(test_slow_change_proof_fuzz_varlen)'
//! ```
//!
//! The `--profile ci` flag is required because these are slow tests filtered
//! out by the default nextest profile.
//!
//! # Debugging an `EndRootMismatch`
//!
//! The verification in `verify_change_proof_root_hash` works by:
//!
//! 1. Forking the proposal into a "proving trie"
//! 2. Reconciling proof nodes into it (inserting branch structure from `end_root`)
//! 3. Collapsing intermediate branches between consecutive proof nodes
//! 4. Computing a hybrid hash: in-range children from the proving trie,
//!    out-of-range children from proof node hashes
//! 5. Comparing against `end_root`
//!
//! When the hash doesn't match, add debug output right before the
//! `EndRootMismatch` return (search for that string in `merkle/mod.rs`).
//! A useful snippet to print the proof structure:
//!
//! ```ignore
//! for (label, nodes) in [("start", start_nodes), ("end", end_nodes)] {
//!     eprintln!("{label}_proof:");
//!     for (i, pn) in nodes.iter().enumerate() {
//!         let kn: Vec<u8> = pn.key.iter().map(|c| c.as_u8()).collect();
//!         let kp: Vec<u8> = kn.iter().copied().take(8).collect();
//!         let ch: Vec<u8> = pn.child_hashes.iter()
//!             .filter(|(_, c)| c.is_some())
//!             .map(|(n, _)| n.as_u8()).collect();
//!         eprintln!("  [{i}] key[..8]={kp:?} len={} val={} ch={ch:?}",
//!             kn.len(), pn.value_digest.is_some());
//!     }
//! }
//! ```
//!
//! # Key functions involved in change proof verification
//!
//! All in `firewood/src/merkle/mod.rs`:
//!
//! - `verify_change_proof_root_hash` — orchestrates the verification
//! - `compute_outside_children` — determines which children at proof nodes
//!   are outside the proven range (left/right of the boundary keys)
//! - `collapse_strip` — removes non-on-path children/values from intermediate
//!   nodes between consecutive proof nodes to match `end_root`'s structure
//! - `compute_root_hash_with_proofs` — computes the hybrid root hash
//! - `reconcile_branch_proof_node` — inserts proof structure into the proving trie
//!
//! # Common bug patterns with variable-length keys
//!
//! These bugs tend to only surface with variable-length keys because short
//! keys create shallow trie structures where prefix relationships, divergent
//! nodes in exclusion proofs, and boundary-matching terminals are common.
//!
//! Things to look for:
//!
//! - **Short keys creating intermediate values**: A key like `0xAC` creates a
//!   value at an intermediate node on the path to a longer key `0xACC0...`.
//!   If this value is out-of-range, `collapse_strip` must remove it so the
//!   node can be flattened.
//! - **Terminal matching the boundary key**: When `end_key` exactly matches the
//!   terminal proof node, all the terminal's children extend beyond `end_key`
//!   and must be marked as outside.
//! - **Boundary shorter than terminal path**: When `end_key` is a prefix of the
//!   terminal's key (e.g., `end_key`=`0xDC`, terminal=`0xDC3F...`), intermediate
//!   nodes at `end_key`'s depth have children that all exceed `end_key`.

use firewood_storage::{
    BranchNode, Child, Children, HashType, HashableShunt, Node, PathComponent, ValueDigest,
};

/// Recursively compute the hash of an in-memory `Node` subtree.
///
/// Unlike `hash_node` from storage, this handles `Child::Node` children
/// by recursing into them rather than requiring pre-computed hashes.
fn hash_subtree(node: &Node, prefix: &[PathComponent]) -> HashType {
    let branch = match node {
        Node::Leaf(_) => return HashableShunt::from_node(prefix, node).to_hash(),
        Node::Branch(b) => b,
    };

    let full_key: Vec<PathComponent> = prefix
        .iter()
        .chain(branch.partial_path.as_components().iter())
        .copied()
        .collect();

    let child_hashes = compute_child_hashes(branch, &full_key);
    let value = branch.value.as_deref().map(ValueDigest::Value);

    HashableShunt::new(
        prefix,
        branch.partial_path.as_components(),
        value,
        child_hashes,
    )
    .to_hash()
}

fn compute_child_hashes(
    branch: &BranchNode,
    full_key: &[PathComponent],
) -> Children<Option<HashType>> {
    let mut child_hashes: Children<Option<HashType>> = Children::new();
    let mut child_prefix: Vec<PathComponent> = full_key.to_vec();

    for (nibble, child_opt) in &branch.children {
        let Some(child) = child_opt else { continue };
        match child {
            Child::AddressWithHash(_, hash) | Child::MaybePersisted(_, hash) => {
                child_hashes[nibble] = Some(hash.clone());
            }
            Child::Node(child_node) => {
                child_prefix.push(nibble);
                child_hashes[nibble] = Some(hash_subtree(child_node, &child_prefix));
                child_prefix.pop();
            }
        }
    }
    child_hashes
}

/// Compare two in-memory trie subtrees and print divergences to stderr.
///
/// Walks both trees in parallel. When hashes match at a node, descent stops.
/// When they differ, the path, depth, and mismatched hashes are printed,
/// then the function recurses into children to narrow down the divergence.
///
/// `prefix` is the nibble path from the trie root to these nodes.
#[cfg_attr(test, allow(dead_code))]
pub(crate) fn diff_tries(a: &Node, b: &Node, prefix: &[PathComponent]) {
    let hash_a = hash_subtree(a, prefix);
    let hash_b = hash_subtree(b, prefix);

    if hash_a == hash_b {
        return;
    }

    let depth = prefix.len();
    let path_str = nibbles_to_hex(prefix);
    eprintln!("DIFF depth={depth} path={path_str}");
    eprintln!("  A: {hash_a:?}");
    eprintln!("  B: {hash_b:?}");

    // Describe structural differences
    match (a, b) {
        (Node::Leaf(la), Node::Leaf(lb)) => {
            if la.partial_path != lb.partial_path {
                eprintln!(
                    "  partial_path: A={:x} B={:x}",
                    la.partial_path, lb.partial_path
                );
            }
            if la.value != lb.value {
                eprintln!(
                    "  value: A={}bytes B={}bytes",
                    la.value.len(),
                    lb.value.len()
                );
            }
        }
        (Node::Branch(ba), Node::Branch(bb)) => {
            diff_branches(ba, bb, prefix);
        }
        (Node::Leaf(_), Node::Branch(_)) => {
            eprintln!("  node type: A=Leaf B=Branch");
        }
        (Node::Branch(_), Node::Leaf(_)) => {
            eprintln!("  node type: A=Branch B=Leaf");
        }
    }
}

fn diff_branches(a: &BranchNode, b: &BranchNode, prefix: &[PathComponent]) {
    if a.partial_path != b.partial_path {
        eprintln!(
            "  partial_path: A={:x} B={:x}",
            a.partial_path, b.partial_path
        );
    }

    match (&a.value, &b.value) {
        (Some(va), Some(vb)) if va != vb => {
            eprintln!("  value: A={}bytes B={}bytes", va.len(), vb.len());
        }
        (Some(v), None) => eprintln!("  value: A={}bytes B=None", v.len()),
        (None, Some(v)) => eprintln!("  value: A=None B={}bytes", v.len()),
        _ => {}
    }

    let full_key_a: Vec<PathComponent> = prefix
        .iter()
        .chain(a.partial_path.as_components().iter())
        .copied()
        .collect();
    let full_key_b: Vec<PathComponent> = prefix
        .iter()
        .chain(b.partial_path.as_components().iter())
        .copied()
        .collect();

    // Only recurse into children if partial paths match (same trie position).
    if full_key_a != full_key_b {
        return;
    }

    let child_hashes_a = compute_child_hashes(a, &full_key_a);
    let child_hashes_b = compute_child_hashes(b, &full_key_b);

    for nibble in PathComponent::ALL {
        let ha = &child_hashes_a[nibble];
        let hb = &child_hashes_b[nibble];

        match (ha, hb) {
            (Some(ha), Some(hb)) if ha != hb => {
                eprintln!("  child {nibble}: hashes differ");
                // Try to recurse if both are in-memory nodes
                if let (Some(Child::Node(ca)), Some(Child::Node(cb))) =
                    (&a.children[nibble], &b.children[nibble])
                {
                    let mut child_prefix = full_key_a.clone();
                    child_prefix.push(nibble);
                    diff_tries(ca, cb, &child_prefix);
                }
            }
            (Some(_), None) => eprintln!("  child {nibble}: A=present B=absent"),
            (None, Some(_)) => eprintln!("  child {nibble}: A=absent B=present"),
            _ => {}
        }
    }
}

fn nibbles_to_hex(nibbles: &[PathComponent]) -> String {
    if nibbles.is_empty() {
        return String::from("[]");
    }
    let mut s = String::with_capacity(nibbles.len().wrapping_add(2));
    s.push('[');
    for (i, n) in nibbles.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push(char::from_digit(u32::from(n.as_u8()), 16).unwrap_or('?'));
    }
    s.push(']');
    s
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::merkle::tests::init_merkle;

    #[test]
    fn identical_tries_produce_no_output() {
        let items = [("abc", "1"), ("abd", "2"), ("xyz", "3")];
        let merkle = init_merkle(items);
        let root = merkle.root().unwrap();

        // Hashes match at the root so descent never starts.
        diff_tries(&root, &root, &[]);
    }

    #[test]
    fn detects_value_change() {
        let merkle_a = init_merkle([("abc", "old")]);
        let merkle_b = init_merkle([("abc", "new")]);
        let root_a = merkle_a.root().unwrap();
        let root_b = merkle_b.root().unwrap();

        // Prints DIFF lines to stderr; verify with --nocapture.
        diff_tries(&root_a, &root_b, &[]);
    }

    #[test]
    fn detects_structural_difference() {
        let merkle_a = init_merkle([("abc", "1"), ("abd", "2")]);
        let merkle_b = init_merkle([("abc", "1"), ("abd", "2"), ("abe", "3")]);
        let root_a = merkle_a.root().unwrap();
        let root_b = merkle_b.root().unwrap();

        // Trie B has an extra key, creating a structural difference.
        diff_tries(&root_a, &root_b, &[]);
    }
}
