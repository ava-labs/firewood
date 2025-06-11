// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

use std::cmp::Ordering;
use std::fmt;
use storage::{Child, FileIoError, Node, Path, SharedNode, TrieReader};

use crate::db::BatchOp;
use crate::merkle::{Key, Merkle, Value};
use crate::stream::{
    IterationNode, NodeStreamState, as_enumerated_children_iter, key_from_nibble_iter,
};

// State structs for different types of diff iteration nodes

/// Two nodes that need to be compared against each other.
/// This state occurs when we have matching children in both trees that need to be processed,
/// or when we're starting the comparison at the root level with nodes from both trees.
///
/// The "unvisited" part means that we haven't actually consumed the value
/// in a branch, or we haven't consumed the value of the leaf.
#[derive(Debug)]
struct UnvisitedPairState {
    node_left: SharedNode,
    node_right: SharedNode,
}

/// A node that exists only in the left tree (tree1) and needs to be processed for deletion.
/// This state occurs when comparing children and the left tree has a child at a position
/// where the right tree doesn't, or when the entire left subtree needs to be deleted.
#[derive(Debug)]
struct UnvisitedLeftState {
    node: SharedNode,
}

/// A node that exists only in the right tree (tree2) and needs to be processed for addition.
/// This state occurs when comparing children and the right tree has a child at a position
/// where the left tree doesn't, or when the entire right subtree needs to be added.
#[derive(Debug)]
struct UnvisitedRightState {
    node: SharedNode,
}

/// Two branch nodes whose children are being compared.
/// This state occurs when we have branch nodes from both trees and we need to iterate
/// through their children to find differences between the subtrees.
struct VisitedPairState {
    children_iter_left: Box<dyn Iterator<Item = (u8, Child)> + Send>,
    children_iter_right: Box<dyn Iterator<Item = (u8, Child)> + Send>,
}

impl fmt::Debug for VisitedPairState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VisitedPairState")
            .field("children_iter_left", &"<iterator>")
            .field("children_iter_right", &"<iterator>")
            .finish()
    }
}

/// A branch node from the left tree only, whose children all need to be processed as deletions.
/// This state occurs when there are no remaining children on the right side to compare against,
/// or when we have a branch node that exists only in the left tree.
struct VisitedLeftState {
    children_iter: Box<dyn Iterator<Item = (u8, Child)> + Send>,
}

impl fmt::Debug for VisitedLeftState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VisitedLeftState")
            .field("children_iter", &"<iterator>")
            .finish()
    }
}

/// A branch node from the right tree only, whose children all need to be processed as additions.
/// This state occurs when there are no remaining children on the left side to compare against,
/// or when we have a branch node that exists only in the right tree.
struct VisitedRightState {
    children_iter: Box<dyn Iterator<Item = (u8, Child)> + Send>,
}

impl fmt::Debug for VisitedRightState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VisitedRightState")
            .field("children_iter", &"<iterator>")
            .finish()
    }
}
trait StateVisitor {
    fn visit<L: TrieReader, R: TrieReader>(
        self,
        key: &Path,
        iter_stack: &mut Vec<DiffIterationNode>,
        readers: (&L, &R),
    ) -> Result<(), FileIoError>;
}

impl StateVisitor for VisitedLeftState {
    fn visit<L: TrieReader, R: TrieReader>(
        mut self,
        key: &Path,
        iter_stack: &mut Vec<DiffIterationNode>,
        readers: (&L, &R),
    ) -> Result<(), FileIoError> {
        if let Some((pos, child)) = self.children_iter.next() {
            let node = match child {
                Child::AddressWithHash(addr, _) => readers.0.read_node(addr)?,
                Child::Node(node) => node.clone().into(),
            };

            let child_partial_path = node.partial_path().iter().copied();
            let child_key: Path = {
                let nibbles: Vec<u8> = key
                    .iter()
                    .copied()
                    .chain(std::iter::once(pos))
                    .chain(child_partial_path)
                    .collect();
                Path::from(nibbles.as_slice())
            };

            // Continue with remaining children
            iter_stack.push(DiffIterationNode {
                key: key.clone(),
                state: DiffIterationNodeState::VisitedLeft(VisitedLeftState {
                    children_iter: self.children_iter,
                }),
            });

            iter_stack.push(DiffIterationNode {
                key: child_key,
                state: DiffIterationNodeState::UnvisitedLeft(UnvisitedLeftState { node }),
            });
        }
        Ok(())
    }
}

