// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Witness-based commit verification for remote storage.
//!
//! The server provides a **witness proof** containing the minimal set of trie
//! nodes the client needs to independently re-execute batch operations and
//! verify the resulting root hash.
//!
//! ## Protocol
//!
//! 1. Server applies batch operations to create a new committed revision
//! 2. Server collects all trie nodes below depth K that were on affected paths
//! 3. Server sends `WitnessProof { batch_ops, new_root_hash, witness_nodes }`
//! 4. Client grafts witness nodes onto its truncated trie
//! 5. Client re-executes the batch ops using `Merkle::insert`/`remove`
//! 6. Client hashes the result and verifies it matches `new_root_hash`
//! 7. Client extracts new truncated trie from the re-execution result

use crate::merkle::Merkle;
use crate::remote::TruncatedTrie;
use crate::remote::client::BatchOp;
use crate::v2::api::BatchOp as CoreBatchOp;
use crate::v2::api::Error as ApiError;
use firewood_storage::{
    BranchNode, Child, Children, HashedNodeReader, ImmutableProposal, MemStore, Node, NodeError,
    NodeHashAlgorithm, NodeStore, PathComponent, SharedNode, TrieHash, TrieReader,
};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// A witness proof for verifying a commit on the client side.
///
/// Contains the batch operations that were applied, the resulting root hash,
/// and the minimal set of trie nodes needed for independent re-execution.
#[derive(Debug, Clone)]
pub struct WitnessProof {
    /// The batch operations that were applied.
    pub batch_ops: Box<[BatchOp]>,
    /// The new root hash that re-execution should produce.
    pub new_root_hash: TrieHash,
    /// Trie nodes below the truncation depth needed for re-execution.
    /// Each entry is keyed by its nibble path from the trie root.
    pub witness_nodes: Box<[WitnessNode]>,
}

/// A trie node included in a witness proof.
///
/// The node's children may be:
/// - [`Child::Node`] for inline children that are part of the witness
/// - [`Child::Proxy`] for unchanged subtrees the client doesn't need
#[derive(Debug, Clone)]
pub struct WitnessNode {
    /// The nibble path from the trie root to this node.
    ///
    /// This is the concatenation of all partial paths and child indices
    /// traversed from the root to reach this node.
    pub path: Box<[u8]>,
    /// The node data.
    pub node: Node,
}

/// Errors that can occur during witness operations.
#[derive(Debug, thiserror::Error)]
pub enum WitnessError {
    /// The re-execution produced a different root hash than expected.
    #[error("root hash mismatch: expected {expected}, computed {computed}")]
    RootHashMismatch {
        /// The expected root hash from the witness proof.
        expected: TrieHash,
        /// The root hash computed from re-execution.
        computed: TrieHash,
    },

    /// The truncated trie has no root (not bootstrapped).
    #[error("truncated trie has no root node")]
    NoRoot,

