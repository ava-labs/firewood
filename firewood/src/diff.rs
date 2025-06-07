// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

use std::cmp::Ordering;
use storage::{Child, Node, SharedNode, TrieReader};

use crate::db::BatchOp;
use crate::merkle::{Key, Merkle, Value};
use crate::stream::{MerkleKeyValueStream, as_enumerated_children_iter, key_from_nibble_iter};

/// Iteration node that tracks state for both trees simultaneously
enum DiffIterationNode {
    /// Two unvisited nodes that should be compared
    UnvisitedPair {
        key: Key,
        node1: SharedNode,
        node2: SharedNode,
    },
    /// A node that exists only in tree1 (needs to be deleted)
    UnvisitedLeft { key: Key, node: SharedNode },
    /// A node that exists only in tree2 (needs to be added)
    UnvisitedRight { key: Key, node: SharedNode },
    /// A pair of visited branch nodes - track which children to compare next
    VisitedPair {
        key: Key,
        children_iter1: Box<dyn Iterator<Item = (u8, Child)> + Send>,
        children_iter2: Box<dyn Iterator<Item = (u8, Child)> + Send>,
    },
    /// A visited branch node from tree1 only (all its children need to be deleted)
    VisitedLeft {
        key: Key,
        children_iter: Box<dyn Iterator<Item = (u8, Child)> + Send>,
    },
    /// A visited branch node from tree2 only (all its children need to be added)
    VisitedRight {
        key: Key,
        children_iter: Box<dyn Iterator<Item = (u8, Child)> + Send>,
    },
}

// Due to the boxed iterators, we can't use the default Debug impl for DiffIterationNode
impl std::fmt::Debug for DiffIterationNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiffIterationNode::UnvisitedPair { key, node1, node2 } => f
                .debug_struct("UnvisitedPair")
                .field("key", key)
                .field("node1", node1)
                .field("node2", node2)
                .finish(),
            DiffIterationNode::UnvisitedLeft { key, node } => f
                .debug_struct("UnvisitedLeft")
                .field("key", key)
                .field("node", node)
                .finish(),
            DiffIterationNode::UnvisitedRight { key, node } => f
                .debug_struct("UnvisitedRight")
                .field("key", key)
                .field("node", node)
                .finish(),
            DiffIterationNode::VisitedPair { key, .. } => f
                .debug_struct("VisitedPair")
                .field("key", key)
                .field("children_iter1", &"<iterator>")
                .field("children_iter2", &"<iterator>")
                .finish(),
            DiffIterationNode::VisitedLeft { key, .. } => f
                .debug_struct("VisitedLeft")
                .field("key", key)
                .field("children_iter", &"<iterator>")
                .finish(),
            DiffIterationNode::VisitedRight { key, .. } => f
                .debug_struct("VisitedRight")
                .field("key", key)
                .field("children_iter", &"<iterator>")
                .finish(),
        }
    }
}

/// Optimized node iterator that compares two merkle trees and skips matching subtrees
struct DiffMerkleNodeStream<'a, T: TrieReader, U: TrieReader> {
    tree1: &'a T,
    tree2: &'a U,
    iter_stack: Vec<DiffIterationNode>,
}

impl<'a, T: TrieReader, U: TrieReader> DiffMerkleNodeStream<'a, T, U> {
    fn new(tree1: &'a T, tree2: &'a U, start_key: Key) -> Result<Self, storage::FileIoError> {
        let mut iter_stack = Vec::new();

        match (tree1.root_node(), tree2.root_node()) {
            (Some(root1), Some(root2)) => {
                iter_stack.push(DiffIterationNode::UnvisitedPair {
                    key: start_key,
                    node1: root1,
                    node2: root2,
                });
            }
            (Some(root1), None) => {
                iter_stack.push(DiffIterationNode::UnvisitedLeft {
                    key: start_key,
                    node: root1,
                });
            }
            (None, Some(root2)) => {
                iter_stack.push(DiffIterationNode::UnvisitedRight {
                    key: start_key,
                    node: root2,
                });
            }
            (None, None) => {
                // Both trees are empty, nothing to iterate
            }
        }

        Ok(Self {
            tree1,
            tree2,
            iter_stack,
        })
    }