impl StateVisitor for VisitedPairState {
    fn visit<L: TrieReader, R: TrieReader>(
        mut self,
        key: &Path,
        iter_stack: &mut Vec<DiffIterationNode>,
        readers: (&L, &R),
    ) -> Result<(), FileIoError> {
        // Compare children from both trees
        let child_left_opt = self.children_iter_left.next();
        let child_right_opt = self.children_iter_right.next();

        match (child_left_opt, child_right_opt) {
            (Some((pos_left, child_left)), Some((pos_right, child_right))) => {
                match pos_left.cmp(&pos_right) {
                    Ordering::Equal => {
                        // Same position - check if subtrees are identical
                        if DiffMerkleNodeStream::<L, R>::child_identical(&child_left, &child_right)
                        {
                            // Identical subtrees, skip them and continue with remaining children
                            iter_stack.push(DiffIterationNode {
                                key: key.clone(),
                                state: DiffIterationNodeState::VisitedPair(VisitedPairState {
                                    children_iter_left: self.children_iter_left,
                                    children_iter_right: self.children_iter_right,
                                }),
                            });
                            return Ok(());
                        }

                        // Different subtrees, need to compare them
                        let node_left = match child_left {
                            Child::AddressWithHash(addr, _) => readers.0.read_node(addr)?,
                            Child::Node(node) => node.clone().into(),
                        };

                        let node_right = match child_right {
                            Child::AddressWithHash(addr, _) => readers.1.read_node(addr)?,
                            Child::Node(node) => node.clone().into(),
                        };

                        let child_partial_path = node_left.partial_path().iter().copied();

                        let child_key: Path = {
                            let nibbles: Vec<u8> = key
                                .iter()
                                .copied()
                                .chain(std::iter::once(pos_left))
                                .chain(child_partial_path)
                                .collect();
                            Path::from(nibbles.as_slice())
                        };

                        // Continue with remaining children
                        iter_stack.push(DiffIterationNode {
                            key: key.clone(),
                            state: DiffIterationNodeState::VisitedPair(VisitedPairState {
                                children_iter_left: self.children_iter_left,
                                children_iter_right: self.children_iter_right,
                            }),
                        });

                        iter_stack.push(DiffIterationNode {
                            key: child_key,
                            state: DiffIterationNodeState::UnvisitedPair(UnvisitedPairState {
                                node_left,
                                node_right,
                            }),
                        });
                    }
                    Ordering::Less => {
                        // pos_left < pos_right: child exists in tree_left but not tree_right
                        let node_left = match child_left {
                            Child::AddressWithHash(addr, _) => readers.0.read_node(addr)?,
                            Child::Node(node) => node.clone().into(),
                        };

                        let child_partial_path = node_left.partial_path().iter().copied();
                        let child_key: Path = {
                            let nibbles: Vec<u8> = key
                                .iter()
                                .copied()
                                .chain(std::iter::once(pos_left))
                                .chain(child_partial_path)
                                .collect();
                            Path::from(nibbles.as_slice())
                        };

                        // Put back child_right for next iteration
                        let new_iter_right = std::iter::once((pos_right, child_right))
                            .chain(self.children_iter_right);
                        iter_stack.push(DiffIterationNode {
                            key: key.clone(),
                            state: DiffIterationNodeState::VisitedPair(VisitedPairState {
                                children_iter_left: self.children_iter_left,
                                children_iter_right: Box::new(new_iter_right),
                            }),
                        });

                        iter_stack.push(DiffIterationNode {
                            key: child_key,
                            state: DiffIterationNodeState::UnvisitedLeft(UnvisitedLeftState {
                                node: node_left,
                            }),
                        });
                    }
                    Ordering::Greater => {
                        // pos_left > pos_right: child exists in tree_right but not tree_left
                        let node_right = match child_right {
                            Child::AddressWithHash(addr, _) => readers.1.read_node(addr)?,
                            Child::Node(node) => node.clone().into(),
                        };

                        let child_partial_path = node_right.partial_path().iter().copied();
                        let child_key: Path = {
                            let nibbles: Vec<u8> = key
                                .iter()
                                .copied()
                                .chain(std::iter::once(pos_right))
                                .chain(child_partial_path)
                                .collect();
                            Path::from(nibbles.as_slice())
                        };

                        // Put back child_left for next iteration
                        let new_iter_left =
                            std::iter::once((pos_left, child_left)).chain(self.children_iter_left);
                        iter_stack.push(DiffIterationNode {
                            key: key.clone(),
                            state: DiffIterationNodeState::VisitedPair(VisitedPairState {
                                children_iter_left: Box::new(new_iter_left),
                                children_iter_right: self.children_iter_right,
                            }),
                        });

                        iter_stack.push(DiffIterationNode {
                            key: child_key,
                            state: DiffIterationNodeState::UnvisitedRight(UnvisitedRightState {
                                node: node_right,
                            }),
                        });
                    }
                }
            }
            (Some((pos_left, child_left)), None) => {
                // Only tree_left has remaining children
                let node_left = match child_left {
                    Child::AddressWithHash(addr, _) => readers.0.read_node(addr)?,
                    Child::Node(node) => node.clone().into(),
                };

                let child_partial_path = node_left.partial_path().iter().copied();
                let child_key: Path = {
                    let nibbles: Vec<u8> = key
                        .iter()
                        .copied()
                        .chain(std::iter::once(pos_left))
                        .chain(child_partial_path)
                        .collect();
                    Path::from(nibbles.as_slice())
                };

                // Continue with remaining children from tree_left
                iter_stack.push(DiffIterationNode {
                    key: key.clone(),
                    state: DiffIterationNodeState::VisitedLeft(VisitedLeftState {
                        children_iter: self.children_iter_left,
                    }),
                });

                iter_stack.push(DiffIterationNode {
                    key: child_key,
                    state: DiffIterationNodeState::UnvisitedLeft(UnvisitedLeftState {
                        node: node_left,
                    }),
                });
            }
            (None, Some((pos_right, child_right))) => {
                // Only tree_right has remaining children
                let node_right = match child_right {
                    Child::AddressWithHash(addr, _) => readers.1.read_node(addr)?,
                    Child::Node(node) => node.clone().into(),
                };

                let child_partial_path = node_right.partial_path().iter().copied();
                let child_key: Path = {
                    let nibbles: Vec<u8> = key
                        .iter()
                        .copied()
                        .chain(std::iter::once(pos_right))
                        .chain(child_partial_path)
                        .collect();
                    Path::from(nibbles.as_slice())
                };

                // Continue with remaining children from tree_right
                iter_stack.push(DiffIterationNode {
                    key: key.clone(),
                    state: DiffIterationNodeState::VisitedRight(VisitedRightState {
                        children_iter: self.children_iter_right,
                    }),
                });

                iter_stack.push(DiffIterationNode {
                    key: child_key,
                    state: DiffIterationNodeState::UnvisitedRight(UnvisitedRightState {
                        node: node_right,
                    }),
                });
            }
            (None, None) => {
                // No more children in either tree, continue
            }
        }
        Ok(())
    }
}

impl StateVisitor for VisitedRightState {
    fn visit<L: TrieReader, R: TrieReader>(
        mut self,
        key: &Path,
        iter_stack: &mut Vec<DiffIterationNode>,
        readers: (&L, &R),
    ) -> Result<(), FileIoError> {
        if let Some((pos, child)) = self.children_iter.next() {
            let node = match child {
                Child::AddressWithHash(addr, _) => readers.1.read_node(addr)?,
                Child::Node(node) => node.clone().into(),
            };

            let child_partial_path = node.partial_path().iter().copied();
            let child_key: Path = {
                let nibbles: Vec<u8> = key
                    .iter()
                    .copied()
                    .chain(std::iter::once(pos))
                    .chain(child_partial_path)
                    .collect();
                Path::from(nibbles.as_slice())
            };

            // Continue with remaining children
            iter_stack.push(DiffIterationNode {
                key: key.clone(),
                state: DiffIterationNodeState::VisitedRight(VisitedRightState {
                    children_iter: self.children_iter,
                }),
            });

            iter_stack.push(DiffIterationNode {
                key: child_key,
                state: DiffIterationNodeState::UnvisitedRight(UnvisitedRightState { node }),
            });
        }
        Ok(())
    }
}

