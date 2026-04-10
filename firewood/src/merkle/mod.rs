// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#[cfg(test)]
pub(crate) mod tests;

pub(crate) mod changes;
pub(crate) mod childmask;
pub(crate) mod collapse;
mod merge;
/// Parallel merkle
pub mod parallel;

use crate::api::{
    self, BatchIter, FrozenProof, FrozenRangeProof, KeyType, KeyValuePair, ValueType,
};
use crate::iter::{MerkleKeyValueIter, PathIterator};
use crate::merkle::changes::{ChangeProof, DiffMerkleNodeStream};
use crate::{Proof, ProofCollection, ProofError, ProofNode, RangeProof};
use firewood_metrics::firewood_counter;
use firewood_storage::MemStore;
use firewood_storage::{
    BranchNode, Child, Children, FileIoError, HashType, HashableShunt, HashedNodeReader,
    ImmutableProposal, IntoHashType, LeafNode, MaybePersistedNode, Mutable, MutableKind,
    NibblesIterator, Node, NodeStore, Path, PathBuf, PathComponent, Propose, ReadableStorage,
    SharedNode, TrieHash, TrieReader, U4, ValueDigest,
};
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::io::Error;
use std::iter::once;
use std::num::NonZeroUsize;
use std::sync::Arc;

/// Keys are boxed u8 slices
pub type Key = Box<[u8]>;

/// Values are boxed u8 slices
pub type Value = Box<[u8]>;

use childmask::ChildMask;

macro_rules! write_attributes {
    ($writer:ident, $node:expr, $value:expr) => {
        if !$node.partial_path.0.is_empty() {
            write!($writer, " pp={:x}", $node.partial_path)
                .map_err(|e| FileIoError::from_generic_no_file(e, "write attributes"))?;
        }
        if !$value.is_empty() {
            match std::str::from_utf8($value) {
                Ok(string) if string.chars().all(char::is_alphanumeric) => {
                    write!($writer, " val={:.6}", string)
                        .map_err(|e| FileIoError::from_generic_no_file(e, "write attributes"))?;
                    if string.len() > 6 {
                        $writer.write_all(b"...").map_err(|e| {
                            FileIoError::from_generic_no_file(e, "write attributes")
                        })?;
                    }
                }
                _ => {
                    let hex = hex::encode($value);
                    write!($writer, " val={:.6}", hex)
                        .map_err(|e| FileIoError::from_generic_no_file(e, "write attributes"))?;
                    if hex.len() > 6 {
                        $writer.write_all(b"...").map_err(|e| {
                            FileIoError::from_generic_no_file(e, "write attributes")
                        })?;
                    }
                }
            }
        }
    };
}

/// Returns the value mapped to by `key` in the subtrie rooted at `node`.
fn get_helper<T: TrieReader>(
    nodestore: &T,
    node: &Node,
    key: &[u8],
) -> Result<Option<SharedNode>, FileIoError> {
    // 4 possibilities for the position of the `key` relative to `node`:
    // 1. The node is at `key`
    // 2. The key is above the node (i.e. its ancestor)
    // 3. The key is below the node (i.e. its descendant)
    // 4. Neither is an ancestor of the other
    let path_overlap = PrefixOverlap::from(key, node.partial_path());
    let unique_key = path_overlap.unique_a;
    let unique_node = path_overlap.unique_b;

    match (
        unique_key.split_first().map(|(index, path)| (*index, path)),
        unique_node.split_first(),
    ) {
        (_, Some(_)) => {
            // Case (2) or (4)
            Ok(None)
        }
        (None, None) => Ok(Some(node.clone().into())), // 1. The node is at `key`
        (Some((child_index, remaining_key)), None) => {
            let child_index = PathComponent::try_new(child_index).expect("index is in bounds");
            // 3. The key is below the node (i.e. its descendant)
            match node {
                Node::Leaf(_) => Ok(None),
                Node::Branch(node) => match node.children[child_index].as_ref() {
                    None => Ok(None),
                    Some(Child::Node(child)) => get_helper(nodestore, child, remaining_key),
                    Some(Child::AddressWithHash(addr, _)) => {
                        let child = nodestore.read_node(*addr)?;
                        get_helper(nodestore, &child, remaining_key)
                    }
                    Some(Child::MaybePersisted(maybe_persisted, _)) => {
                        let child = maybe_persisted.as_shared_node(nodestore)?;
                        get_helper(nodestore, &child, remaining_key)
                    }
                },
            }
        }
    }
}

#[derive(Debug)]
/// Merkle operations against a nodestore
pub struct Merkle<T> {
    nodestore: T,
}

impl<T> Merkle<T> {
    pub(crate) fn into_inner(self) -> T {
        self.nodestore
    }
}

impl<T> From<T> for Merkle<T> {
    fn from(nodestore: T) -> Self {
        Merkle { nodestore }
    }
}

/// Verify one edge (left or right) of a range proof.
///
/// Empty proofs are accepted without verification. Otherwise, checks that the
/// requested bound is consistent with the edge key-value pair, then verifies
/// the proof against the root hash.
fn verify_edge<H: ProofCollection + ?Sized>(
    requested_bound: Option<&[u8]>,
    edge_kv: Option<(&[u8], &[u8])>,
    edge_proof: &Proof<H>,
    root_hash: &TrieHash,
    bound_is_lower: bool,
) -> Result<(), api::Error> {
    if edge_proof.is_empty() {
        return Ok(());
    }

    // Validate bound vs edge key ordering
    if let (Some(bound), Some((edge_key, _))) = (requested_bound, edge_kv) {
        let out_of_order = if bound_is_lower {
            bound > edge_key
        } else {
            bound < edge_key
        };
        if out_of_order {
            let proof_error = if bound_is_lower {
                ProofError::RangeProofStartBeyondFirstKey
            } else {
                ProofError::RangeProofEndBeforeLastKey
            };
            return Err(api::Error::ProofError(proof_error));
        }
    }

    // Verify the proof for this edge
    if let Some(bound) = requested_bound {
        let expected_value: Option<&[u8]> =
            edge_kv.and_then(|(key, value)| (bound == key).then_some(value));
        edge_proof.verify(bound, expected_value, root_hash)?;
    } else if let Some((edge_key, edge_value)) = edge_kv {
        edge_proof.verify(edge_key, Some(edge_value), root_hash)?;
    }

    Ok(())
}