    /// Check if two children have the same hash (indicating identical subtrees)
    fn children_have_same_hash(child1: &Child, child2: &Child) -> bool {
        match (child1, child2) {
            (Child::AddressWithHash(_, hash1), Child::AddressWithHash(_, hash2)) => hash1 == hash2,
            // we can't optimize if there is no hash yet
            _ => false,
        }
    }

    fn next_internal(&mut self) -> Option<Result<DiffIterationResult, storage::FileIoError>> {
        while let Some(iter_node) = self.iter_stack.pop() {
            match iter_node {
                DiffIterationNode::UnvisitedPair { key, node1, node2 } => {
                    // Note: We can't directly compare node hashes here since nodes don't store their own hashes.
                    // Hash-based optimization only works at the child level where Child::AddressWithHash contains precomputed hashes.

                    match (&*node1, &*node2) {
                        (Node::Branch(branch1), Node::Branch(branch2)) => {
                            // Compare values first
                            let value_diff = match (&branch1.value, &branch2.value) {
                                (Some(v1), Some(v2)) if v1 == v2 => None,
                                (Some(_), Some(v2)) => Some(DiffIterationResult::Changed {
                                    key: key_from_nibble_iter(key.iter().copied()),
                                    new_value: v2.to_vec(),
                                }),
                                (Some(_), None) => Some(DiffIterationResult::Deleted {
                                    key: key_from_nibble_iter(key.iter().copied()),
                                }),
                                (None, Some(v2)) => Some(DiffIterationResult::Added {
                                    key: key_from_nibble_iter(key.iter().copied()),
                                    value: v2.to_vec(),
                                }),
                                (None, None) => None,
                            };

                            // Set up to compare children
                            self.iter_stack.push(DiffIterationNode::VisitedPair {
                                key: key.clone(),
                                children_iter1: Box::new(as_enumerated_children_iter(branch1)),
                                children_iter2: Box::new(as_enumerated_children_iter(branch2)),
                            });

                            if let Some(result) = value_diff {
                                return Some(Ok(result));
                            }
                        }
                        (Node::Branch(branch), Node::Leaf(leaf)) => {
                            // Branch vs Leaf - need to process all branch children as deletions
                            // and add the leaf
                            self.iter_stack.push(DiffIterationNode::VisitedLeft {
                                key: key.clone(),
                                children_iter: Box::new(as_enumerated_children_iter(branch)),
                            });

                            return Some(Ok(DiffIterationResult::Added {
                                key: key_from_nibble_iter(key.iter().copied()),
                                value: leaf.value.to_vec(),
                            }));
                        }
                        (Node::Leaf(_leaf), Node::Branch(branch)) => {
                            // Leaf vs Branch - delete leaf, process all branch children as additions
                            self.iter_stack.push(DiffIterationNode::VisitedRight {
                                key: key.clone(),
                                children_iter: Box::new(as_enumerated_children_iter(branch)),
                            });

                            return Some(Ok(DiffIterationResult::Deleted {
                                key: key_from_nibble_iter(key.iter().copied()),
                            }));
                        }
                        (Node::Leaf(leaf1), Node::Leaf(leaf2)) => {
                            // Two leaves - compare values
                            if leaf1.value != leaf2.value {
                                return Some(Ok(DiffIterationResult::Changed {
                                    key: key_from_nibble_iter(key.iter().copied()),
                                    new_value: leaf2.value.to_vec(),
                                }));
                            }
                        }
                    }
                }
                DiffIterationNode::UnvisitedLeft { key, node } => {
                    // Node exists only in tree1 - mark for deletion
                    match &*node {
                        Node::Branch(branch) => {
                            self.iter_stack.push(DiffIterationNode::VisitedLeft {
                                key: key.clone(),
                                children_iter: Box::new(as_enumerated_children_iter(branch)),
                            });

                            if branch.value.is_some() {
                                return Some(Ok(DiffIterationResult::Deleted {
                                    key: key_from_nibble_iter(key.iter().copied()),
                                }));
                            }
                        }
                        Node::Leaf(_) => {
                            return Some(Ok(DiffIterationResult::Deleted {
                                key: key_from_nibble_iter(key.iter().copied()),
                            }));
                        }
                    }
                }
                DiffIterationNode::UnvisitedRight { key, node } => {
                    // Node exists only in tree2 - mark for addition
                    match &*node {
                        Node::Branch(branch) => {
                            self.iter_stack.push(DiffIterationNode::VisitedRight {
                                key: key.clone(),
                                children_iter: Box::new(as_enumerated_children_iter(branch)),
                            });

                            if let Some(value) = &branch.value {
                                return Some(Ok(DiffIterationResult::Added {
                                    key: key_from_nibble_iter(key.iter().copied()),
                                    value: value.to_vec(),
                                }));
                            }
                        }
                        Node::Leaf(leaf) => {
                            return Some(Ok(DiffIterationResult::Added {
                                key: key_from_nibble_iter(key.iter().copied()),
                                value: leaf.value.to_vec(),
                            }));
                        }
                    }
                }
                DiffIterationNode::VisitedPair {
                    key,
                    mut children_iter1,
                    mut children_iter2,
                } => {
                    // Compare children from both trees
                    let child1_opt = children_iter1.next();
                    let child2_opt = children_iter2.next();

                    match (child1_opt, child2_opt) {
                        (Some((pos1, child1)), Some((pos2, child2))) => {
                            match pos1.cmp(&pos2) {
                                Ordering::Equal => {
                                    // Same position - check if subtrees are identical
                                    if Self::children_have_same_hash(&child1, &child2) {
                                        // Identical subtrees, skip them and continue with remaining children
                                        self.iter_stack.push(DiffIterationNode::VisitedPair {
                                            key,
                                            children_iter1,
                                            children_iter2,
                                        });
                                        continue;
                                    } else {
                                        // Different subtrees, need to compare them
                                        let child_key: Key = key
                                            .iter()
                                            .copied()
                                            .chain(std::iter::once(pos1))
                                            .collect();

                                        // Continue with remaining children
                                        self.iter_stack.push(DiffIterationNode::VisitedPair {
                                            key,
                                            children_iter1,
                                            children_iter2,
                                        });

                                        // Load and compare the child nodes
                                        let node1 = match child1 {
                                            Child::AddressWithHash(addr, _) => {
                                                match self.tree1.read_node(addr) {
                                                    Ok(node) => node,
                                                    Err(e) => return Some(Err(e)),
                                                }
                                            }
                                            Child::Node(node) => node.clone().into(),
                                        };
                                        let node2 = match child2 {
                                            Child::AddressWithHash(addr, _) => {
                                                match self.tree2.read_node(addr) {
                                                    Ok(node) => node,
                                                    Err(e) => return Some(Err(e)),
                                                }
                                            }
                                            Child::Node(node) => node.clone().into(),
                                        };

                                        self.iter_stack.push(DiffIterationNode::UnvisitedPair {
                                            key: child_key,
                                            node1,
                                            node2,
                                        });
                                    }
                                }
                                Ordering::Less => {
                                    // pos1 < pos2: child exists in tree1 but not tree2
                                    let child_key: Key =
                                        key.iter().copied().chain(std::iter::once(pos1)).collect();

                                    // Put back child2 for next iteration
                                    let new_iter2 =
                                        std::iter::once((pos2, child2)).chain(children_iter2);
                                    self.iter_stack.push(DiffIterationNode::VisitedPair {
                                        key,
                                        children_iter1,
                                        children_iter2: Box::new(new_iter2),
                                    });

                                    let node1 = match child1 {
                                        Child::AddressWithHash(addr, _) => {
                                            match self.tree1.read_node(addr) {
                                                Ok(node) => node,
                                                Err(e) => return Some(Err(e)),
                                            }
                                        }
                                        Child::Node(node) => node.clone().into(),
                                    };

                                    self.iter_stack.push(DiffIterationNode::UnvisitedLeft {
                                        key: child_key,
                                        node: node1,
                                    });
                                }
                                Ordering::Greater => {
                                    // pos1 > pos2: child exists in tree2 but not tree1
                                    let child_key: Key =
                                        key.iter().copied().chain(std::iter::once(pos2)).collect();

                                    // Put back child1 for next iteration
                                    let new_iter1 =
                                        std::iter::once((pos1, child1)).chain(children_iter1);
                                    self.iter_stack.push(DiffIterationNode::VisitedPair {
                                        key,
                                        children_iter1: Box::new(new_iter1),
                                        children_iter2,
                                    });

                                    let node2 = match child2 {
                                        Child::AddressWithHash(addr, _) => {
                                            match self.tree2.read_node(addr) {
                                                Ok(node) => node,
                                                Err(e) => return Some(Err(e)),
                                            }
                                        }
                                        Child::Node(node) => node.clone().into(),
                                    };

                                    self.iter_stack.push(DiffIterationNode::UnvisitedRight {
                                        key: child_key,
                                        node: node2,
                                    });
                                }
                            }
                        }
                        (Some((pos1, child1)), None) => {
                            // Only tree1 has remaining children
                            let child_key: Key =
                                key.iter().copied().chain(std::iter::once(pos1)).collect();

                            // Continue with remaining children from tree1
                            self.iter_stack.push(DiffIterationNode::VisitedLeft {
                                key,
                                children_iter: children_iter1,
                            });

                            let node1 = match child1 {
                                Child::AddressWithHash(addr, _) => {
                                    match self.tree1.read_node(addr) {
                                        Ok(node) => node,
                                        Err(e) => return Some(Err(e)),
                                    }
                                }
                                Child::Node(node) => node.clone().into(),
                            };

                            self.iter_stack.push(DiffIterationNode::UnvisitedLeft {
                                key: child_key,
                                node: node1,
                            });
                        }
                        (None, Some((pos2, child2))) => {
                            // Only tree2 has remaining children
                            let child_key: Key =
                                key.iter().copied().chain(std::iter::once(pos2)).collect();

                            // Continue with remaining children from tree2
                            self.iter_stack.push(DiffIterationNode::VisitedRight {
                                key,
                                children_iter: children_iter2,
                            });

                            let node2 = match child2 {
                                Child::AddressWithHash(addr, _) => {
                                    match self.tree2.read_node(addr) {
                                        Ok(node) => node,
                                        Err(e) => return Some(Err(e)),
                                    }
                                }
                                Child::Node(node) => node.clone().into(),
                            };

                            self.iter_stack.push(DiffIterationNode::UnvisitedRight {
                                key: child_key,
                                node: node2,
                            });
                        }
                        (None, None) => {
                            // No more children in either tree, continue
                            continue;
                        }
                    }
                }
                DiffIterationNode::VisitedLeft {
                    key,
                    mut children_iter,
                } => {
                    if let Some((pos, child)) = children_iter.next() {
                        let child_key: Key =
                            key.iter().copied().chain(std::iter::once(pos)).collect();

                        // Continue with remaining children
                        self.iter_stack
                            .push(DiffIterationNode::VisitedLeft { key, children_iter });

                        let node = match child {
                            Child::AddressWithHash(addr, _) => match self.tree1.read_node(addr) {
                                Ok(node) => node,
                                Err(e) => return Some(Err(e)),
                            },
                            Child::Node(node) => node.clone().into(),
                        };

                        self.iter_stack.push(DiffIterationNode::UnvisitedLeft {
                            key: child_key,
                            node,
                        });
                    }
                }
                DiffIterationNode::VisitedRight {
                    key,
                    mut children_iter,
                } => {
                    if let Some((pos, child)) = children_iter.next() {
                        let child_key: Key =
                            key.iter().copied().chain(std::iter::once(pos)).collect();

                        // Continue with remaining children
                        self.iter_stack
                            .push(DiffIterationNode::VisitedRight { key, children_iter });

                        let node = match child {
                            Child::AddressWithHash(addr, _) => match self.tree2.read_node(addr) {
                                Ok(node) => node,
                                Err(e) => return Some(Err(e)),
                            },
                            Child::Node(node) => node.clone().into(),
                        };

                        self.iter_stack.push(DiffIterationNode::UnvisitedRight {
                            key: child_key,
                            node,
                        });
                    }
                }
            }
        }
        None
    }
}