/// Enum containing all possible states for a diff iteration node
#[derive(Debug)]
enum DiffIterationNodeState {
    /// Two unvisited nodes that should be compared
    UnvisitedPair(UnvisitedPairState),
    /// A node that exists only in tree1 (needs to be deleted)
    UnvisitedLeft(UnvisitedLeftState),
    /// A node that exists only in tree2 (needs to be added)
    UnvisitedRight(UnvisitedRightState),
    /// A pair of visited branch nodes - track which children to compare next
    VisitedPair(VisitedPairState),
    /// A visited branch node from tree1 only (all its children need to be deleted)
    VisitedLeft(VisitedLeftState),
    /// A visited branch node from tree2 only (all its children need to be added)
    VisitedRight(VisitedRightState),
}

/// Iteration node that tracks state for both trees simultaneously
struct DiffIterationNode {
    key: Path,
    state: DiffIterationNodeState,
}

impl fmt::Debug for DiffIterationNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.state {
            DiffIterationNodeState::UnvisitedPair(ref state) => f
                .debug_struct("UnvisitedPair")
                .field("key", &self.key)
                .field("node_left", &state.node_left)
                .field("node_right", &state.node_right)
                .finish(),
            DiffIterationNodeState::UnvisitedLeft(ref state) => f
                .debug_struct("UnvisitedLeft")
                .field("key", &self.key)
                .field("node", &state.node)
                .finish(),
            DiffIterationNodeState::UnvisitedRight(ref state) => f
                .debug_struct("UnvisitedRight")
                .field("key", &self.key)
                .field("node", &state.node)
                .finish(),
            DiffIterationNodeState::VisitedPair(_) => f
                .debug_struct("VisitedPair")
                .field("key", &self.key)
                .field("children_iter_left", &"<iterator>")
                .field("children_iter_right", &"<iterator>")
                .finish(),
            DiffIterationNodeState::VisitedLeft(_) => f
                .debug_struct("VisitedLeft")
                .field("key", &self.key)
                .field("children_iter", &"<iterator>")
                .finish(),
            DiffIterationNodeState::VisitedRight(_) => f
                .debug_struct("VisitedRight")
                .field("key", &self.key)
                .field("children_iter", &"<iterator>")
                .finish(),
        }
    }
}

impl DiffIterationNode {
    /// Convert an `IterationNode` to a `DiffIterationNode` for left tree operations (deletions).
    fn from_left(node: IterationNode) -> Self {
        match node {
            IterationNode::Unvisited { key, node } => {
                let path = Path::from(key.as_ref());
                DiffIterationNode {
                    key: path,
                    state: DiffIterationNodeState::UnvisitedLeft(UnvisitedLeftState { node }),
                }
            }
            IterationNode::Visited { key, children_iter } => {
                let path = Path::from(key.as_ref());
                DiffIterationNode {
                    key: path,
                    state: DiffIterationNodeState::VisitedLeft(VisitedLeftState { children_iter }),
                }
            }
        }
    }

    /// Convert an `IterationNode` to a `DiffIterationNode` for right tree operations (additions).
    fn from_right(node: IterationNode) -> Self {
        match node {
            IterationNode::Unvisited { key, node } => {
                let path = Path::from(key.as_ref());
                DiffIterationNode {
                    key: path,
                    state: DiffIterationNodeState::UnvisitedRight(UnvisitedRightState { node }),
                }
            }
            IterationNode::Visited { key, children_iter } => {
                let path = Path::from(key.as_ref());
                DiffIterationNode {
                    key: path,
                    state: DiffIterationNodeState::VisitedRight(VisitedRightState {
                        children_iter,
                    }),
                }
            }
        }
    }
}

/// State for the diff iterator that tracks lazy initialization
#[derive(Debug)]
enum DiffNodeStreamState {
    /// The iterator state is lazily initialized when next_internal is called
    /// for the first time. The iteration start key is stored here.
    StartFromKey(Key),
    /// The iterator is actively iterating over nodes
    Iterating { iter_stack: Vec<DiffIterationNode> },
}

impl From<Key> for DiffNodeStreamState {
    fn from(key: Key) -> Self {
        Self::StartFromKey(key)
    }
}

/// Optimized node iterator that compares two merkle trees and skips matching subtrees
#[derive(Debug)]
struct DiffMerkleNodeStream<'a, T: TrieReader, U: TrieReader> {
    tree_left: &'a T,
    tree_right: &'a U,
    state: DiffNodeStreamState,
}

impl<'a, T: TrieReader, U: TrieReader> DiffMerkleNodeStream<'a, T, U> {
    fn new(tree_left: &'a T, tree_right: &'a U, start_key: Key) -> Self {
        Self {
            tree_left,
            tree_right,
            state: DiffNodeStreamState::from(start_key),
        }
    }

    /// Check if two children have the same hash or refer to the same node.
    ///
    /// This is used to determine if two subtrees are identical.
    ///
    /// For mutable trees, we can compare node addresses to detect identical subtrees
    /// This works because identical subtrees will share the same underlying Node reference
    /// For immutable trees, we can't know if they are the same, assume not
    fn child_identical(child_left: &Child, child_right: &Child) -> bool {
        match (child_left, child_right) {
            (Child::AddressWithHash(_, hash_left), Child::AddressWithHash(_, hash_right)) => {
                hash_left == hash_right
            }
            (Child::Node(node_left), Child::Node(node_right)) => {
                // For mutable trees, we can compare node addresses to detect identical subtrees
                // This works because identical subtrees will share the same underlying Node reference
                std::ptr::eq(node_left, node_right)
            }
            // Different child types so we can't know if they are the same, assume not
            _ => false,
        }
    }

