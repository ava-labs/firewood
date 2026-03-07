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

use crate::remote::client::ClientOp;
use firewood_storage::{
    BranchNode, Child, Children, HashedNodeReader, NibblesIterator, Node, NodeError,
    PathComponent, SharedNode, TrieHash, TrieReader,
};
use std::collections::BTreeMap;

/// A witness proof for verifying a commit on the client side.
///
/// Contains the batch operations that were applied, the resulting root hash,
/// and the minimal set of trie nodes needed for independent re-execution.
#[derive(Debug, Clone)]
pub struct WitnessProof {
    /// The batch operations that were applied.
    pub batch_ops: Box<[ClientOp]>,
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
    batch_ops: &[ClientOp],
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
            ClientOp::Put { key, .. } | ClientOp::Delete { key } => key,
        };

        let key_nibbles: Vec<u8> = NibblesIterator::new(key).collect();
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

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used)]

    use super::*;
    use crate::merkle::Merkle;
    use firewood_storage::{ImmutableProposal, MemStore, NodeStore};
    use std::sync::Arc;

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

    #[test]
    fn test_generate_witness_empty_trie() {
        let trie = create_test_trie(&[]);
        let ops = vec![ClientOp::Put {
            key: b"hello".to_vec().into(),
            value: b"world".to_vec().into(),
        }];
        let witness = generate_witness(trie.nodestore(), &ops, TrieHash::empty(), 4).unwrap();
        assert!(witness.witness_nodes.is_empty());
    }
}
