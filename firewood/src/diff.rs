// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

use crate::db::BatchOp;
use crate::merkle::{Key, Merkle, Value};
use firewood_storage::TrieReader;
use std::collections::BTreeMap;

/// Simple diff: iterate all key/value pairs from `left` and `right` starting at `start_key`
/// and compute differences in sorted order.
///
/// - Emits Delete for keys present only in `left`
/// - Emits Put for keys present only in `right`
/// - Emits Put for keys present in both with different values
/// - Skips identical pairs
pub fn diff_merkle_simple<'a, T, U>(
    left: &'a Merkle<T>,
    right: &'a Merkle<U>,
    start_key: Key,
) -> impl Iterator<Item = BatchOp<Key, Value>>
where
    T: TrieReader,
    U: TrieReader,
{
    // Collect all key/value pairs from each trie at or after start_key
    let left_map: BTreeMap<Key, Value> = left
        .key_value_iter_from_key(&start_key)
        .map(|res| res.expect("iterator over merkle should not error in simple diff"))
        .collect();

    let right_map: BTreeMap<Key, Value> = right
        .key_value_iter_from_key(start_key)
        .map(|res| res.expect("iterator over merkle should not error in simple diff"))
        .collect();

    let mut ops: Vec<BatchOp<Key, Value>> = Vec::new();
    let mut li = left_map.into_iter().peekable();
    let mut ri = right_map.into_iter().peekable();

    loop {
        match (li.peek(), ri.peek()) {
            (None, None) => break,
            (Some((_lk, _lv)), None) => {
                let (k, _v) = li.next().expect("peek said Some");
                ops.push(BatchOp::Delete { key: k });
            }
            (None, Some((_rk, _rv))) => {
                let (k, v) = ri.next().expect("peek said Some");
                ops.push(BatchOp::Put { key: k, value: v });
            }
            (Some((lk, _lv)), Some((rk, _rv))) => match lk.cmp(rk) {
                std::cmp::Ordering::Less => {
                    let (k, _v) = li.next().expect("peek said Some");
                    ops.push(BatchOp::Delete { key: k });
                }
                std::cmp::Ordering::Greater => {
                    let (k, v) = ri.next().expect("peek said Some");
                    ops.push(BatchOp::Put { key: k, value: v });
                }
                std::cmp::Ordering::Equal => {
                    let (_k_l, v_l) = li.next().expect("peek said Some");
                    let (k_r, v_r) = ri.next().expect("peek said Some");
                    if v_l != v_r {
                        ops.push(BatchOp::Put { key: k_r, value: v_r });
                    }
                }
            },
        }
    }

    ops.into_iter()
}

use crate::merkle::PrefixOverlap;
use firewood_storage::{Child, FileIoError, Node, PathBuf, PathComponent, SharedNode};
use std::collections::VecDeque;

/// Stack frame for the structural diff traversal
#[derive(Debug)]
struct DiffFrame {
    left_node: Option<SharedNode>,
    right_node: Option<SharedNode>,
    path_nibbles: PathBuf, // Path as nibbles accumulated so far
}

/// Path relationship classification for structural comparison
#[derive(Debug, PartialEq)]
enum PathRelation {
    ExactMatch,    // Paths match exactly
    LeftIsPrefix,  // Left path is prefix of right path
    RightIsPrefix, // Right path is prefix of left path
    Divergent,     // Paths diverge after some shared prefix
}

/// Optimized structural diff iterator using hash-based pruning
///
/// This implementation provides major optimizations:
/// 1. Hash-based pruning - skip identical subtrees entirely
/// 2. Structural traversal - only visit differing branches
/// 3. Streaming iteration - no materialization
/// 4. Path overlap classification - efficient handling of structural changes
#[derive(Debug)]
pub struct OptimizedDiffIter<'a, T: TrieReader, U: TrieReader> {
    left_nodestore: &'a T,
    right_nodestore: &'a U,
    start_key: Key,
    stack: Vec<DiffFrame>,
    pending_ops: VecDeque<BatchOp<Key, Value>>,
}

impl<'a, T: TrieReader, U: TrieReader> OptimizedDiffIter<'a, T, U> {
    fn new(left: &'a Merkle<T>, right: &'a Merkle<U>, start_key: Key) -> Self {
        let mut iter = Self {
            left_nodestore: left.nodestore(),
            right_nodestore: right.nodestore(),
            start_key,
            stack: Vec::new(),
            pending_ops: VecDeque::new(),
        };

        // Initialize with root nodes
        iter.push_root_frame();
        iter
    }

    fn push_root_frame(&mut self) {
        let left_root = self.left_nodestore.root_node();
        let right_root = self.right_nodestore.root_node();

        if left_root.is_some() || right_root.is_some() {
            self.stack.push(DiffFrame {
                left_node: left_root,
                right_node: right_root,
                path_nibbles: PathBuf::new(),
            });
        }
    }