    /// Returns the initial state for a diff iterator over the given trees which starts at `key`.
    fn get_diff_iterator_initial_state(
        tree_left: &T,
        tree_right: &U,
        key: &[u8],
    ) -> Result<DiffNodeStreamState, storage::FileIoError> {
        let root_left_opt = tree_left.root_node();
        let root_right_opt = tree_right.root_node();

        match (root_left_opt, root_right_opt) {
            (None, None) => {
                // Both trees are empty
                Ok(DiffNodeStreamState::Iterating { iter_stack: vec![] })
            }
            (Some(_root_left), None) => {
                // Only tree_left has content - use single tree traversal for deletions
                let initial_state = NodeStreamState::get_iterator_initial_state(tree_left, key)?;
                let iter_stack = match initial_state {
                    NodeStreamState::StartFromKey(_) => vec![], // Should not happen
                    NodeStreamState::Iterating { iter_stack } => {
                        // Convert IterationNode to DiffIterationNode for left tree operations
                        iter_stack
                            .into_iter()
                            .map(DiffIterationNode::from_left)
                            .collect()
                    }
                };
                Ok(DiffNodeStreamState::Iterating { iter_stack })
            }
            (None, Some(_root_right)) => {
                // Only tree_right has content - use single tree traversal for additions
                let initial_state = NodeStreamState::get_iterator_initial_state(tree_right, key)?;
                let iter_stack = match initial_state {
                    NodeStreamState::StartFromKey(_) => vec![], // Should not happen
                    NodeStreamState::Iterating { iter_stack } => {
                        // Convert IterationNode to DiffIterationNode for right tree operations
                        iter_stack
                            .into_iter()
                            .map(DiffIterationNode::from_right)
                            .collect()
                    }
                };
                Ok(DiffNodeStreamState::Iterating { iter_stack })
            }
            (Some(root_left), Some(root_right)) => {
                // Both trees have content - need to compare them
                // For now, just start from the roots and let the hash optimization
                // skip identical subtrees during iteration
                let root_left_partial = root_left.partial_path().iter().copied();
                let root_key: Path = Path::from_nibbles_iterator(root_left_partial);
                Ok(DiffNodeStreamState::Iterating {
                    iter_stack: vec![DiffIterationNode {
                        key: root_key,
                        state: DiffIterationNodeState::UnvisitedPair(UnvisitedPairState {
                            node_left: root_left,
                            node_right: root_right,
                        }),
                    }],
                })
            }
        }
    }

