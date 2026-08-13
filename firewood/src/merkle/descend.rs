// Copyright (C) 2024, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Read-only descent to a nibble-path position in the trie.
//!
//! [`Merkle::path_iter`] cannot serve this purpose: it takes byte keys, and
//! an odd-length nibble position — routine for probe targets, since a child
//! edge adds one nibble to its parent's path — has no byte-key name.
//!
//! [`Merkle::path_iter`]: super::Merkle::path_iter

use firewood_storage::{Child, FileIoError, HashType, Node, PathComponent, SharedNode, TrieReader};

/// Where a nibble-path probe landed in the local trie.
#[derive(Debug)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "PR2's subtree_hash is the first caller")
)]
pub(crate) enum ProbeOutcome {
    /// No local keys carry the probed prefix: the descent diverged inside a
    /// compressed path, hit an absent child slot, or ran past a leaf.
    Empty,
    /// The probe ends exactly on a child edge: this is the parent's stored
    /// hash for that child, usable verbatim in either hash mode.
    EdgeExact(HashType),
    /// The probe landed on `node`, with the first `consumed` components of
    /// the node's partial path at or above the probe point. A caller forming
    /// this position's subtree commitment re-encodes the node with the probe
    /// as its prefix and `partial_path()[consumed..]` as its partial path.
    ///
    /// This variant does **not** guarantee the node's own child slots carry
    /// hashes: `children_hashes()` silently reports a `Child::Node` slot as
    /// absent, so a caller forming a commitment must treat a `Child::Node`
    /// slot as an error, never as an absent child.
    AtNode {
        /// The node covering the probed prefix.
        node: SharedNode,
        /// How many of the node's partial-path components the probe consumed.
        consumed: usize,
    },
    /// The descent needed a hash that does not exist: the probed path runs
    /// through or ends at a [`Child::Node`], which carries no hash. Distinct
    /// from [`Self::Empty`] because reading "unhashed" as "no local keys"
    /// would let a caller order the deletion of locally correct data.
    UnhashedChild,
}

/// Walks the trie from the root, consuming `prefix` one component at a time.
///
/// # Errors
///
/// Propagates node-read failures from the underlying store.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "PR2's subtree_hash is the first caller")
)]
pub(crate) fn descend_to_prefix<T: TrieReader>(
    nodestore: &T,
    prefix: &[PathComponent],
) -> Result<ProbeOutcome, FileIoError> {
    let Some(mut node) = nodestore.root_node() else {
        return Ok(ProbeOutcome::Empty);
    };
    let mut remaining = prefix;

    loop {
        let (common, partial_len) = {
            let partial = node.partial_path().as_components();
            let common = partial
                .iter()
                .zip(remaining.iter())
                .take_while(|(a, b)| a == b)
                .count();
            (common, partial.len())
        };

        if common == remaining.len() {
            // The probe ends inside (or exactly at the end of) this node's
            // partial path: the node covers the probed prefix.
            return Ok(ProbeOutcome::AtNode {
                node,
                consumed: common,
            });
        }
        if common < partial_len {
            // Diverged inside the compressed path: no keys under the probe.
            return Ok(ProbeOutcome::Empty);
        }

        // Here `common == partial_len < remaining.len()`: the partial path
        // is fully consumed and at least one probe component remains, so the
        // next component selects a child edge.
        let Some((&edge, rest)) = remaining
            .get(common..)
            .and_then(<[PathComponent]>::split_first)
        else {
            // Unreachable by the branch conditions above; `Empty` is safe
            // because a fully-consumed probe already returned `AtNode`.
            return Ok(ProbeOutcome::Empty);
        };

        let Node::Branch(branch) = &*node else {
            // A leaf has no children: nothing extends past it.
            return Ok(ProbeOutcome::Empty);
        };

        // If the borrow checker rejects the reassignment of `node` inside this
        // match — the scrutinee borrows `node` through `branch` — do not fight
        // it inline. Compute a small `enum Next { Done(ProbeOutcome),
        // Read(LinearAddress), Shared(MaybePersistedNode) }` inside the match,
        // let the borrow end, then act on it afterwards. NLL should accept the
        // form below as written, since nothing derived from the loan is live
        // past the assignment.
        match &branch.children[edge] {
            None => return Ok(ProbeOutcome::Empty),
            Some(Child::Node(_)) => {
                // In-memory, unhashed child. Even for pure traversal this is
                // reported rather than followed: every outcome below it
                // either needs a hash this subtree cannot supply or lands on
                // a node whose commitment cannot be trusted.
                return Ok(ProbeOutcome::UnhashedChild);
            }
            Some(Child::AddressWithHash(address, hash)) => {
                if rest.is_empty() {
                    return Ok(ProbeOutcome::EdgeExact(hash.clone()));
                }
                node = nodestore.read_node(*address)?;
            }
            Some(Child::MaybePersisted(maybe_persisted, hash)) => {
                if rest.is_empty() {
                    return Ok(ProbeOutcome::EdgeExact(hash.clone()));
                }
                node = maybe_persisted.as_shared_node(nodestore)?;
            }
        }
        remaining = rest;
    }
}
