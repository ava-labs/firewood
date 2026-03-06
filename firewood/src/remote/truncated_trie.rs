// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! A truncated in-memory trie for remote/client-side storage.
//!
//! The [`TruncatedTrie`] holds the top K levels of a Merkle trie in memory,
//! with children at depth K replaced by [`Child::Proxy`] nodes that store only
//! the child's hash. This allows a client to verify proofs and track state
//! without holding the full trie.

use firewood_storage::{
    BranchNode, Child, Children, HashType, HashedNodeReader, MaybePersistedNode, Node, NodeError,
    Path, SharedNode, TrieHash, hash_node,
};

/// A truncated in-memory trie containing only the top K levels.
///
/// Nodes below the truncation depth are replaced with [`Child::Proxy`] stubs
/// that store only the child's hash. The root hash of the truncated trie
/// matches the root hash of the full trie, since Merkle hashes at any node
/// depend only on the node's data and its children's hashes.
#[derive(Debug, Clone)]
pub struct TruncatedTrie {
    /// The root hash of the trie (matches the full trie's root hash).
    root_hash: Option<TrieHash>,
    /// The in-memory root node (with Proxy leaves at depth K).
    root: Option<Node>,
    /// The truncation depth in node levels (number of branch-node hops from the root).
    truncation_depth: usize,
}

impl TruncatedTrie {
    /// Creates a new empty `TruncatedTrie` with the given truncation depth.
    #[must_use]
    pub const fn new(truncation_depth: usize) -> Self {
        Self {
            root_hash: None,
            root: None,
            truncation_depth,
        }
    }

    /// Creates a `TruncatedTrie` from pre-existing parts (used by deserialization).
    #[must_use]
    pub const fn from_parts(
        root_hash: Option<TrieHash>,
        root: Option<Node>,
        truncation_depth: usize,
    ) -> Self {
        Self {
            root_hash,
            root,
            truncation_depth,
        }
    }

    /// Constructs a `TruncatedTrie` from a full trie by walking the top K levels
    /// and replacing children at depth K with [`Child::Proxy`] stubs.
    ///
    /// The resulting truncated trie has the same root hash as the full trie.
    ///
    /// # Arguments
    ///
    /// * `trie` - A `HashedNodeReader` providing access to the full trie
    /// * `depth` - The truncation depth in node levels
    ///
    /// # Errors
    ///
    /// Returns a [`NodeError`] if nodes cannot be read from the trie.
    pub fn from_trie<T: HashedNodeReader>(trie: &T, depth: usize) -> Result<Self, NodeError> {
        let Some(root_node) = trie.root_node() else {
            return Ok(Self {
                root_hash: None,
                root: None,
                truncation_depth: depth,
            });
        };

        let (truncated_root, root_hash) = truncate_node(trie, &root_node, 0, depth, &Path::new())?;

        Ok(Self {
            root_hash: Some(root_hash.into_triehash()),
            root: Some(truncated_root),
            truncation_depth: depth,
        })
    }

    /// Returns the root hash of the truncated trie.
    #[must_use]
    pub const fn root_hash(&self) -> Option<&TrieHash> {
        self.root_hash.as_ref()
    }

    /// Returns a reference to the root node.
    #[must_use]
    pub const fn root(&self) -> Option<&Node> {
        self.root.as_ref()
    }

    /// Returns the truncation depth in node levels.
    #[must_use]
    pub const fn truncation_depth(&self) -> usize {
        self.truncation_depth
    }

    /// Verifies that the truncated trie is consistent with the given root hash.
    ///
    /// Recomputes the root hash from the in-memory nodes bottom-up and checks
    /// that it matches the expected hash. Proxy children's hashes are used as
    /// leaf hashes at depth K.
    #[must_use]
    pub fn verify_root_hash(&self, expected: &TrieHash) -> bool {
        match &self.root {
            None => false,
            Some(root) => {
                let computed = hash_node(root, &Path::new());
                computed.into_triehash() == *expected
            }
        }
    }

    /// Updates the truncated trie from a new committed root node.
    ///
    /// Extracts the top K levels from `new_root` and rebuilds the truncated
    /// trie with new [`Child::Proxy`] children at depth K.
    ///
    /// # Arguments
    ///
    /// * `trie` - A `HashedNodeReader` providing access to the new full trie state
    ///
    /// # Errors
    ///
    /// Returns a [`NodeError`] if nodes cannot be read from the trie.
    pub fn update_from_trie<T: HashedNodeReader>(&mut self, trie: &T) -> Result<(), NodeError> {
        let Some(root_node) = trie.root_node() else {
            self.root = None;
            self.root_hash = None;
            return Ok(());
        };

        let (truncated_root, root_hash) =
            truncate_node(trie, &root_node, 0, self.truncation_depth, &Path::new())?;

        self.root_hash = Some(root_hash.into_triehash());
        self.root = Some(truncated_root);
        Ok(())
    }