    fn next_internal(&mut self) -> Option<Result<DiffIterationResult, storage::FileIoError>> {
        // Handle lazy initialization
        let iter_stack = match &mut self.state {
            DiffNodeStreamState::StartFromKey(key) => {
                match Self::get_diff_iterator_initial_state(self.tree_left, self.tree_right, key) {
                    Ok(new_state) => {
                        self.state = new_state;
                        return self.next_internal();
                    }
                    Err(e) => return Some(Err(e)),
                }
            }
            DiffNodeStreamState::Iterating { iter_stack } => iter_stack,
        };

        // We remove the most recent DiffIterationNode, but in some cases we will push it back onto the stack.
        while let Some(iter_node) = iter_stack.pop() {
            match iter_node.state {
                DiffIterationNodeState::UnvisitedPair(ref state) => {
                    match (&*state.node_left, &*state.node_right) {
                        (Node::Branch(branch_left), Node::Branch(branch_right)) => {
                            // Compare values first
                            let value_diff = match (&branch_left.value, &branch_right.value) {
                                (Some(v_left), Some(v_right)) if v_left == v_right => None,
                                (Some(_), Some(v_right)) => {
                                    // The key already includes the complete path including partial paths
                                    Some(DiffIterationResult::Changed {
                                        key: key_from_nibble_iter(iter_node.key.iter().copied()),
                                        new_value: v_right.to_vec(),
                                    })
                                }
                                (Some(_), None) => {
                                    // The key already includes the complete path including partial paths
                                    Some(DiffIterationResult::Deleted {
                                        key: key_from_nibble_iter(iter_node.key.iter().copied()),
                                    })
                                }
                                (None, Some(v_right)) => {
                                    // The key already includes the complete path including partial paths
                                    Some(DiffIterationResult::Added {
                                        key: key_from_nibble_iter(iter_node.key.iter().copied()),
                                        value: v_right.to_vec(),
                                    })
                                }
                                (None, None) => None,
                            };

                            // Set up to compare children
                            iter_stack.push(DiffIterationNode {
                                key: iter_node.key.clone(),
                                state: DiffIterationNodeState::VisitedPair(VisitedPairState {
                                    children_iter_left: Box::new(as_enumerated_children_iter(
                                        branch_left,
                                    )),
                                    children_iter_right: Box::new(as_enumerated_children_iter(
                                        branch_right,
                                    )),
                                }),
                            });

                            if let Some(result) = value_diff {
                                return Some(Ok(result));
                            }
                        }
                        (Node::Branch(branch), Node::Leaf(leaf)) => {
                            // Branch vs Leaf - need to process all branch children as deletions
                            // and add the leaf
                            iter_stack.push(DiffIterationNode {
                                key: iter_node.key.clone(),
                                state: DiffIterationNodeState::VisitedLeft(VisitedLeftState {
                                    children_iter: Box::new(as_enumerated_children_iter(branch)),
                                }),
                            });

                            // The key already includes the complete path including partial paths
                            return Some(Ok(DiffIterationResult::Added {
                                key: key_from_nibble_iter(iter_node.key.iter().copied()),
                                value: leaf.value.to_vec(),
                            }));
                        }
                        (Node::Leaf(_leaf), Node::Branch(branch)) => {
                            // Leaf vs Branch - delete leaf, process all branch children as additions
                            iter_stack.push(DiffIterationNode {
                                key: iter_node.key.clone(),
                                state: DiffIterationNodeState::VisitedRight(VisitedRightState {
                                    children_iter: Box::new(as_enumerated_children_iter(branch)),
                                }),
                            });

                            // The key already includes the complete path including partial paths
                            return Some(Ok(DiffIterationResult::Deleted {
                                key: key_from_nibble_iter(iter_node.key.iter().copied()),
                            }));
                        }
                        (Node::Leaf(leaf1), Node::Leaf(leaf2)) => {
                            // Two leaves - compare values
                            if leaf1.value != leaf2.value {
                                // The key already includes the complete path including partial paths
                                return Some(Ok(DiffIterationResult::Changed {
                                    key: key_from_nibble_iter(iter_node.key.iter().copied()),
                                    new_value: leaf2.value.to_vec(),
                                }));
                            }
                        }
                    }
                }
                DiffIterationNodeState::UnvisitedLeft(ref state) => {
                    // Node exists only in tree1 - mark for deletion
                    match &*state.node {
                        Node::Branch(branch) => {
                            iter_stack.push(DiffIterationNode {
                                key: iter_node.key.clone(),
                                state: DiffIterationNodeState::VisitedLeft(VisitedLeftState {
                                    children_iter: Box::new(as_enumerated_children_iter(branch)),
                                }),
                            });

                            if branch.value.is_some() {
                                // The key already includes the complete path including partial paths
                                let key = key_from_nibble_iter(iter_node.key.iter().copied());
                                return Some(Ok(DiffIterationResult::Deleted { key }));
                            }
                        }
                        Node::Leaf(_leaf) => {
                            // The key already includes the complete path including partial paths
                            let key = key_from_nibble_iter(iter_node.key.iter().copied());
                            return Some(Ok(DiffIterationResult::Deleted { key }));
                        }
                    }
                }
                DiffIterationNodeState::UnvisitedRight(ref state) => {
                    // Node exists only in tree2 - mark for addition
                    match &*state.node {
                        Node::Branch(branch) => {
                            iter_stack.push(DiffIterationNode {
                                key: iter_node.key.clone(),
                                state: DiffIterationNodeState::VisitedRight(VisitedRightState {
                                    children_iter: Box::new(as_enumerated_children_iter(branch)),
                                }),
                            });

                            if let Some(value) = &branch.value {
                                // The key already includes the complete path including partial paths
                                let key = key_from_nibble_iter(iter_node.key.iter().copied());
                                return Some(Ok(DiffIterationResult::Added {
                                    key,
                                    value: value.to_vec(),
                                }));
                            }
                        }
                        Node::Leaf(_leaf) => {
                            // The key already includes the complete path including partial paths
                            let key = key_from_nibble_iter(iter_node.key.iter().copied());
                            return Some(Ok(DiffIterationResult::Added {
                                key,
                                value: _leaf.value.to_vec(),
                            }));
                        }
                    }
                }
                DiffIterationNodeState::VisitedPair(state) => {
                    if let Err(e) = state.visit(
                        &iter_node.key,
                        iter_stack,
                        (self.tree_left, self.tree_right),
                    ) {
                        return Some(Err(e));
                    }
                }
                DiffIterationNodeState::VisitedLeft(state) => {
                    if let Err(e) = state.visit(
                        &iter_node.key,
                        iter_stack,
                        (self.tree_left, self.tree_right),
                    ) {
                        return Some(Err(e));
                    }
                }
                DiffIterationNodeState::VisitedRight(state) => {
                    if let Err(e) = state.visit(
                        &iter_node.key,
                        iter_stack,
                        (self.tree_left, self.tree_right),
                    ) {
                        return Some(Err(e));
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

/// Optimized diff stream that uses DiffMerkleNodeStream for hash-based optimizations
struct DiffMerkleKeyValueStreams<'a, T: TrieReader, U: TrieReader> {
    node_stream: DiffMerkleNodeStream<'a, T, U>,
}

/// Iterator that compares two merkle trees by streaming their key-value pairs
/// and yielding differences as DiffIterationResult items.
#[derive(Debug)]
struct DiffKeyValueStreams<'a, T: TrieReader, U: TrieReader> {
    iter_left: crate::stream::MerkleKeyValueStream<'a, T>,
    iter_right: crate::stream::MerkleKeyValueStream<'a, U>,
    next_left: Option<Result<(Key, Value), storage::FileIoError>>,
    next_right: Option<Result<(Key, Value), storage::FileIoError>>,
}

impl<'a, T: TrieReader, U: TrieReader> DiffKeyValueStreams<'a, T, U> {
    fn new(
        m1: &'a crate::merkle::Merkle<T>,
        m2: &'a crate::merkle::Merkle<U>,
        start_key: Key,
    ) -> Self {
        let mut iter_left = if start_key.is_empty() {
            m1.key_value_iter()
        } else {
            m1.key_value_iter_from_key(start_key.clone())
        };

        let mut iter_right = if start_key.is_empty() {
            m2.key_value_iter()
        } else {
            m2.key_value_iter_from_key(start_key)
        };

        // Get the first item from each iterator
        let next_left = iter_left.next_internal();
        let next_right = iter_right.next_internal();

        Self {
            iter_left,
            iter_right,
            next_left,
            next_right,
        }
    }

    /// Implements a merge-sort-like algorithm to compare key-value pairs from two merkle trees.
    ///
    /// The algorithm maintains the current key-value pair from each tree (next1, next2) and compares them:
    /// - If keys are equal but values differ: yields a Change operation
    /// - If keys are equal and values match: advances both iterators (no diff needed)  
    /// - If key1 < key2: key1 exists only in tree1, yields a Delete operation
    /// - If key1 > key2: key2 exists only in tree2, yields an Add operation
    /// - If one iterator is exhausted: remaining items from the other tree are all deletions or additions
    ///
    /// This approach ensures all differences are detected in a single pass through both trees.
    fn next(&mut self) -> Option<Result<DiffIterationResult, storage::FileIoError>> {
        loop {
            match (&self.next_left, &self.next_right) {
                (Some(Ok((key_left, value_left))), Some(Ok((key_right, value_right)))) => {
                    match key_left.cmp(key_right) {
                        std::cmp::Ordering::Equal => {
                            // Same key in both trees
                            if value_left != value_right {
                                // Values differ - generate a change operation
                                let result = DiffIterationResult::Changed {
                                    key: key_right.clone(),
                                    new_value: value_right.clone(),
                                };
                                // Advance both iterators
                                self.next_left = self.iter_left.next_internal();
                                self.next_right = self.iter_right.next_internal();
                                return Some(Ok(result));
                            } else {
                                // Values are identical - advance both iterators and continue
                                self.next_left = self.iter_left.next_internal();
                                self.next_right = self.iter_right.next_internal();
                                // Continue the loop instead of recursing
                                continue;
                            }
                        }
                        std::cmp::Ordering::Less => {
                            // key_left < key_right: key_left exists only in tree1 (deletion)
                            let result = DiffIterationResult::Deleted {
                                key: key_left.clone(),
                            };
                            // Advance iterator_left
                            self.next_left = self.iter_left.next_internal();
                            return Some(Ok(result));
                        }
                        std::cmp::Ordering::Greater => {
                            // key_left > key_right: key_right exists only in tree2 (addition)
                            let result = DiffIterationResult::Added {
                                key: key_right.clone(),
                                value: value_right.clone(),
                            };
                            // Advance iterator_right
                            self.next_right = self.iter_right.next_internal();
                            return Some(Ok(result));
                        }
                    }
                }
                (Some(Ok((key_left, _value_left))), None) => {
                    // Only tree1 has remaining items (all deletions)
                    let result = DiffIterationResult::Deleted {
                        key: key_left.clone(),
                    };
                    self.next_left = self.iter_left.next_internal();
                    return Some(Ok(result));
                }
                (None, Some(Ok((key_right, value_right)))) => {
                    // Only tree2 has remaining items (all additions)
                    let result = DiffIterationResult::Added {
                        key: key_right.clone(),
                        value: value_right.clone(),
                    };
                    self.next_right = self.iter_right.next_internal();
                    return Some(Ok(result));
                }
                (Some(Err(_)), _) => {
                    // Return the error from the first iterator
                    if let Some(Err(error)) = self.next_left.take() {
                        return Some(Err(error));
                    }
                }
                (_, Some(Err(_))) => {
                    // Return the error from the second iterator
                    if let Some(Err(error)) = self.next_right.take() {
                        return Some(Err(error));
                    }
                }
                (None, None) => {
                    // Both iterators are exhausted
                    return None;
                }
            }
        }
    }
}

impl<T: TrieReader, U: TrieReader> fmt::Debug for DiffMerkleKeyValueStreams<'_, T, U> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiffMerkleKeyValueStreams")
            .field("node_stream", &"<DiffMerkleNodeStream>")
            .finish()
    }
}

impl<'a, T: TrieReader, U: TrieReader> DiffMerkleKeyValueStreams<'a, T, U> {
    fn new(tree_left: &'a T, tree_right: &'a U, start_key: Key) -> Self {
        Self {
            node_stream: DiffMerkleNodeStream::new(tree_left, tree_right, start_key),
        }
    }
}

impl<T: TrieReader, U: TrieReader> Iterator for DiffMerkleKeyValueStreams<'_, T, U> {
    type Item = Result<DiffIterationResult, storage::FileIoError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.node_stream.next_internal()
    }
}

