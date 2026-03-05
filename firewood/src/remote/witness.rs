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
use crate::remote::client::BatchOp;
use crate::remote::TruncatedTrie;
use firewood_storage::{
    BranchNode, Child, Children, FileIoError, HashedNodeReader, ImmutableProposal, LinearAddress,
    MemStore, Node, NodeError, NodeHashAlgorithm, NodeReader, NodeStore, PathComponent, SharedNode,
    TrieHash, TrieReader,
};
use std::collections::BTreeMap;
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
    batch_ops: &[BatchOp],
    new_root_hash: TrieHash,
    truncation_depth: usize,
) -> Result<WitnessProof, WitnessError> {
    let mut collected: BTreeMap<Vec<u8>, Node> = BTreeMap::new();

    let Some(root_node) = nodestore.root_node() else {
        // Empty trie: no witness nodes needed
        return Ok(WitnessProof {
            batch_ops: batch_ops.into(),
            new_root_hash,
            witness_nodes: Box::new([]),
        });
    };

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
/// For branches on deletion paths with only 2 children, also collects the
/// sibling child (needed for branch flattening during `remove`).
fn collect_path_nodes<T: TrieReader>(
    nodestore: &T,
    node: &SharedNode,
    key_nibbles: &[u8],
    current_depth: usize,
    current_path: Vec<u8>,
    truncation_depth: usize,
    collected: &mut BTreeMap<Vec<u8>, Node>,
) -> Result<(), WitnessError> {
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

            // Check if key matches the branch's partial path
            let partial_path: &[u8] = branch.partial_path.as_ref();
            let key_matches = key_nibbles
                .get(..partial_path_len)
                .is_some_and(|prefix| prefix == partial_path);

            if !key_matches {
                // Key diverges from this node's path. If below truncation depth,
                // still add this node since it's needed for the divergence case.
                if current_depth >= truncation_depth && !collected.contains_key(&current_path) {
                    let witness_node = convert_node_for_witness(nodestore, node)?;
                    collected.insert(current_path, witness_node);
                }
                return Ok(());
            }

            // Key matches partial path. If below truncation depth, add this node.
            if current_depth >= truncation_depth && !collected.contains_key(&current_path) {
                let witness_node = convert_node_for_witness(nodestore, node)?;
                collected.insert(current_path.clone(), witness_node);
            }

            // Continue following the key to the next child
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
            let child_depth = current_depth
                .saturating_add(partial_path_len)
                .saturating_add(1);

            // Collect the child on the key's path
            let Some(child_pc) = PathComponent::try_new(child_index) else {
                return Ok(());
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
/// When a branch has only 2 children and a key deletion removes one, the branch
/// is flattened with `flatten_branch`, which calls `read_for_update` on the
/// remaining child. The witness must include that sibling.
fn collect_siblings_for_flatten<T: TrieReader>(
    nodestore: &T,
    branch: &BranchNode,
    branch_path: &[u8],
    depth_after_partial: usize,
    truncation_depth: usize,
    collected: &mut BTreeMap<Vec<u8>, Node>,
) -> Result<(), WitnessError> {
    let child_count = branch
        .children
        .iter()
        .filter(|(_, c)| c.is_some())
        .count();

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
            let witness_node = convert_node_for_witness(nodestore, &child_node)?;
            collected.insert(child_path, witness_node);
        }
    }

    Ok(())
}

/// Converts a server-side node for inclusion in a witness.
///
/// Replaces `AddressWithHash` and `MaybePersisted` children with `Proxy(hash)`
/// stubs (since the client has no access to the server's storage addresses).
fn convert_node_for_witness<T: TrieReader>(
    nodestore: &T,
    node: &SharedNode,
) -> Result<Node, NodeError> {
    match node.as_ref() {
        Node::Leaf(leaf) => Ok(Node::Leaf(leaf.clone())),
        Node::Branch(branch) => {
            let mut new_children = Children::new();
            for (idx, child_opt) in &branch.children {
                let Some(child) = child_opt else { continue };
                // Keep the hash but discard the address/node data
                if let Some(hash) = child.hash() {
                    *new_children.get_mut(idx) = Some(Child::Proxy(hash.clone()));
                } else {
                    // Child::Node without a hash - shouldn't happen in committed tries
                    // but handle gracefully by keeping the node inline
                    let child_node = child.as_shared_node(nodestore)?;
                    let witness_child = convert_node_for_witness(nodestore, &child_node)?;
                    *new_children.get_mut(idx) = Some(Child::Node(witness_child));
                }
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

/// A `NodeReader` that panics on read, used for extracting in-memory nodes
/// from `MaybePersisted` children that are known to be unpersisted.
struct NullNodeReader;

impl NodeReader for NullNodeReader {
    fn read_node(&self, _addr: LinearAddress) -> Result<SharedNode, FileIoError> {
        Err(FileIoError::new(
            std::io::Error::other("NullNodeReader: no storage available"),
            None,
            0,
            Some("NullNodeReader::read_node".into()),
        ))
    }
}

/// Verifies a witness proof and returns the updated truncated trie.
///
/// 1. Clones the truncated trie root and converts it to a plain `Node` tree
/// 2. Grafts witness nodes onto the tree (replacing `Proxy` stubs)
/// 3. Creates an in-memory `Merkle` and re-executes the batch operations
/// 4. Hashes the result and verifies it matches `new_root_hash`
/// 5. Extracts the updated truncated trie from the re-execution result
///
/// # Errors
///
/// Returns a [`WitnessError`] if:
/// - The truncated trie has no root
/// - Re-execution fails (e.g., a needed node is missing from the witness)
/// - The root hash doesn't match after re-execution
pub fn verify_witness(
    truncated_trie: &TruncatedTrie,
    witness: &WitnessProof,
) -> Result<TruncatedTrie, WitnessError> {
    // 1. Convert truncated trie root to a plain Node tree
    let root = truncated_trie.root().ok_or(WitnessError::NoRoot)?;
    let mut node_tree = to_node_tree(root);

    // 2. Build witness lookup map and graft onto the tree
    let witness_map: BTreeMap<&[u8], &Node> = witness
        .witness_nodes
        .iter()
        .map(|wn| (wn.path.as_ref(), &wn.node))
        .collect();

    graft_witness_nodes(&mut node_tree, &witness_map, &mut Vec::new());

    // 3. Create in-memory Merkle for re-execution
    let algo = if cfg!(feature = "ethhash") {
        NodeHashAlgorithm::Ethereum
    } else {
        NodeHashAlgorithm::MerkleDB
    };
    let memstore = MemStore::new(Vec::new(), algo);
    let nodestore =
        NodeStore::new_proposal_with_root(Arc::new(memstore), node_tree);
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
    let new_trie =
        TruncatedTrie::from_trie(hashed.nodestore(), truncated_trie.truncation_depth())?;

    Ok(new_trie)
}

/// Converts a truncated trie node to a plain `Node` tree.
///
/// `MaybePersisted` children (used in truncated tries for above-K nodes)
/// are unwrapped to `Child::Node`. `Proxy` children remain as-is.
fn to_node_tree(node: &Node) -> Node {
    match node {
        Node::Leaf(leaf) => Node::Leaf(leaf.clone()),
        Node::Branch(branch) => {
            let mut new_children = Children::new();
            for (idx, child_opt) in &branch.children {
                *new_children.get_mut(idx) = match child_opt {
                    None => None,
                    Some(Child::Node(child)) => Some(Child::Node(to_node_tree(child))),
                    Some(Child::MaybePersisted(maybe, _)) => {
                        // All MaybePersisted in truncated tries are Unpersisted.
                        // Use NullNodeReader since storage won't be accessed.
                        match maybe.as_shared_node(&NullNodeReader) {
                            Ok(shared) => Some(Child::Node(to_node_tree(&shared))),
                            Err(_) => None,
                        }
                    }
                    Some(Child::Proxy(hash) | Child::AddressWithHash(_, hash)) => {
                        Some(Child::Proxy(hash.clone()))
                    }
                };
            }
            Node::Branch(Box::new(BranchNode {
                partial_path: branch.partial_path.clone(),
                value: branch.value.clone(),
                children: new_children,
            }))
        }
    }
}

/// Grafts witness nodes onto a node tree by replacing `Proxy` children
/// with actual nodes from the witness.
///
/// Recursively walks the node tree. At each `Proxy` child, checks if the
/// witness contains a node at that path. If so, replaces the `Proxy` with
/// a `Child::Node` containing the witness node (which may itself have
/// `Proxy` children that get further grafted).
fn graft_witness_nodes(
    node: &mut Node,
    witness_map: &BTreeMap<&[u8], &Node>,
    current_path: &mut Vec<u8>,
) {
    let Node::Branch(branch) = node else { return };

    for (idx, child_opt) in &mut branch.children {
        let Some(child) = child_opt else { continue };

        // Build the child's full nibble path
        let path_start = current_path.len();
        current_path.extend(branch.partial_path.as_ref());
        current_path.push(idx.as_u8());

        match child {
            Child::Proxy(_hash) => {
                // Check if witness has a node at this path
                if let Some(witness_node) = witness_map.get(current_path.as_slice()) {
                    let mut grafted = (*witness_node).clone();
                    // Recursively graft deeper witness nodes
                    graft_witness_nodes(&mut grafted, witness_map, current_path);
                    *child = Child::Node(grafted);
                }
            }
            Child::Node(child_node) => {
                // Already expanded - recursively graft children
                graft_witness_nodes(child_node, witness_map, current_path);
            }
            Child::MaybePersisted(..) | Child::AddressWithHash(..) => {
                // These shouldn't appear after to_node_tree conversion
            }
        }

        current_path.truncate(path_start);
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used)]

    use super::*;
    use firewood_storage::{HashedNodeReader, MemStore, NodeStore};

    type ImmutableMerkle =
        Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>>;

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
    fn apply_batch(
        merkle: &ImmutableMerkle,
        ops: &[BatchOp],
    ) -> ImmutableMerkle {
        let nodestore = NodeStore::new(merkle.nodestore()).unwrap();
        let mut new_merkle = Merkle::from(nodestore);

        for op in ops {
            match op {
                BatchOp::Put { key, value } => {
                    new_merkle
                        .insert(key, value.clone())
                        .unwrap();
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
        let witness = generate_witness(
            trie.nodestore(),
            &ops,
            TrieHash::empty(),
            4,
        )
        .unwrap();
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
        let witness = generate_witness(
            old_trie.nodestore(),
            &ops,
            new_root_hash,
            truncation_depth,
        )
        .unwrap();

        // Create client's truncated trie from the old state
        let truncated = TruncatedTrie::from_trie(
            old_trie.nodestore(),
            truncation_depth,
        )
        .unwrap();
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
        let witness = generate_witness(
            old_trie.nodestore(),
            &ops,
            new_root_hash,
            truncation_depth,
        )
        .unwrap();

        let truncated = TruncatedTrie::from_trie(
            old_trie.nodestore(),
            truncation_depth,
        )
        .unwrap();

        let updated_trie = verify_witness(&truncated, &witness).unwrap();
        assert_eq!(*updated_trie.root_hash().unwrap(), witness.new_root_hash);
    }

    #[test]
    fn test_witness_wrong_root_hash_fails() {
        let old_trie = create_test_trie(&[
            (b"apple", b"red"),
            (b"banana", b"yellow"),
        ]);

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
        let witness = generate_witness(
            old_trie.nodestore(),
            &ops,
            new_root_hash,
            truncation_depth,
        )
        .unwrap();

        let truncated = TruncatedTrie::from_trie(
            old_trie.nodestore(),
            truncation_depth,
        )
        .unwrap();

        let updated_trie = verify_witness(&truncated, &witness).unwrap();
        assert_eq!(*updated_trie.root_hash().unwrap(), witness.new_root_hash);
    }

    #[test]
    fn test_multi_commit_sequence() {
        // Verify that the truncated trie stays consistent across multiple
        // sequential witness-verified commits.
        let initial_trie = create_test_trie(&[
            (b"apple", b"red"),
            (b"banana", b"yellow"),
        ]);
        let truncation_depth = 2;

        let mut truncated = TruncatedTrie::from_trie(
            initial_trie.nodestore(),
            truncation_depth,
        )
        .unwrap();

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
        let expected_truncated = TruncatedTrie::from_trie(
            new_server.nodestore(),
            truncation_depth,
        )
        .unwrap();
        assert_eq!(
            *truncated.root_hash().unwrap(),
            *expected_truncated.root_hash().unwrap()
        );
    }

    #[test]
    fn test_witness_deep_truncation() {
        // Use deeper truncation to exercise the code with more nodes
        let old_trie = create_test_trie(&[
            (b"a", b"1"),
            (b"ab", b"2"),
            (b"abc", b"3"),
            (b"abd", b"4"),
        ]);

        let ops = vec![BatchOp::Put {
            key: b"abe".to_vec().into(),
            value: b"5".to_vec().into(),
        }];

        let new_trie = apply_batch(&old_trie, &ops);
        let new_root_hash = new_trie.nodestore().root_hash().unwrap();

        // Truncation depth 1 means most nodes are below K
        let truncation_depth = 1;
        let witness = generate_witness(
            old_trie.nodestore(),
            &ops,
            new_root_hash,
            truncation_depth,
        )
        .unwrap();

        assert!(
            !witness.witness_nodes.is_empty(),
            "should have witness nodes below truncation depth"
        );

        let truncated = TruncatedTrie::from_trie(
            old_trie.nodestore(),
            truncation_depth,
        )
        .unwrap();

        let updated_trie = verify_witness(&truncated, &witness).unwrap();
        assert_eq!(*updated_trie.root_hash().unwrap(), witness.new_root_hash);
    }
}
