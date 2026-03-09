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
use crate::remote::client::ClientOp;
use crate::v2::api::BatchOp;
use crate::v2::api::Error as ApiError;
use firewood_storage::{
    BranchNode, Child, Children, HashedNodeReader, ImmutableProposal, MemStore, NibblesIterator,
    Node, NodeError, NodeHashAlgorithm, NodeStore, PathComponent, SharedNode, TrieHash, TrieReader,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;

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

    /// The witness proof's batch operations do not match the expected operations.
    #[error("batch ops mismatch at index {index}: {detail}")]
    OpsMismatch {
        /// The index where the first mismatch was detected.
        index: usize,
        /// Human-readable description of the mismatch.
        detail: String,
    },
}

// -- Server-side witness generation --

/// A core batch operation with owned key and value data.
pub type OwnedBatchOp = BatchOp<Box<[u8]>, Box<[u8]>>;

impl From<ClientOp> for OwnedBatchOp {
    fn from(op: ClientOp) -> Self {
        match op {
            ClientOp::Put { key, value } => BatchOp::Put { key, value },
            ClientOp::Delete { key } => BatchOp::Delete { key },
        }
    }
}

/// Expands `DeleteRange` operations into individual `Delete` operations.
///
/// Scans the parent revision state for keys matching each prefix and emits
/// individual deletes. Also expands keys added by earlier `Put` ops in the
/// same batch (intra-batch semantics).
///
/// # Arguments
///
/// * `view` - The parent revision to scan for existing keys (typically the
///   pre-batch state, e.g. an `ImmutableProposal` or committed revision)
/// * `ops` - Core batch operations that may contain `DeleteRange`
///
/// # Errors
///
/// Returns a [`WitnessError`] if the view's iterator fails.
pub fn expand_delete_ranges(
    view: &dyn crate::v2::api::DynDbView,
    ops: &[OwnedBatchOp],
) -> Result<Vec<ClientOp>, WitnessError> {
    let mut expanded = Vec::with_capacity(ops.len());
    // Track intra-batch state for prefix overlap
    let mut added_keys: BTreeSet<Box<[u8]>> = BTreeSet::new();
    let mut deleted_keys: HashSet<Box<[u8]>> = HashSet::new();

    for op in ops {
        match op {
            BatchOp::Put { key, value } => {
                added_keys.insert(key.clone());
                deleted_keys.remove(key);
                expanded.push(ClientOp::Put {
                    key: key.clone(),
                    value: value.clone(),
                });
            }
            BatchOp::Delete { key } => {
                added_keys.remove(key);
                deleted_keys.insert(key.clone());
                expanded.push(ClientOp::Delete { key: key.clone() });
            }
            BatchOp::DeleteRange { prefix } => {
                // Collect keys to delete from parent state + intra-batch puts
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
                    expanded.push(ClientOp::Delete { key });
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

// -- Client-side witness verification --

/// Validates that the witness proof's batch operations are consistent with the
/// expected operations, handling `DeleteRange` expansion.
///
/// Walks `expected_ops` and `witness.batch_ops` in lockstep:
/// - `Put`/`Delete`: 1:1 comparison against the witness `ClientOp`
/// - `DeleteRange { prefix }`: consumes consecutive `Delete` witness ops
///   whose keys start with the prefix; records the prefix
///
/// Returns the collected `delete_range_prefixes` (empty if no `DeleteRange`).
///
/// # Errors
///
/// Returns [`WitnessError::OpsMismatch`] if:
/// - A `Put`/`Delete` doesn't match its witness counterpart
/// - The witness has leftover or missing ops
fn validate_witness_ops(
    witness: &WitnessProof,
    expected_ops: &[OwnedBatchOp],
) -> Result<Vec<Box<[u8]>>, WitnessError> {
    let witness_ops = &witness.batch_ops;
    let mut wi = 0usize;
    let mut delete_range_prefixes = Vec::new();

    // Walk expected_ops and witness.batch_ops in lockstep using `wi` as the witness cursor.
    for (ei, exp) in expected_ops.iter().enumerate() {
        match exp {
            // Exact 1:1 match required
            BatchOp::Put { key, value } => {
                let Some(wop) = witness_ops.get(wi) else {
                    return Err(WitnessError::OpsMismatch {
                        index: ei,
                        detail: format!(
                            "witness ops exhausted; expected Put(key_len={})",
                            key.len()
                        ),
                    });
                };
                match wop {
                    ClientOp::Put { key: wk, value: wv } => {
                        if key.as_ref() != wk.as_ref() || value.as_ref() != wv.as_ref() {
                            return Err(WitnessError::OpsMismatch {
                                index: ei,
                                detail: format!(
                                    "Put mismatch at witness index {wi}: \
                                     expected key_len={} value_len={}, \
                                     got key_len={} value_len={}",
                                    key.len(),
                                    value.len(),
                                    wk.len(),
                                    wv.len()
                                ),
                            });
                        }
                    }
                    ClientOp::Delete { .. } => {
                        return Err(WitnessError::OpsMismatch {
                            index: ei,
                            detail: format!("expected Put at witness index {wi}, got Delete"),
                        });
                    }
                }
                wi = wi.strict_add(1);
            }
            // Exact 1:1 match required
            BatchOp::Delete { key } => {
                let Some(wop) = witness_ops.get(wi) else {
                    return Err(WitnessError::OpsMismatch {
                        index: ei,
                        detail: format!(
                            "witness ops exhausted; expected Delete(key_len={})",
                            key.len()
                        ),
                    });
                };
                match wop {
                    ClientOp::Delete { key: wk } => {
                        if key.as_ref() != wk.as_ref() {
                            return Err(WitnessError::OpsMismatch {
                                index: ei,
                                detail: format!(
                                    "Delete mismatch at witness index {wi}: \
                                     expected key_len={}, got key_len={}",
                                    key.len(),
                                    wk.len()
                                ),
                            });
                        }
                    }
                    ClientOp::Put { .. } => {
                        return Err(WitnessError::OpsMismatch {
                            index: ei,
                            detail: format!("expected Delete at witness index {wi}, got Put"),
                        });
                    }
                }
                wi = wi.strict_add(1);
            }
            // Consume zero or more consecutive Delete ops matching prefix
            BatchOp::DeleteRange { prefix } => {
                while let Some(ClientOp::Delete { key }) = witness_ops.get(wi) {
                    if !key.starts_with(prefix.as_ref()) {
                        break;
                    }
                    wi = wi.strict_add(1);
                }
                delete_range_prefixes.push(prefix.clone());
            }
        }
    }

    // Ensure all witness ops were consumed
    if wi != witness_ops.len() {
        return Err(WitnessError::OpsMismatch {
            index: expected_ops.len(),
            detail: format!(
                "witness has {} extra ops after index {wi}",
                witness_ops.len().strict_sub(wi),
            ),
        });
    }

    Ok(delete_range_prefixes)
}

/// For each `DeleteRange(prefix)` in `expected_ops`, computes the keys
/// allowed to survive in the post-execution trie.
///
/// A key survives only if it is `Put` AFTER the `DeleteRange` and not
/// subsequently deleted by a `Delete` or covered by another `DeleteRange`.
fn allowed_surviving_keys(
    expected_ops: &[OwnedBatchOp],
) -> Vec<(&[u8], HashSet<&[u8]>)> {
    let mut result = Vec::new();
    for (i, op) in expected_ops.iter().enumerate() {
        let BatchOp::DeleteRange { prefix } = op else {
            continue;
        };
        let mut allowed: HashSet<&[u8]> = HashSet::new();
        for subsequent in expected_ops.get(i.strict_add(1)..).unwrap_or_default() {
            match subsequent {
                BatchOp::Put { key, .. } if key.as_ref().starts_with(prefix.as_ref()) => {
                    allowed.insert(key.as_ref());
                }
                BatchOp::Delete { key } => {
                    allowed.remove(key.as_ref());
                }
                BatchOp::DeleteRange { prefix: p2 } => {
                    allowed.retain(|k| !k.starts_with(p2.as_ref()));
                }
                _ => {}
            }
        }
        result.push((prefix.as_ref(), allowed));
    }
    result
}

/// Verifies a witness proof and returns the updated truncated trie.
///
/// This is the core of client-side commit verification. The server sends
/// a [`WitnessProof`] containing the batch operations, the expected new
/// root hash, and the minimal set of trie nodes needed for independent
/// re-execution. The client:
///
/// 1. Validates that the witness's batch ops match `expected_ops`
///    (handling `DeleteRange` expansion)
/// 2. Converts its truncated trie root to a plain `Node` tree
/// 3. Grafts witness nodes onto the tree (replacing `Proxy` stubs)
/// 4. Creates an in-memory `Merkle` and re-executes the batch operations
/// 5. Hashes the result and verifies it matches `new_root_hash`
/// 6. Verifies `DeleteRange` completeness (no leftover keys with prefix)
/// 7. Extracts the updated truncated trie from the re-execution result
///
/// If the root hash matches, the client can trust the commit without
/// having seen the full trie.
///
/// # Errors
///
/// Returns a [`WitnessError`] if:
/// - The batch ops don't match `expected_ops` ([`WitnessError::OpsMismatch`])
/// - The truncated trie has no root ([`WitnessError::NoRoot`])
/// - The node tree contains unexpected child types ([`WitnessError::UnexpectedChild`])
/// - Re-execution fails, e.g. a needed node is missing ([`WitnessError::TrieError`])
/// - The root hash doesn't match ([`WitnessError::RootHashMismatch`])
pub fn verify_witness(
    truncated_trie: &TruncatedTrie,
    witness: &WitnessProof,
    expected_ops: &[OwnedBatchOp],
) -> Result<TruncatedTrie, WitnessError> {
    let delete_range_prefixes = validate_witness_ops(witness, expected_ops)?;

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
            ClientOp::Put { key, value } => merkle.insert(key, value.clone()),
            ClientOp::Delete { key } => merkle.remove(key).map(|_| ()),
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

    // 6. DeleteRange completeness check: for each DeleteRange prefix, verify
    //    that every surviving key was explicitly Put AFTER the DeleteRange
    //    and not subsequently deleted or covered by another DeleteRange.
    if !delete_range_prefixes.is_empty() {
        let surviving = allowed_surviving_keys(expected_ops);
        let view: &dyn crate::v2::api::DynDbView = hashed.nodestore();
        for (prefix, allowed) in &surviving {
            let iter = view.iter_from(prefix).map_err(WitnessError::ApiError)?;
            for item in iter {
                let (key, _) = item?;
                if !key.starts_with(prefix) {
                    break;
                }
                if !allowed.contains(key.as_ref()) {
                    return Err(WitnessError::OpsMismatch {
                        index: 0,
                        detail: format!("DeleteRange prefix has surviving key (len={})", key.len()),
                    });
                }
            }
        }
    }

    // 7. Extract new truncated trie
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

    /// Helper to convert `&[ClientOp]` → `Vec<OwnedBatchOp>` and call `verify_witness`.
    fn verify(
        trie: &TruncatedTrie,
        w: &WitnessProof,
        ops: &[ClientOp],
    ) -> Result<TruncatedTrie, WitnessError> {
        let owned: Vec<OwnedBatchOp> = ops.iter().cloned().map(Into::into).collect();
        verify_witness(trie, w, &owned)
    }

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
    fn apply_batch(merkle: &ImmutableMerkle, ops: &[ClientOp]) -> ImmutableMerkle {
        let nodestore = NodeStore::new(merkle.nodestore()).unwrap();
        let mut new_merkle = Merkle::from(nodestore);

        for op in ops {
            match op {
                ClientOp::Put { key, value } => {
                    new_merkle.insert(key, value.clone()).unwrap();
                }
                ClientOp::Delete { key } => {
                    new_merkle.remove(key).unwrap();
                }
            }
        }

        new_merkle.try_into().unwrap()
    }

    /// Witness for an empty trie has no witness nodes.
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

    /// Insert ops produce a valid witness that verifies correctly.
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
            ClientOp::Put {
                key: b"date".to_vec().into(),
                value: b"brown".to_vec().into(),
            },
            ClientOp::Put {
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
        let updated_trie = verify(&truncated, &witness, &ops).unwrap();
        assert_eq!(*updated_trie.root_hash().unwrap(), witness.new_root_hash);
    }

    /// Delete ops produce a valid witness that verifies correctly.
    #[test]
    fn test_witness_delete_roundtrip() {
        let old_trie = create_test_trie(&[
            (b"apple", b"red"),
            (b"banana", b"yellow"),
            (b"cherry", b"dark"),
        ]);

        let ops = vec![ClientOp::Delete {
            key: b"banana".to_vec().into(),
        }];

        let new_trie = apply_batch(&old_trie, &ops);
        let new_root_hash = new_trie.nodestore().root_hash().unwrap();

        let truncation_depth = 2;
        let witness =
            generate_witness(old_trie.nodestore(), &ops, new_root_hash, truncation_depth).unwrap();

        let truncated = TruncatedTrie::from_trie(old_trie.nodestore(), truncation_depth).unwrap();

        let updated_trie = verify(&truncated, &witness, &ops).unwrap();
        assert_eq!(*updated_trie.root_hash().unwrap(), witness.new_root_hash);
    }

    /// Witness with wrong `new_root_hash` is rejected.
    #[test]
    fn test_witness_wrong_root_hash_fails() {
        let old_trie = create_test_trie(&[(b"apple", b"red"), (b"banana", b"yellow")]);

        let ops = vec![ClientOp::Put {
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

        let result = verify(&truncated, &witness, &ops);
        assert!(matches!(result, Err(WitnessError::RootHashMismatch { .. })));
    }

    /// Mixed insert/delete batch verifies correctly.
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
            ClientOp::Put {
                key: b"key1".to_vec().into(),
                value: b"updated1".to_vec().into(),
            },
            ClientOp::Delete {
                key: b"key3".to_vec().into(),
            },
            ClientOp::Put {
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

        let updated_trie = verify(&truncated, &witness, &ops).unwrap();
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
        let ops1 = vec![ClientOp::Put {
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
        truncated = verify(&truncated, &witness1, &ops1).unwrap();
        assert_eq!(*truncated.root_hash().unwrap(), new_root);
        server_trie = new_server;

        // Commit 2: update apple, delete banana
        let ops2 = vec![
            ClientOp::Put {
                key: b"apple".to_vec().into(),
                value: b"green".to_vec().into(),
            },
            ClientOp::Delete {
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
        truncated = verify(&truncated, &witness2, &ops2).unwrap();
        assert_eq!(*truncated.root_hash().unwrap(), new_root);
        server_trie = new_server;

        // Commit 3: insert date, elderberry
        let ops3 = vec![
            ClientOp::Put {
                key: b"date".to_vec().into(),
                value: b"brown".to_vec().into(),
            },
            ClientOp::Put {
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
        truncated = verify(&truncated, &witness3, &ops3).unwrap();
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

        let ops = vec![ClientOp::Put {
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

        let updated_trie = verify(&truncated, &witness, &ops).unwrap();
        assert_eq!(*updated_trie.root_hash().unwrap(), witness.new_root_hash);
    }

    // -- Tests for expand_delete_ranges --

    fn core_put(key: &[u8], value: &[u8]) -> OwnedBatchOp {
        BatchOp::Put {
            key: key.into(),
            value: value.into(),
        }
    }

    fn core_delete(key: &[u8]) -> OwnedBatchOp {
        BatchOp::Delete { key: key.into() }
    }

    fn core_delete_range(prefix: &[u8]) -> OwnedBatchOp {
        BatchOp::DeleteRange {
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
        assert!(matches!(&expanded[0], ClientOp::Put { key, value }
            if &**key == b"c" && &**value == b"3"));
        assert!(matches!(&expanded[1], ClientOp::Delete { key }
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
        assert!(matches!(&expanded[0], ClientOp::Delete { key } if &**key == b"a/1"));
        assert!(matches!(&expanded[1], ClientOp::Delete { key } if &**key == b"a/2"));
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
        assert!(matches!(&expanded[0], ClientOp::Put { key, .. } if &**key == b"a/2"));
        // Deletes are sorted by BTreeSet
        assert!(matches!(&expanded[1], ClientOp::Delete { key } if &**key == b"a/1"));
        assert!(matches!(&expanded[2], ClientOp::Delete { key } if &**key == b"a/2"));
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
        let updated_trie = verify(&truncated, &witness, &expanded).unwrap();
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
        assert!(matches!(&expanded[0], ClientOp::Put { key, .. } if &**key == b"c/1"));
        assert!(matches!(&expanded[1], ClientOp::Delete { key } if &**key == b"a/1"));
        assert!(matches!(&expanded[2], ClientOp::Delete { key } if &**key == b"a/2"));
        assert!(matches!(&expanded[3], ClientOp::Put { key, .. } if &**key == b"b/2"));
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
        assert!(matches!(&expanded[0], ClientOp::Delete { key } if &**key == b"a/1"));
        assert!(matches!(&expanded[1], ClientOp::Delete { key } if &**key == b"a/2"));
        assert!(matches!(&expanded[2], ClientOp::Delete { key } if &**key == b"a/3"));
    }

    // -- Tests for validate_witness_ops --

    /// Helper to build a `WitnessProof` with the given `batch_ops` for validation tests.
    fn witness_with_ops(ops: Vec<ClientOp>) -> WitnessProof {
        WitnessProof {
            batch_ops: ops.into(),
            new_root_hash: TrieHash::empty(),
            witness_nodes: Box::new([]),
        }
    }

    fn put_op(key: &[u8], value: &[u8]) -> ClientOp {
        ClientOp::Put {
            key: key.into(),
            value: value.into(),
        }
    }

    fn delete_op(key: &[u8]) -> ClientOp {
        ClientOp::Delete { key: key.into() }
    }

    /// Convert `ClientOp` slice to `Vec<OwnedBatchOp>` for `validate_witness_ops` tests.
    fn to_owned_ops(ops: &[ClientOp]) -> Vec<OwnedBatchOp> {
        ops.iter().cloned().map(Into::into).collect()
    }

    /// Matching Put/Delete ops validate successfully.
    #[test]
    fn test_validate_ops_matching() {
        let ops = vec![put_op(b"a", b"1"), delete_op(b"b")];
        let witness = witness_with_ops(ops.clone());
        assert!(validate_witness_ops(&witness, &to_owned_ops(&ops)).is_ok());
    }

    /// Two empty op lists validate successfully.
    #[test]
    fn test_validate_ops_both_empty() {
        let witness = witness_with_ops(vec![]);
        assert!(validate_witness_ops(&witness, &[]).is_ok());
    }

    /// Extra witness ops cause `OpsMismatch`.
    #[test]
    fn test_validate_ops_witness_longer() {
        let witness = witness_with_ops(vec![put_op(b"a", b"1"), put_op(b"b", b"2")]);
        let expected = to_owned_ops(&[put_op(b"a", b"1")]);
        let err = validate_witness_ops(&witness, &expected).unwrap_err();
        assert!(matches!(err, WitnessError::OpsMismatch { .. }));
    }

    /// Missing witness ops cause `OpsMismatch`.
    #[test]
    fn test_validate_ops_expected_longer() {
        let witness = witness_with_ops(vec![put_op(b"a", b"1")]);
        let expected = to_owned_ops(&[put_op(b"a", b"1"), put_op(b"b", b"2")]);
        let err = validate_witness_ops(&witness, &expected).unwrap_err();
        assert!(matches!(err, WitnessError::OpsMismatch { .. }));
    }

    /// Wrong op type at same index causes `OpsMismatch`.
    #[test]
    fn test_validate_ops_type_mismatch() {
        let witness = witness_with_ops(vec![put_op(b"a", b"1")]);
        let expected = to_owned_ops(&[delete_op(b"a")]);
        let err = validate_witness_ops(&witness, &expected).unwrap_err();
        assert!(matches!(err, WitnessError::OpsMismatch { index: 0, .. }));
    }

    /// Same op type with different key causes `OpsMismatch`.
    #[test]
    fn test_validate_ops_key_mismatch() {
        let witness = witness_with_ops(vec![put_op(b"a", b"1")]);
        let expected = to_owned_ops(&[put_op(b"b", b"1")]);
        let err = validate_witness_ops(&witness, &expected).unwrap_err();
        assert!(matches!(err, WitnessError::OpsMismatch { index: 0, .. }));
    }

    /// Same `Put` key with different value causes `OpsMismatch`.
    #[test]
    fn test_validate_ops_value_mismatch() {
        let witness = witness_with_ops(vec![put_op(b"a", b"1")]);
        let expected = to_owned_ops(&[put_op(b"a", b"2")]);
        let err = validate_witness_ops(&witness, &expected).unwrap_err();
        assert!(matches!(err, WitnessError::OpsMismatch { index: 0, .. }));
    }

    /// Mismatch detected at index 1, not index 0.
    #[test]
    fn test_validate_ops_mismatch_at_second_index() {
        let witness = witness_with_ops(vec![put_op(b"a", b"1"), put_op(b"b", b"2")]);
        let expected = to_owned_ops(&[put_op(b"a", b"1"), delete_op(b"b")]);
        let err = validate_witness_ops(&witness, &expected).unwrap_err();
        assert!(matches!(err, WitnessError::OpsMismatch { index: 1, .. }));
    }

    #[test]
    fn test_validate_ops_delete_range_lockstep() {
        // DeleteRange should consume consecutive Delete ops with matching prefix
        let witness = witness_with_ops(vec![
            delete_op(b"a/1"),
            delete_op(b"a/2"),
            put_op(b"b", b"1"),
        ]);
        let expected: Vec<OwnedBatchOp> = vec![core_delete_range(b"a/"), core_put(b"b", b"1")];
        let prefixes = validate_witness_ops(&witness, &expected).unwrap();
        assert_eq!(prefixes.len(), 1);
        assert_eq!(&*prefixes[0], b"a/");
    }

    #[test]
    fn test_validate_ops_delete_range_empty_match() {
        // DeleteRange with no matching deletes should succeed (zero consumed)
        let witness = witness_with_ops(vec![put_op(b"b", b"1")]);
        let expected: Vec<OwnedBatchOp> = vec![core_delete_range(b"a/"), core_put(b"b", b"1")];
        let prefixes = validate_witness_ops(&witness, &expected).unwrap();
        assert_eq!(prefixes.len(), 1);
    }

    #[test]
    fn test_verify_witness_wrong_expected_ops() {
        // verify_witness should return OpsMismatch before even checking root hash.
        let old_trie = create_test_trie(&[(b"apple", b"red"), (b"banana", b"yellow")]);

        let ops = vec![put_op(b"cherry", b"dark")];
        let new_trie = apply_batch(&old_trie, &ops);
        let new_root_hash = new_trie.nodestore().root_hash().unwrap();

        let witness = generate_witness(old_trie.nodestore(), &ops, new_root_hash, 2).unwrap();

        let truncated = TruncatedTrie::from_trie(old_trie.nodestore(), 2).unwrap();

        // Pass wrong expected ops
        let wrong_ops = vec![put_op(b"cherry", b"red")];
        let result = verify(&truncated, &witness, &wrong_ops);
        assert!(matches!(result, Err(WitnessError::OpsMismatch { .. })));
    }

    /// Correct expected ops pass validation and verify.
    #[test]
    fn test_verify_witness_correct_expected_ops() {
        let old_trie = create_test_trie(&[(b"apple", b"red"), (b"banana", b"yellow")]);

        let ops = vec![put_op(b"cherry", b"dark")];
        let new_trie = apply_batch(&old_trie, &ops);
        let new_root_hash = new_trie.nodestore().root_hash().unwrap();

        let witness =
            generate_witness(old_trie.nodestore(), &ops, new_root_hash.clone(), 2).unwrap();

        let truncated = TruncatedTrie::from_trie(old_trie.nodestore(), 2).unwrap();

        let updated = verify(&truncated, &witness, &ops).unwrap();
        assert_eq!(*updated.root_hash().unwrap(), new_root_hash);
    }

    #[test]
    fn test_verify_witness_with_delete_range() {
        // End-to-end: verify_witness with OwnedBatchOp containing DeleteRange
        let old_trie = create_test_trie(&[(b"a/1", b"v1"), (b"a/2", b"v2"), (b"b/1", b"v3")]);
        let view: &dyn crate::v2::api::DynDbView = old_trie.nodestore();

        let batch_ops = vec![core_delete_range(b"a/")];
        let expanded = expand_delete_ranges(view, &batch_ops).unwrap();

        let new_trie = apply_batch(&old_trie, &expanded);
        let new_root_hash = new_trie.nodestore().root_hash().unwrap();

        let truncation_depth = 2;
        let witness = generate_witness(
            old_trie.nodestore(),
            &expanded,
            new_root_hash,
            truncation_depth,
        )
        .unwrap();

        let truncated = TruncatedTrie::from_trie(old_trie.nodestore(), truncation_depth).unwrap();
        // Call verify_witness directly with OwnedBatchOp (including DeleteRange)
        let updated = verify_witness(&truncated, &witness, &batch_ops).unwrap();
        assert_eq!(*updated.root_hash().unwrap(), witness.new_root_hash);
    }

    /// Attack 1: [Put("a/1", "v"), DeleteRange("a/")] — "a/1" was Put
    /// before the `DeleteRange`, so it must be deleted. A malicious server
    /// that omits the delete should be caught.
    #[test]
    fn test_verify_witness_pre_dr_put_must_not_survive() {
        // Old trie has b/1 so it's not empty after the ops.
        let old_trie = create_test_trie(&[(b"b/1", b"v")]);
        let view: &dyn crate::v2::api::DynDbView = old_trie.nodestore();

        // Batch: Put a/1, then DeleteRange a/ — net effect: a/1 is deleted.
        let batch_ops = vec![core_put(b"a/1", b"v"), core_delete_range(b"a/")];
        let expanded = expand_delete_ranges(view, &batch_ops).unwrap();

        // Honest server applies the expanded ops (a/1 is deleted).
        let new_trie = apply_batch(&old_trie, &expanded);
        let honest_hash = new_trie.nodestore().root_hash().unwrap();

        // Malicious server: omits the delete of a/1 from the witness.
        // The witness batch_ops have Put(a/1) but no Delete(a/1).
        let malicious_witness_ops: Vec<ClientOp> = vec![ClientOp::Put {
            key: b"a/1".to_vec().into(),
            value: b"v".to_vec().into(),
        }];

        // The malicious trie still has a/1.
        let malicious_trie = apply_batch(&old_trie, &malicious_witness_ops);
        let malicious_hash = malicious_trie.nodestore().root_hash().unwrap();

        let truncation_depth = 2;
        let witness = WitnessProof {
            batch_ops: malicious_witness_ops.into(),
            new_root_hash: malicious_hash.clone(),
            witness_nodes: generate_witness(
                old_trie.nodestore(),
                &[ClientOp::Put {
                    key: b"a/1".to_vec().into(),
                    value: b"v".to_vec().into(),
                }],
                malicious_hash.clone(),
                truncation_depth,
            )
            .unwrap()
            .witness_nodes,
        };

        let truncated = TruncatedTrie::from_trie(old_trie.nodestore(), truncation_depth).unwrap();
        let err = verify_witness(&truncated, &witness, &batch_ops).unwrap_err();
        assert!(
            matches!(err, WitnessError::OpsMismatch { .. }),
            "expected OpsMismatch, got: {err:?}"
        );
        // Honest witness should succeed.
        assert_ne!(honest_hash, malicious_hash);
    }

    /// Attack 2: [DR("a/"), Put("a/1", "v"), DR("a/")] — the second
    /// `DeleteRange` deletes a/1, so it must not survive.
    #[test]
    fn test_verify_witness_put_between_two_drs_must_not_survive() {
        let old_trie = create_test_trie(&[(b"a/0", b"v0"), (b"b/1", b"v")]);
        let view: &dyn crate::v2::api::DynDbView = old_trie.nodestore();

        // Batch: DR(a/), Put(a/1), DR(a/) — net: a/0 and a/1 both deleted.
        let batch_ops = vec![
            core_delete_range(b"a/"),
            core_put(b"a/1", b"v"),
            core_delete_range(b"a/"),
        ];
        let expanded = expand_delete_ranges(view, &batch_ops).unwrap();

        let new_trie = apply_batch(&old_trie, &expanded);
        let honest_hash = new_trie.nodestore().root_hash().unwrap();

        // Malicious server: keeps a/1 alive (only deletes a/0).
        let malicious_ops: Vec<ClientOp> = vec![
            ClientOp::Delete {
                key: b"a/0".to_vec().into(),
            },
            ClientOp::Put {
                key: b"a/1".to_vec().into(),
                value: b"v".to_vec().into(),
            },
        ];
        let malicious_trie = apply_batch(&old_trie, &malicious_ops);
        let malicious_hash = malicious_trie.nodestore().root_hash().unwrap();

        let truncation_depth = 2;
        // Witness: Delete(a/0) for first DR, Put(a/1), then nothing for second DR
        let witness_batch: Vec<ClientOp> = vec![
            ClientOp::Delete {
                key: b"a/0".to_vec().into(),
            },
            ClientOp::Put {
                key: b"a/1".to_vec().into(),
                value: b"v".to_vec().into(),
            },
        ];

        let witness = WitnessProof {
            batch_ops: witness_batch.clone().into(),
            new_root_hash: malicious_hash.clone(),
            witness_nodes: generate_witness(
                old_trie.nodestore(),
                &witness_batch,
                malicious_hash.clone(),
                truncation_depth,
            )
            .unwrap()
            .witness_nodes,
        };

        let truncated = TruncatedTrie::from_trie(old_trie.nodestore(), truncation_depth).unwrap();
        let err = verify_witness(&truncated, &witness, &batch_ops).unwrap_err();
        assert!(
            matches!(err, WitnessError::OpsMismatch { .. }),
            "expected OpsMismatch, got: {err:?}"
        );
        assert_ne!(honest_hash, malicious_hash);
    }

    /// Positive case: [DR("a/"), Put("a/3", "v3")] — the Put comes after
    /// the DR, so a/3 legitimately survives.
    #[test]
    fn test_verify_witness_post_dr_put_survives() {
        let old_trie = create_test_trie(&[(b"a/1", b"v1"), (b"a/2", b"v2"), (b"b/1", b"v3")]);
        let view: &dyn crate::v2::api::DynDbView = old_trie.nodestore();

        let batch_ops = vec![core_delete_range(b"a/"), core_put(b"a/3", b"v3")];
        let expanded = expand_delete_ranges(view, &batch_ops).unwrap();

        let new_trie = apply_batch(&old_trie, &expanded);
        let new_root_hash = new_trie.nodestore().root_hash().unwrap();

        let truncation_depth = 2;
        let witness = generate_witness(
            old_trie.nodestore(),
            &expanded,
            new_root_hash,
            truncation_depth,
        )
        .unwrap();

        let truncated = TruncatedTrie::from_trie(old_trie.nodestore(), truncation_depth).unwrap();
        let updated = verify_witness(&truncated, &witness, &batch_ops).unwrap();
        assert_eq!(*updated.root_hash().unwrap(), witness.new_root_hash);
    }
}