#[derive(Debug)]
struct DiffIterator<'a, T: TrieReader, U: TrieReader> {
    streams: DiffKeyValueStreams<'a, T, U>,
}

fn diff_merkle_iterator<'a, T1: TrieReader, T2: TrieReader>(
    m1: &'a Merkle<T1>,
    m2: &'a Merkle<T2>,
    start_key: Key,
) -> DiffIterator<'a, T1, T2> {
    // For now, ignore the complex node-level optimization and use a simple
    // key-value level approach that won't pick up internal trie restructuring
    let streams: DiffKeyValueStreams<'a, T1, T2> = DiffKeyValueStreams::new(m1, m2, start_key);

    DiffIterator { streams }
}

impl<T: TrieReader, U: TrieReader> Iterator for DiffIterator<'_, T, U> {
    type Item = Result<BatchOp<Key, Value>, storage::FileIoError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.streams.next() {
            Some(Ok(diff_result)) => {
                // Convert DiffIterationResult to BatchOp
                let batch_op = match diff_result {
                    DiffIterationResult::Added { key, value } => BatchOp::Put { key, value },
                    DiffIterationResult::Deleted { key } => BatchOp::Delete { key },
                    DiffIterationResult::Changed { key, new_value } => BatchOp::Put {
                        key,
                        value: new_value,
                    },
                };
                Some(Ok(batch_op))
            }
            Some(Err(error)) => {
                // Return the error instead of skipping it
                Some(Err(error))
            }
            None => None, // Iterator exhausted
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

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([]));
        assert!(diff_iter.next().is_none());
    }

    #[test]
    fn test_diff_empty_trees() {
        let m1 = make_immutable(create_test_merkle());
        let m2 = make_immutable(create_test_merkle());

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([]));
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

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([]));
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

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([]));

        let op1 = diff_iter.next().unwrap().unwrap();
        assert!(
            matches!(op1, BatchOp::Put { key, value } if key == Box::from(b"key1".as_slice()) && value == b"value1")
        );

        let op2 = diff_iter.next().unwrap().unwrap();
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

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([]));

        let op1 = diff_iter.next().unwrap().unwrap();
        assert!(matches!(op1, BatchOp::Delete { key } if key == Box::from(b"key1".as_slice())));

        let op2 = diff_iter.next().unwrap().unwrap();
        assert!(matches!(op2, BatchOp::Delete { key } if key == Box::from(b"key2".as_slice())));

        assert!(diff_iter.next().is_none());
    }

    #[test]
    fn test_diff_modifications() {
        let m1 = populate_merkle(create_test_merkle(), &[(b"key1", b"old_value")]);
        let m2 = populate_merkle(create_test_merkle(), &[(b"key1", b"new_value")]);

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([]));

        let op = diff_iter.next().unwrap().unwrap();
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

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([]));

        let op1 = diff_iter.next().unwrap().unwrap();
        assert!(matches!(op1, BatchOp::Delete { key } if key == Box::from(b"key1".as_slice())));

        let op2 = diff_iter.next().unwrap().unwrap();
        assert!(
            matches!(op2, BatchOp::Put { key, value } if key == Box::from(b"key2".as_slice()) && value == b"new_value")
        );

        let op3 = diff_iter.next().unwrap().unwrap();
        assert!(matches!(op3, BatchOp::Delete { key } if key == Box::from(b"key3".as_slice())));

        let op4 = diff_iter.next().unwrap().unwrap();
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
        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::from(b"bbb".as_slice()));

        let op1 = diff_iter.next().unwrap().unwrap();
        assert!(
            matches!(op1, BatchOp::Put { ref key, ref value } if **key == *b"bbb" && **value == *b"modified"),
            "Expected first operation to be Put bbb=modified, got: {op1:?}",
        );

        let op2 = diff_iter.next().unwrap().unwrap();
        assert!(matches!(op2, BatchOp::Delete { key } if key == Box::from(b"ccc".as_slice())));

        let op3 = diff_iter.next().unwrap().unwrap();
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

        let diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([]));

        let ops: Vec<_> = diff_iter.collect::<Result<Vec<_>, _>>().unwrap();

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

    #[test_case(false, false, 5000)]
    #[test_case(false, true, 5000)]
    #[test_case(true, false, 5000)]
    #[test_case(true, true, 5000)]
    #[allow(clippy::indexing_slicing)]
    fn diff_random_with_deletions(trie1_mutable: bool, trie2_mutable: bool, num_items: usize) {
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

        // Generate random key-value pairs
        let mut items: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        for _ in 0..num_items {
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

        // Get the actual values from the trees before deletion (handles duplicate keys correctly)
        let actual_value1 = m1.get_value(deleted_key1).unwrap().unwrap();
        let actual_value2 = m2.get_value(deleted_key2).unwrap().unwrap();

        // Delete different keys from each merkle
        m1.remove(deleted_key1).unwrap();
        m2.remove(deleted_key2).unwrap();

        // Convert to the appropriate type based on test parameters
        let ops: Vec<BatchOp<Box<[u8]>, Vec<u8>>> = if trie1_mutable && trie2_mutable {
            // Both mutable
            diff_merkle_iterator(&m1, &m2, Box::new([]))
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        } else if trie1_mutable && !trie2_mutable {
            // m1 mutable, m2 immutable
            let m2_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m2.try_into().unwrap();
            diff_merkle_iterator(&m1, &m2_immut, Box::new([]))
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        } else if !trie1_mutable && trie2_mutable {
            // m1 immutable, m2 mutable
            let m1_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m1.try_into().unwrap();
            diff_merkle_iterator(&m1_immut, &m2, Box::new([]))
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        } else {
            // Both immutable
            let m1_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m1.try_into().unwrap();
            let m2_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m2.try_into().unwrap();
            diff_merkle_iterator(&m1_immut, &m2_immut, Box::new([]))
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
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
                put_value,
                actual_value2.as_ref(),
                "Put value should match original value. Expected: {:?}, Got: {:?}",
                actual_value2,
                put_value
            );
        } else if delete_key == deleted_key2 {
            assert_eq!(
                put_key, deleted_key1,
                "Put key should match the key deleted from m1"
            );

            assert_eq!(
                put_value,
                actual_value1.as_ref(),
                "Put value should match original value. Expected: {:?}, Got: {:?}",
                actual_value1,
                put_value
            );
        } else {
            panic!("Delete key doesn't match either of our deleted keys");
        }
    }

    #[test]
    fn test_hash_optimization_reduces_node_reads() {
        // This test focuses specifically on validating that hash optimization reduces node reads
        // by comparing baseline full-traversal reads vs optimized diff operation reads.
        // Diff correctness is validated by other tests.
        use metrics::{Key, Label, Recorder};
        use metrics_util::registry::{AtomicStorage, Registry};
        use std::sync::Arc;
        use std::sync::atomic::Ordering;

        /// Test metrics recorder that captures counter values for testing
        #[derive(Debug, Clone)]
        struct TestRecorder {
            registry: Arc<Registry<Key, AtomicStorage>>,
        }

        impl TestRecorder {
            fn new() -> Self {
                Self {
                    registry: Arc::new(Registry::atomic()),
                }
            }

            fn get_counter_value(
                &self,
                key_name: &'static str,
                labels: &[(&'static str, &'static str)],
            ) -> u64 {
                let key = if labels.is_empty() {
                    Key::from_name(key_name)
                } else {
                    let label_vec: Vec<Label> =
                        labels.iter().map(|(k, v)| Label::new(*k, *v)).collect();
                    Key::from_name(key_name).with_extra_labels(label_vec)
                };

                self.registry
                    .get_counter_handles()
                    .into_iter()
                    .find(|(k, _)| k == &key)
                    .map(|(_, counter)| counter.load(Ordering::Relaxed))
                    .unwrap_or(0)
            }
        }

        impl Recorder for TestRecorder {
            fn describe_counter(
                &self,
                _key: metrics::KeyName,
                _unit: Option<metrics::Unit>,
                _description: metrics::SharedString,
            ) {
            }
            fn describe_gauge(
                &self,
                _key: metrics::KeyName,
                _unit: Option<metrics::Unit>,
                _description: metrics::SharedString,
            ) {
            }
            fn describe_histogram(
                &self,
                _key: metrics::KeyName,
                _unit: Option<metrics::Unit>,
                _description: metrics::SharedString,
            ) {
            }

            fn register_counter(
                &self,
                key: &Key,
                _metadata: &metrics::Metadata<'_>,
            ) -> metrics::Counter {
                self.registry
                    .get_or_create_counter(key, |c| c.clone().into())
            }

            fn register_gauge(
                &self,
                key: &Key,
                _metadata: &metrics::Metadata<'_>,
            ) -> metrics::Gauge {
                self.registry.get_or_create_gauge(key, |c| c.clone().into())
            }

            fn register_histogram(
                &self,
                key: &Key,
                _metadata: &metrics::Metadata<'_>,
            ) -> metrics::Histogram {
                self.registry
                    .get_or_create_histogram(key, |c| c.clone().into())
            }
        }

        // Set up test recorder - if it fails, skip the entire test
        let recorder = TestRecorder::new();
        if metrics::set_global_recorder(recorder.clone()).is_err() {
            println!("⚠️  Could not set test recorder (already set) - skipping test");
            return;
        }

        // Create test data with substantial shared content and unique content
        let tree1_items = [
            // Large shared content that will form identical subtrees
            (
                b"shared/branch_a/deep/file1".as_slice(),
                b"shared_value1".as_slice(),
            ),
            (
                b"shared/branch_a/deep/file2".as_slice(),
                b"shared_value2".as_slice(),
            ),
            (
                b"shared/branch_a/deep/file3".as_slice(),
                b"shared_value3".as_slice(),
            ),
            (b"shared/branch_b/file1".as_slice(), b"shared_b1".as_slice()),
            (b"shared/branch_b/file2".as_slice(), b"shared_b2".as_slice()),
            (
                b"shared/branch_c/deep/nested/file".as_slice(),
                b"shared_nested".as_slice(),
            ),
            (b"shared/common".as_slice(), b"common_value".as_slice()),
            // Unique to tree1
            (b"tree1_unique/x".as_slice(), b"x_value".as_slice()),
            (b"tree1_unique/y".as_slice(), b"y_value".as_slice()),
            (b"tree1_unique/z".as_slice(), b"z_value".as_slice()),
        ];

        let tree2_items = [
            // Identical shared content
            (
                b"shared/branch_a/deep/file1".as_slice(),
                b"shared_value1".as_slice(),
            ),
            (
                b"shared/branch_a/deep/file2".as_slice(),
                b"shared_value2".as_slice(),
            ),
            (
                b"shared/branch_a/deep/file3".as_slice(),
                b"shared_value3".as_slice(),
            ),
            (b"shared/branch_b/file1".as_slice(), b"shared_b1".as_slice()),
            (b"shared/branch_b/file2".as_slice(), b"shared_b2".as_slice()),
            (
                b"shared/branch_c/deep/nested/file".as_slice(),
                b"shared_nested".as_slice(),
            ),
            (b"shared/common".as_slice(), b"common_value".as_slice()),
            // Unique to tree2
            (b"tree2_unique/p".as_slice(), b"p_value".as_slice()),
            (b"tree2_unique/q".as_slice(), b"q_value".as_slice()),
            (b"tree2_unique/r".as_slice(), b"r_value".as_slice()),
        ];

        // Create immutable trees (required for hash-based optimization)
        let m1 = populate_merkle(create_test_merkle(), &tree1_items);
        let m2 = populate_merkle(create_test_merkle(), &tree2_items);

        // BASELINE: Measure total reads from complete tree traversals
        let baseline_reads_before =
            recorder.get_counter_value("firewood.read_node", &[("from", "proposal")]);

        // Traverse tree1 completely
        let tree1_iter = m1.key_value_iter();
        let tree1_count = tree1_iter.count();

        // Traverse tree2 completely
        let tree2_iter = m2.key_value_iter();
        let tree2_count = tree2_iter.count();

        let baseline_reads_after =
            recorder.get_counter_value("firewood.read_node", &[("from", "proposal")]);
        let baseline_reads = baseline_reads_after - baseline_reads_before;

        println!(
            "Baseline - Tree1 items: {}, Tree2 items: {}",
            tree1_count, tree2_count
        );
        println!(
            "Baseline total reads (both trees fully traversed): {}",
            baseline_reads
        );

        // DIFF TEST: Measure reads from hash-optimized diff operation
        let diff_reads_before =
            recorder.get_counter_value("firewood.read_node", &[("from", "proposal")]);

        let diff_stream =
            DiffMerkleKeyValueStreams::new(m1.nodestore(), m2.nodestore(), Box::new([]));
        let diff_results_count = diff_stream.count();

        let diff_reads_after =
            recorder.get_counter_value("firewood.read_node", &[("from", "proposal")]);
        let diff_reads = diff_reads_after - diff_reads_before;

        println!("Diff operation reads: {}", diff_reads);
        println!("Diff results count: {}", diff_results_count);

        // Both should have some reads since we're using immutable proposals
        assert!(
            baseline_reads > 0,
            "Expected baseline reads from tree traversals"
        );
        assert!(diff_reads > 0, "Expected reads from diff operation");

        // Verify hash optimization is working - should read FEWER nodes than full traversal
        assert!(
            diff_reads < baseline_reads,
            "Hash optimization failed: diff reads ({}) should be less than baseline ({}) for trees with shared content",
            diff_reads,
            baseline_reads
        );

        println!(
            "✅ Node read optimization verified: {} vs {} reads",
            diff_reads, baseline_reads
        );

        // Verify we found some diff operations (exact count and content validated by other tests)
        assert!(
            diff_results_count > 0,
            "Expected to find diff operations for trees with different content"
        );

        println!("   - Baseline reads: {}", baseline_reads);
        println!(
            "   - Diff reads: {} ({:.1}% of baseline)",
            diff_reads,
            (diff_reads as f64 / baseline_reads as f64) * 100.0
        );
        println!(
            "   - Node read reduction: {} ({:.1}%)",
            baseline_reads - diff_reads,
            ((baseline_reads - diff_reads) as f64 / baseline_reads as f64) * 100.0
        );
        println!("   - Diff operations found: {}", diff_results_count);
    }

    #[test]
    fn test_diff_processes_all_branch_children() {
        // This test verifies the bug fix: ensure that after finding different children
        // at the same position in a branch, the algorithm continues to process remaining children
        let m1 = create_test_merkle();
        let m1 = populate_merkle(
            m1,
            &[
                (b"branch_a/file", b"shared_value"),    // This will be identical
                (b"branch_b/file", b"value1"),          // This will be changed
                (b"branch_c/file", b"left_only_value"), // This will be deleted
            ],
        );

        let m2 = create_test_merkle();
        let m2 = populate_merkle(
            m2,
            &[
                (b"branch_a/file", b"shared_value"),     // Identical to tree1
                (b"branch_b/file", b"value1_modified"),  // Different value
                (b"branch_d/file", b"right_only_value"), // This will be added
            ],
        );

        let diff_stream =
            DiffMerkleKeyValueStreams::new(m1.nodestore(), m2.nodestore(), Key::default());

        let results: Vec<_> = diff_stream.collect::<Result<Vec<_>, _>>().unwrap();

        // Should find all differences:
        // 1. branch_b/file modified
        // 2. branch_c/file deleted
        // 3. branch_d/file added
        assert_eq!(results.len(), 3, "Should find all 3 differences");

        // Verify specific operations
        let mut changes = 0;
        let mut deletions = 0;
        let mut additions = 0;

        for result in &results {
            match result {
                DiffIterationResult::Changed { key, .. } => {
                    changes += 1;
                    assert_eq!(&**key, b"branch_b/file");
                }
                DiffIterationResult::Deleted { key } => {
                    deletions += 1;
                    assert_eq!(&**key, b"branch_c/file");
                }
                DiffIterationResult::Added { key, .. } => {
                    additions += 1;
                    assert_eq!(&**key, b"branch_d/file");
                }
            }
        }

        assert_eq!(changes, 1, "Should have 1 change");
        assert_eq!(deletions, 1, "Should have 1 deletion");
        assert_eq!(additions, 1, "Should have 1 addition");
    }
}
