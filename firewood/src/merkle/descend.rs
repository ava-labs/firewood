// Copyright (C) 2024, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Read-only descent to a nibble-path position in the trie.
//!
//! [`Merkle::path_iter`] cannot serve this purpose: it takes byte keys, and
//! an odd-length nibble position — routine for probe targets, since a child
//! edge adds one nibble to its parent's path — has no byte-key name.
//!
//! [`Merkle::path_iter`]: super::Merkle::path_iter

use firewood_storage::{
    Child, Children, FileIoError, HashMode, HashType, HashableShunt, HashedNodeReader, Node,
    PathComponent, SharedNode, TrieReader, ValueDigest,
};

use crate::api;

/// Where a nibble-path probe landed in the local trie.
#[derive(Debug)]
pub(crate) enum ProbeOutcome {
    /// The trie is empty, or the descent diverged inside a compressed path,
    /// hit an absent child slot, or ran past a leaf: no local keys carry the
    /// probed prefix.
    Empty,
    /// The probe ends exactly on a child edge: this is the parent's stored
    /// hash for that child, usable verbatim in either hash mode. Under the
    /// Ethereum mode, though, a [`HashType`] may be the `Rlp` variant — an inline
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
    /// Under the Ethereum mode an account-depth node's stored value may hold
    /// a stale `storageRoot`; re-encoding needs no repair for it (see
    /// [`subtree_hash`]). The child hashes the hasher derives that field
    /// from are what the paragraph above warns may be missing, and an absent
    /// one is an error, never a value to substitute around.
    AtNode {
        /// The node covering the probed prefix.
        node: SharedNode,
        /// How many of the node's partial-path components the probe consumed.
        consumed: usize,
    },
    /// The descent could not establish a hash for the probed position. The
    /// only reachable cause is a probed path that runs through or ends at a
    /// [`Child::Node`], which carries no hash; this is also the outcome the
    /// descent falls back to if one of its own structural invariants is ever
    /// broken. Distinct from [`Self::Empty`] because reading "unhashed" as
    /// "no local keys" would let a caller order the deletion of locally
    /// correct data.
    UnhashedChild,
}

/// Walks the trie from the root, consuming `prefix` one component at a time.
///
/// # Errors
///
/// Propagates any [`FileIoError`] from the underlying store, including one
/// from reading the root node. The root is resolved through
/// `root_as_maybe_persisted_node` rather than `root_node`, whose `Option`
/// return folds a failed root read into "no root" and would make a transient
/// I/O error look like an empty trie.
pub(crate) fn descend_to_prefix<T: TrieReader>(
    nodestore: &T,
    prefix: &[PathComponent],
) -> Result<ProbeOutcome, FileIoError> {
    let Some(root) = nodestore.root_as_maybe_persisted_node() else {
        return Ok(ProbeOutcome::Empty);
    };
    let mut node = root.as_shared_node(nodestore)?;
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

/// The canonical hash of the local subtree under `prefix`: the value a
/// sealed sibling stub at that position would hold if the local trie were
/// the target. `Ok(None)` means no local key carries the prefix.
///
/// Three positions yield a hash. A probe ending exactly on a child edge
/// returns the parent's stored hash for that child, verbatim. A probe ending
/// inside or at the start of a node's partial path re-encodes the node with
/// the probe as its parent prefix and the unconsumed remainder as its partial
/// path, through [`HashableShunt`] under `H`. Under the MerkleDB scheme that
/// re-encoding hashes the same full path and reproduces the stored hash;
/// under the Ethereum scheme the partial path is part of the preimage, so
/// the result differs from the stored hash and matches what a trie with a
/// child edge at `prefix` would store.
///
/// No account `storageRoot` repair is needed on the way. Databases written
/// before `firewood-v1-hfix` store account values with a stale `storageRoot`
/// field, but the Ethereum hasher derives that field while building the
/// preimage (from the child hashes, or the empty-trie root when there are
/// none), so a re-encoded account node hashes canonically on any database
/// version.
///
/// # Errors
///
/// [`api::Error::UnhashedView`] when the probe runs through or lands on a
/// node whose child slots include a [`Child::Node`], which carries no hash.
/// Node reads, the root included, propagate their [`FileIoError`]. Neither
/// case is folded into `Ok(None)`: a consumer reads `None` as "the target
/// has keys here and the local trie has none" or as "nothing to delete", and
/// either reading over an unreadable or unhashed subtree orders the wrong
/// remedy.
pub(crate) fn subtree_hash<H: HashMode, T: HashedNodeReader>(
    view: &T,
    prefix: &[PathComponent],
) -> Result<Option<HashType>, api::Error> {
    debug_assert_eq!(
        view.node_hash_algorithm(),
        H::ALGORITHM,
        "subtree hashes must be formed under the view's own hash mode"
    );

    match descend_to_prefix(view, prefix)? {
        ProbeOutcome::Empty => Ok(None),
        ProbeOutcome::EdgeExact(hash) => Ok(Some(hash)),
        ProbeOutcome::UnhashedChild => Err(api::Error::UnhashedView {
            reason: "the probed path runs through a child held in memory without a hash",
        }),
        ProbeOutcome::AtNode { node, consumed } => {
            let partial = node.partial_path().as_components();
            let remaining = partial.get(consumed..).unwrap_or_default();
            let (value_digest, children) = hashable_parts(&node)?;
            Ok(Some(H::to_hash(&HashableShunt::new(
                prefix,
                remaining,
                value_digest,
                children,
            ))))
        }
    }
}

/// A node's value digest and child hashes, as [`HashableShunt`] takes them.
pub(crate) type HashableParts<'a> = (Option<ValueDigest<&'a [u8]>>, Children<Option<HashType>>);

/// The value digest and child hashes of `node`, as [`HashableShunt`] takes
/// them: a leaf contributes its value and no children, a branch its value
/// and its children's stored hashes.
///
/// # Errors
///
/// [`api::Error::UnhashedView`] when a child slot holds a [`Child::Node`],
/// which carries no hash. `children_hashes()` would silently report that
/// slot as absent, and a commitment formed over an absent child is wrong
/// rather than merely incomplete.
pub(crate) fn hashable_parts(node: &Node) -> Result<HashableParts<'_>, api::Error> {
    match node {
        Node::Leaf(leaf) => Ok((Some(ValueDigest::Value(&*leaf.value)), Children::new())),
        Node::Branch(branch) => {
            if branch
                .children
                .iter()
                .any(|(_, child)| matches!(child, Some(Child::Node(_))))
            {
                return Err(api::Error::UnhashedView {
                    reason: "the node has a child held in memory without a hash",
                });
            }
            Ok((
                branch.value.as_deref().map(ValueDigest::Value),
                branch.children_hashes(),
            ))
        }
    }
}