/// For a proof edge path, computes which child indices at each proof node are
/// "outside" the proven range and should use the proof's child hashes.
///
/// For the left edge: children with index < the on-path child are outside.
/// For the right edge: children with index > the on-path child are outside.
///
/// The `boundary_key` (in bytes, not nibbles) is used to determine the on-path
/// nibble at the terminal proof node, since there is no subsequent proof node
/// to derive it from.
fn compute_outside_children(
    proof_nodes: &[ProofNode],
    boundary_key: Option<&[u8]>,
    is_left_edge: bool,
) -> Result<HashMap<PathBuf, ChildMask>, ProofError> {
    let mut result: HashMap<PathBuf, ChildMask> = HashMap::new();

    // Non-terminal nodes: derive the on-path nibble from the next proof node
    for (parent, child) in proof_nodes.iter().zip(proof_nodes.iter().skip(1)) {
        let on_path_nibble = child
            .key
            .get(parent.key.len())
            .ok_or(ProofError::ShouldBePrefixOfNextKey)?;
        let entry = result.entry(parent.key.clone()).or_default();
        *entry = if is_left_edge {
            entry.set_below(on_path_nibble.0)
        } else {
            entry.set_above(on_path_nibble.0)
        };
    }

    // Terminal node: derive the on-path nibble from the boundary key.
    // If the boundary key diverges within the terminal node's partial path,
    // either all or none of the children are outside the range.
    if let (Some(terminal), Some(boundary)) = (proof_nodes.last(), boundary_key) {
        let boundary_nibbles: Vec<u8> = NibblesIterator::new(boundary).collect();

        // Find the first position where boundary and terminal key diverge,
        // capturing the diverging values to avoid re-indexing.
        let divergence = terminal
            .key
            .iter()
            .zip(boundary_nibbles.iter())
            .find(|(tk, bn)| tk.as_u8() != **bn);

        if let Some((tk, bn)) = divergence {
            // Boundary diverges within the terminal's key at this position.
            // If boundary is "past" the terminal (left edge: B > terminal,
            // right edge: B < terminal), all children are outside.
            let all_outside = if is_left_edge {
                *bn > tk.as_u8()
            } else {
                *bn < tk.as_u8()
            };
            if all_outside {
                result.insert(terminal.key.clone(), ChildMask::ALL);
            }
            // Otherwise (boundary is "before" terminal), no children are outside.
        } else if let Some(on_path_byte) = boundary_nibbles.get(terminal.key.len()) {
            // Terminal is an ancestor of the boundary key. The next
            // nibble tells us which child leads toward the boundary.
            // Mark children on the far side of that nibble as outside,
            // and also mark the on-path child itself: its subtree may
            // contain keys beyond the proven range, so we must use the
            // proof's hash rather than recomputing it.
            let on_path_nibble = U4::new_masked(*on_path_byte);
            let entry = result.entry(terminal.key.clone()).or_default();
            *entry = if is_left_edge {
                entry.set_below(on_path_nibble)
            } else {
                entry.set_above(on_path_nibble)
            }
            .set(on_path_nibble);
        } else if !is_left_edge {
            // Boundary is a prefix of or exactly matches the terminal key.
            // For the right edge, all children extend beyond end_key (they
            // represent keys longer than end_key sharing its prefix), so
            // they are outside the proven range.
            result.insert(terminal.key.clone(), ChildMask::ALL);
        }
        // For the left edge when boundary matches/is-prefix-of terminal,
        // children extend beyond start_key and are in-range — no marking.
    }

    Ok(result)
}

/// Recursively computes the hash of a node in the proving trie, merging
/// child hashes from proof nodes for subtrees outside the proven range.
///
/// For branch nodes, children present in the in-memory trie get their hash
/// computed recursively. Children not in the trie that are **outside** the
/// proven range (as indicated by `outside_children`) get their hash from the
/// corresponding proof node. Children not in the trie that are **inside** the
/// range are left as `None`, causing a hash mismatch if they should exist.
fn compute_root_hash_with_proofs(
    node: &Node,
    path_prefix: &[PathComponent],
    proof_nodes: &HashMap<PathBuf, &ProofNode>,
    outside_children: &HashMap<PathBuf, ChildMask>,
) -> HashType {
    let branch = match node {
        Node::Leaf(_) => return HashableShunt::from_node(path_prefix, node).to_hash(),
        Node::Branch(branch) => branch,
    };

    // Build full key for this node: path_prefix ++ partial_path
    let full_key: PathBuf = path_prefix
        .iter()
        .chain(branch.partial_path.as_components().iter())
        .copied()
        .collect();

    let mut child_hashes: Children<Option<HashType>> = Children::new();

    // For children outside the proven range, use proof node hashes
    if let (Some(proof_node), Some(outside)) =
        (proof_nodes.get(&full_key), outside_children.get(&full_key))
    {
        for (nibble, hash) in proof_node.child_hashes.iter_present() {
            if outside.is_set(nibble.0) {
                child_hashes[nibble] = Some(hash.clone());
            }
        }
    }

    // For children in the in-memory trie, compute hashes recursively.
    // These children were inserted from the proven key-value pairs, so they
    // are *inside* the proven range. A nibble cannot be both inside (present
    // in the trie) and outside (marked in outside_children) at the same time,
    // so this does not conflict with the proof hashes set above.
    let mut child_prefix: PathBuf = full_key.iter().copied().collect();
    for (nibble, child_opt) in &branch.children {
        if let Some(Child::Node(child_node)) = child_opt {
            child_prefix.push(nibble);
            let child_hash = compute_root_hash_with_proofs(
                child_node,
                &child_prefix,
                proof_nodes,
                outside_children,
            );
            child_hashes[nibble] = Some(child_hash);
            child_prefix.pop();
        }
    }

    let value = branch.value.as_deref().map(ValueDigest::Value);
    HashableShunt::new(
        path_prefix,
        branch.partial_path.as_components(),
        value,
        child_hashes,
    )
    .to_hash()
}

/// Verify that a range proof is valid for the specified key range and root hash.
///
/// This function validates a range proof by constructing a partial trie from the
/// proof data and verifying that it produces the expected root hash. The proof may
/// contain fewer key-value pairs than requested if the peer chose to limit the
/// response size.
///
/// # Verification Process
///
/// 1. **Structural validation**: Ensure key-value pairs are in strictly ascending order,
///    within the requested `[first_key, last_key]` range, and reject unexpected proofs
/// 2. **Boundary proof verification**: Cryptographically verify start/end proofs against
///    the provided root hash
/// 3. **Trie reconstruction**: Build an in-memory Merkle trie from the key-value pairs
///    and reconcile it with proof nodes
/// 4. **Root hash comparison**: Verify the reconstructed trie's root hash matches the
///    expected hash
///
/// # Errors
///
/// Returns [`api::Error::ProofError`] if the proof is structurally invalid,
/// keys are outside the requested range, boundary proofs fail verification,
/// or the reconstructed root hash doesn't match.
#[allow(clippy::too_many_lines)]
pub fn verify_range_proof<H: ProofCollection<Node = ProofNode>>(
    first_key: Option<impl KeyType>,
    last_key: Option<impl KeyType>,
    root_hash: &TrieHash,
    proof: &RangeProof<impl KeyType, impl ValueType, H>,
) -> Result<(), api::Error> {
    let first_key_bytes: Option<&[u8]> = first_key.as_ref().map(AsRef::as_ref);
    let last_key_bytes: Option<&[u8]> = last_key.as_ref().map(AsRef::as_ref);

    // Reject invalid range where start > end
    if let (Some(start), Some(end)) = (first_key_bytes, last_key_bytes)
        && start > end
    {
        return Err(api::Error::ProofError(ProofError::StartAfterEnd));
    }

    // check that the keys are in ascending order and within the requested range
    let key_values = proof.key_values();
    if !key_values
        .iter()
        .map(|(key, _)| key.as_ref())
        .is_sorted_by(|a, b| a < b)
    {
        return Err(api::Error::ProofError(
            ProofError::NonMonotonicIncreaseRange,
        ));
    }

    // Validate all keys are within the requested [first_key, last_key] range
    for (key, _) in key_values {
        let k = key.as_ref();
        if first_key_bytes.is_some_and(|start| k < start)
            || last_key_bytes.is_some_and(|end| k > end)
        {
            return Err(api::Error::ProofError(ProofError::KeyOutsideRange));
        }
    }

    if key_values.is_empty() && first_key_bytes.is_none() && last_key_bytes.is_none() {
        return Err(api::Error::ProofError(ProofError::Empty));
    }

    // Reject start proof when no start key is specified (non-canonical proof)
    if first_key_bytes.is_none() && !proof.start_proof().is_empty() {
        return Err(api::Error::ProofError(ProofError::UnexpectedStartProof));
    }

    // Require end proof when there are key-value pairs or an end key is specified
    if proof.end_proof().is_empty() && (last_key_bytes.is_some() || !key_values.is_empty()) {
        return Err(api::Error::ProofError(ProofError::NoEndProof));
    }

    let left_edge_key = key_values.first();
    let right_edge_key = key_values.last();

    let left_edge = left_edge_key.map(|(k, v)| (k.as_ref(), v.as_ref()));
    let right_edge = right_edge_key.map(|(k, v)| (k.as_ref(), v.as_ref()));

    if !proof.start_proof().is_empty() {
        verify_edge(
            first_key_bytes,
            left_edge,
            proof.start_proof(),
            root_hash,
            true,
        )?;
    }
    if !proof.end_proof().is_empty() {
        verify_edge(
            last_key_bytes,
            right_edge,
            proof.end_proof(),
            root_hash,
            false,
        )?;
    }

    let all_proof_nodes: Box<[&ProofNode]> = proof
        .start_proof()
        .as_ref()
        .iter()
        .chain(proof.end_proof().as_ref())
        .collect();

    verify_proof_node_values(
        &all_proof_nodes,
        first_key_bytes,
        last_key_bytes,
        key_values,
    )?;

    verify_range_proof_root_hash(
        &all_proof_nodes,
        key_values,
        proof,
        first_key_bytes,
        last_key_bytes,
        root_hash,
    )
}