#[derive(Debug)]
enum DiffIterationResult {
    Added { key: Key, value: Value },
    Deleted { key: Key },
    Changed { key: Key, new_value: Value },
}

/// Key-value stream pair for comparing two merkle trees
struct DiffMerkleKeyValueStreams<'a, T: TrieReader, U: TrieReader> {
    iter1: MerkleKeyValueStream<'a, T>,
    iter2: MerkleKeyValueStream<'a, U>,
    pending1: Option<Result<(Key, Value), crate::v2::api::Error>>,
    pending2: Option<Result<(Key, Value), crate::v2::api::Error>>,
}

impl<'a, T: TrieReader, U: TrieReader> DiffMerkleKeyValueStreams<'a, T, U> {
    const fn new_from_iters(
        iter1: MerkleKeyValueStream<'a, T>,
        iter2: MerkleKeyValueStream<'a, U>,
    ) -> Self {
        Self {
            iter1,
            iter2,
            pending1: None,
            pending2: None,
        }
    }

    /// Get the next key-value pair from tree1's iterator
    fn next_from_tree1(&mut self) -> Option<Result<(Key, Value), crate::v2::api::Error>> {
        if let Some(pending) = self.pending1.take() {
            return Some(pending);
        }
        self.iter1.next()
    }

    /// Get the next key-value pair from tree2's iterator
    fn next_from_tree2(&mut self) -> Option<Result<(Key, Value), crate::v2::api::Error>> {
        if let Some(pending) = self.pending2.take() {
            return Some(pending);
        }
        self.iter2.next()
    }
}