    /// Updates the truncated trie directly from a new root node that has
    /// already been truncated (e.g., from witness re-execution).
    pub fn update_from_truncated_root(&mut self, root: Option<Node>) {
        match &root {
            None => {
                self.root_hash = None;
            }
            Some(node) => {
                let hash = hash_node(node, &Path::new());
                self.root_hash = Some(hash.into_triehash());
            }
        }
        self.root = root;
    }
}

/// Recursively truncates a trie node at the given depth, computing Merkle
/// hashes bottom-up.
///
/// Depth is measured in **node hops**: each branch node visited counts as one
/// hop regardless of its partial-path length. When `current_depth >= max_depth`,
/// the node's children are replaced with [`Child::Proxy`] hash-only stubs.
/// Otherwise the node is kept in memory and its children are recursively
/// truncated at `current_depth + 1`.
///
/// Returns `(truncated_node, hash)`. Children in the truncated trie are stored
/// as [`Child::MaybePersisted`] above depth K (preserving in-memory node data
/// with their computed hash) or [`Child::Proxy`] at depth K (hash-only stubs).
///
/// `MaybePersisted` for intermediate children lets `hash_node()` find child
/// hashes via `children_hashes()` while the truncated trie still retains
/// in-memory node data for traversal.
///
/// Note: `path_prefix` tracks the full nibble-level path from the root for
/// hash computation. It is unrelated to the node-hop depth counter.
fn truncate_node<T: HashedNodeReader>(
    trie: &T,
    node: &SharedNode,
    current_depth: usize,
    max_depth: usize,
    path_prefix: &Path,
) -> Result<(Node, HashType), NodeError> {
    match node.as_ref() {
        // Leaves have no children to proxy — keep as-is and compute hash.
        Node::Leaf(leaf) => {
            let leaf_node = Node::Leaf(leaf.clone());
            let hash = hash_node(&leaf_node, path_prefix);
            Ok((leaf_node, hash))
        }
        Node::Branch(branch) => {
            if current_depth >= max_depth {
                // At or beyond the truncation depth: replace all children
                // with Proxy stubs that carry only the child's hash.
                let children = proxy_all_children(branch)?;
                let truncated = Node::Branch(Box::new(BranchNode {
                    partial_path: branch.partial_path.clone(),
                    value: branch.value.clone(),
                    children,
                }));
                let hash = hash_node(&truncated, path_prefix);
                Ok((truncated, hash))
            } else {
                // Above the truncation depth: recurse into each child,
                // advancing the hop counter by 1.
                let mut new_children = Children::new();
                for (idx, child_opt) in &branch.children {
                    let Some(child) = child_opt else {
                        continue;
                    };
                    let child_node = child.as_shared_node(trie)?;

                    // Build the child's full nibble path for hash computation:
                    // parent prefix + this branch's partial path + child index.
                    let child_path_prefix = Path::from_nibbles_iterator(
                        path_prefix
                            .iter()
                            .chain(branch.partial_path.iter())
                            .chain(std::iter::once(&idx.as_u8()))
                            .copied(),
                    );

                    let (truncated_child, child_hash) = truncate_node(
                        trie,
                        &child_node,
                        current_depth.saturating_add(1),
                        max_depth,
                        &child_path_prefix,
                    )?;

                    // Wrap as MaybePersisted: carries both the in-memory node
                    // data (for traversal) and precomputed hash (for hashing).
                    let shared = SharedNode::new(truncated_child);
                    let maybe_persisted = MaybePersistedNode::from(shared);
                    *new_children.get_mut(idx) =
                        Some(Child::MaybePersisted(maybe_persisted, child_hash));
                }
                let truncated = Node::Branch(Box::new(BranchNode {
                    partial_path: branch.partial_path.clone(),
                    value: branch.value.clone(),
                    children: new_children,
                }));
                let hash = hash_node(&truncated, path_prefix);
                Ok((truncated, hash))
            }
        }
    }
}