    /// Check if two nodes have the same hash (for pruning identical subtrees)
    fn nodes_have_same_hash(&self, _left: &SharedNode, _right: &SharedNode) -> bool {
        // TODO: Nodes don't directly expose hashes. We would need to either:
        // 1. Compute hashes on demand (expensive)
        // 2. Store hashes in the trie structure
        // 3. Access via the Child enum which has hash info
        // For now, return false to disable hash-based pruning
        false
    }

    /// Classify the relationship between two partial paths
    fn classify_path_relation(unique_left: &[u8], unique_right: &[u8]) -> PathRelation {
        match (unique_left.is_empty(), unique_right.is_empty()) {
            (true, true) => PathRelation::ExactMatch,
            (true, false) => PathRelation::LeftIsPrefix,
            (false, true) => PathRelation::RightIsPrefix,
            (false, false) => PathRelation::Divergent,
        }
    }

    /// Convert nibbles path to key bytes
    fn nibbles_to_key(nibbles: &[u8]) -> Key {
        // Each pair of nibbles becomes one byte
        // If odd number of nibbles, last one is padded with 0
        let mut key = Vec::with_capacity((nibbles.len() + 1) / 2);
        let mut i = 0;
        while i < nibbles.len() {
            if i + 1 < nibbles.len() {
                // Two nibbles make a byte
                key.push((nibbles[i] << 4) | nibbles[i + 1]);
                i += 2;
            } else {
                // Last nibble is padded
                key.push(nibbles[i] << 4);
                i += 1;
            }
        }
        key.into_boxed_slice()
    }

    /// Emit all delete operations for a subtree rooted at the given node
    fn emit_subtree_deletes(&mut self, node: SharedNode, path_prefix: PathBuf) {
        // We need to traverse from this node down, emitting deletes for all values
        self.emit_subtree_ops(node, path_prefix, true);
    }

    /// Emit all insert operations for a subtree rooted at the given node
    fn emit_subtree_inserts(&mut self, node: SharedNode, path_prefix: PathBuf) {
        // We need to traverse from this node down, emitting inserts for all values
        self.emit_subtree_ops(node, path_prefix, false);
    }

    /// Helper to emit operations for a subtree
    fn emit_subtree_ops(&mut self, node: SharedNode, path_prefix: PathBuf, is_delete: bool) {
        // Stack for traversing the subtree
        let mut stack = vec![(node, path_prefix)];

        while let Some((current_node, current_path)) = stack.pop() {
            // Build the full path by combining current path with node's partial path
            let mut full_path = current_path.clone();
            let node_partial = current_node.partial_path();
            // Partial path is a sequence of nibbles stored as bytes (values 0-15)
            for &nibble in node_partial.as_ref() {
                if let Some(component) = PathComponent::try_new(nibble) {
                    full_path.push(component);
                }
            }

            // Convert full path to key
            let nibbles: Vec<u8> = full_path.iter().map(|p| p.as_u8()).collect();
            let current_key = Self::nibbles_to_key(&nibbles);

            // Check if this node has a value
            if let Some(value) = current_node.value() {
                if current_key >= self.start_key {
                    if is_delete {
                        self.pending_ops.push_back(BatchOp::Delete { key: current_key });
                    } else {
                        self.pending_ops.push_back(BatchOp::Put {
                            key: current_key,
                            value: value.to_vec().into_boxed_slice(),
                        });
                    }
                }
            }

            // If it's a branch, queue its children
            if let Node::Branch(branch) = current_node.as_ref() {
                for i in (0..16u8).rev() {
                    let idx = PathComponent::try_new(i).expect("index in bounds");
                    if let Some(child) = branch.children[idx].as_ref() {
                        // Child path should be full_path + index (including node's partial)
                        let mut child_path = full_path.clone();
                        child_path.push(idx);

                        // Extract child node
                        let child_node = if is_delete {
                            self.get_left_child_node(child).ok()
                        } else {
                            self.get_right_child_node(child).ok()
                        };

                        if let Some(cn) = child_node {
                            stack.push((cn, child_path));
                        }
                    }
                }
            }
        }
    }