enum DiffIteratorState {
    Unknown,
    Matching,
    SavedLeft(Option<Result<(Key, Value), crate::v2::api::Error>>),
    SavedRight(Option<Result<(Key, Value), crate::v2::api::Error>>),
}

struct DiffIterator<'a, T: TrieReader, U: TrieReader> {
    streams: DiffMerkleKeyValueStreams<'a, T, U>,
    state: DiffIteratorState,
}

fn diff_merkle_iterator<'a, T1: TrieReader, T2: TrieReader>(
    m1: &'a Merkle<T1>,
    m2: &'a Merkle<T2>,
    start_key: Key,
) -> Result<DiffIterator<'a, T1, T2>, storage::FileIoError> {
    let iter1 = m1.key_value_iter_from_key(start_key.clone());
    let iter2 = m2.key_value_iter_from_key(start_key);
    let streams = DiffMerkleKeyValueStreams::new_from_iters(iter1, iter2);

    Ok(DiffIterator {
        streams,
        state: DiffIteratorState::Unknown,
    })
}

impl<T: TrieReader, U: TrieReader> Iterator for DiffIterator<'_, T, U> {
    type Item = BatchOp<Key, Value>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let (left_key, right_key) =
                match std::mem::replace(&mut self.state, DiffIteratorState::Matching) {
                    DiffIteratorState::Matching | DiffIteratorState::Unknown => (
                        self.streams.next_from_tree1(),
                        self.streams.next_from_tree2(),
                    ),
                    DiffIteratorState::SavedLeft(saved) => (saved, self.streams.next_from_tree2()),
                    DiffIteratorState::SavedRight(saved) => (self.streams.next_from_tree1(), saved),
                };

            match (left_key, right_key) {
                (Some(Ok((key1, value1))), Some(Ok((key2, value2)))) => {
                    match key1.cmp(&key2) {
                        Ordering::Equal => {
                            self.state = DiffIteratorState::Matching;
                            if value1 == value2 {
                                continue; // Skip matching values, continue to next iteration
                            } else {
                                return Some(BatchOp::Put {
                                    key: key2,
                                    value: value2,
                                }); // Value changed
                            }
                        }
                        Ordering::Less => {
                            // Save the right key-value for next iteration
                            self.state = DiffIteratorState::SavedRight(Some(Ok((key2, value2))));
                            return Some(BatchOp::Delete { key: key1 });
                        }
                        Ordering::Greater => {
                            // Save the left key-value for next iteration
                            self.state = DiffIteratorState::SavedLeft(Some(Ok((key1, value1))));
                            return Some(BatchOp::Put {
                                key: key2,
                                value: value2,
                            });
                        }
                    }
                }
                (Some(Ok((key, _))), None) => {
                    // Key exists in m1 but not m2 - delete it
                    return Some(BatchOp::Delete { key });
                }
                (None, Some(Ok((key, value)))) => {
                    // Key exists in m2 but not m1 - add it
                    return Some(BatchOp::Put { key, value });
                }
                _ => return None, // Both iterators exhausted or error
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use storage::{ImmutableProposal, MemStore, MutableProposal, NodeStore};

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
    fn test_diff_empty_mutable_trees() {
        // This is unlikely to happen in practice, but it helps cover the case where
        // hashes do not exist yet.
        let m1 = create_test_merkle();
        let m2 = create_test_merkle();

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([])).unwrap();
        assert!(diff_iter.next().is_none());
    }

    #[test]
    fn test_diff_empty_trees() {
        let m1 = make_immutable(create_test_merkle());
        let m2 = make_immutable(create_test_merkle());

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([])).unwrap();
        assert!(diff_iter.next().is_none());
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

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([])).unwrap();
        assert!(diff_iter.next().is_none());
    }

    #[test]
    fn test_diff_additions_only() {
        let items = [
            (b"key1".as_slice(), b"value1".as_slice()),
            (b"key2".as_slice(), b"value2".as_slice()),
        ];

        let m1 = make_immutable(create_test_merkle());
        let m2 = populate_merkle(create_test_merkle(), &items);

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([])).unwrap();

        let op1 = diff_iter.next().unwrap();
        assert!(
            matches!(op1, BatchOp::Put { key, value } if key == Box::from(b"key1".as_slice()) && value == b"value1")
        );

        let op2 = diff_iter.next().unwrap();
        assert!(
            matches!(op2, BatchOp::Put { key, value } if key == Box::from(b"key2".as_slice()) && value == b"value2")
        );

        assert!(diff_iter.next().is_none());
    }

    #[test]
    fn test_diff_deletions_only() {
        let items = [
            (b"key1".as_slice(), b"value1".as_slice()),
            (b"key2".as_slice(), b"value2".as_slice()),
        ];

        let m1 = populate_merkle(create_test_merkle(), &items);
        let m2 = make_immutable(create_test_merkle());

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([])).unwrap();

        let op1 = diff_iter.next().unwrap();
        assert!(matches!(op1, BatchOp::Delete { key } if key == Box::from(b"key1".as_slice())));

        let op2 = diff_iter.next().unwrap();
        assert!(matches!(op2, BatchOp::Delete { key } if key == Box::from(b"key2".as_slice())));

        assert!(diff_iter.next().is_none());
    }

    #[test]
    fn test_diff_modifications() {
        let m1 = populate_merkle(create_test_merkle(), &[(b"key1", b"old_value")]);
        let m2 = populate_merkle(create_test_merkle(), &[(b"key1", b"new_value")]);

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([])).unwrap();

        let op = diff_iter.next().unwrap();
        assert!(
            matches!(op, BatchOp::Put { key, value } if key == Box::from(b"key1".as_slice()) && value == b"new_value")
        );

        assert!(diff_iter.next().is_none());
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

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([])).unwrap();

        let op1 = diff_iter.next().unwrap();
        assert!(matches!(op1, BatchOp::Delete { key } if key == Box::from(b"key1".as_slice())));

        let op2 = diff_iter.next().unwrap();
        assert!(
            matches!(op2, BatchOp::Put { key, value } if key == Box::from(b"key2".as_slice()) && value == b"new_value")
        );

        let op3 = diff_iter.next().unwrap();
        assert!(matches!(op3, BatchOp::Delete { key } if key == Box::from(b"key3".as_slice())));

        let op4 = diff_iter.next().unwrap();
        assert!(
            matches!(op4, BatchOp::Put { key, value } if key == Box::from(b"key4".as_slice()) && value == b"value4")
        );

        assert!(diff_iter.next().is_none());
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
        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::from(b"bbb".as_slice())).unwrap();

        let op1 = diff_iter.next().unwrap();
        assert!(
            matches!(op1, BatchOp::Put { key, value } if key == Box::from(b"bbb".as_slice()) && value == b"modified")
        );

        let op2 = diff_iter.next().unwrap();
        assert!(matches!(op2, BatchOp::Delete { key } if key == Box::from(b"ccc".as_slice())));

        let op3 = diff_iter.next().unwrap();
        assert!(
            matches!(op3, BatchOp::Put { key, value } if key == Box::from(b"ddd".as_slice()) && value == b"value4")
        );

        assert!(diff_iter.next().is_none());
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

        let diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([])).unwrap();

        let ops: Vec<_> = diff_iter.collect();

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

    use test_case::test_case;

    #[test_case(false, false)]
    #[test_case(false, true)]
    #[test_case(true, false)]
    #[test_case(true, true)]
    #[allow(clippy::indexing_slicing)]
    fn diff_random_with_deletions(trie1_mutable: bool, trie2_mutable: bool) {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng, rng};

        // Read FIREWOOD_TEST_SEED from environment or use random seed
        let seed = std::env::var("FIREWOOD_TEST_SEED")
            .ok()
            .map_or_else(
                || None,
                |s| Some(str::parse(&s).expect("couldn't parse FIREWOOD_TEST_SEED; must be a u64")),
            )
            .unwrap_or_else(|| rng().random());

        eprintln!("Seed {seed}: to rerun with this data, export FIREWOOD_TEST_SEED={seed}");
        let mut rng = StdRng::seed_from_u64(seed);

        // Generate 5000 random key-value pairs
        let mut items: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        for _ in 0..5000 {
            let key_len = rng.random_range(1..=32);
            let value_len = rng.random_range(1..=64);

            let key: Vec<u8> = (0..key_len).map(|_| rng.random()).collect();
            let value: Vec<u8> = (0..value_len).map(|_| rng.random()).collect();

            items.push((key, value));
        }

        // Create two identical merkles
        let mut m1 = create_test_merkle();
        let mut m2 = create_test_merkle();

        for (key, value) in &items {
            m1.insert(key, value.clone().into_boxed_slice()).unwrap();
            m2.insert(key, value.clone().into_boxed_slice()).unwrap();
        }

        // Pick two different random indices to delete
        let delete_idx1 = rng.random_range(0..items.len());
        let mut delete_idx2 = rng.random_range(0..items.len());
        while delete_idx2 == delete_idx1 {
            delete_idx2 = rng.random_range(0..items.len());
        }

        let deleted_key1 = &items[delete_idx1].0;
        let deleted_key2 = &items[delete_idx2].0;
        let deleted_value2 = &items[delete_idx2].1;

        // Delete different keys from each merkle
        m1.remove(deleted_key1).unwrap();
        m2.remove(deleted_key2).unwrap();

        // Convert to the appropriate type based on test parameters
        let ops: Vec<BatchOp<Box<[u8]>, Vec<u8>>> = if trie1_mutable && trie2_mutable {
            // Both mutable
            diff_merkle_iterator(&m1, &m2, Box::new([]))
                .unwrap()
                .collect()
        } else if trie1_mutable && !trie2_mutable {
            // m1 mutable, m2 immutable
            let m2_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m2.try_into().unwrap();
            diff_merkle_iterator(&m1, &m2_immut, Box::new([]))
                .unwrap()
                .collect()
        } else if !trie1_mutable && trie2_mutable {
            // m1 immutable, m2 mutable
            let m1_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m1.try_into().unwrap();
            diff_merkle_iterator(&m1_immut, &m2, Box::new([]))
                .unwrap()
                .collect()
        } else {
            // Both immutable
            let m1_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m1.try_into().unwrap();
            let m2_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m2.try_into().unwrap();
            diff_merkle_iterator(&m1_immut, &m2_immut, Box::new([]))
                .unwrap()
                .collect()
        };

        // Should have exactly 2 operations: 1 delete + 1 put
        assert_eq!(ops.len(), 2);

        // Verify the operations are correct - check that we have one delete and one put
        let mut delete_keys: Vec<&[u8]> = Vec::new();
        let mut put_operations: Vec<(&[u8], &[u8])> = Vec::new();

        for op in &ops {
            match op {
                BatchOp::Delete { key } => {
                    delete_keys.push(&**key);
                }
                BatchOp::Put { key, value } => {
                    put_operations.push((&**key, &**value));
                }
                BatchOp::DeleteRange { .. } => {
                    panic!("DeleteRange not expected in this test");
                }
            }
        }

        // Should have exactly one delete and one put
        assert_eq!(
            delete_keys.len(),
            1,
            "Should have exactly one delete operation"
        );
        assert_eq!(
            put_operations.len(),
            1,
            "Should have exactly one put operation"
        );

        // The delete should be for one of our deleted keys, put should be for the other
        let delete_key = delete_keys[0];
        let (put_key, put_value) = put_operations[0];

        // Either deleted_key1 was deleted and deleted_key2 was put, or vice versa
        if delete_key == deleted_key1 {
            assert_eq!(
                put_key, deleted_key2,
                "Put key should match the key deleted from m2"
            );
            assert_eq!(
                put_value, deleted_value2,
                "Put value should match original value"
            );
        } else if delete_key == deleted_key2 {
            assert_eq!(
                put_key, deleted_key1,
                "Put key should match the key deleted from m1"
            );
            assert_eq!(
                put_value, &items[delete_idx1].1,
                "Put value should match original value"
            );
        } else {
            panic!("Delete key doesn't match either of our deleted keys");
        }
    }
}