    /// A trie error occurred during witness operations.
    #[error("trie operation failed: {0}")]
    TrieError(#[from] NodeError),

    /// An API error occurred during witness operations (e.g., iterator creation).
    #[error("API error: {0}")]
    ApiError(ApiError),

    /// An unexpected child type was encountered during witness processing.
    ///
    /// This indicates a bug in the conversion logic — after `to_node_tree()`,
    /// only `Child::Node` and `Child::Proxy` should remain.
    #[error("unexpected child type at nibble path {path:02x?}")]
    UnexpectedChild {
        /// The nibble path where the unexpected child was found.
        path: Box<[u8]>,
    },

    /// A nibble value was out of range (expected 0–15).
    #[error("invalid nibble value {value} at depth {depth}")]
    InvalidNibble {
        /// The invalid nibble value.
        value: u8,
        /// The depth at which the invalid value was found.
        depth: usize,
    },
}

// -- Server-side witness generation --

/// A core batch operation with owned key and value data.
pub type OwnedCoreBatchOp = CoreBatchOp<Box<[u8]>, Box<[u8]>>;

/// Expands `DeleteRange` operations into individual `Delete` operations.
///
/// Scans the parent revision state for keys matching each prefix and emits
/// individual deletes. Also expands keys added by earlier `Put` ops in the
/// same batch (intra-batch semantics).
///
/// # Arguments
///
/// * `view` - The committed revision to scan for existing keys
/// * `ops` - Core batch operations that may contain `DeleteRange`
///
/// # Errors
///
/// Returns a [`WitnessError`] if the view's iterator fails.
pub fn expand_delete_ranges(
    view: &dyn crate::v2::api::DynDbView,
    ops: &[OwnedCoreBatchOp],
) -> Result<Vec<BatchOp>, WitnessError> {
    let mut expanded = Vec::with_capacity(ops.len());
    let mut added_keys: BTreeSet<Box<[u8]>> = BTreeSet::new();
    let mut deleted_keys: BTreeSet<Box<[u8]>> = BTreeSet::new();

    for op in ops {
        match op {
            CoreBatchOp::Put { key, value } => {
                added_keys.insert(key.clone());
                deleted_keys.remove(key);
                expanded.push(BatchOp::Put {
                    key: key.clone(),
                    value: value.clone(),
                });
            }
            CoreBatchOp::Delete { key } => {
                added_keys.remove(key);
                deleted_keys.insert(key.clone());
                expanded.push(BatchOp::Delete { key: key.clone() });
            }
            CoreBatchOp::DeleteRange { prefix } => {
                let mut to_delete: BTreeSet<Box<[u8]>> = BTreeSet::new();

                // Scan parent state for keys with this prefix
                let iter = view.iter_from(prefix).map_err(WitnessError::ApiError)?;
                for item in iter {
                    let (key, _) = item?;
                    if !key.starts_with(prefix.as_ref()) {
                        break;
                    }
                    if !deleted_keys.contains(&key) {
                        to_delete.insert(key);
                    }
                }

                // Expand intra-batch puts matching prefix
                let matching: Vec<_> = added_keys
                    .range(prefix.clone()..)
                    .take_while(|k| k.starts_with(prefix.as_ref()))
                    .cloned()
                    .collect();
                for key in &matching {
                    to_delete.insert(key.clone());
                    added_keys.remove(key);
                }

                for key in &to_delete {
                    deleted_keys.insert(key.clone());
                }

                for key in to_delete {
                    expanded.push(BatchOp::Delete { key });
                }
            }
        }
    }

    Ok(expanded)
}

/// Generates a witness proof for a set of batch operations.
///
/// Walks the old trie along each key's path and collects all nodes below
/// `truncation_depth` that the client would need to replay the operations.
///
/// # Arguments
///
/// * `nodestore` - The old committed trie state
/// * `batch_ops` - The batch operations to generate a witness for
/// * `new_root_hash` - The root hash after applying the batch ops
/// * `truncation_depth` - The client's truncation depth in nibble levels
///
/// # Errors
///
/// Returns a [`WitnessError`] if nodes cannot be read from the trie.
pub fn generate_witness<T: TrieReader + HashedNodeReader>(
    nodestore: &T,
    batch_ops: &[BatchOp],
    new_root_hash: TrieHash,
    truncation_depth: usize,
) -> Result<WitnessProof, WitnessError> {
    // BTreeMap gives deterministic ordering of witness nodes for reproducible proofs.
    let mut collected: BTreeMap<Vec<u8>, Node> = BTreeMap::new();

    let Some(root_node) = nodestore.root_node() else {
        // Empty trie: no witness nodes needed
        return Ok(WitnessProof {
            batch_ops: batch_ops.into(),
            new_root_hash,
            witness_nodes: Box::new([]),
        });
    };

    // For each batch op, walk the trie along the op's key path and collect
    // nodes below truncation depth.
    for op in batch_ops {
        let key = match op {
            BatchOp::Put { key, .. } | BatchOp::Delete { key } => key,
        };

        let key_nibbles = key_to_nibbles(key);
        collect_path_nodes(
            nodestore,
            &root_node,
            &key_nibbles,
            0,
            Vec::new(),
            truncation_depth,
            &mut collected,
        )?;
    }

    // Convert collected nodes: replace their non-witness children with Proxy stubs
    let witness_nodes: Vec<WitnessNode> = collected
        .into_iter()
        .map(|(path, node)| WitnessNode {
            path: path.into(),
            node,
        })
        .collect();

    Ok(WitnessProof {
        batch_ops: batch_ops.into(),
        new_root_hash,
        witness_nodes: witness_nodes.into(),
    })
}

/// Converts a byte key to a nibble sequence (each byte → two nibbles).
///
/// Trie traversal indexes children by single nibbles (0–15), so byte keys
/// must be split: the high nibble of each byte comes first, then the low.
/// For example, `[0xAB]` becomes `[0xA, 0xB]`.
fn key_to_nibbles(key: &[u8]) -> Vec<u8> {
    let mut nibbles = Vec::with_capacity(key.len().saturating_mul(2));
    for byte in key {
        nibbles.push(byte >> 4);
        nibbles.push(byte & 0x0f);
    }
    nibbles
}

/// Walks the trie following a key's nibble path, collecting nodes below the
/// truncation depth into the witness set.
///
/// Starting from the given node, the function:
/// 1. Checks if the node is at or below `truncation_depth` and collects it
/// 2. Matches the branch's partial path against the remaining key nibbles
/// 3. Descends into the child indicated by the next nibble
/// 4. At branches with ≤ 2 children, also collects siblings (needed if
///    a deletion causes the branch to be flattened by `flatten_branch`)
///
/// Nodes above the truncation depth are skipped — the client already has
/// them in its truncated trie.
fn collect_path_nodes<T: TrieReader>(
    nodestore: &T,
    node: &SharedNode,
    key_nibbles: &[u8],
    current_depth: usize,
    current_path: Vec<u8>,
    truncation_depth: usize,
    collected: &mut BTreeMap<Vec<u8>, Node>,
) -> Result<(), WitnessError> {
    // Dispatch on node type — leaves are collected directly; branches require
    // path matching and recursive descent.
    match node.as_ref() {
        Node::Leaf(leaf) => {
            if current_depth >= truncation_depth {
                collected
                    .entry(current_path)
                    .or_insert_with(|| Node::Leaf(leaf.clone()));
            }
        }
        Node::Branch(branch) => {
            let partial_path_len = branch.partial_path.len();

            // Extract the key prefix of the same length as the branch's partial
            // path and compare — if they differ, the key diverges here.
            let partial_path: &[u8] = branch.partial_path.as_ref();
            let key_matches = key_nibbles
                .get(..partial_path_len)
                .is_some_and(|prefix| prefix == partial_path);

            if !key_matches {
                // Key diverges from this node's path. If below truncation depth,
                // still add this node since it's needed for the divergence case.
                if current_depth >= truncation_depth && !collected.contains_key(&current_path) {
                    let witness_node = convert_node_for_witness(node)?;
                    collected.insert(current_path, witness_node);
                }
                return Ok(());
            }

            // Key matches partial path. If below truncation depth, add this node.
            if current_depth >= truncation_depth && !collected.contains_key(&current_path) {
                let witness_node = convert_node_for_witness(node)?;
                collected.insert(current_path.clone(), witness_node);
            }

            // Continue following the key to the next child.
            // Unreachable when key_matches is true (key length ≥ partial_path_len
            // guaranteed), but included as a defensive guard.
            let Some(remaining_key) = key_nibbles.get(partial_path_len..) else {
                return Ok(());
            };

            let Some((&child_index, child_remaining)) = remaining_key.split_first() else {
                // Key ends at this node

                // For deletion: if this branch has <=2 children and is below
                // truncation depth, collect siblings for flatten_branch safety
                if current_depth >= truncation_depth {
                    collect_siblings_for_flatten(
                        nodestore,
                        branch,
                        &current_path,
                        current_depth.saturating_add(partial_path_len),
                        truncation_depth,
                        collected,
                    )?;
                }
                return Ok(());
            };
            // Depth increases by the partial path length (consumed nibbles)
            // plus 1 (the child index nibble).
            let child_depth = current_depth
                .saturating_add(partial_path_len)
                .saturating_add(1);

            // Collect the child on the key's path
            let Some(child_pc) = PathComponent::try_new(child_index) else {
                return Err(WitnessError::InvalidNibble {
                    value: child_index,
                    depth: current_depth.saturating_add(partial_path_len),
                });
            };
            if let Some(child) = branch.children[child_pc].as_ref() {
                let child_node = child.as_shared_node(nodestore)?;
                let mut child_path = current_path.clone();
                child_path.extend(branch.partial_path.as_ref());
                child_path.push(child_index);

                collect_path_nodes(
                    nodestore,
                    &child_node,
                    child_remaining,
                    child_depth,
                    child_path,
                    truncation_depth,
                    collected,
                )?;
            }

            // For deletion safety: if this branch has <=2 children and is below
            // truncation depth, also collect the sibling
            if current_depth >= truncation_depth {
                collect_siblings_for_flatten(
                    nodestore,
                    branch,
                    &current_path,
                    current_depth.saturating_add(partial_path_len),
                    truncation_depth,
                    collected,
                )?;
            }
        }
    }

    Ok(())
}

/// Collects sibling children at branches that might be flattened during deletion.
///
/// When a branch has only 2 children and a key deletion removes one, the
/// Merkle `remove` operation calls `flatten_branch`, which merges the
/// remaining child into the parent via `read_for_update`. If that sibling
/// is a `Proxy` stub (below truncation depth), the re-execution will fail.
///
/// This function ensures the witness includes those siblings so the client
/// can perform the flatten without hitting a Proxy.
fn collect_siblings_for_flatten<T: TrieReader>(
    nodestore: &T,
    branch: &BranchNode,
    branch_path: &[u8],
    depth_after_partial: usize,
    truncation_depth: usize,
    collected: &mut BTreeMap<Vec<u8>, Node>,
) -> Result<(), WitnessError> {
    // Count populated children to determine if this branch could be
    // collapsed after a deletion.
    let child_count = branch.children.iter().filter(|(_, c)| c.is_some()).count();

    // Only need siblings if the branch could be flattened (<=2 children)
    if child_count > 2 {
        return Ok(());
    }

    for (idx, child_opt) in &branch.children {
        let Some(child) = child_opt else { continue };

        let child_depth = depth_after_partial.saturating_add(1);
        let mut child_path = branch_path.to_vec();
        child_path.extend(branch.partial_path.as_ref());
        child_path.push(idx.as_u8());

        if collected.contains_key(&child_path) {
            continue;
        }

        if child_depth >= truncation_depth {
            let child_node = child.as_shared_node(nodestore)?;
            let witness_node = convert_node_for_witness(&child_node)?;
            collected.insert(child_path, witness_node);
        }
    }

    Ok(())
}

/// Converts a server-side node for inclusion in a witness.
///
/// Children in a committed trie may be `AddressWithHash` (on-disk) or
/// `MaybePersisted` (in-memory). The client has no access to the server's
/// disk, so all children are replaced with `Proxy(hash)` stubs that carry
/// only the Merkle hash.
///
/// # Invariant
///
/// All children in a committed trie must have hashes. If a child has no
/// hash, this function returns [`NodeError::UnhashedChild`].
fn convert_node_for_witness(node: &SharedNode) -> Result<Node, NodeError> {
    match node.as_ref() {
        Node::Leaf(leaf) => Ok(Node::Leaf(leaf.clone())),
        Node::Branch(branch) => {
            let mut new_children = Children::new();
            // Iterate over all children. For each, discard the storage address
            // and keep only the hash as a Proxy stub.
            for (idx, child_opt) in &branch.children {
                let Some(child) = child_opt else { continue };
                // All children in a committed trie must have hashes.
                // `child.hash()` covers AddressWithHash, Proxy, and MaybePersisted variants.
                let hash = child.hash().ok_or(NodeError::UnhashedChild)?;
                *new_children.get_mut(idx) = Some(Child::Proxy(hash.clone()));
            }
            Ok(Node::Branch(Box::new(BranchNode {
                partial_path: branch.partial_path.clone(),
                value: branch.value.clone(),
                children: new_children,
            })))
        }
    }
}

// -- Client-side witness verification --

/// Verifies a witness proof and returns the updated truncated trie.
///
/// This is the core of client-side commit verification. The server sends
/// a [`WitnessProof`] containing the batch operations, the expected new
/// root hash, and the minimal set of trie nodes needed for independent
/// re-execution. The client:
///
/// 1. Converts its truncated trie root to a plain `Node` tree
/// 2. Grafts witness nodes onto the tree (replacing `Proxy` stubs)
/// 3. Creates an in-memory `Merkle` and re-executes the batch operations
/// 4. Hashes the result and verifies it matches `new_root_hash`
/// 5. Extracts the updated truncated trie from the re-execution result
///
/// If the root hash matches, the client can trust the commit without
/// having seen the full trie.
///
/// # Errors
///
/// Returns a [`WitnessError`] if:
/// - The truncated trie has no root ([`WitnessError::NoRoot`])
/// - The node tree contains unexpected child types ([`WitnessError::UnexpectedChild`])
/// - Re-execution fails, e.g. a needed node is missing ([`WitnessError::TrieError`])
/// - The root hash doesn't match ([`WitnessError::RootHashMismatch`])
pub fn verify_witness(
    truncated_trie: &TruncatedTrie,
    witness: &WitnessProof,
) -> Result<TruncatedTrie, WitnessError> {
    // 1. Convert truncated trie root to a plain Node tree
    let root = truncated_trie.root().ok_or(WitnessError::NoRoot)?;
    let mut node_tree = to_node_tree(root)?;

    // 2. Build witness lookup map and graft onto the tree
    let witness_map: BTreeMap<&[u8], &Node> = witness
        .witness_nodes
        .iter()
        .map(|wn| (wn.path.as_ref(), &wn.node))
        .collect();

    graft_witness_nodes(&mut node_tree, &witness_map, &mut Vec::new())?;

    // 3. Create in-memory Merkle for re-execution.
    // Select the hash algorithm to match the trie's configuration —
    // Keccak-256 for Ethereum, SHA-256 for MerkleDB.
    let algo = if cfg!(feature = "ethhash") {
        NodeHashAlgorithm::Ethereum
    } else {
        NodeHashAlgorithm::MerkleDB
    };
    // Create an in-memory store with the grafted node tree as its root.
    // This is a throwaway store used only for re-execution.
    let memstore = MemStore::new(Vec::new(), algo);
    let nodestore = NodeStore::new_proposal_with_root(Arc::new(memstore), node_tree);
    let mut merkle = Merkle::from(nodestore);

    // 4. Apply batch operations
    for op in &*witness.batch_ops {
        let result = match op {
            BatchOp::Put { key, value } => merkle.insert(key, value.clone()),
            BatchOp::Delete { key } => merkle.remove(key).map(|_| ()),
        };
        if let Err(e) = result {
            return Err(WitnessError::TrieError(e));
        }
    }

    // 5. Hash the result (convert MutableProposal → ImmutableProposal)
    let hashed: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
        merkle.try_into().map_err(WitnessError::TrieError)?;

    // An empty trie has no root node, so root_hash() returns None.
    // Use the canonical empty hash for comparison.
    let computed_hash = hashed
        .nodestore()
        .root_hash()
        .unwrap_or_else(TrieHash::empty);

    if computed_hash != witness.new_root_hash {
        return Err(WitnessError::RootHashMismatch {
            expected: witness.new_root_hash.clone(),
            computed: computed_hash,
        });
    }

    // 6. Extract new truncated trie
    let new_trie = TruncatedTrie::from_trie(hashed.nodestore(), truncated_trie.truncation_depth())?;

    Ok(new_trie)
}

/// Converts a truncated trie node to a plain `Node` tree suitable for
/// in-memory Merkle operations.
///
/// The truncated trie uses `MaybePersisted` children for above-K nodes
/// and `Proxy` / `AddressWithHash` for below-K stubs. This function
/// normalises the tree so that:
/// - `MaybePersisted` → unwrapped to `Child::Node` (recursive)
/// - `Child::Node` → recursively converted
/// - `Proxy` and `AddressWithHash` → `Child::Proxy` (hash only)
///
/// The result contains only `Child::Node` and `Child::Proxy` variants,
/// ready for grafting witness nodes and re-execution.
///
/// # Errors
///
/// Returns [`WitnessError::TrieError`] if a `MaybePersisted` child
/// unexpectedly requires storage access (indicates a corrupted truncated
/// trie).
fn to_node_tree(node: &Node) -> Result<Node, WitnessError> {
    match node {
        Node::Leaf(leaf) => Ok(Node::Leaf(leaf.clone())),
        Node::Branch(branch) => {
            let mut new_children = Children::new();
            // Map each child to its storage-independent form: inline nodes are
            // recursively converted, proxies are preserved, and MaybePersisted
            // nodes are unwrapped.
            for (idx, child_opt) in &branch.children {
                *new_children.get_mut(idx) = match child_opt {
                    None => None,
                    Some(Child::Node(child)) => Some(Child::Node(to_node_tree(child)?)),
                    Some(Child::MaybePersisted(maybe, _)) => {
                        // All MaybePersisted in truncated tries are Unpersisted.
                        // Use as_unpersisted_node since no storage backend is available.
                        let shared = maybe.as_unpersisted_node()?;
                        Some(Child::Node(to_node_tree(&shared)?))
                    }
                    // AddressWithHash children (from committed storage) become
                    // Proxy stubs — the client has no use for storage addresses.
                    Some(Child::Proxy(hash) | Child::AddressWithHash(_, hash)) => {
                        Some(Child::Proxy(hash.clone()))
                    }
                };
            }
            Ok(Node::Branch(Box::new(BranchNode {
                partial_path: branch.partial_path.clone(),
                value: branch.value.clone(),
                children: new_children,
            })))
        }
    }
}

/// Grafts witness nodes onto a node tree by replacing `Proxy` children
/// with actual nodes from the witness.
///
/// Recursively walks the node tree, maintaining a nibble path buffer.
/// At each `Proxy` child, checks if the witness map contains a node at
/// that path. If so, replaces the `Proxy` with a `Child::Node` containing
/// the witness data (which may itself have `Proxy` children that get
/// further grafted by the recursive call).
///
/// The `current_path` buffer is grown and truncated as the recursion
/// descends and backtracks, avoiding per-child allocations.
///
/// # Errors
///
/// Returns [`WitnessError::UnexpectedChild`] if a `MaybePersisted` or
/// `AddressWithHash` child is encountered — these should have been
/// eliminated by [`to_node_tree`].
fn graft_witness_nodes(
    node: &mut Node,
    witness_map: &BTreeMap<&[u8], &Node>,
    current_path: &mut Vec<u8>,
) -> Result<(), WitnessError> {
    // Leaves have no children to graft — only branches need processing.
    let Node::Branch(branch) = node else {
        return Ok(());
    };

    for (idx, child_opt) in &mut branch.children {
        let Some(child) = child_opt else { continue };

        // Build the full nibble path to this child by appending the parent's
        // partial path and the child's index nibble.
        let path_start = current_path.len();
        current_path.extend(branch.partial_path.as_ref());
        current_path.push(idx.as_u8());

        match child {
            // Proxy children are below-truncation stubs. If the witness
            // contains a node at this path, expand the proxy into a full node.
            Child::Proxy(_hash) => {
                if let Some(witness_node) = witness_map.get(current_path.as_slice()) {
                    let mut grafted = (*witness_node).clone();
                    // Recursively graft deeper witness nodes
                    graft_witness_nodes(&mut grafted, witness_map, current_path)?;
                    *child = Child::Node(grafted);
                }
            }
            Child::Node(child_node) => {
                // Already expanded - recursively graft children
                graft_witness_nodes(child_node, witness_map, current_path)?;
            }
            Child::MaybePersisted(..) | Child::AddressWithHash(..) => {
                return Err(WitnessError::UnexpectedChild {
                    path: current_path.as_slice().into(),
                });
            }
        }

        // Restore the path buffer to its state before this child, so sibling
        // children get the correct prefix.
        current_path.truncate(path_start);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;
    use firewood_storage::{HashedNodeReader, MemStore, NodeStore};

    type ImmutableMerkle = Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>>;

    /// Helper to create a test trie and return an immutable merkle.
    fn create_test_trie(keys: &[(&[u8], &[u8])]) -> ImmutableMerkle {
        let memstore = MemStore::default();
        let nodestore = NodeStore::new_empty_proposal(Arc::new(memstore));
        let mut merkle = Merkle::from(nodestore);

        for (key, value) in keys {
            merkle
                .insert(key, value.to_vec().into_boxed_slice())
                .unwrap();
        }

        merkle.try_into().unwrap()
    }

    /// Helper to apply batch ops to a trie and return the new immutable merkle.
    fn apply_batch(merkle: &ImmutableMerkle, ops: &[BatchOp]) -> ImmutableMerkle {
        let nodestore = NodeStore::new(merkle.nodestore()).unwrap();
        let mut new_merkle = Merkle::from(nodestore);

        for op in ops {
            match op {
                BatchOp::Put { key, value } => {
                    new_merkle.insert(key, value.clone()).unwrap();
                }
                BatchOp::Delete { key } => {
                    new_merkle.remove(key).unwrap();
                }
            }
        }

        new_merkle.try_into().unwrap()
    }

    #[test]
    fn test_key_to_nibbles() {
        assert_eq!(key_to_nibbles(&[0xAB, 0xCD]), vec![0xA, 0xB, 0xC, 0xD]);
        assert_eq!(key_to_nibbles(&[0x00]), vec![0, 0]);
        assert_eq!(key_to_nibbles(&[0xFF]), vec![0xF, 0xF]);
        assert!(key_to_nibbles(&[]).is_empty());
    }

    #[test]
    fn test_generate_witness_empty_trie() {
        let trie = create_test_trie(&[]);
        let ops = vec![BatchOp::Put {
            key: b"hello".to_vec().into(),
            value: b"world".to_vec().into(),
        }];
        let witness = generate_witness(trie.nodestore(), &ops, TrieHash::empty(), 4).unwrap();
        assert!(witness.witness_nodes.is_empty());
    }

    #[test]
    fn test_witness_insert_roundtrip() {
        // Create initial trie with some keys
        let old_trie = create_test_trie(&[
            (b"apple", b"red"),
            (b"banana", b"yellow"),
            (b"cherry", b"dark"),
        ]);
        let old_root_hash = old_trie.nodestore().root_hash().unwrap();

        // Define batch operations
        let ops = vec![
            BatchOp::Put {
                key: b"date".to_vec().into(),
                value: b"brown".to_vec().into(),
            },
            BatchOp::Put {
                key: b"apple".to_vec().into(),
                value: b"green".to_vec().into(),
            },
        ];

        // Apply batch to get new root hash
        let new_trie = apply_batch(&old_trie, &ops);
        let new_root_hash = new_trie.nodestore().root_hash().unwrap();

        // Generate witness from old trie
        let truncation_depth = 2;
        let witness =
            generate_witness(old_trie.nodestore(), &ops, new_root_hash, truncation_depth).unwrap();

        // Create client's truncated trie from the old state
        let truncated = TruncatedTrie::from_trie(old_trie.nodestore(), truncation_depth).unwrap();
        assert_eq!(*truncated.root_hash().unwrap(), old_root_hash);

        // Verify witness on client side
        let updated_trie = verify_witness(&truncated, &witness).unwrap();
        assert_eq!(*updated_trie.root_hash().unwrap(), witness.new_root_hash);
    }

    #[test]
    fn test_witness_delete_roundtrip() {
        let old_trie = create_test_trie(&[
            (b"apple", b"red"),
            (b"banana", b"yellow"),
            (b"cherry", b"dark"),
        ]);

        let ops = vec![BatchOp::Delete {
            key: b"banana".to_vec().into(),
        }];

        let new_trie = apply_batch(&old_trie, &ops);
        let new_root_hash = new_trie.nodestore().root_hash().unwrap();

        let truncation_depth = 2;
        let witness =
            generate_witness(old_trie.nodestore(), &ops, new_root_hash, truncation_depth).unwrap();

        let truncated = TruncatedTrie::from_trie(old_trie.nodestore(), truncation_depth).unwrap();

        let updated_trie = verify_witness(&truncated, &witness).unwrap();
        assert_eq!(*updated_trie.root_hash().unwrap(), witness.new_root_hash);
    }

    #[test]
    fn test_witness_wrong_root_hash_fails() {
        let old_trie = create_test_trie(&[(b"apple", b"red"), (b"banana", b"yellow")]);

        let ops = vec![BatchOp::Put {
            key: b"cherry".to_vec().into(),
            value: b"dark".to_vec().into(),
        }];

        // Use wrong root hash
        let witness = generate_witness(
            old_trie.nodestore(),
            &ops,
            TrieHash::empty(), // wrong!
            2,
        )
        .unwrap();

        let truncated = TruncatedTrie::from_trie(old_trie.nodestore(), 2).unwrap();

        let result = verify_witness(&truncated, &witness);
        assert!(matches!(result, Err(WitnessError::RootHashMismatch { .. })));
    }

    #[test]
    fn test_witness_mixed_ops_roundtrip() {
        let old_trie = create_test_trie(&[
            (b"key1", b"val1"),
            (b"key2", b"val2"),
            (b"key3", b"val3"),
            (b"key4", b"val4"),
            (b"key5", b"val5"),
        ]);

        let ops = vec![
            BatchOp::Put {
                key: b"key1".to_vec().into(),
                value: b"updated1".to_vec().into(),
            },
            BatchOp::Delete {
                key: b"key3".to_vec().into(),
            },
            BatchOp::Put {
                key: b"key6".to_vec().into(),
                value: b"val6".to_vec().into(),
            },
        ];

        let new_trie = apply_batch(&old_trie, &ops);
        let new_root_hash = new_trie.nodestore().root_hash().unwrap();

        let truncation_depth = 2;
        let witness =
            generate_witness(old_trie.nodestore(), &ops, new_root_hash, truncation_depth).unwrap();

        let truncated = TruncatedTrie::from_trie(old_trie.nodestore(), truncation_depth).unwrap();

        let updated_trie = verify_witness(&truncated, &witness).unwrap();
        assert_eq!(*updated_trie.root_hash().unwrap(), witness.new_root_hash);
    }

    #[test]
    fn test_multi_commit_sequence() {
        // Verify that the truncated trie stays consistent across multiple
        // sequential witness-verified commits.
        let initial_trie = create_test_trie(&[(b"apple", b"red"), (b"banana", b"yellow")]);
        let truncation_depth = 2;

        let mut truncated =
            TruncatedTrie::from_trie(initial_trie.nodestore(), truncation_depth).unwrap();

        // Track the server-side trie through sequential commits
        let mut server_trie = initial_trie;

        // Commit 1: insert cherry
        let ops1 = vec![BatchOp::Put {
            key: b"cherry".to_vec().into(),
            value: b"dark".to_vec().into(),
        }];
        let new_server = apply_batch(&server_trie, &ops1);
        let new_root = new_server.nodestore().root_hash().unwrap();
        let witness1 = generate_witness(
            server_trie.nodestore(),
            &ops1,
            new_root.clone(),
            truncation_depth,
        )
        .unwrap();
        truncated = verify_witness(&truncated, &witness1).unwrap();
        assert_eq!(*truncated.root_hash().unwrap(), new_root);
        server_trie = new_server;

        // Commit 2: update apple, delete banana
        let ops2 = vec![
            BatchOp::Put {
                key: b"apple".to_vec().into(),
                value: b"green".to_vec().into(),
            },
            BatchOp::Delete {
                key: b"banana".to_vec().into(),
            },
        ];
        let new_server = apply_batch(&server_trie, &ops2);
        let new_root = new_server.nodestore().root_hash().unwrap();
        let witness2 = generate_witness(
            server_trie.nodestore(),
            &ops2,
            new_root.clone(),
            truncation_depth,
        )
        .unwrap();
        truncated = verify_witness(&truncated, &witness2).unwrap();
        assert_eq!(*truncated.root_hash().unwrap(), new_root);
        server_trie = new_server;

        // Commit 3: insert date, elderberry
        let ops3 = vec![
            BatchOp::Put {
                key: b"date".to_vec().into(),
                value: b"brown".to_vec().into(),
            },
            BatchOp::Put {
                key: b"elderberry".to_vec().into(),
                value: b"purple".to_vec().into(),
            },
        ];
        let new_server = apply_batch(&server_trie, &ops3);
        let new_root = new_server.nodestore().root_hash().unwrap();
        let witness3 = generate_witness(
            server_trie.nodestore(),
            &ops3,
            new_root.clone(),
            truncation_depth,
        )
        .unwrap();
        truncated = verify_witness(&truncated, &witness3).unwrap();
        assert_eq!(*truncated.root_hash().unwrap(), new_root);

        // Final truncated trie should match server's current state
        let expected_truncated =
            TruncatedTrie::from_trie(new_server.nodestore(), truncation_depth).unwrap();
        assert_eq!(
            *truncated.root_hash().unwrap(),
            *expected_truncated.root_hash().unwrap()
        );
    }

    #[test]
    fn test_witness_deep_truncation() {
        // Use deeper truncation to exercise the code with more nodes
        let old_trie =
            create_test_trie(&[(b"a", b"1"), (b"ab", b"2"), (b"abc", b"3"), (b"abd", b"4")]);

        let ops = vec![BatchOp::Put {
            key: b"abe".to_vec().into(),
            value: b"5".to_vec().into(),
        }];

        let new_trie = apply_batch(&old_trie, &ops);
        let new_root_hash = new_trie.nodestore().root_hash().unwrap();

        // Truncation depth 1 means most nodes are below K
        let truncation_depth = 1;
        let witness =
            generate_witness(old_trie.nodestore(), &ops, new_root_hash, truncation_depth).unwrap();

        assert!(
            !witness.witness_nodes.is_empty(),
            "should have witness nodes below truncation depth"
        );

        let truncated = TruncatedTrie::from_trie(old_trie.nodestore(), truncation_depth).unwrap();

        let updated_trie = verify_witness(&truncated, &witness).unwrap();
        assert_eq!(*updated_trie.root_hash().unwrap(), witness.new_root_hash);
    }

    // -- Tests for expand_delete_ranges --

    fn core_put(key: &[u8], value: &[u8]) -> OwnedCoreBatchOp {
        CoreBatchOp::Put {
            key: key.into(),
            value: value.into(),
        }
    }

    fn core_delete(key: &[u8]) -> OwnedCoreBatchOp {
        CoreBatchOp::Delete { key: key.into() }
    }

    fn core_delete_range(prefix: &[u8]) -> OwnedCoreBatchOp {
        CoreBatchOp::DeleteRange {
            prefix: prefix.into(),
        }
    }

    #[test]
    fn test_expand_no_delete_range() {
        // Without DeleteRange ops, expand should pass through unchanged.
        let trie = create_test_trie(&[(b"a", b"1"), (b"b", b"2")]);
        let view: &dyn crate::v2::api::DynDbView = trie.nodestore();

        let ops = vec![core_put(b"c", b"3"), core_delete(b"a")];
        let expanded = expand_delete_ranges(view, &ops).unwrap();

        assert_eq!(expanded.len(), 2);
        assert!(matches!(&expanded[0], BatchOp::Put { key, value }
            if &**key == b"c" && &**value == b"3"));
        assert!(matches!(&expanded[1], BatchOp::Delete { key }
            if &**key == b"a"));
    }

    #[test]
    fn test_expand_basic_delete_range() {
        // DeleteRange should expand to individual Delete ops for matching keys.
        let trie = create_test_trie(&[(b"a/1", b"v1"), (b"a/2", b"v2"), (b"b/1", b"v3")]);
        let view: &dyn crate::v2::api::DynDbView = trie.nodestore();

        let ops = vec![core_delete_range(b"a/")];
        let expanded = expand_delete_ranges(view, &ops).unwrap();

        assert_eq!(expanded.len(), 2);
        // BTreeSet ensures sorted order
        assert!(matches!(&expanded[0], BatchOp::Delete { key } if &**key == b"a/1"));
        assert!(matches!(&expanded[1], BatchOp::Delete { key } if &**key == b"a/2"));
    }

    #[test]
    fn test_expand_intra_batch_put_then_delete_range() {
        // A Put before a DeleteRange in the same batch: the put key should
        // also be expanded into a Delete.
        let trie = create_test_trie(&[(b"a/1", b"v1")]);
        let view: &dyn crate::v2::api::DynDbView = trie.nodestore();

        let ops = vec![core_put(b"a/2", b"new"), core_delete_range(b"a/")];
        let expanded = expand_delete_ranges(view, &ops).unwrap();

        // Should have: Put("a/2", "new"), Delete("a/1"), Delete("a/2")
        assert_eq!(expanded.len(), 3);
        assert!(matches!(&expanded[0], BatchOp::Put { key, .. } if &**key == b"a/2"));
        // Deletes are sorted by BTreeSet
        assert!(matches!(&expanded[1], BatchOp::Delete { key } if &**key == b"a/1"));
        assert!(matches!(&expanded[2], BatchOp::Delete { key } if &**key == b"a/2"));
    }

    #[test]
    fn test_expand_empty_prefix_match() {
        // DeleteRange with a prefix that matches no keys should produce no deletes.
        let trie = create_test_trie(&[(b"a/1", b"v1")]);
        let view: &dyn crate::v2::api::DynDbView = trie.nodestore();

        let ops = vec![core_delete_range(b"z/")];
        let expanded = expand_delete_ranges(view, &ops).unwrap();

        assert!(expanded.is_empty());
    }

    #[test]
    fn test_expand_empty_trie() {
        // DeleteRange on an empty trie should produce no deletes.
        let trie = create_test_trie(&[]);
        let view: &dyn crate::v2::api::DynDbView = trie.nodestore();

        let ops = vec![core_delete_range(b"a/")];
        let expanded = expand_delete_ranges(view, &ops).unwrap();

        assert!(expanded.is_empty());
    }

    #[test]
    fn test_expand_delete_range_witness_roundtrip() {
        // End-to-end test: expand DeleteRange, generate witness, verify.
        let old_trie = create_test_trie(&[(b"a/1", b"v1"), (b"a/2", b"v2"), (b"b/1", b"v3")]);
        let view: &dyn crate::v2::api::DynDbView = old_trie.nodestore();

        // Expand DeleteRange into individual deletes
        let core_ops = vec![core_delete_range(b"a/")];
        let expanded = expand_delete_ranges(view, &core_ops).unwrap();

        // Apply the expanded ops to get the new trie
        let new_trie = apply_batch(&old_trie, &expanded);
        let new_root_hash = new_trie.nodestore().root_hash().unwrap();

        // Generate and verify witness
        let truncation_depth = 2;
        let witness = generate_witness(
            old_trie.nodestore(),
            &expanded,
            new_root_hash,
            truncation_depth,
        )
        .unwrap();

        let truncated = TruncatedTrie::from_trie(old_trie.nodestore(), truncation_depth).unwrap();
        let updated_trie = verify_witness(&truncated, &witness).unwrap();
        assert_eq!(*updated_trie.root_hash().unwrap(), witness.new_root_hash);
    }

    #[test]
    fn test_expand_mixed_ops_with_delete_range() {
        // Mix of Put, Delete, and DeleteRange in a single batch.
        let old_trie = create_test_trie(&[(b"a/1", b"v1"), (b"a/2", b"v2"), (b"b/1", b"v3")]);
        let view: &dyn crate::v2::api::DynDbView = old_trie.nodestore();

        let core_ops = vec![
            core_put(b"c/1", b"v4"),
            core_delete_range(b"a/"),
            core_put(b"b/2", b"v5"),
        ];
        let expanded = expand_delete_ranges(view, &core_ops).unwrap();

        // Expected: Put("c/1"), Delete("a/1"), Delete("a/2"), Put("b/2")
        assert_eq!(expanded.len(), 4);
        assert!(matches!(&expanded[0], BatchOp::Put { key, .. } if &**key == b"c/1"));
        assert!(matches!(&expanded[1], BatchOp::Delete { key } if &**key == b"a/1"));
        assert!(matches!(&expanded[2], BatchOp::Delete { key } if &**key == b"a/2"));
        assert!(matches!(&expanded[3], BatchOp::Put { key, .. } if &**key == b"b/2"));
    }

    #[test]
    fn test_expand_delete_before_delete_range() {
        // A Delete followed by a DeleteRange with the same prefix should not
        // produce a redundant delete for the already-deleted key.
        let old_trie = create_test_trie(&[(b"a/1", b"v1"), (b"a/2", b"v2"), (b"a/3", b"v3")]);
        let view: &dyn crate::v2::api::DynDbView = old_trie.nodestore();

        let core_ops = vec![core_delete(b"a/1"), core_delete_range(b"a/")];
        let expanded = expand_delete_ranges(view, &core_ops).unwrap();

        // Expected: Delete("a/1"), Delete("a/2"), Delete("a/3") — no redundant second Delete("a/1")
        assert_eq!(expanded.len(), 3);
        assert!(matches!(&expanded[0], BatchOp::Delete { key } if &**key == b"a/1"));
        assert!(matches!(&expanded[1], BatchOp::Delete { key } if &**key == b"a/2"));
        assert!(matches!(&expanded[2], BatchOp::Delete { key } if &**key == b"a/3"));
    }
}