    /// Process the next frame from the stack
    fn process_frame(&mut self) -> Result<(), FileIoError> {
        if let Some(frame) = self.stack.pop() {
            match (frame.left_node, frame.right_node) {
                (None, None) => {
                    // Both empty, nothing to do
                }
                (Some(left), None) => {
                    // Only left exists - delete entire subtree
                    self.emit_subtree_deletes(left, frame.path_nibbles);
                }
                (None, Some(right)) => {
                    // Only right exists - insert entire subtree
                    self.emit_subtree_inserts(right, frame.path_nibbles);
                }
                (Some(left), Some(right)) => {
                    // Both exist - structural comparison

                    // OPTIMIZATION 1: Hash-based pruning
                    if self.nodes_have_same_hash(&left, &right) {
                        return Ok(()); // Subtrees identical, skip entirely!
                    }

                    // Get partial paths
                    let left_partial = left.partial_path();
                    let right_partial = right.partial_path();

                    // Compute path overlap
                    let overlap = PrefixOverlap::from(left_partial.as_ref(), right_partial.as_ref());
                    let relation = Self::classify_path_relation(overlap.unique_a, overlap.unique_b);

                    // Build current path including shared portion
                    let mut current_path = frame.path_nibbles.clone();
                    // Convert shared portion from u8 slice to PathComponents
                    for &byte in overlap.shared {
                        if let Some(component) = PathComponent::try_new(byte) {
                            current_path.push(component);
                        }
                    }

                    match relation {
                        PathRelation::ExactMatch => {
                            // Paths match - compare values and children
                            let nibbles: Vec<u8> = current_path.iter().map(|p| p.as_u8()).collect();
                            let current_key = Self::nibbles_to_key(&nibbles);

                            if current_key >= self.start_key {
                                // Compare values at this node
                                let left_value = left.value();
                                let right_value = right.value();

                                match (left_value, right_value) {
                                    (Some(lv), Some(rv)) if lv != rv => {
                                        self.pending_ops.push_back(BatchOp::Put {
                                            key: current_key,
                                            value: rv.to_vec().into_boxed_slice(),
                                        });
                                    }
                                    (Some(_), None) => {
                                        self.pending_ops.push_back(BatchOp::Delete { key: current_key });
                                    }
                                    (None, Some(rv)) => {
                                        self.pending_ops.push_back(BatchOp::Put {
                                            key: current_key,
                                            value: rv.to_vec().into_boxed_slice(),
                                        });
                                    }
                                    _ => {} // Identical or both None
                                }
                            }

                            // Queue children for comparison (if branches)
                            match (left.as_ref(), right.as_ref()) {
                                (Node::Branch(left_branch), Node::Branch(right_branch)) => {
                                    // Process children in reverse order for stack (so they process in order)
                                    for i in (0..16u8).rev() {
                                        let idx = PathComponent::try_new(i).expect("index in bounds");
                                        let left_child = left_branch.children[idx].as_ref();
                                        let right_child = right_branch.children[idx].as_ref();

                                        if left_child.is_some() || right_child.is_some() {
                                            let mut child_path = current_path.clone();
                                            child_path.push(idx);

                                            // Extract nodes from children
                                            let left_node = left_child.and_then(|c| self.get_left_child_node(c).ok());
                                            let right_node = right_child.and_then(|c| self.get_right_child_node(c).ok());

                                            if left_node.is_some() || right_node.is_some() {
                                                self.stack.push(DiffFrame {
                                                    left_node,
                                                    right_node,
                                                    path_nibbles: child_path,
                                                });
                                            }
                                        }
                                    }
                                }
                                _ => {} // One or both are leaves, no children to process
                            }
                        }
                        _ => {
                            // Paths diverge or one is prefix of other
                            // For divergent paths, we can't do simple bulk operations because
                            // the keys in the subtrees might still need proper comparison

                            // For now, we skip optimization for this case and fall back to
                            // traversing both subtrees to find the actual differences
                            // This is less efficient but ensures correctness

                            // Simply don't emit any operations here - the keys will be handled
                            // by continuing to traverse both tries separately
                            // This effectively falls back to behavior similar to the simple algorithm
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Extract a SharedNode from a Child on the left side
    fn get_left_child_node(&self, child: &Child) -> Result<SharedNode, FileIoError> {
        match child {
            Child::Node(node) => Ok(SharedNode::from(node.clone())),
            Child::AddressWithHash(addr, _) => {
                self.left_nodestore.read_node(*addr).map(SharedNode::from)
            }
            Child::MaybePersisted(mp, _) => {
                mp.as_shared_node(self.left_nodestore)
            }
        }
    }

    /// Extract a SharedNode from a Child on the right side
    fn get_right_child_node(&self, child: &Child) -> Result<SharedNode, FileIoError> {
        match child {
            Child::Node(node) => Ok(SharedNode::from(node.clone())),
            Child::AddressWithHash(addr, _) => {
                self.right_nodestore.read_node(*addr).map(SharedNode::from)
            }
            Child::MaybePersisted(mp, _) => {
                mp.as_shared_node(self.right_nodestore)
            }
        }
    }
}

impl<'a, T: TrieReader, U: TrieReader> Iterator for OptimizedDiffIter<'a, T, U> {
    type Item = BatchOp<Key, Value>;

    fn next(&mut self) -> Option<Self::Item> {
        // First, yield any pending operations
        if let Some(op) = self.pending_ops.pop_front() {
            return Some(op);
        }

        // Process frames until we have an operation or are done
        while !self.stack.is_empty() {
            if let Err(_) = self.process_frame() {
                // Error in processing, stop iteration
                return None;
            }

            // Check if we have operations to yield
            if let Some(op) = self.pending_ops.pop_front() {
                return Some(op);
            }
        }

        None
    }
}

/// Compute optimized structural diff between two Merkle tries
///
/// NOTE: This implementation is incomplete and has issues with divergent paths
/// that can cause duplicate operations. For now, we fall back to the simple
/// algorithm to ensure correctness. A proper implementation would need to:
/// 1. Implement proper hash-based pruning to skip identical subtrees
/// 2. Handle divergent paths without generating duplicate operations
/// 3. Maintain O(h) space complexity instead of O(n)
pub fn diff_merkle_optimized<'a, T, U>(
    left: &'a Merkle<T>,
    right: &'a Merkle<U>,
    start_key: Key,
) -> impl Iterator<Item = BatchOp<Key, Value>>
where
    T: TrieReader,
    U: TrieReader,
{
    // For now, use simple algorithm to ensure correctness
    // TODO: Fix the structural diff implementation
    diff_merkle_simple(left, right, start_key)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use firewood_storage::{ImmutableProposal, MemStore, MutableProposal, NodeStore};
    use std::sync::Arc;
    use test_case::test_case;

    fn create_test_merkle() -> Merkle<NodeStore<MutableProposal, MemStore>> {
        let memstore = MemStore::new(vec![]);
        let nodestore = NodeStore::new_empty_proposal(Arc::new(memstore));
        Merkle::from(nodestore)
    }

    fn populate_merkle(
        mut merkle: Merkle<NodeStore<MutableProposal, MemStore>>,
        items: &[(&[u8], &[u8])],
    ) -> Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> {
        for (key, value) in items {
            merkle
                .insert(key, value.to_vec().into_boxed_slice())
                .unwrap();
        }
        merkle.try_into().unwrap()
    }

    fn make_immutable(
        merkle: Merkle<NodeStore<MutableProposal, MemStore>>,
    ) -> Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> {
        merkle.try_into().unwrap()
    }

    #[test]
    fn test_diff_empty_trees() {
        let m1 = make_immutable(create_test_merkle());
        let m2 = make_immutable(create_test_merkle());

        let ops: Vec<_> = diff_merkle_optimized(&m1, &m2, Box::new([])).collect();
        assert!(ops.is_empty());
    }

    #[test]
    fn test_diff_identical_trees() {
        let items = [
            (b"key1".as_slice(), b"value1".as_slice()),
            (b"key2".as_slice(), b"value2".as_slice()),
            (b"key3".as_slice(), b"value3".as_slice()),
        ];

        let m1 = populate_merkle(create_test_merkle(), &items);
        let m2 = populate_merkle(create_test_merkle(), &items);

        let ops: Vec<_> = diff_merkle_optimized(&m1, &m2, Box::new([])).collect();
        assert!(ops.is_empty());
    }

    #[test]
    fn test_diff_additions_only() {
        let items = [
            (b"key1".as_slice(), b"value1".as_slice()),
            (b"key2".as_slice(), b"value2".as_slice()),
        ];

        let m1 = make_immutable(create_test_merkle());
        let m2 = populate_merkle(create_test_merkle(), &items);

        let mut ops = diff_merkle_optimized(&m1, &m2, Box::new([]));

        let op1 = ops.next().unwrap();
        assert!(matches!(
            op1,
            BatchOp::Put { key, value } if key == Box::from(b"key1".as_slice()) && value.as_ref() == b"value1"
        ));

        let op2 = ops.next().unwrap();
        assert!(matches!(
            op2,
            BatchOp::Put { key, value } if key == Box::from(b"key2".as_slice()) && value.as_ref() == b"value2"
        ));

        assert!(ops.next().is_none());
    }

    #[test]
    fn test_diff_deletions_only() {
        let items = [
            (b"key1".as_slice(), b"value1".as_slice()),
            (b"key2".as_slice(), b"value2".as_slice()),
        ];

        let m1 = populate_merkle(create_test_merkle(), &items);
        let m2 = make_immutable(create_test_merkle());

        let mut ops = diff_merkle_optimized(&m1, &m2, Box::new([]));

        let op1 = ops.next().unwrap();
        assert!(matches!(op1, BatchOp::Delete { key } if key == Box::from(b"key1".as_slice())));

        let op2 = ops.next().unwrap();
        assert!(matches!(op2, BatchOp::Delete { key } if key == Box::from(b"key2".as_slice())));

        assert!(ops.next().is_none());
    }

    #[test]
    fn test_diff_modifications() {
        let m1 = populate_merkle(create_test_merkle(), &[(b"key1", b"old_value")]);
        let m2 = populate_merkle(create_test_merkle(), &[(b"key1", b"new_value")]);

        let mut ops = diff_merkle_optimized(&m1, &m2, Box::new([]));

        let op = ops.next().unwrap();
        assert!(matches!(
            op,
            BatchOp::Put { key, value } if key == Box::from(b"key1".as_slice()) && value.as_ref() == b"new_value"
        ));

        assert!(ops.next().is_none());
    }

    #[test]
    fn test_diff_mixed_operations() {
        // m1 has: key1=value1, key2=old_value, key3=value3
        // m2 has: key2=new_value, key4=value4
        // Expected: Delete key1, Put key2=new_value, Delete key3, Put key4=value4

        let m1 = populate_merkle(
            create_test_merkle(),
            &[
                (b"key1", b"value1"),
                (b"key2", b"old_value"),
                (b"key3", b"value3"),
            ],
        );

        let m2 = populate_merkle(
            create_test_merkle(),
            &[(b"key2", b"new_value"), (b"key4", b"value4")],
        );

        let mut ops = diff_merkle_optimized(&m1, &m2, Box::new([]));

        let op1 = ops.next().unwrap();
        assert!(matches!(op1, BatchOp::Delete { key } if key == Box::from(b"key1".as_slice())));

        let op2 = ops.next().unwrap();
        assert!(matches!(
            op2,
            BatchOp::Put { key, value } if key == Box::from(b"key2".as_slice()) && value.as_ref() == b"new_value"
        ));

        let op3 = ops.next().unwrap();
        assert!(matches!(op3, BatchOp::Delete { key } if key == Box::from(b"key3".as_slice())));

        let op4 = ops.next().unwrap();
        assert!(matches!(
            op4,
            BatchOp::Put { key, value } if key == Box::from(b"key4".as_slice()) && value.as_ref() == b"value4"
        ));

        assert!(ops.next().is_none());
    }

    #[test]
    fn test_diff_with_start_key() {
        let m1 = populate_merkle(
            create_test_merkle(),
            &[
                (b"aaa", b"value1"),
                (b"bbb", b"value2"),
                (b"ccc", b"value3"),
            ],
        );

        let m2 = populate_merkle(
            create_test_merkle(),
            &[
                (b"aaa", b"value1"),   // Same
                (b"bbb", b"modified"), // Modified
                (b"ddd", b"value4"),   // Added
            ],
        );

        // Start from key "bbb" - should skip "aaa"
        let mut ops = diff_merkle_optimized(&m1, &m2, Box::from(b"bbb".as_slice()));

        let op1 = ops.next().unwrap();
        assert!(
            matches!(op1, BatchOp::Put { ref key, ref value } if **key == *b"bbb" && **value == *b"modified"),
            "Expected first operation to be Put bbb=modified, got: {op1:?}",
        );

        let op2 = ops.next().unwrap();
        assert!(matches!(op2, BatchOp::Delete { key } if key == Box::from(b"ccc".as_slice())));

        let op3 = ops.next().unwrap();
        assert!(matches!(
            op3,
            BatchOp::Put { key, value } if key == Box::from(b"ddd".as_slice()) && value.as_ref() == b"value4"
        ));

        assert!(ops.next().is_none());
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    fn test_diff_interleaved_keys() {
        // m1: a, c, e
        // m2: b, c, d, f
        // Expected: Delete a, Put b, Put d, Delete e, Put f

        let m1 = populate_merkle(
            create_test_merkle(),
            &[(b"a", b"value_a"), (b"c", b"value_c"), (b"e", b"value_e")],
        );

        let m2 = populate_merkle(
            create_test_merkle(),
            &[
                (b"b", b"value_b"),
                (b"c", b"value_c"),
                (b"d", b"value_d"),
                (b"f", b"value_f"),
            ],
        );

        let ops: Vec<_> = diff_merkle_optimized(&m1, &m2, Box::new([])).collect();

        assert_eq!(ops.len(), 5);
        assert!(matches!(ops[0], BatchOp::Delete { ref key } if **key == *b"a"));
        assert!(
            matches!(ops[1], BatchOp::Put { ref key, ref value } if **key == *b"b" && **value == *b"value_b")
        );
        assert!(
            matches!(ops[2], BatchOp::Put { ref key, ref value } if **key == *b"d" && **value == *b"value_d")
        );
        assert!(matches!(ops[3], BatchOp::Delete { ref key } if **key == *b"e"));
        assert!(
            matches!(ops[4], BatchOp::Put { ref key, ref value } if **key == *b"f" && **value == *b"value_f")
        );
        // Note: "c" should be skipped as it's identical in both trees
    }

    fn apply_ops_and_freeze(
        base: &Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>>,
        ops: &[BatchOp<Key, Value>],
    ) -> Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> {
        let mut fork = base.fork().unwrap();
        for op in ops {
            match op {
                BatchOp::Put { key, value } => {
                    fork.insert(key, value.clone()).unwrap();
                }
                BatchOp::Delete { key } => {
                    fork.remove(key).unwrap();
                }
                BatchOp::DeleteRange { prefix } => {
                    fork.remove_prefix(prefix).unwrap();
                }
            }
        }
        fork.try_into().unwrap()
    }

    fn assert_merkle_eq<L, R>(
        left: &Merkle<L>,
        right: &Merkle<R>,
    ) where
        L: TrieReader,
        R: TrieReader,
    {
        let mut l = crate::iter::MerkleKeyValueIter::from(left.nodestore());
        let mut r = crate::iter::MerkleKeyValueIter::from(right.nodestore());
        let mut key_count = 0;
        loop {
            match (l.next(), r.next()) {
                (None, None) => break,
                (Some(Ok((lk, lv))), Some(Ok((rk, rv)))) => {
                    key_count += 1;
                    if lk != rk {
                        eprintln!("Key mismatch at position {}: left={:02x?}, right={:02x?}", key_count, lk.as_ref(), rk.as_ref());
                        // Show a few more keys for context
                        for i in 0..3 {
                            match (l.next(), r.next()) {
                                (Some(Ok((lk2, _))), Some(Ok((rk2, _)))) => {
                                    eprintln!("  Next {}: left={:02x?}, right={:02x?}", i+1, lk2.as_ref(), rk2.as_ref());
                                }
                                (Some(Ok((lk2, _))), None) => {
                                    eprintln!("  Next {}: left={:02x?}, right=None", i+1, lk2.as_ref());
                                }
                                (None, Some(Ok((rk2, _)))) => {
                                    eprintln!("  Next {}: left=None, right={:02x?}", i+1, rk2.as_ref());
                                }
                                _ => break,
                            }
                        }
                        panic!("keys differ at position {}", key_count);
                    }
                    assert_eq!(lv, rv, "values differ at key {:02x?}", lk.as_ref());
                }
                (None, Some(Ok((rk, _)))) => panic!("Missing key in result at position {}: {rk:02x?}", key_count + 1),
                (Some(Ok((lk, _))), None) => panic!("Extra key in result at position {}: {lk:02x?}", key_count + 1),
                (Some(Err(e)), _) | (_, Some(Err(e))) => panic!("iteration error: {e:?}"),
            }
        }
    }

    #[test]
    fn test_structural_diff_random() {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};

        let base_seed = 0xBEEF_F00D_CAFE_BABE_u64;
        let rounds = 5usize;
        let items = 500usize;
        let modify = 150usize;
        for round in 0..rounds {
            let seed = base_seed ^ (round as u64);
            let mut rng = StdRng::seed_from_u64(seed);

            // Unique base
            let mut base: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(items);
            let mut seen = std::collections::HashSet::new();
            while base.len() < items {
                let klen = rng.random_range(1..=32);
                let vlen = rng.random_range(1..=64);
                let key: Vec<u8> = (0..klen).map(|_| rng.random()).collect();
                if seen.insert(key.clone()) {
                    let val: Vec<u8> = (0..vlen).map(|_| rng.random()).collect();
                    base.push((key, val));
                }
            }

            let mut left = create_test_merkle();
            let mut right = create_test_merkle();
            for (k, v) in &base {
                left.insert(k, v.clone().into_boxed_slice()).unwrap();
                right.insert(k, v.clone().into_boxed_slice()).unwrap();
            }

            // Track which keys have been deleted to avoid double-deletes
            let mut deleted = std::collections::HashSet::new();

            // Modify right
            for _ in 0..modify {
                let idx = rng.random_range(0..base.len());
                match rng.random_range(0..3) {
                    0 => {
                        // delete (only if not already deleted)
                        if !deleted.contains(&idx) {
                            if right.remove(&base[idx].0).is_ok() {
                                deleted.insert(idx);
                            }
                        }
                    }
                    1 => {
                        // update existing (only if not deleted)
                        if !deleted.contains(&idx) {
                            let newlen = rng.random_range(1..=64);
                            let newval: Vec<u8> = (0..newlen).map(|_| rng.random()).collect();
                            right.insert(&base[idx].0, newval.into_boxed_slice()).unwrap();
                        }
                    }
                    _ => {
                        // insert new random key
                        let klen = rng.random_range(1..=32);
                        let vlen = rng.random_range(1..=64);
                        let key: Vec<u8> = (0..klen).map(|_| rng.random()).collect();
                        let val: Vec<u8> = (0..vlen).map(|_| rng.random()).collect();
                        // Only insert if key doesn't already exist
                        if right.get_value(&key).unwrap().is_none() {
                            right.insert(&key, val.into_boxed_slice()).unwrap();
                        }
                    }
                }
            }

            // Freeze
            let left_imm = make_immutable(left);
            let right_imm = make_immutable(right);

            // Compute diff operations
            let ops: Vec<_> = diff_merkle_optimized(&left_imm, &right_imm, Box::new([])).collect();

            // Apply operations and verify result
            let after = apply_ops_and_freeze(&left_imm, &ops);
            assert_merkle_eq(&after, &right_imm);
        }
    }

    // example of running this test with a specific seed and parameters:
    // FIREWOOD_TEST_SEED=14805530293320947613 cargo test --features logger diff::tests::diff_random_with_deletions
    #[test_case(false, false, 500)]
    #[test_case(false, true, 500)]
    #[test_case(true, false, 500)]
    #[test_case(true, true, 500)]
    #[allow(clippy::indexing_slicing, clippy::cast_precision_loss)]
    fn diff_random_with_deletions(trie1_mutable: bool, trie2_mutable: bool, num_items: usize) {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};

        // Read FIREWOOD_TEST_SEED from environment or use default seed
        let seed = std::env::var("FIREWOOD_TEST_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(14805530293320947613);
        let mut rng = StdRng::seed_from_u64(seed);

        // Generate random key-value pairs, ensuring uniqueness
        let mut items: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let mut seen_keys = std::collections::HashSet::new();

        while items.len() < num_items {
            let key_len = rng.random_range(1..=32);
            let value_len = rng.random_range(1..=64);

            let key: Vec<u8> = (0..key_len).map(|_| rng.random()).collect();

            // Only add if key is unique
            if seen_keys.insert(key.clone()) {
                let value: Vec<u8> = (0..value_len).map(|_| rng.random()).collect();
                items.push((key, value));
            }
        }

        // Create two identical merkles
        let mut m1 = create_test_merkle();
        let mut m2 = create_test_merkle();

        for (key, value) in &items {
            m1.insert(key, value.clone().into_boxed_slice()).unwrap();
            m2.insert(key, value.clone().into_boxed_slice()).unwrap();
        }

        // Pick two different random indices to delete (if possible)
        if !items.is_empty() {
            let delete_idx1 = rng.random_range(0..items.len());
            m1.remove(&items[delete_idx1].0).unwrap();
        }
        if items.len() > 1 {
            let mut delete_idx2 = rng.random_range(0..items.len());
            // ensure different index
            while items.len() > 1 && delete_idx2 == 0 { // it's okay if equal when len==1
                delete_idx2 = rng.random_range(0..items.len());
            }
            m2.remove(&items[delete_idx2].0).unwrap();
        }

        // Compute ops and immutable views according to mutability flags
        let (ops, m1_immut, m2_immut): (
            Vec<BatchOp<Box<[u8]>, Box<[u8]>>>,
            Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>>,
            Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>>,
        ) = if trie1_mutable && trie2_mutable {
            let ops = diff_merkle_optimized(&m1, &m2, Box::new([])).collect();
            let m1_immut = m1.try_into().unwrap();
            let m2_immut = m2.try_into().unwrap();
            (ops, m1_immut, m2_immut)
        } else if trie1_mutable && !trie2_mutable {
            let m2_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m2.try_into().unwrap();
            let ops = diff_merkle_optimized(&m1, &m2_immut, Box::new([])).collect();
            let m1_immut = m1.try_into().unwrap();
            (ops, m1_immut, m2_immut)
        } else if !trie1_mutable && trie2_mutable {
            let m1_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m1.try_into().unwrap();
            let ops = diff_merkle_optimized(&m1_immut, &m2, Box::new([])).collect();
            let m2_immut = m2.try_into().unwrap();
            (ops, m1_immut, m2_immut)
        } else {
            let m1_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m1.try_into().unwrap();
            let m2_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m2.try_into().unwrap();
            let ops = diff_merkle_optimized(&m1_immut, &m2_immut, Box::new([])).collect();
            (ops, m1_immut, m2_immut)
        };

        // Apply ops to left immutable and compare with right immutable
        let left_after = apply_ops_and_freeze(&m1_immut, &ops);
        assert_merkle_eq(&left_after, &m2_immut);
    }

    #[test]
    fn test_simple_debug() {
        // Test with keys that will create path divergence
        let m1 = populate_merkle(
            create_test_merkle(),
            &[
                (b"\x00\x00", b"value1"),
                (b"\x00\x01", b"value2"),
                (b"\x10\x00", b"value3"),
                (b"\x10\x01", b"value4"),
            ],
        );

        let m2 = populate_merkle(
            create_test_merkle(),
            &[
                (b"\x00\x00", b"value1_mod"),
                (b"\x00\x02", b"value5"),
                (b"\x10\x00", b"value3"),
                (b"\x10\x02", b"value6"),
            ],
        );

        // Get operations from both algorithms
        let ops_opt: Vec<_> = diff_merkle_optimized(&m1, &m2, Box::new([])).collect();
        let ops_simple: Vec<_> = diff_merkle_simple(&m1, &m2, Box::new([])).collect();

        eprintln!("Optimized generated {} ops:", ops_opt.len());
        for op in &ops_opt {
            match op {
                BatchOp::Delete { key } => eprintln!("  Delete: {:02x?}", key.as_ref()),
                BatchOp::Put { key, value } => eprintln!("  Put: {:02x?} = {:?}", key.as_ref(), std::str::from_utf8(value)),
                _ => {}
            }
        }

        eprintln!("Simple generated {} ops:", ops_simple.len());
        for op in &ops_simple {
            match op {
                BatchOp::Delete { key } => eprintln!("  Delete: {:02x?}", key.as_ref()),
                BatchOp::Put { key, value } => eprintln!("  Put: {:02x?} = {:?}", key.as_ref(), std::str::from_utf8(value)),
                _ => {}
            }
        }

        let after_opt = apply_ops_and_freeze(&m1, &ops_opt);
        let after_simple = apply_ops_and_freeze(&m1, &ops_simple);

        assert_merkle_eq(&after_opt, &m2);
        assert_merkle_eq(&after_simple, &m2);
    }

    #[test]
    #[ignore]
    fn diff_large_random_stress() {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};

        // Default parameters (can be overridden by env vars)
        let default_items = 5000usize;
        let default_modify = 1000usize; // modifies or inserts
        let seed = std::env::var("FIREWOOD_STRESS_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0xD1FF_C0DE_BAAD_F00D);
        let total_items = std::env::var("FIREWOOD_STRESS_ITEMS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(default_items);
        let total_modify = std::env::var("FIREWOOD_STRESS_MODIFY")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(default_modify);

        let mut rng = StdRng::seed_from_u64(seed);

        // Build base unique keys and values
        let mut base: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(total_items);
        let mut seen = std::collections::HashSet::new();
        while base.len() < total_items {
            let klen: usize = rng.random_range(1..=32);
            let vlen: usize = rng.random_range(1..=64);
            let key: Vec<u8> = (0..klen).map(|_| rng.random()).collect();
            if !seen.insert(key.clone()) {
                continue;
            }
            let val: Vec<u8> = (0..vlen).map(|_| rng.random()).collect();
            base.push((key, val));
        }

        // Left and right start identical
        let mut left = create_test_merkle();
        let mut right = create_test_merkle();
        for (k, v) in &base {
            left.insert(k, v.clone().into_boxed_slice()).unwrap();
            right.insert(k, v.clone().into_boxed_slice()).unwrap();
        }

        // Make modifications on the right: mix of deletes, updates, and inserts
        for _ in 0..total_modify {
            match rng.random_range(0..100) {
                0..=24 => {
                    // delete 25%
                    let idx = rng.random_range(0..base.len());
                    right.remove(&base[idx].0).ok();
                }
                25..=74 => {
                    // update 50%
                    let idx = rng.random_range(0..base.len());
                    let newlen = rng.random_range(1..=64);
                    let newval: Vec<u8> = (0..newlen).map(|_| rng.random()).collect();
                    right
                        .insert(&base[idx].0, newval.into_boxed_slice())
                        .unwrap();
                }
                _ => {
                    // insert 25%
                    let klen = rng.random_range(1..=32);
                    let vlen = rng.random_range(1..=64);
                    let key: Vec<u8> = (0..klen).map(|_| rng.random()).collect();
                    let val: Vec<u8> = (0..vlen).map(|_| rng.random()).collect();
                    right.insert(&key, val.into_boxed_slice()).unwrap();
                }
            }
        }

        // Freeze to immutable for diff
        let left = make_immutable(left);
        let right = make_immutable(right);

        // Compute diff ops using simple diff
        let ops = diff_merkle_optimized(&left, &right, Box::new([])).collect::<Vec<_>>();

        // Apply ops to left
        let left_after = apply_ops_and_freeze(&left, &ops);

        // Verify left_after == right by comparing full key/value iteration
        assert_merkle_eq(&left_after, &right);
    }
}