/// Verifies that proof nodes with values within the range are included in `key_values`.
/// Without this check, an attacker could hide key-value pairs that exist on an edge
/// proof path by omitting them from `key_values` while reconciliation silently inserts
/// them, making the root hash correct.
fn verify_proof_node_values(
    proof_nodes: &[&ProofNode],
    first_key_bytes: Option<&[u8]>,
    last_key_bytes: Option<&[u8]>,
    key_values: &[(impl KeyType, impl ValueType)],
) -> Result<(), api::Error> {
    for proof_node in proof_nodes {
        // Only even-nibble-length keys correspond to byte keys with values
        if !proof_node.key.len().is_multiple_of(2) {
            continue;
        }
        // Only check nodes that have values
        if !matches!(proof_node.value_digest, Some(ValueDigest::Value(_))) {
            continue;
        }
        let key_nibbles: Vec<u8> = proof_node
            .key
            .iter()
            .map(|component| component.as_u8())
            .collect();
        let node_key_bytes: Vec<u8> = Path::from(key_nibbles.as_slice()).bytes_iter().collect();
        let in_range = first_key_bytes.is_none_or(|start| node_key_bytes.as_slice() >= start)
            && last_key_bytes.is_none_or(|end| node_key_bytes.as_slice() <= end);
        if in_range {
            match key_values.binary_search_by(|(k, _)| k.as_ref().cmp(node_key_bytes.as_slice())) {
                Err(_) => {
                    return Err(api::Error::ProofError(
                        ProofError::ProofNodeHasUnincludedValue,
                    ));
                }
                Ok(idx) => {
                    let proof_value = match &proof_node.value_digest {
                        Some(ValueDigest::Value(v)) => v.as_ref(),
                        _ => unreachable!("guarded by matches! check above"),
                    };
                    let Some((_, kv_value)) = key_values.get(idx) else {
                        unreachable!("binary_search returned valid index");
                    };
                    if kv_value.as_ref() != proof_value {
                        return Err(api::Error::ProofError(ProofError::ProofNodeValueMismatch));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Reconstructs the trie from key-value pairs and proof nodes, then verifies
/// that the computed root hash matches the expected one.
fn verify_range_proof_root_hash<H: ProofCollection<Node = ProofNode>>(
    all_proof_nodes: &[&ProofNode],
    key_values: &[(impl KeyType, impl ValueType)],
    proof: &RangeProof<impl KeyType, impl ValueType, H>,
    first_key_bytes: Option<&[u8]>,
    last_key_bytes: Option<&[u8]>,
    root_hash: &TrieHash,
) -> Result<(), api::Error> {
    // Build in-memory merkle from key-value pairs
    let memstore = MemStore::default();
    let nodestore = NodeStore::new_empty_proposal(memstore.into());
    let mut proving_merkle: Merkle<NodeStore<Mutable<Propose>, MemStore>> = Merkle::from(nodestore);

    for (key, value) in key_values {
        proving_merkle.insert(key.as_ref(), value.as_ref().into())?;
    }

    // Reconcile proof nodes into the proving trie and build a lookup map.
    // "Reconcile" means adjusting the proving trie's branch structure
    // (partial paths and child layout) to match the proof, so that hash
    // computation produces the same trie shape as the original.
    // Conflicting proof nodes (same key, different data) are rejected.
    let mut proof_node_map: HashMap<PathBuf, &ProofNode> = HashMap::new();
    for proof_node in all_proof_nodes {
        proving_merkle.reconcile_branch_proof_node(proof_node)?;
        match proof_node_map.entry(proof_node.key.clone()) {
            std::collections::hash_map::Entry::Occupied(existing) => {
                if *existing.get() != *proof_node {
                    return Err(api::Error::ProofError(ProofError::ConflictingProofNodes));
                }
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(proof_node);
            }
        }
    }

    // Compute which children at each edge node are outside the proven range
    let mut outside_children =
        compute_outside_children(proof.start_proof().as_ref(), first_key_bytes, true)?;
    for (key, flags) in compute_outside_children(proof.end_proof().as_ref(), last_key_bytes, false)?
    {
        let entry = outside_children.entry(key).or_default();
        *entry |= flags;
    }

    // Compute root hash of the proving trie with proof sibling hashes
    let Some(root_node) = proving_merkle.root() else {
        return Err(api::Error::ProofError(ProofError::Empty));
    };

    let computed =
        compute_root_hash_with_proofs(&root_node, &[], &proof_node_map, &outside_children);

    if computed != root_hash.clone().into_hash_type() {
        return Err(api::Error::ProofError(ProofError::UnexpectedHash));
    }

    Ok(())
}

impl<T: TrieReader> Merkle<T> {
    pub(crate) fn root(&self) -> Option<SharedNode> {
        self.nodestore.root_node()
    }

    // Must be pub because it is used in FFI calls.
    pub const fn nodestore(&self) -> &T {
        &self.nodestore
    }

    /// Returns a proof that the given key has a certain value,
    /// or that the key isn't in the trie.
    ///
    /// ## Errors
    ///
    /// Returns an error if the trie is empty or an error occurs while reading from storage.
    pub fn prove(&self, key: &[u8]) -> Result<FrozenProof, ProofError> {
        self.root().ok_or(ProofError::Empty)?;

        // PathIterator yields every node on the path from root toward
        // `key`, including divergent nodes whose partial_path doesn't
        // match (they prove the key's absence in exclusion proofs).
        let proof = self
            .path_iter(key)?
            .map(|node| node.map(ProofNode::from))
            .collect::<Result<_, _>>()?;

        Ok(Proof::new(proof))
    }

    /// Merges a sequence of key-value pairs with the base merkle trie, yielding
    /// a sequence of [`BatchOp`]s that describe the changes.
    ///
    /// The key-value range is considered total, meaning keys within the inclusive
    /// bounds that are present within the base trie but not in the key-value iterator
    /// will be yielded as [`BatchOp::Delete`] or [`BatchOp::DeleteRange`] operations.
    ///
    /// # Invariant
    ///
    /// The key-value pairs provided by the `key_values` iterator must be sorted in
    /// ascending order, not contain duplicates, and must lie within the specified
    /// `first_key` and `last_key` bounds (if provided). Behavior is unspecified if
    /// this invariant is violated and no verification is performed.
    ///
    /// # Errors
    ///
    /// Returns an error if there is an issue reading from the underlying trie storage.
    ///
    /// [`BatchOp`]: crate::db::BatchOp
    /// [`BatchOp::Delete`]: crate::db::BatchOp::Delete
    /// [`BatchOp::DeleteRange`]: crate::db::BatchOp::DeleteRange
    pub fn merge_key_value_range<V: ValueType>(
        &self,
        first_key: Option<impl KeyType>,
        last_key: Option<impl KeyType>,
        key_values: impl IntoIterator<Item: KeyValuePair<Value = V>>,
    ) -> impl BatchIter {
        merge::MergeKeyValueIter::new(self, first_key, last_key, key_values)
    }

    pub(crate) fn path_iter<'a>(
        &self,
        key: &'a [u8],
    ) -> Result<PathIterator<'_, 'a, T>, FileIoError> {
        PathIterator::new(&self.nodestore, key)
    }

    pub(super) fn key_value_iter_from_key<K: AsRef<[u8]>>(
        &self,
        key: K,
    ) -> MerkleKeyValueIter<'_, T> {
        // TODO danlaine: change key to &[u8]
        MerkleKeyValueIter::from_key(&self.nodestore, key.as_ref())
    }

    /// Generate a cryptographic proof for a range of key-value pairs in the Merkle trie.
    ///
    /// This method creates a range proof that can be used to verify the existence (or absence)
    /// of a contiguous set of keys within the trie. The proof includes boundary proofs and
    /// the actual key-value pairs within the specified range.
    ///
    /// # Parameters
    ///
    /// * `start_key` - The optional lower bound of the range (inclusive).
    ///   - If `Some(key)`, the proof will include all keys >= this key
    ///   - If `None`, the proof starts from the beginning of the trie
    ///
    /// * `end_key` - The optional upper bound of the range (inclusive).
    ///   - If `Some(key)`, the proof will include all keys <= this key
    ///   - If `None`, the proof extends to the end of the trie
    ///
    /// * `limit` - Optional maximum number of key-value pairs to include in the proof.
    ///   - If `Some(n)`, at most n key-value pairs will be included
    ///   - If `None`, all key-value pairs in the range will be included
    ///   - Useful for paginating through large ranges
    ///   - **NOTE**: avalanchego's limit is based on the entire packet size and not the
    ///     number of key-value pairs. Currently, we only limit by the number of pairs.
    ///
    /// # Returns
    ///
    /// A `FrozenRangeProof` containing:
    /// - Start proof: Merkle proof for the first key in the range
    /// - End proof: Merkle proof for the last key in the range
    /// - Key-value pairs: All entries within the specified bounds (up to the limit)
    ///
    /// # Errors
    ///
    /// * `api::Error::InvalidRange` - If `start_key` > `end_key` when both are provided.
    ///   This ensures the range bounds are logically consistent.
    ///
    /// * `api::Error::RangeProofOnEmptyTrie` - If the trie is empty and the caller
    ///   requests a proof for the entire trie (both `start_key` and `end_key` are `None`).
    ///   This prevents generating meaningless proofs for non-existent data.
    ///
    /// * `api::Error` - Various other errors can occur during proof generation, such as:
    ///   - I/O errors when reading nodes from storage
    ///   - Corrupted trie structure
    ///   - Invalid node references
    pub(super) fn range_proof(
        &self,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        limit: Option<NonZeroUsize>,
    ) -> Result<FrozenRangeProof, api::Error> {
        if let (Some(k1), Some(k2)) = (&start_key, &end_key)
            && k1 > k2
        {
            return Err(api::Error::InvalidRange {
                start_key: k1.to_vec().into(),
                end_key: k2.to_vec().into(),
            });
        }

        // if there is a requested lower bound, the start proof must always be
        // for that key even if (especially if) the key is not present in the
        // trie so that the requestor can assert no keys exist between the
        // requested key and the first provided key.
        let start_proof = start_key
            .map(|key| self.prove(key))
            .transpose()?
            .unwrap_or_default();

        let mut iter = self
            .key_value_iter_from_key(start_key.unwrap_or_default())
            .stop_after_key(end_key);

        // don't consume the iterator so we can determine if we hit the
        // limit or exhausted the iterator later
        let key_values = iter
            .by_ref()
            .take(limit.map_or(usize::MAX, NonZeroUsize::get))
            .collect::<Result<Box<_>, FileIoError>>()?;

        if key_values.is_empty() && start_key.is_none() && end_key.is_none() {
            // unbounded range proof yielded no key-values, so the trie must be empty
            return Err(api::Error::RangeProofOnEmptyTrie);
        }

        let end_proof = if let Some(limit) = limit
            && limit.get() <= key_values.len()
            && iter.next().is_some()
        {
            // limit was provided, we hit it, and there is at least one more key
            // end proof is for the last key provided
            key_values.last().map(|(largest_key, _)| &**largest_key)
        } else {
            // limit was not hit or not provided, end proof is for the requested
            // end key so that we can prove we have all keys up to that key
            end_key
        }
        .map(|end_key| self.prove(end_key))
        .transpose()?
        .unwrap_or_default();

        Ok(RangeProof::new(start_proof, end_proof, key_values))
    }

    pub(crate) fn get_value(&self, key: &[u8]) -> Result<Option<Value>, FileIoError> {
        let Some(node) = self.get_node(key)? else {
            return Ok(None);
        };
        Ok(node.value().map(|v| v.to_vec().into_boxed_slice()))
    }

    pub(crate) fn get_node(&self, key: &[u8]) -> Result<Option<SharedNode>, FileIoError> {
        let Some(root) = self.root() else {
            return Ok(None);
        };

        let key = Path::from_nibbles_iterator(NibblesIterator::new(key));
        get_helper(&self.nodestore, &root, &key)
    }

    /// Dump a node, recursively, to a dot file.
    ///
    /// The `keep_alive` vec holds references to all `MaybePersistedNode`s
    /// created during the dump so their addresses remain stable for dup
    /// detection in `seen`.
    pub(crate) fn dump_node<W: std::io::Write + ?Sized>(
        &self,
        node: &MaybePersistedNode,
        hash: Option<&HashType>,
        seen: &mut HashSet<String>,
        keep_alive: &mut Vec<MaybePersistedNode>,
        writer: &mut W,
    ) -> Result<(), FileIoError> {
        writeln!(writer, "  {node}[label=\"{node}")
            .map_err(Error::other)
            .map_err(|e| FileIoError::new(e, None, 0, None))?;
        if let Some(hash) = hash {
            write!(writer, " H={hash:.6?}")
                .map_err(Error::other)
                .map_err(|e| FileIoError::new(e, None, 0, None))?;
        }

        match &*node.as_shared_node(&self.nodestore)? {
            Node::Branch(b) => {
                write_attributes!(writer, b, &b.value.clone().unwrap_or(Box::from([])));
                writeln!(writer, "\"]")
                    .map_err(|e| FileIoError::from_generic_no_file(e, "write branch"))?;
                for (childidx, child) in &b.children {
                    let (child, child_hash) = match child {
                        None => continue,
                        Some(node) => (node.as_maybe_persisted_node(), node.hash()),
                    };

                    let inserted = seen.insert(format!("{child}"));
                    keep_alive.push(child.clone());
                    if inserted {
                        writeln!(writer, "  {node} -> {child}[label=\"{childidx:x}\"]")
                            .map_err(|e| FileIoError::from_generic_no_file(e, "write branch"))?;
                        self.dump_node(&child, child_hash, seen, keep_alive, writer)?;
                    } else {
                        // We have already seen this child, which shouldn't happen.
                        // Indicate this with a red edge.
                        writeln!(
                            writer,
                            "  {node} -> {child}[label=\"{childidx:x} (dup)\" color=red]"
                        )
                        .map_err(|e| FileIoError::from_generic_no_file(e, "write branch"))?;
                    }
                }
            }
            Node::Leaf(l) => {
                write_attributes!(writer, l, &l.value);
                writeln!(writer, "\" shape=rect]")
                    .map_err(|e| FileIoError::from_generic_no_file(e, "write leaf"))?;
            }
        }
        Ok(())
    }

    /// Dump the trie to a dot file without requiring hashed nodes.
    ///
    /// This works on any `TrieReader` including mutable proposals that
    /// haven't been frozen yet. The root hash will be omitted from the output.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to the output writer fails.
    pub(crate) fn dump<W: std::io::Write + ?Sized>(&self, writer: &mut W) -> Result<(), Error> {
        let root = self.nodestore.root_as_maybe_persisted_node();

        writeln!(writer, "digraph Merkle {{\n  rankdir=LR;").map_err(Error::other)?;
        if let Some(root) = root {
            writeln!(writer, " root -> {root}")
                .map_err(Error::other)
                .map_err(|e| FileIoError::new(e, None, 0, None))
                .map_err(Error::other)?;
            let mut seen = HashSet::new();
            let mut keep_alive = Vec::new();
            self.dump_node(&root, None, &mut seen, &mut keep_alive, writer)
                .map_err(Error::other)?;
        }
        writeln!(writer, "}}")
            .map_err(Error::other)
            .map_err(|e| FileIoError::new(e, None, 0, None))
            .map_err(Error::other)?;

        Ok(())
    }

    /// Dump the trie to a string (for testing or logging).
    ///
    /// # Errors
    ///
    /// Returns an error if writing to the string fails.
    pub(crate) fn dump_to_string(&self) -> Result<String, Error> {
        let mut buffer = Vec::new();
        self.dump(&mut buffer)?;
        String::from_utf8(buffer).map_err(Error::other)
    }
}

impl<T: HashedNodeReader> Merkle<T> {
    /// Generate a change proof
    ///
    /// # Errors
    ///
    /// * `api::Error::InvalidRange` - If `start_key` > `end_key` when both are provided.
    ///   This ensures the range bounds are logically consistent.
    ///
    /// * `api::Error` - Various other errors can occur during proof generation, such as:
    ///   - I/O errors when reading nodes from storage
    ///   - Corrupted trie structure
    ///   - Invalid node references
    pub fn change_proof(
        &self,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        source_trie: &T,
        limit: Option<NonZeroUsize>,
    ) -> Result<api::FrozenChangeProof, api::Error> {
        if let (Some(k1), Some(k2)) = (&start_key, &end_key)
            && k1 > k2
        {
            return Err(api::Error::InvalidRange {
                start_key: k1.to_vec().into(),
                end_key: k2.to_vec().into(),
            });
        }

        // If there is a requested lower bound, the start proof must always be
        // for that key even if (especially if) the key is not present in the
        // trie so that the requestor can assert no keys exist between the
        // requested key and the first provided key.
        let start_proof = start_key
            .map(|key| self.prove(key))
            .transpose()?
            .unwrap_or_default();

        // Create a difference iterator between the two tries with the given start
        // key and end key.
        let iter_stop_key: Key = end_key.unwrap_or_default().into();
        let mut iter = DiffMerkleNodeStream::new(
            source_trie,
            self.nodestore(),
            start_key.unwrap_or_default().into(),
        )?
        .map_while(|op| match op {
            Ok(op) => {
                if iter_stop_key.is_empty() || op.key() <= &iter_stop_key {
                    Some(Ok(op))
                } else {
                    None
                }
            }
            Err(e) => Some(Err(e)),
        });

        // Create an array of `BatchOp`s representing the difference between the two
        // revisions. The size of the array is bounded by `limit`.
        let batch_ops = iter
            .by_ref()
            .take(limit.map_or(usize::MAX, NonZeroUsize::get))
            .collect::<Result<Box<_>, FileIoError>>()?;

        let end_proof = if let Some(limit) = limit
            && limit.get() <= batch_ops.len()
            && iter.next().is_some()
        {
            // limit was provided, we hit it, and there is at least one more key
            // end proof is for the last key provided
            batch_ops.last().map(|largest_key| &**largest_key.key())
        } else {
            // limit was not hit or not provided, end proof is for the requested
            // end key so that we can prove we have all keys up to that key
            end_key
        }
        .map(|end_key| self.prove(end_key))
        .transpose()?
        .unwrap_or_default();

        Ok(ChangeProof::new(start_proof, end_proof, batch_ops))
    }
}

#[cfg(test)]
impl<F: firewood_storage::Parentable, S: ReadableStorage> Merkle<NodeStore<F, S>> {
    /// Forks the current Merkle trie into a new mutable proposal.
    ///
    /// ## Errors
    ///
    /// Returns an error if the nodestore cannot be created. See [`NodeStore::new`].
    pub fn fork(&self) -> Result<Merkle<NodeStore<Mutable<Propose>, S>>, FileIoError> {
        NodeStore::new(&self.nodestore).map(Into::into)
    }
}

impl<S: ReadableStorage> TryFrom<Merkle<NodeStore<Mutable<Propose>, S>>>
    for Merkle<NodeStore<Arc<ImmutableProposal>, S>>
{
    type Error = FileIoError;
    fn try_from(m: Merkle<NodeStore<Mutable<Propose>, S>>) -> Result<Self, Self::Error> {
        Ok(Merkle {
            nodestore: m.nodestore.try_into()?,
        })
    }
}

#[cfg(any(test, feature = "test_utils"))]
impl<S: ReadableStorage> Merkle<NodeStore<Mutable<Propose>, S>> {
    /// Convert a merkle backed by a `Mutable<Propose>` into an `ImmutableProposal`
    ///
    /// This function is only used in benchmarks and tests
    ///
    /// ## Panics
    ///
    /// Panics if the conversion fails. This should only be used in tests or benchmarks.
    #[must_use]
    pub fn hash(self) -> Merkle<NodeStore<Arc<ImmutableProposal>, S>> {
        self.try_into().expect("failed to convert")
    }
}

impl<K: MutableKind, S: ReadableStorage> Merkle<NodeStore<Mutable<K>, S>> {
    fn read_for_update(&mut self, child: Child) -> Result<Node, FileIoError> {
        match child {
            Child::Node(node) => Ok(node),
            Child::AddressWithHash(addr, _) => self.nodestore.read_for_update(addr.into()),
            Child::MaybePersisted(node, _) => self.nodestore.read_for_update(node),
        }
    }

    /// Map `key` to `value` in the trie.
    /// Each element of key is 2 nibbles.
    #[cfg_attr(feature = "test_utils", expect(clippy::missing_errors_doc))]
    pub fn insert(&mut self, key: &[u8], value: Value) -> Result<(), FileIoError> {
        self.insert_from_iter(NibblesIterator::new(key), value)
    }

    /// Map `key` to `value` in the trie when `key` is a `NibblesIterator`
    pub(crate) fn insert_from_iter(
        &mut self,
        key: NibblesIterator<'_>,
        value: Value,
    ) -> Result<(), FileIoError> {
        let key = Path::from_nibbles_iterator(key);
        let root = self.nodestore.root_mut();
        let Some(root_node) = std::mem::take(root) else {
            // The trie is empty. Create a new leaf node with `value` and set
            // it as the root.
            let root_node = Node::Leaf(LeafNode {
                partial_path: key,
                value,
            });
            *root = root_node.into();
            return Ok(());
        };

        let root_node = self.insert_helper(root_node, key.as_ref(), value)?;
        *self.nodestore.root_mut() = root_node.into();
        Ok(())
    }

    /// Map `key` to `value` into the subtrie rooted at `node`.
    /// Each element of `key` is 1 nibble.
    /// Returns the new root of the subtrie.
    fn insert_helper(
        &mut self,
        mut node: Node,
        key: &[u8],
        value: Value,
    ) -> Result<Node, FileIoError> {
        // 4 possibilities for the position of the `key` relative to `node`:
        // 1. The node is at `key`
        // 2. The key is above the node (i.e. its ancestor)
        // 3. The key is below the node (i.e. its descendant)
        // 4. Neither is an ancestor of the other
        let path_overlap = PrefixOverlap::from(key, node.partial_path().as_ref());

        let unique_key = path_overlap.unique_a;
        let unique_node = path_overlap.unique_b;

        match (
            unique_key
                .split_first()
                .map(|(index, path)| (*index, path.into())),
            unique_node
                .split_first()
                .map(|(index, path)| (*index, path.into())),
        ) {
            (None, None) => {
                // 1. The node is at `key`
                node.update_value(value);
                firewood_counter!(INSERT, "operation" => "update").increment(1);
                Ok(node)
            }
            (None, Some((child_index, partial_path))) => {
                let child_index = PathComponent::try_new(child_index).expect("valid component");
                // 2. The key is above the node (i.e. its ancestor)
                // Make a new branch node and insert the current node as a child.
                //    ...                ...
                //     |     -->          |
                //    node               key
                //                        |
                //                       node
                let mut branch = BranchNode {
                    partial_path: path_overlap.shared.into(),
                    value: Some(value),
                    children: Children::new(),
                };

                // Shorten the node's partial path since it has a new parent.
                node.update_partial_path(partial_path);
                branch.children[child_index] = Some(Child::Node(node));
                firewood_counter!(INSERT, "operation" => "above").increment(1);

                Ok(Node::Branch(Box::new(branch)))
            }
            (Some((child_index, partial_path)), None) => {
                let child_index = PathComponent::try_new(child_index).expect("valid component");
                // 3. The key is below the node (i.e. its descendant)
                //    ...                         ...
                //     |                           |
                //    node         -->            node
                //     |                           |
                //    ... (key may be below)       ... (key is below)
                match node {
                    Node::Branch(ref mut branch) => {
                        let Some(child) = branch.children.take(child_index) else {
                            // There is no child at this index.
                            // Create a new leaf and put it here.
                            let new_leaf = Node::Leaf(LeafNode {
                                value,
                                partial_path,
                            });
                            branch.children[child_index] = Some(Child::Node(new_leaf));
                            firewood_counter!(INSERT, "operation" => "below").increment(1);
                            return Ok(node);
                        };
                        let child = self.read_for_update(child)?;
                        let child = self.insert_helper(child, partial_path.as_ref(), value)?;
                        branch.children[child_index] = Some(Child::Node(child));
                        Ok(node)
                    }
                    Node::Leaf(leaf) => {
                        // Turn this node into a branch node and put a new leaf as a child.
                        let mut branch = BranchNode {
                            partial_path: leaf.partial_path,
                            value: Some(leaf.value),
                            children: Children::new(),
                        };

                        let new_leaf = Node::Leaf(LeafNode {
                            value,
                            partial_path,
                        });

                        branch.children[child_index] = Some(Child::Node(new_leaf));

                        firewood_counter!(INSERT, "operation" => "split").increment(1);
                        Ok(Node::Branch(Box::new(branch)))
                    }
                }
            }
            (Some((key_index, key_partial_path)), Some((node_index, node_partial_path))) => {
                let key_index = PathComponent::try_new(key_index).expect("valid component");
                let node_index = PathComponent::try_new(node_index).expect("valid component");
                // 4. Neither is an ancestor of the other
                //    ...                         ...
                //     |                           |
                //    node         -->            branch
                //     |                           |    \
                //                               node   key
                // Make a branch node that has both the current node and a new leaf node as children.
                let mut branch = BranchNode {
                    partial_path: path_overlap.shared.into(),
                    value: None,
                    children: Children::new(),
                };

                node.update_partial_path(node_partial_path);
                branch.children[node_index] = Some(Child::Node(node));

                let new_leaf = Node::Leaf(LeafNode {
                    value,
                    partial_path: key_partial_path,
                });
                branch.children[key_index] = Some(Child::Node(new_leaf));

                firewood_counter!(INSERT, "operation" => "split").increment(1);
                Ok(Node::Branch(Box::new(branch)))
            }
        }
    }

    /// Ensures a branch exists at `key` in the subtrie rooted at `node`.
    /// Each element of `key` is 1 nibble.
    fn insert_branch_helper(&mut self, mut node: Node, key: &[u8]) -> Result<Node, FileIoError> {
        let path_overlap = PrefixOverlap::from(key, node.partial_path().as_ref());

        let unique_key = path_overlap.unique_a;
        let unique_node = path_overlap.unique_b;

        match (
            unique_key
                .split_first()
                .map(|(index, path)| (*index, path.into())),
            unique_node
                .split_first()
                .map(|(index, path)| (*index, path.into())),
        ) {
            (None, None) => match node {
                Node::Branch(_) => Ok(node),
                Node::Leaf(leaf) => {
                    let branch = BranchNode {
                        partial_path: leaf.partial_path,
                        value: Some(leaf.value),
                        children: Children::new(),
                    };
                    Ok(Node::Branch(Box::new(branch)))
                }
            },
            (None, Some((child_index, partial_path))) => {
                let child_index = PathComponent::try_new(child_index).expect("valid component");

                let mut branch = BranchNode {
                    partial_path: path_overlap.shared.into(),
                    value: None,
                    children: Children::new(),
                };

                node.update_partial_path(partial_path);
                branch.children[child_index] = Some(Child::Node(node));

                Ok(Node::Branch(Box::new(branch)))
            }
            (Some((child_index, partial_path)), None) => {
                let child_index = PathComponent::try_new(child_index).expect("valid component");

                match node {
                    Node::Branch(ref mut branch) => {
                        let Some(child) = branch.children.take(child_index) else {
                            let new_branch = Node::Branch(Box::new(BranchNode {
                                partial_path,
                                value: None,
                                children: Children::new(),
                            }));
                            branch.children[child_index] = Some(Child::Node(new_branch));
                            return Ok(node);
                        };

                        let child = self.read_for_update(child)?;
                        let child = self.insert_branch_helper(child, partial_path.as_ref())?;
                        branch.children[child_index] = Some(Child::Node(child));
                        Ok(node)
                    }
                    Node::Leaf(leaf) => {
                        let mut branch = BranchNode {
                            partial_path: leaf.partial_path,
                            value: Some(leaf.value),
                            children: Children::new(),
                        };

                        let new_branch = Node::Branch(Box::new(BranchNode {
                            partial_path,
                            value: None,
                            children: Children::new(),
                        }));
                        branch.children[child_index] = Some(Child::Node(new_branch));

                        Ok(Node::Branch(Box::new(branch)))
                    }
                }
            }
            (Some((key_index, key_partial_path)), Some((node_index, node_partial_path))) => {
                let key_index = PathComponent::try_new(key_index).expect("valid component");
                let node_index = PathComponent::try_new(node_index).expect("valid component");

                let mut branch = BranchNode {
                    partial_path: path_overlap.shared.into(),
                    value: None,
                    children: Children::new(),
                };

                node.update_partial_path(node_partial_path);
                branch.children[node_index] = Some(Child::Node(node));

                let new_branch = Node::Branch(Box::new(BranchNode {
                    partial_path: key_partial_path,
                    value: None,
                    children: Children::new(),
                }));
                branch.children[key_index] = Some(Child::Node(new_branch));

                Ok(Node::Branch(Box::new(branch)))
            }
        }
    }

    /// Removes the value associated with the given `key`.
    /// Returns the value that was removed, if any.
    /// Otherwise returns `None`.
    /// Each element of `key` is 2 nibbles.
    pub(crate) fn remove(&mut self, key: &[u8]) -> Result<Option<Value>, FileIoError> {
        self.remove_from_iter(NibblesIterator::new(key))
    }

    /// Removes the value associated with the given `key` where `key` is a `NibblesIterator`
    /// Returns the value that was removed, if any.
    /// Otherwise returns `None`.
    /// Each element of `key` is 2 nibbles.
    pub(crate) fn remove_from_iter(
        &mut self,
        key: NibblesIterator<'_>,
    ) -> Result<Option<Value>, FileIoError> {
        let key = Path::from_nibbles_iterator(key);
        let root = self.nodestore.root_mut();
        let Some(root_node) = std::mem::take(root) else {
            // The trie is empty. There is nothing to remove.
            firewood_counter!(REMOVE, "prefix" => "false", "result" => "nonexistent").increment(1);
            return Ok(None);
        };

        let (root_node, removed_value) = self.remove_helper(root_node, &key)?;
        *self.nodestore.root_mut() = root_node;
        if removed_value.is_some() {
            firewood_counter!(REMOVE, "prefix" => "false", "result" => "success").increment(1);
        } else {
            firewood_counter!(REMOVE, "prefix" => "false", "result" => "nonexistent").increment(1);
        }
        Ok(removed_value)
    }

    /// Removes the value associated with the given `key` from the subtrie rooted at `node`.
    /// Returns the new root of the subtrie and the value that was removed, if any.
    /// Each element of `key` is 1 nibble.
    fn remove_helper(
        &mut self,
        node: Node,
        key: &[u8],
    ) -> Result<(Option<Node>, Option<Value>), FileIoError> {
        // 4 possibilities for the position of the `key` relative to `node`:
        // 1. The node is at `key`
        // 2. The key is above the node (i.e. its ancestor)
        // 3. The key is below the node (i.e. its descendant)
        // 4. Neither is an ancestor of the other
        let path_overlap = PrefixOverlap::from(key, node.partial_path().as_ref());

        let unique_key = path_overlap.unique_a;
        let unique_node = path_overlap.unique_b;

        match (
            unique_key
                .split_first()
                .map(|(index, path)| (*index, Path::from(path))),
            unique_node.split_first(),
        ) {
            (_, Some(_)) => {
                // Case (2) or (4)
                Ok((Some(node), None))
            }
            (None, None) => {
                // 1. The node is at `key`
                match node {
                    Node::Branch(mut branch) => {
                        let Some(removed_value) = branch.value.take() else {
                            // The branch has no value. Return the node as is.
                            return Ok((Some(Node::Branch(branch)), None));
                        };

                        Ok((self.flatten_branch(branch)?, Some(removed_value)))
                    }
                    Node::Leaf(leaf) => Ok((None, Some(leaf.value))),
                }
            }
            (Some((child_index, child_partial_path)), None) => {
                let child_index = PathComponent::try_new(child_index).expect("valid component");
                // 3. The key is below the node (i.e. its descendant)
                match node {
                    // we found a non-matching leaf node, so the value does not exist
                    Node::Leaf(_) => Ok((Some(node), None)),
                    Node::Branch(mut branch) => {
                        let Some(child) = branch.children.take(child_index) else {
                            // child does not exist, so the value does not exist
                            return Ok((Some(Node::Branch(branch)), None));
                        };
                        let child = self.read_for_update(child)?;

                        let (child, removed_value) =
                            self.remove_helper(child, child_partial_path.as_ref())?;

                        branch.children[child_index] = child.map(Child::Node);

                        Ok((self.flatten_branch(branch)?, removed_value))
                    }
                }
            }
        }
    }

    /// Removes any key-value pairs with keys that have the given `prefix`.
    /// Returns the number of key-value pairs removed.
    pub(crate) fn remove_prefix(&mut self, prefix: &[u8]) -> Result<usize, FileIoError> {
        self.remove_prefix_from_iter(NibblesIterator::new(prefix))
    }

    /// Removes any key-value pairs with keys that have the given `prefix` where `prefix` is a `NibblesIterator`
    /// Returns the number of key-value pairs removed.
    pub(crate) fn remove_prefix_from_iter(
        &mut self,
        prefix: NibblesIterator<'_>,
    ) -> Result<usize, FileIoError> {
        let prefix = Path::from_nibbles_iterator(prefix);
        let root = self.nodestore.root_mut();
        let Some(root_node) = std::mem::take(root) else {
            // The trie is empty. There is nothing to remove.
            firewood_counter!(REMOVE, "prefix" => "true", "result" => "nonexistent").increment(1);
            return Ok(0);
        };

        let mut deleted = 0;
        let root_node = self.remove_prefix_helper(root_node, &prefix, &mut deleted)?;
        firewood_counter!(REMOVE, "prefix" => "true", "result" => "success")
            .increment(deleted as u64);
        *self.nodestore.root_mut() = root_node;
        Ok(deleted)
    }

    fn remove_prefix_helper(
        &mut self,
        node: Node,
        key: &[u8],
        deleted: &mut usize,
    ) -> Result<Option<Node>, FileIoError> {
        // 4 possibilities for the position of the `key` relative to `node`:
        // 1. The node is at `key`, in which case we need to delete this node and all its children.
        // 2. The key is above the node (i.e. its ancestor), so the parent needs to be restructured (TODO).
        // 3. The key is below the node (i.e. its descendant), so continue traversing the trie.
        // 4. Neither is an ancestor of the other, in which case there's no work to do.
        let path_overlap = PrefixOverlap::from(key, node.partial_path().as_ref());

        let unique_key = path_overlap.unique_a;
        let unique_node = path_overlap.unique_b;

        match (
            unique_key
                .split_first()
                .map(|(index, path)| (*index, Path::from(path))),
            unique_node.split_first(),
        ) {
            (None, _) => {
                // 1. The node is at `key`, or we're just above it
                // so we can start deleting below here
                match node {
                    Node::Branch(branch) => {
                        if branch.value.is_some() {
                            // a KV pair was in the branch itself
                            *deleted = deleted.saturating_add(1);
                        }
                        self.delete_children(branch, deleted)?;
                    }
                    Node::Leaf(_) => {
                        // the prefix matched only a leaf, so we remove it and indicate only one item was removed
                        *deleted = deleted.saturating_add(1);
                    }
                }
                Ok(None)
            }
            (_, Some(_)) => {
                // Case (2) or (4)
                Ok(Some(node))
            }
            (Some((child_index, child_partial_path)), None) => {
                let child_index = PathComponent::try_new(child_index).expect("valid component");
                // 3. The key is below the node (i.e. its descendant)
                match node {
                    Node::Leaf(_) => Ok(Some(node)),
                    Node::Branch(mut branch) => {
                        let Some(child) = branch.children.take(child_index) else {
                            return Ok(Some(Node::Branch(branch)));
                        };
                        let child = self.read_for_update(child)?;

                        let child =
                            self.remove_prefix_helper(child, child_partial_path.as_ref(), deleted)?;

                        branch.children[child_index] = child.map(Child::Node);

                        self.flatten_branch(branch)
                    }
                }
            }
        }
    }

    /// Recursively deletes all children of a branch node.
    fn delete_children(
        &mut self,
        mut branch: Box<BranchNode>,
        deleted: &mut usize,
    ) -> Result<(), FileIoError> {
        if branch.value.is_some() {
            // a KV pair was in the branch itself
            *deleted = deleted.saturating_add(1);
        }
        for (_, child) in &mut branch.children {
            let Some(child) = child.take() else {
                continue;
            };
            let child = self.read_for_update(child)?;
            match child {
                Node::Branch(child_branch) => {
                    self.delete_children(child_branch, deleted)?;
                }
                Node::Leaf(_) => {
                    *deleted = deleted.saturating_add(1);
                }
            }
        }
        Ok(())
    }

    /// Flattens a branch node into a new node.
    ///
    /// - If the branch has no value and no children, returns `None`.
    /// - If the branch has a value and no children, it becomes a leaf node.
    /// - If the branch has no value and exactly one child, it is replaced by that child
    ///   with an updated partial path.
    /// - If the branch has a value and any children, it is returned as-is.
    /// - If the branch has no value and multiple children, it is returned as-is.
    fn flatten_branch(
        &mut self,
        mut branch_node: Box<BranchNode>,
    ) -> Result<Option<Node>, FileIoError> {
        let mut children_iter = branch_node.children.each_mut().into_iter();

        let (child_index, child) = loop {
            let Some((child_index, child_slot)) = children_iter.next() else {
                // The branch has no children. Turn it into a leaf.
                return match branch_node.value {
                    Some(value) => Ok(Some(Node::Leaf(LeafNode {
                        value,
                        partial_path: branch_node.partial_path,
                    }))),
                    None => Ok(None),
                };
            };

            let Some(child) = child_slot.take() else {
                continue;
            };

            if branch_node.value.is_some() || children_iter.any(|(_, slot)| slot.is_some()) {
                // put the child back in its slot since we removed it
                child_slot.replace(child);

                // explicitly drop the iterator to release the mutable borrow on branch_node
                drop(children_iter);

                // The branch has a value or more than 1 child, so it can't be flattened.
                return Ok(Some(Node::Branch(branch_node)));
            }

            // we have found the only child
            break (child_index, child);
        };

        // The branch has only 1 child. Remove the branch and return the child.
        let mut child = self.read_for_update(child)?;

        // The child's partial path is the concatenation of its (now removed) parent,
        // its (former) child index, and its partial path.
        let child_partial_path = Path::from_nibbles_iterator(
            branch_node
                .partial_path
                .iter()
                .chain(once(&child_index.as_u8()))
                .chain(child.partial_path().iter())
                .copied(),
        );
        child.update_partial_path(child_partial_path);

        Ok(Some(child))
    }
}

/// The outcome of reconciling a branch proof node against the in-memory trie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReconcileResult {
    /// The proof node had no value to reconcile (hash-only or absent).
    NoValue,
    /// The branch already had a matching value.
    ValueAlreadyMatches,
    /// A value was inserted into a branch that previously had none.
    ValueInserted,
}

impl Merkle<NodeStore<Mutable<Propose>, MemStore>> {
    /// Returns the node mapped to by `key_nibbles` where each key element is a
    /// single nibble.
    pub(crate) fn get_node_from_nibbles(
        &self,
        key_nibbles: &[u8],
    ) -> Result<Option<SharedNode>, FileIoError> {
        let Some(root) = self.root() else {
            return Ok(None);
        };

        get_helper(&self.nodestore, &root, key_nibbles)
    }

    /// Ensures a branch exists at `key_nibbles` where each key element is a
    /// single nibble.
    ///
    /// This creates missing branch structure without inserting a value at the
    /// target key. Existing values and descendants are preserved.
    pub(crate) fn insert_branch_from_nibbles(
        &mut self,
        key_nibbles: &[u8],
    ) -> Result<(), FileIoError> {
        let root = self.nodestore.root_mut();
        let Some(root_node) = std::mem::take(root) else {
            let branch = BranchNode {
                partial_path: key_nibbles.into(),
                value: None,
                children: Children::new(),
            };
            *root = Node::Branch(Box::new(branch)).into();
            return Ok(());
        };

        let root_node = self.insert_branch_helper(root_node, key_nibbles)?;
        *self.nodestore.root_mut() = root_node.into();
        Ok(())
    }

    /// Reconciles a branch proof node against the in-memory proving merkle.
    ///
    /// This helper never overwrites an existing branch value. It only
    /// creates missing branch structure and inserts a value when the
    /// branch exists without one.
    ///
    /// ## Arguments
    ///
    /// * `proof_node` - A branch proof node containing the key (as nibble
    ///   path components) and an optional value digest to reconcile.
    ///
    /// ## Errors
    ///
    /// Returns an error if an existing value conflicts with the proof.
    pub(crate) fn reconcile_branch_proof_node(
        &mut self,
        proof_node: &ProofNode,
    ) -> Result<ReconcileResult, ProofError> {
        let key_nibbles = proof_node
            .key
            .iter()
            .map(|component| component.as_u8())
            .collect::<Vec<_>>();
        let key_nibbles = key_nibbles.as_slice();

        if !key_nibbles.len().is_multiple_of(2)
            && matches!(proof_node.value_digest, Some(ValueDigest::Value(_)))
        {
            return Err(ProofError::ValueAtOddNibbleLength);
        }

        self.insert_branch_from_nibbles(key_nibbles)?;

        let Some(ValueDigest::Value(proof_value)) = proof_node.value_digest.as_ref() else {
            // Hash-only value digests and absent values are validated in later
            // proof-hash reconstruction steps.
            return Ok(ReconcileResult::NoValue);
        };

        let Some(node) = self.get_node_from_nibbles(key_nibbles)? else {
            return Err(ProofError::NodeNotInTrie);
        };
        let Some(branch) = node.as_branch() else {
            return Err(ProofError::NodeNotInTrie);
        };

        match branch.value.as_deref() {
            Some(existing_value) if existing_value != proof_value.as_ref() => {
                Err(ProofError::UnexpectedValue)
            }
            Some(_) => Ok(ReconcileResult::ValueAlreadyMatches),
            None => {
                let key_bytes: Vec<u8> = Path::from(key_nibbles).bytes_iter().collect();
                self.insert(&key_bytes, proof_value.clone())?;
                Ok(ReconcileResult::ValueInserted)
            }
        }
    }
}

/// The [`PrefixOverlap`] type represents the _shared_ and _unique_ parts of two potentially overlapping slices.
/// As the type-name implies, the `shared` property only constitues a shared *prefix*.
/// The `unique_*` properties, [`unique_a`][`PrefixOverlap::unique_a`] and [`unique_b`][`PrefixOverlap::unique_b`]
/// are set based on the argument order passed into the [`from`][`PrefixOverlap::from`] constructor.
#[derive(Debug)]
struct PrefixOverlap<'a, T> {
    shared: &'a [T],
    unique_a: &'a [T],
    unique_b: &'a [T],
}

impl<'a, T: PartialEq> PrefixOverlap<'a, T> {
    fn from(a: &'a [T], b: &'a [T]) -> Self {
        let split_index = a
            .iter()
            .zip(b)
            .position(|(a, b)| *a != *b)
            .unwrap_or_else(|| std::cmp::min(a.len(), b.len()));

        let (shared, unique_a) = a.split_at(split_index);
        let unique_b = b.get(split_index..).expect("");

        Self {
            shared,
            unique_a,
            unique_b,
        }
    }
}