/// Replace all children with [`Child::Proxy`] stubs using their hashes.
///
/// # Errors
///
/// Returns [`NodeError::UnhashedChild`] if any child has no hash. This
/// cannot happen when the trie is backed by a [`HashedNodeReader`]
/// (i.e. an immutable or committed nodestore), since all children in
/// those stores are guaranteed to have hashes.
fn proxy_all_children(branch: &BranchNode) -> Result<Children<Option<Child>>, NodeError> {
    let mut new_children = Children::new();
    for (idx, child_opt) in &branch.children {
        let Some(child) = child_opt else {
            continue;
        };
        let hash = child.hash().ok_or(NodeError::UnhashedChild)?;
        *new_children.get_mut(idx) = Some(Child::Proxy(hash.clone()));
    }
    Ok(new_children)
}

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used)]

    use super::*;
    use firewood_storage::{HashedNodeReader, MemStore, NodeStore};
    use std::sync::Arc;

    use crate::merkle::Merkle;

    type ImmutableMerkle = Merkle<NodeStore<Arc<firewood_storage::ImmutableProposal>, MemStore>>;

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

        // Convert to immutable to get hashes
        merkle.try_into().unwrap()
    }

    #[test]
    fn test_empty_trie_truncation() {
        let trie = create_test_trie(&[]);

        let truncated = TruncatedTrie::from_trie(trie.nodestore(), 4).unwrap();
        assert!(truncated.root_hash().is_none());
        assert!(truncated.root().is_none());
    }

    #[test]
    fn test_single_key_trie_truncation() {
        let trie = create_test_trie(&[(b"hello", b"world")]);
        let truncated = TruncatedTrie::from_trie(trie.nodestore(), 4).unwrap();

        // Root hash should match the full trie
        let expected_hash = trie.nodestore().root_hash().unwrap();
        assert_eq!(*truncated.root_hash().unwrap(), expected_hash);

        // Verify root hash
        assert!(truncated.verify_root_hash(&expected_hash));
    }

    #[test]
    fn test_truncated_trie_root_hash_matches_full() {
        let keys: Vec<(&[u8], &[u8])> = vec![
            (b"apple", b"red"),
            (b"banana", b"yellow"),
            (b"cherry", b"red"),
            (b"date", b"brown"),
            (b"elderberry", b"purple"),
        ];
        let trie = create_test_trie(&keys);
        let full_root_hash = trie.nodestore().root_hash().unwrap();

        // Truncate at different depths
        for depth in 1..=6 {
            let truncated = TruncatedTrie::from_trie(trie.nodestore(), depth).unwrap();

            let truncated_hash = truncated.root_hash().unwrap();
            assert_eq!(
                *truncated_hash, full_root_hash,
                "Root hash mismatch at truncation depth {depth}"
            );

            assert!(
                truncated.verify_root_hash(&full_root_hash),
                "verify_root_hash failed at depth {depth}"
            );
        }
    }

    #[test]
    fn test_truncated_trie_has_proxy_children() {
        // Create a trie with enough keys to have branch nodes at depth > 1
        let mut keys: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        for i in 0u8..20 {
            keys.push((vec![i, 0, 0, 0], vec![i]));
        }
        let key_refs: Vec<(&[u8], &[u8])> = keys
            .iter()
            .map(|(k, v)| (k.as_slice(), v.as_slice()))
            .collect();
        let trie = create_test_trie(&key_refs);

        let truncated = TruncatedTrie::from_trie(trie.nodestore(), 0).unwrap();

        // The root should be a branch node
        let root = truncated.root().unwrap();
        if let Node::Branch(branch) = root {
            // At node depth 0, all children become Proxy nodes
            let has_proxy = branch
                .children
                .iter()
                .any(|(_, child)| matches!(child, Some(Child::Proxy(_))));
            assert!(has_proxy, "Expected proxy children at node depth 0");
        }
    }

    #[test]
    fn test_verify_root_hash_rejects_wrong_hash() {
        let trie = create_test_trie(&[(b"key", b"value")]);
        let truncated = TruncatedTrie::from_trie(trie.nodestore(), 4).unwrap();

        let wrong_hash = TrieHash::empty();
        assert!(!truncated.verify_root_hash(&wrong_hash));
    }

    #[test]
    fn test_truncation_depth_zero_proxies_all() {
        let trie = create_test_trie(&[(b"key", b"value")]);
        let truncated = TruncatedTrie::from_trie(trie.nodestore(), 0).unwrap();

        // Even at depth 0, root hash should match
        let expected = trie.nodestore().root_hash().unwrap();
        assert_eq!(*truncated.root_hash().unwrap(), expected);
    }

    fn count_proxies(node: &Node) -> usize {
        match node {
            Node::Leaf(_) => 0,
            Node::Branch(branch) => {
                let mut count = 0usize;
                for (_, child) in &branch.children {
                    match child {
                        Some(Child::Proxy(_)) => count = count.saturating_add(1),
                        Some(Child::Node(n)) => {
                            count = count.saturating_add(count_proxies(n));
                        }
                        _ => {}
                    }
                }
                count
            }
        }
    }

    #[test]
    fn test_deep_truncation_preserves_all_nodes() {
        let trie = create_test_trie(&[(b"key", b"value")]);
        let full_hash = trie.nodestore().root_hash().unwrap();

        // Truncation depth larger than trie depth should preserve everything
        let truncated = TruncatedTrie::from_trie(trie.nodestore(), 100).unwrap();

        assert_eq!(*truncated.root_hash().unwrap(), full_hash);

        // Root should have no proxy children (trie is too shallow)
        let root = truncated.root().unwrap();
        assert_eq!(
            count_proxies(root),
            0,
            "No proxies expected for deep truncation"
        );
    }
}
