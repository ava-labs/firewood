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
    /// The descent diverged inside a compressed path, hit an absent child
    /// slot, or ran past a leaf — the three cases where no local keys carry
    /// the probed prefix. A failed read of the *root* node also lands here
    /// (see the `# Errors` section on [`descend_to_prefix`]), so `Empty` is
    /// not proof that no keys exist under the probed prefix.
    Empty,
    /// The probe ends exactly on a child edge: this is the parent's stored
    /// hash for that child, usable verbatim in either hash mode. Under
    /// `ethhash`, though, a [`HashType`] may be the `Rlp` variant — an inline
    /// RLP encoding rather than a 32-byte hash — so a consumer must compare
    /// `HashType` values and must not assume a `TrieHash`.
    EdgeExact(HashType),
    /// The probe landed on `node`, with the first `consumed` components of
    /// the node's partial path at or above the probe point. A caller forming
    /// this position's subtree commitment re-encodes the node with the probe
    /// as its prefix and `partial_path().as_components().get(consumed..)` as
    /// its partial path.
    ///
    /// This variant does **not** guarantee the node's own child slots carry
    /// hashes: `children_hashes()` silently reports a `Child::Node` slot as
    /// absent, so a caller forming a commitment must treat a `Child::Node`
    /// slot as an error, never as an absent child.
    ///
    /// Under `ethhash`, a caller re-encoding the node must also consult
    /// `must_recompute_storage_hash()` on the nodestore and, if it returns
    /// `true`, apply the account `storageRoot` repair before using an
    /// account-depth node's value — otherwise the commitment it computes will
    /// disagree with the canonical one. `EdgeExact` is immune to this: it
    /// returns the parent's already-stored commitment rather than re-deriving
    /// one from a value.
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
/// Reads below the root propagate any [`FileIoError`] from the underlying
/// store. A failure to read the *root* node does not: the underlying
/// `root_node` accessor returns an `Option` and discards the error, so that
/// case is reported as [`ProbeOutcome::Empty`] instead of an error.
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
            // Unreachable given the checks above: `common <= remaining.len()`
            // makes `get` always `Some`, and `common == remaining.len()`
            // already returned `AtNode` above, so this slice is always
            // non-empty. Return the conservative outcome anyway, so that a
            // future refactor that breaks those invariants fails toward
            // "refuse to conclude" rather than toward "provably no keys
            // here."
            return Ok(ProbeOutcome::UnhashedChild);
        };

        let Node::Branch(branch) = &*node else {
            // A leaf has no children: nothing extends past it.
            return Ok(ProbeOutcome::Empty);
        };

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
