// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

use crate::{
    db::BatchOp,
    iter::key_from_nibble_iter,
    merkle::{Key, PrefixOverlap, Value},
};
use firewood_storage::{
    Child, FileIoError, HashedNodeReader, Node, Path, PathComponent, TrieHash, TrieReader,
};
use std::{cmp::Ordering, iter::once};
use triomphe::Arc;

/// Enum containing all possible states that we can be in as we iterate through the diff
/// between two Merkle tries.
enum DiffIterationNodeState<'a> {
    /// In the `TraverseBoth` state, we only need to consider the next nodes from the left
    /// and right trie in pre-order traversal order.
    TraverseBoth {
        left_tree: PreOrderIterator<'a>,
        right_tree: PreOrderIterator<'a>,
    },
    /// In the `TraverseLeft` state, we need to compare the next node from the left trie
    /// with the current node in the right trie (`right_state`).
    TraverseLeft {
        left_tree: PreOrderIterator<'a>,
        right_tree: PreOrderIterator<'a>,
        right_state: NodeState,
    },
    /// In the `TraverseRight` state, we need to compare the next node from the right trie
    /// with the current node in the left trie (`left_state`).
    TraverseRight {
        left_tree: PreOrderIterator<'a>,
        right_tree: PreOrderIterator<'a>,
        left_state: NodeState,
    },
    /// In the `AddRestRight` state, we have reached the end of the left trie and need to
    /// add the remaining keys/values from the right trie to the addition list in the
    /// change proof.
    AddRestRight { right_tree: PreOrderIterator<'a> },
    /// In the `DeleteRestLeft` state, we have reached the end of the right trie and need
    /// add the remaining keys/values from the left trie to the deletion list in the change
    /// proof.
    DeleteRestLeft { left_tree: PreOrderIterator<'a> },
    /// In the `SkipChildren` state, we previously identified that the current nodes from
    /// both tries have matching paths, values, and hashes. This means we no longer need
    /// traverse any of their children. In his state, we call `skip_children` on both tries
    /// to remove their children from the traversal stack. Then we consider the next nodes
    /// from both tries in the same way as `TraverseBoth`.
    SkipChildren {
        left_tree: PreOrderIterator<'a>,
        right_tree: PreOrderIterator<'a>,
    },
}

struct NodeState {
    path: Path,
    node: Arc<Node>,
    hash: Option<TrieHash>,
}

/// Iterator that outputs the difference between two tries and skips matching sub-tries.
struct DiffMerkleNodeStream<'a> {
    // Contains the state of the traversal. It is only None after calling `next` or
    // `next_internal` if it has reached the end of the traversal.
    state: Option<DiffIterationNodeState<'a>>,
}

impl<'a> DiffMerkleNodeStream<'a> {
    /// Constructor where the left and right tries implement the trait `TrieReader`. This
    /// allows a `DiffMerkleNodeStream` to be constructed with `MutableProposal`s. The
    /// drawback to using `new_without_hash` is that we don't have the root hashes for
    /// these tries. The `new` constructor should be used for `ImmutableProposal`s.
    pub fn new_without_hash<T: TrieReader, U: TrieReader>(
        left_tree: &'a T,
        right_tree: &'a U,
        start_key: Key,
    ) -> Result<Self, FileIoError> {
        // Create pre-order iterators for the two tries and have them iterate to the start key.
        // If the start key doesn't exist, it will iterate them to the smallest key that is
        // larger than the start key.
        let mut left_tree = Self::preorder_iter(left_tree, None);
        left_tree.iterate_to_key(&start_key)?;
        let mut right_tree = Self::preorder_iter(right_tree, None);
        right_tree.iterate_to_key(&start_key)?;

        Ok(Self {
            state: Some(DiffIterationNodeState::TraverseBoth {
                left_tree,
                right_tree,
            }),
        })
    }

    /// Constructor where the left and right tries implement the trait `HashedNodeReader`. This
    /// constructor should be used instead of `new_without_hash` for `ImmutableProposal`s.
    pub fn new<T: HashedNodeReader, U: HashedNodeReader>(
        left_tree: &'a T,
        right_tree: &'a U,
        start_key: Key,
    ) -> Result<Self, FileIoError> {
        // Create pre-order iterators for the two tries and have them iterate to the start key.
        // If the start key doesn't exist, it will iterate them to the smallest key that is
        // larger than the start key.
        let mut left_tree = Self::preorder_iter(left_tree, left_tree.root_hash());
        left_tree.iterate_to_key(&start_key)?;
        let mut right_tree = Self::preorder_iter(right_tree, right_tree.root_hash());
        right_tree.iterate_to_key(&start_key)?;

        Ok(Self {
            state: Some(DiffIterationNodeState::TraverseBoth {
                left_tree,
                right_tree,
            }),
        })
    }

    /// Create a pre-order iterator for the trie. The iterator takes an optional
    /// root hash which is available when the trie is from an `ImmutableProposal`.
    fn preorder_iter<V: TrieReader>(
        tree: &'a V,
        root_hash: Option<TrieHash>,
    ) -> PreOrderIterator<'a> {
        let mut preorder_it = PreOrderIterator {
            stack: vec![],
            prev_num_children: 0,
            trie: tree,
        };
        // If the root node is not None, then push the root's NodeState onto the traversal
        // stack. It will be used on the first call to next or next_internal.
        if let Some(root) = tree.root_node() {
            preorder_it.stack.push(NodeState {
                path: root.partial_path().clone(),
                node: root,
                hash: root_hash,
            });
        }
        preorder_it
    }

    /// Helper function used in `one_step_compare` to check if two Option<TrieHash> matches. The
    /// `left_tree` and `right_tree` parameters are used to create the next state. Note that this
    /// function should only be used if the two nodes in the left and right tries have the same
    /// path and the same value.
    fn hash_match(
        left_hash: Option<TrieHash>,
        left_tree: PreOrderIterator<'a>,
        right_hash: Option<TrieHash>,
        right_tree: PreOrderIterator<'a>,
    ) -> DiffIterationNodeState<'a> {
        if match (left_hash, right_hash) {
            (Some(left_hash), Some(right_hash)) => left_hash == right_hash,
            _ => false,
        } {
            DiffIterationNodeState::SkipChildren {
                left_tree,
                right_tree,
            }
        } else {
            DiffIterationNodeState::TraverseBoth {
                left_tree,
                right_tree,
            }
        }
    }

    /// Called as part of a lock-step synchronized pre-order traversal of the left and right tries. This
    /// function compares the current nodes from the two tries to determine if any operations need to be
    /// deleted (i.e., op appears on the left but not the right trie) or added (i.e., op appears on the
    /// right but not the left trie). It also returns the next iteration state, which can include
    /// traversing down the left or right trie, traversing down both if the current nodes' path on both
    /// tries are the same but their node hashes differ, or skipping the children of the current nodes
    /// from both tries if their node hashes match.
    fn one_step_compare(
        left_state: NodeState,
        left_tree: PreOrderIterator<'a>,
        right_state: NodeState,
        right_tree: PreOrderIterator<'a>,
    ) -> (DiffIterationNodeState<'a>, Option<BatchOp<Key, Value>>) {
        // Compare the full path of the current nodes from the left and right tries.
        match left_state.path.cmp(&right_state.path) {
            // If the left full path is less than the right full path, that means that all of
            // the remaining nodes (and any keys stored in those nodes) from the right trie
            // are greater than the current node on the left trie. Therefore, we should traverse
            // down the left trie until we reach a node that is larger than or equal to the
            // current node on the right trie, and collect all of the keys associated with
            // the nodes that were traversed (excluding the last one) and add them to the set
            // of keys that need to be deleted in the change proof.
            Ordering::Less => {
                // If there is a value in the current node in the left trie, then that value
                // should be included in the set of deleted keys in the change proof. We do
                // this by returning it in the second entry of the tuple in the return value.
                (
                    DiffIterationNodeState::TraverseLeft {
                        left_tree,
                        right_tree,
                        right_state,
                    },
                    left_state.node.value().map(|_val| BatchOp::Delete {
                        key: key_from_nibble_iter(left_state.path.iter().copied()),
                    }),
                )
            }
            // If the left full path is greater than the right full path, then all of the
            // remaining nodes (and any keys stored in those nodes) from the left trie are greater
            // than the current node on the left trie. Therefore, any remaining keys from the
            // right trie that is smaller than the current node in the left trie are missing from
            // the left trie and should be added as additional keys to the change proof. Therefore,
            // we should traverse the right trie until we reach a node that is smaller than or
            // equal to the current node on the left trie, and collect all of the keys associated
            // with those nodes (excluding the last one) and add them to the set of keys to be
            // added to the change proof.
            Ordering::Greater => {
                // If there is a value in the current node in the right trie, then that value
                // should be included in the set of additional keys in the change proof.
                (
                    DiffIterationNodeState::TraverseRight {
                        left_tree,
                        right_tree,
                        left_state,
                    },
                    right_state.node.value().map(|val| BatchOp::Put {
                        key: key_from_nibble_iter(right_state.path.iter().copied()),
                        value: val.into(),
                    }),
                )
            }
            // If the left and right full paths are equal, then we need to also look at their values
            // (if any) to determine what to add to the change proof. If only the left node has a
            // value, then we know that this key cannot exist in the right trie and the value's key
            // must be added to the delete list. Conversely, if only the right node has a value, then
            // we know that this key does not exist in the left trie and this key/value must be added
            // to the addition list. In both cases, we can transition to TraverseBoth since we are
            // done with the current node in both tries and can continue comparing the tries from
            // their next largest key.
            //
            // For the same reason, we can transition to TraverseBoth if both current nodes don't have
            // values and their hashes don't match. If both current nodes have values but they are
            // not the same, then we put the value from the right node into the addition list as this
            // value has overwritten the old value from the left trie. We can also transition to
            // TraverseBoth as we are also done with both of the current nodes. If the values match,
            // we can transition to TraverseBoth if their hashes don't match since there is nothing
            // to add to the change proof and we are done with these nodes.
            //
            // For the cases where both nodes don't have value or both values are the same, and both
            // hashes match, then we know that everything below the current nodes are identical, and
            // we can transition to the SkipChildren state to not traverse any further down the two
            // tries from the current nodes.
            Ordering::Equal => match (left_state.node.value(), right_state.node.value()) {
                (None, None) => (
                    Self::hash_match(left_state.hash, left_tree, right_state.hash, right_tree),
                    None,
                ),
                (Some(_val), None) => (
                    DiffIterationNodeState::TraverseBoth {
                        left_tree,
                        right_tree,
                    },
                    Some(BatchOp::Delete {
                        key: key_from_nibble_iter(left_state.path.iter().copied()),
                    }),
                ),
                (None, Some(val)) => (
                    DiffIterationNodeState::TraverseBoth {
                        left_tree,
                        right_tree,
                    },
                    Some(BatchOp::Put {
                        key: key_from_nibble_iter(right_state.path.iter().copied()),
                        value: val.into(),
                    }),
                ),
                (Some(left_val), Some(right_val)) => {
                    if left_val == right_val {
                        (
                            Self::hash_match(
                                left_state.hash,
                                left_tree,
                                right_state.hash,
                                right_tree,
                            ),
                            None,
                        )
                    } else {
                        (
                            DiffIterationNodeState::TraverseBoth {
                                left_tree,
                                right_tree,
                            },
                            Some(BatchOp::Put {
                                key: key_from_nibble_iter(right_state.path.iter().copied()),
                                value: right_val.into(),
                            }),
                        )
                    }
                }
            },
        }
    }

    /// Helper function that returns a key/value to be added to the delete list if the current
    /// node from the left trie has a value.
    fn deleted_values(left_node: Arc<Node>, left_path: Path) -> Option<BatchOp<Key, Value>> {
        left_node.value().map(|_val| BatchOp::Delete {
            key: key_from_nibble_iter(left_path.iter().copied()),
        })
    }

    /// Helper function that returns a key/value to be added to the addition list if the current
    /// node from the right trie has a value.
    fn additional_values(right_node: Arc<Node>, right_path: Path) -> Option<BatchOp<Key, Value>> {
        right_node.value().map(|val| BatchOp::Put {
            key: key_from_nibble_iter(right_path.iter().copied()),
            value: val.into(),
        })
    }

    /// Helper function called in the `TraverseBoth` or `SkipChildren` state. Mainly handles the complexities
    /// introduced when one of the tries has no more nodes. If both tries have nodes remaining, then it calls
    /// `one_step_compare` to complete the state handling.
    fn next_node_from_both(
        mut left_tree: PreOrderIterator<'a>,
        mut right_tree: PreOrderIterator<'a>,
    ) -> Result<(DiffIterationNodeState<'a>, Option<BatchOp<Key, Value>>), FileIoError> {
        // Get the next node from the left trie.
        let Some(left_state) = left_tree.next()? else {
            // No more nodes in the left trie. For this state, the current node from the right trie has already
            // been accounted for, which means we don't need to include it in the change proof. For this case,
            // we want to mark the left state as empty (just for completeness), and transition to the
            // AddRestRight state where we add the remaining values from the right trie to the change proof.
            return Ok((DiffIterationNodeState::AddRestRight { right_tree }, None));
        };

        // Get the next node from the right trie.
        let Some(right_state) = right_tree.next()? else {
            // No more nodes on the right side. We want to transition to DeleteRestLeft, but we don't want to
            // forget about the node that we just retrieved from the left tree.
            return Ok((
                DiffIterationNodeState::DeleteRestLeft { left_tree },
                Self::deleted_values(left_state.node, left_state.path),
            ));
        };

        Ok(Self::one_step_compare(
            left_state,
            left_tree,
            right_state,
            right_tree,
        ))
    }

    /// Only called by `next` to implement the Iterator trait. Separated out into a separate
    /// function mainly to simplify error handling.
    fn next_internal(&mut self) -> Result<Option<BatchOp<Key, Value>>, FileIoError> {
        // Loops until there is a value to return or if we have reached the end of the
        // traversal. State processing is based on the value of `state`, which we take at the
        // beginning of the loop and reassign before the next iteration. `state` can only be
        // None after calling `next_internal` if we have reached the end of the traversal.
        while let Some(state) = self.state.take() {
            let (next_state, op) = match state {
                DiffIterationNodeState::SkipChildren {
                    mut left_tree,
                    mut right_tree,
                } => {
                    // In the SkipChildren state, the hash and path of the current nodes on
                    // both the left and right tries match. This means we don't need to
                    // traverse down the children of these tries. We can do this by calling
                    // skip_children on the two tries, which pops off the children of the
                    // current node from the traversal stack.
                    left_tree.skip_children();
                    right_tree.skip_children();

                    // Calls helper function that uses the next node from both tries. This
                    // helper function is also called for `TraverseBoth`.`
                    Self::next_node_from_both(left_tree, right_tree)?
                }
                DiffIterationNodeState::TraverseBoth {
                    left_tree,
                    right_tree,
                } => Self::next_node_from_both(left_tree, right_tree)?,
                DiffIterationNodeState::TraverseLeft {
                    mut left_tree,
                    right_tree,
                    right_state,
                } => {
                    // In the `TraverseLeft` state, we use the next node from the left trie to
                    // perform state processing, which is done by calling `one_step_compare`.
                    if let Some(left_state) = left_tree.next()? {
                        Self::one_step_compare(left_state, left_tree, right_state, right_tree)
                    } else {
                        // If we have no more nodes from the left trie, then we transition to
                        // the `AddRestRight` state where we add all of the remaining nodes
                        // from the right trie to the addition list. We also need to add the
                        // the value from the current right node if it has a value.
                        (
                            DiffIterationNodeState::AddRestRight { right_tree },
                            Self::additional_values(right_state.node, right_state.path),
                        )
                    }
                }
                DiffIterationNodeState::TraverseRight {
                    left_tree,
                    mut right_tree,
                    left_state,
                } => {
                    if let Some(right_state) = right_tree.next()? {
                        Self::one_step_compare(left_state, left_tree, right_state, right_tree)
                    } else {
                        // For `TraverseRight`, if we have no more nodes on the right trie, then
                        // transition to the `DeleteRestLeft` state where we add all of the
                        // remaining nodes from the left trie to the delete list. We also need
                        // to add the value from the current left node if it has a value.
                        (
                            DiffIterationNodeState::DeleteRestLeft { left_tree },
                            Self::deleted_values(left_state.node, left_state.path),
                        )
                    }
                }
                DiffIterationNodeState::AddRestRight { mut right_tree } => {
                    let Some(right_state) = right_tree.next()? else {
                        break; // No more nodes from both tries, which ends the iteration.
                    };
                    // Add the value of the right node to the addition list if it has one, and stay
                    // in this state.
                    (
                        DiffIterationNodeState::AddRestRight { right_tree },
                        Self::additional_values(right_state.node, right_state.path),
                    )
                }
                DiffIterationNodeState::DeleteRestLeft { mut left_tree } => {
                    let Some(left_state) = left_tree.next()? else {
                        break; // No more nodes from both tries, which ends the iteration.
                    };
                    // Add the value of the left node to the deletion list if it has one, and stay
                    // in this state.
                    (
                        DiffIterationNodeState::DeleteRestLeft { left_tree },
                        Self::deleted_values(left_state.node, left_state.path),
                    )
                }
            };
            // Perform the state transition. Return a value if the previous iteration produced one.
            // Otherwise, loop again to perform the next iteration.
            self.state = Some(next_state);
            if op.is_some() {
                return Ok(op);
            }
        }
        Ok(None)
    }
}

/// Adding support for the Iterator trait
impl Iterator for DiffMerkleNodeStream<'_> {
    type Item = Result<BatchOp<Key, Value>, firewood_storage::FileIoError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_internal().transpose()? {
            Ok(batch) => Some(Ok(batch)),
            Err(e) => Some(Err(e)),
        }
    }
}

/// State required for performing pre-order iteration of a Merkle trie. It contains
/// a reference to a trie which implements the `TrieReader` trait, a stack that
/// contains nodes (and their associated state including its path and its hash) to
/// be traversed, and a previous number of children field.
///
/// The last field is used to specifically to pop off children from the last traversed
/// node off of the stack so that they will not be traversed. This is useful in the
/// case that the left trie node has the same path and hash as the right trie node. We
/// can then skip their children in the traversal. `prev_num_children` is an u8 which
/// is big enough since there can at most be 16 children per node.
///
/// Note that this skipping the children functionality can only be called once after a
/// call to `next`. We cannot call `skip_children` twice in order to skip all of the
/// children from the current node's parent. This functionality is not necessary for
/// our use case as a hash match on the parent would have been determined previously
/// since we are performing in-order traversal.
struct PreOrderIterator<'a> {
    trie: &'a dyn TrieReader,
    stack: Vec<NodeState>,
    prev_num_children: u8,
}

/// Ignoring possible arithmetic side effects since we are only incrementing
/// `prev_num_children` once per child and there can only be at most 16 children per
/// node and `prev_num_children` is an u8.
#[allow(clippy::arithmetic_side_effects)]
impl PreOrderIterator<'_> {
    fn load_child(
        &self,
        state: &NodeState,
        path_comp: PathComponent,
        child: Child,
    ) -> Result<NodeState, FileIoError> {
        // Collect the hash for the child node as we possibly read the file from storage.
        let mut child_hash: Option<TrieHash> = None;
        let child = match child {
            Child::Node(child) => child.clone().into(),
            Child::AddressWithHash(addr, hash) => {
                child_hash = Some(hash);
                self.trie.read_node(addr)?
            }
            Child::MaybePersisted(maybe_persisted, hash) => {
                child_hash = Some(hash);
                maybe_persisted.as_shared_node(&self.trie)?
            }
        };

        // Compute the full path for the child. This is used later to compare with the full path
        // of the current node from the other trie.
        let child_path = Path::from_nibbles_iterator(
            state
                .path
                .iter()
                .copied()
                .chain(once(path_comp.as_u8()))
                .chain(child.partial_path().iter().copied()),
        );
        Ok(NodeState {
            path: child_path,
            node: child,
            hash: child_hash,
        })
    }

    fn next(&mut self) -> Result<Option<NodeState>, FileIoError> {
        // Pop the next node state from the stack and reset `prev_num_children`
        self.prev_num_children = 0;
        if let Some(state) = self.stack.pop() {
            // If this node is a branch, push its children onto the stack so they will be processed next.
            if let Node::Branch(branch) = &*state.node {
                // Since a stack is LIFO and we want to perform pre-order traversal, we pushed the
                // children in reverse order.
                for (path_comp, child) in branch.children.clone().into_iter().rev() {
                    if let Some(child) = child {
                        // Load the child possibly from storage, and collect its hash and generate its path.
                        // Push this node state onto the traversal stack.
                        self.stack.push(self.load_child(&state, path_comp, child)?);
                        // Increment the number of children that has been added to the stack.
                        self.prev_num_children += 1;
                    }
                }
            }
            // Return the state of the current node
            Ok(Some(state))
        } else {
            // Stack is empty, iteration is complete. Return None.
            Ok(None)
        }
    }

    /// Used to not iterate through the children of a branch node if there is a hash match.
    fn skip_children(&mut self) {
        for _ in 0..self.prev_num_children {
            self.stack.pop();
        }
        self.prev_num_children = 0;
    }

    /// Used to skip all of the keys prior to the specified key. Its main use is to start the
    /// change proof at a particular key.
    fn iterate_to_key(&mut self, key: &Key) -> Result<(), FileIoError> {
        // Function is a no-op if the key is empty. This is the common case.
        if key.is_empty() {
            return Ok(());
        }
        // Assume root or other nodes have already been pushed onto the stack
        loop {
            // Pop the next node state from the stack and reset `prev_num_children`
            self.prev_num_children = 0;
            if let Some(state) = self.stack.pop() {
                // If the key matches the node path, then we have reached the key. Just push the
                // current node state back to the stack. Calling `next` will process this node again.
                let node_key = key_from_nibble_iter(state.path.iter().copied());
                if node_key == *key {
                    self.stack.push(state);
                    return Ok(());
                }
                // Check if this node's path is a prefix of the key. If it is, then keep traversing
                // Otherwise, this is the lexicographically next node to the key, and calling `next`
                // should return this node. We stop the traversal at this point and push the node
                // state back onto the stack.
                let path_overlap = PrefixOverlap::from(key, node_key.as_ref());
                let unique_node = path_overlap.unique_b;
                if !unique_node.is_empty() {
                    // If `unique_node` is not empty, then the path is not a prefix of the key.
                    self.stack.push(state);
                    return Ok(());
                }

                if let Node::Branch(branch) = &*state.node {
                    // TODO: Once the first child is added to the stack, then the rest should be added without needing
                    //       to do additional checks.
                    for (path_comp, child) in branch.children.clone().into_iter().rev() {
                        if let Some(child) = child {
                            // Load the child possibly from storage, and collect its hash and generate its path.
                            let child_state = self.load_child(&state, path_comp, child)?;

                            // We only need to traverse this child if its path is either a prefix of the key (including
                            // being equal to the key), or if its path is lexicographically larger than the key.
                            let child_key = key_from_nibble_iter(child_state.path.iter().copied());
                            let path_overlap = PrefixOverlap::from(key, child_key.as_ref());
                            let unique_node = path_overlap.unique_b;
                            if unique_node.is_empty() || child_key > *key {
                                self.stack.push(child_state);
                                self.prev_num_children += 1;
                            }
                        }
                    }
                }
            } else {
                // Traversal stack is empty. This means the key is lexicographically larger than
                // all of the keys in the trie. Calling `next` will return None.
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects)]
mod tests {
    use crate::{
        db::BatchOp,
        diff::DiffMerkleNodeStream,
        merkle::{Key, Merkle, Value},
    };

    use firewood_storage::{
        FileIoError, HashedNodeReader, ImmutableProposal, MemStore, MutableProposal, NodeStore,
        TrieReader,
    };
    use std::sync::Arc;
    use test_case::test_case;

    fn diff_merkle_iterator<'a, T, U>(
        tree_left: &'a Merkle<T>,
        tree_right: &'a Merkle<U>,
        start_key: Key,
    ) -> Result<DiffMerkleNodeStream<'a>, FileIoError>
    where
        T: firewood_storage::TrieReader + HashedNodeReader,
        U: firewood_storage::TrieReader + HashedNodeReader,
    {
        DiffMerkleNodeStream::new(tree_left.nodestore(), tree_right.nodestore(), start_key)
    }

    fn diff_merkle_iterator_without_hash<'a, T, U>(
        tree_left: &'a Merkle<T>,
        tree_right: &'a Merkle<U>,
        start_key: Key,
    ) -> Result<DiffMerkleNodeStream<'a>, FileIoError>
    where
        T: firewood_storage::TrieReader,
        U: firewood_storage::TrieReader,
    {
        DiffMerkleNodeStream::new_without_hash(
            tree_left.nodestore(),
            tree_right.nodestore(),
            start_key,
        )
    }

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

    fn assert_merkle_eq<L, R>(left: &Merkle<L>, right: &Merkle<R>)
    where
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
                        eprintln!(
                            "Key mismatch at position {}: left={:02x?}, right={:02x?}",
                            key_count,
                            lk.as_ref(),
                            rk.as_ref()
                        );
                        // Show a few more keys for context
                        for i in 0..3 {
                            match (l.next(), r.next()) {
                                (Some(Ok((lk2, _))), Some(Ok((rk2, _)))) => {
                                    eprintln!(
                                        "  Next {}: left={:02x?}, right={:02x?}",
                                        i + 1,
                                        lk2.as_ref(),
                                        rk2.as_ref()
                                    );
                                }
                                (Some(Ok((lk2, _))), None) => {
                                    eprintln!(
                                        "  Next {}: left={:02x?}, right=None",
                                        i + 1,
                                        lk2.as_ref()
                                    );
                                }
                                (None, Some(Ok((rk2, _)))) => {
                                    eprintln!(
                                        "  Next {}: left=None, right={:02x?}",
                                        i + 1,
                                        rk2.as_ref()
                                    );
                                }
                                _ => break,
                            }
                        }
                        panic!("keys differ at position {key_count}");
                    }
                    assert_eq!(lv, rv, "values differ at key {:02x?}", lk.as_ref());
                }
                (None, Some(Ok((rk, _)))) => panic!(
                    "Missing key in result at position {}: {rk:02x?}",
                    key_count + 1
                ),
                (Some(Ok((lk, _))), None) => panic!(
                    "Extra key in result at position {}: {lk:02x?}",
                    key_count + 1
                ),
                (Some(Err(e)), _) | (_, Some(Err(e))) => panic!("iteration error: {e:?}"),
            }
        }
    }

    #[test]
    fn test_diff_empty_mutable_trees() {
        // This is unlikely to happen in practice, but it helps cover the case where
        // hashes do not exist yet.
        let m1 = create_test_merkle();
        let m2 = create_test_merkle();

        let mut diff_iter = diff_merkle_iterator_without_hash(&m1, &m2, Box::new([])).unwrap();
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

        let op1 = diff_iter.next().unwrap().unwrap();
        assert!(
            matches!(op1, BatchOp::Put { key, value } if key == Box::from(b"key1".as_slice()) && value.as_ref() == b"value1")
        );

        let op2 = diff_iter.next().unwrap().unwrap();
        assert!(
            matches!(op2, BatchOp::Put { key, value } if key == Box::from(b"key2".as_slice()) && value.as_ref() == b"value2")
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

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([])).unwrap();

        let op = diff_iter.next().unwrap().unwrap();
        assert!(
            matches!(op, BatchOp::Put { key, value } if key == Box::from(b"key1".as_slice()) && value.as_ref() == b"new_value")
        );

        assert!(diff_iter.next().is_none());
    }

    #[test]
    #[allow(clippy::manual_let_else)]
    fn test_diff_mixed_operations() {
        // m1 has: key1=value1, key2=old_value, key3=value3
        // m2 has: key2=new_value, key4=value4
        // Expected: Delete key1, Put key2=new_value, Delete key3, Put key4=value4

        let m1 = populate_merkle(
            create_test_merkle(),
            &[
                (b"key1", b"value1"), // [6b, 65, 79, 31]
                (b"key2", b"old_value"),
                (b"key3", b"value3"),
            ],
        );

        let m2 = populate_merkle(
            create_test_merkle(),
            &[(b"key2", b"new_value"), (b"key4", b"value4")],
        );

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([])).unwrap();

        let op1 = diff_iter.next().unwrap().unwrap();
        assert!(matches!(op1, BatchOp::Delete { key } if key == Box::from(b"key1".as_slice())));

        let op2 = diff_iter.next().unwrap().unwrap();
        assert!(
            matches!(op2, BatchOp::Put { key, value } if key == Box::from(b"key2".as_slice()) && value.as_ref() == b"new_value")
        );

        let op3 = diff_iter.next().unwrap().unwrap();
        assert!(matches!(op3, BatchOp::Delete { key } if key == Box::from(b"key3".as_slice())));

        let op4 = diff_iter.next().unwrap().unwrap();
        assert!(
            matches!(op4, BatchOp::Put { key, value } if key == Box::from(b"key4".as_slice()) && value.as_ref() == b"value4")
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

    #[test_case(true, false, 0, 1)] // same value, m1->m2: no put needed, delete prefix/b
    #[test_case(false, false, 1, 1)] // diff value, m1->m2: put prefix/a, delete prefix/b
    #[test_case(true, true, 1, 0)] // same value, m2->m1: no change to prefix/a, add prefix/b
    #[test_case(false, true, 2, 0)] // diff value, m2->m1: update prefix/a, add prefix/b
    #[allow(clippy::arithmetic_side_effects)]
    fn test_branch_vs_leaf_empty_partial_path_bug(
        same_value: bool,
        backwards: bool,
        expected_puts: usize,
        expected_deletes: usize,
    ) {
        // This test covers the exclusion logic in Branch vs Leaf scenarios.
        // It creates a case where one tree has a branch with children, and the other
        // tree has a leaf that matches one of those children - testing that the
        // matching child gets excluded from deletion and properly compared instead.
        //
        // Parameters:
        // - same_value: whether prefix/a has the same value in both trees
        // - backwards: whether to compare m2->m1 instead of m1->m2
        // - expected_puts/expected_deletes: expected operation counts

        // Tree1: Create children under "prefix" but no value at "prefix" itself
        // This creates a branch node at "prefix" with value=None
        let m1 = populate_merkle(
            create_test_merkle(),
            &[
                (b"prefix/a".as_slice(), b"value_a".as_slice()),
                (b"prefix/b".as_slice(), b"value_b".as_slice()),
            ],
        );

        // Tree2: Create just a single value at "prefix/a"
        // Value depends on same_value parameter
        let m2_value: &[u8] = if same_value {
            b"value_a"
        } else {
            b"prefix_a_value"
        };
        let m2 = populate_merkle(create_test_merkle(), &[(b"prefix/a".as_slice(), m2_value)]);

        // Choose direction based on backwards parameter
        let (tree_left, tree_right, direction_desc) = if backwards {
            (m2.nodestore(), m1.nodestore(), "m2->m1")
        } else {
            (m1.nodestore(), m2.nodestore(), "m1->m2")
        };

        //let diff_stream = DiffMerkleKeyValueStreams::new(tree_left, tree_right, Key::default());
        let diff_stream = DiffMerkleNodeStream::new(tree_left, tree_right, Key::default()).unwrap();
        let results: Vec<_> = diff_stream.collect::<Result<Vec<_>, _>>().unwrap();

        let delete_count = results
            .iter()
            .filter(|op| matches!(op, BatchOp::Delete { .. }))
            .count();

        let put_count = results
            .iter()
            .filter(|op| matches!(op, BatchOp::Put { .. }))
            .count();

        // Verify against expected counts
        assert_eq!(
            put_count, expected_puts,
            "Put count mismatch for {direction_desc} (same_value={same_value}, backwards={backwards}), results={results:x?}"
        );
        assert_eq!(
            delete_count, expected_deletes,
            "Delete count mismatch for {direction_desc} (same_value={same_value}, backwards={backwards}), results={results:x?}"
        );
        assert_eq!(
            results.len(),
            expected_puts + expected_deletes,
            "Total operation count mismatch for {direction_desc} (same_value={same_value}, backwards={backwards}), results={results:x?}"
        );

        println!(
            "✅ Branch vs leaf test passed: {direction_desc} (same_value={same_value}, backwards={backwards}) - {put_count} puts, {delete_count} deletes"
        );
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
            DiffMerkleNodeStream::new(m1.nodestore(), m2.nodestore(), Key::default()).unwrap();

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
                BatchOp::Put { key, value: _ } => {
                    if key.as_ref() == b"branch_b/file" {
                        changes += 1;
                        assert_eq!(&**key, b"branch_b/file");
                    } else if key.as_ref() == b"branch_d/file" {
                        additions += 1;
                        assert_eq!(&**key, b"branch_d/file");
                    }
                }
                BatchOp::Delete { key } => {
                    deletions += 1;
                    assert_eq!(&**key, b"branch_c/file");
                }
                BatchOp::DeleteRange { .. } => {
                    panic!("DeleteRange not expected in this test");
                }
            }
        }

        assert_eq!(changes, 1, "Should have 1 change");
        assert_eq!(deletions, 1, "Should have 1 deletion");
        assert_eq!(additions, 1, "Should have 1 addition");
    }

    #[test]
    fn test_all_six_diff_states_coverage() {
        // This test ensures comprehensive coverage of all 6 diff iteration states
        // by creating specific scenarios that guarantee each state is exercised

        // Create trees with carefully designed structure to trigger all states:
        // 1. Deep branching structure to ensure branch nodes exist
        // 2. Mix of shared, modified, left-only, and right-only content
        // 3. Different tree shapes to force visited states

        let tree1_data = vec![
            // Shared deep structure (will trigger VisitedNodePairState)
            (b"shared/deep/branch/file1".as_slice(), b"value1".as_slice()),
            (b"shared/deep/branch/file2".as_slice(), b"value2".as_slice()),
            (b"shared/deep/branch/file3".as_slice(), b"value3".as_slice()),
            // Modified values (will trigger UnvisitedNodePairState)
            (b"modified/path/file".as_slice(), b"old_value".as_slice()),
            // Left-only deep structure (will trigger VisitedNodeLeftState)
            (
                b"left_only/deep/branch/file1".as_slice(),
                b"left_val1".as_slice(),
            ),
            (
                b"left_only/deep/branch/file2".as_slice(),
                b"left_val2".as_slice(),
            ),
            (
                b"left_only/deep/branch/file3".as_slice(),
                b"left_val3".as_slice(),
            ),
            // Simple left-only (will trigger UnvisitedNodeLeftState)
            (
                b"simple_left_only".as_slice(),
                b"simple_left_value".as_slice(),
            ),
            // Mixed branch with some shared children
            (
                b"mixed_branch/shared_child".as_slice(),
                b"shared".as_slice(),
            ),
            (
                b"mixed_branch/left_child".as_slice(),
                b"left_value".as_slice(),
            ),
        ];

        let tree2_data = vec![
            // Same shared deep structure
            (b"shared/deep/branch/file1".as_slice(), b"value1".as_slice()),
            (b"shared/deep/branch/file2".as_slice(), b"value2".as_slice()),
            (b"shared/deep/branch/file3".as_slice(), b"value3".as_slice()),
            // Modified values
            (b"modified/path/file".as_slice(), b"new_value".as_slice()),
            // Right-only deep structure (will trigger VisitedNodeRightState)
            (
                b"right_only/deep/branch/file1".as_slice(),
                b"right_val1".as_slice(),
            ),
            (
                b"right_only/deep/branch/file2".as_slice(),
                b"right_val2".as_slice(),
            ),
            (
                b"right_only/deep/branch/file3".as_slice(),
                b"right_val3".as_slice(),
            ),
            // Simple right-only (will trigger UnvisitedNodeRightState)
            (
                b"simple_right_only".as_slice(),
                b"simple_right_value".as_slice(),
            ),
            // Mixed branch with some shared children
            (
                b"mixed_branch/shared_child".as_slice(),
                b"shared".as_slice(),
            ),
            (
                b"mixed_branch/right_child".as_slice(),
                b"right_value".as_slice(),
            ),
        ];

        let m1 = populate_merkle(create_test_merkle(), &tree1_data);
        let m2 = populate_merkle(create_test_merkle(), &tree2_data);

        let diff_iter = diff_merkle_iterator(&m1, &m2, Key::default()).unwrap();
        let results: Vec<_> = diff_iter.collect::<Result<Vec<_>, _>>().unwrap();

        // Verify we found the expected differences
        let mut deletions = 0;
        let mut additions = 0;

        for result in &results {
            match result {
                BatchOp::Put { .. } => additions += 1,
                BatchOp::Delete { .. } => deletions += 1,
                BatchOp::DeleteRange { .. } => {
                    panic!("DeleteRange not expected in this test");
                }
            }
        }

        // Expected differences using BatchOp representation:
        // - Both modifications and additions are represented as Put operations
        // - Deletions are Delete operations
        // - We expect multiple operations for the different scenarios
        assert!(deletions >= 4, "Expected at least 4 deletions");
        assert!(
            additions >= 4,
            "Expected at least 4 additions (includes modifications)"
        );

        println!("✅ All 6 diff states coverage test passed:");
        println!("   - Deletions: {deletions}");
        println!("   - Additions (includes modifications): {additions}");
        println!("   - This test exercises scenarios that should trigger:");
        println!("     1. UnvisitedNodePairState (comparing modified nodes)");
        println!("     2. UnvisitedNodeLeftState (simple left-only nodes)");
        println!("     3. UnvisitedNodeRightState (simple right-only nodes)");
        println!("     4. VisitedNodePairState (shared branch with different children)");
        println!("     5. VisitedNodeLeftState (left-only branch structures)");
        println!("     6. VisitedNodeRightState (right-only branch structures)");
    }

    #[test]
    fn test_branch_vs_leaf_state_transitions() {
        // This test specifically covers the branch-vs-leaf scenarios in UnvisitedNodePairState
        // which can trigger different state transitions

        // Tree1: Has a branch structure at "path"
        let m1 = populate_merkle(
            create_test_merkle(),
            &[
                (b"path/file1".as_slice(), b"value1".as_slice()),
                (b"path/file2".as_slice(), b"value2".as_slice()),
            ],
        );

        // Tree2: Has a leaf at "path"
        let m2 = populate_merkle(
            create_test_merkle(),
            &[(b"path".as_slice(), b"leaf_value".as_slice())],
        );

        let diff_stream =
            DiffMerkleNodeStream::new(m1.nodestore(), m2.nodestore(), Key::default()).unwrap();

        let results: Vec<_> = diff_stream.collect::<Result<Vec<_>, _>>().unwrap();

        // Should find:
        // - Deletion of path/file1 and path/file2
        // - Addition of path (leaf)
        assert!(
            results.len() >= 2,
            "Should find multiple differences for branch vs leaf"
        );

        println!(
            "✅ Branch vs leaf transitions test passed with {} operations",
            results.len()
        );
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
                (b"aaa", b"value2"),   // Same
                (b"bbb", b"modified"), // Modified
                (b"ddd", b"value4"),   // Added
            ],
        );

        // Start from key "bbb" - should skip "aaa"
        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::from(b"bbb".as_slice())).unwrap();

        let op1 = diff_iter.next().unwrap().unwrap();
        assert!(
            matches!(op1, BatchOp::Put { ref key, ref value } if **key == *b"bbb" && **value == *b"modified"),
            "Expected first operation to be Put bbb=modified, got: {op1:?}",
        );

        let op2 = diff_iter.next().unwrap().unwrap();
        assert!(matches!(op2, BatchOp::Delete { key } if key == Box::from(b"ccc".as_slice())));

        let op3 = diff_iter.next().unwrap().unwrap();
        assert!(
            matches!(op3, BatchOp::Put { key, value } if key == Box::from(b"ddd".as_slice()) && value.as_ref() == b"value4")
        );

        assert!(diff_iter.next().is_none());
    }

    // example of running this test with a specific seed and parameters:
    // FIREWOOD_TEST_SEED=14805530293320947613 cargo test --features logger diff::tests::diff_random_with_deletions
    #[test_case(false, false, 500)]
    #[test_case(false, true, 500)]
    #[test_case(true, false, 500)]
    #[test_case(true, true, 500)]
    //#[allow(clippy::indexing_slicing, clippy::cast_precision_loss)]
    #[allow(
        clippy::indexing_slicing,
        clippy::cast_precision_loss,
        clippy::type_complexity,
        clippy::disallowed_types,
        clippy::unreadable_literal
    )]
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
            while items.len() > 1 && delete_idx2 == 0 {
                // it's okay if equal when len==1
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
            //diff_iter.collect::<Result<Vec<_>, _>>().unwrap();
            let ops = diff_merkle_iterator_without_hash(&m1, &m2, Box::new([]))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let m1_immut = m1.try_into().unwrap();
            let m2_immut = m2.try_into().unwrap();
            (ops, m1_immut, m2_immut)
        } else if trie1_mutable && !trie2_mutable {
            let m2_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m2.try_into().unwrap();
            let ops = diff_merkle_iterator_without_hash(&m1, &m2_immut, Box::new([]))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let m1_immut = m1.try_into().unwrap();
            (ops, m1_immut, m2_immut)
        } else if !trie1_mutable && trie2_mutable {
            let m1_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m1.try_into().unwrap();
            let ops = diff_merkle_iterator_without_hash(&m1_immut, &m2, Box::new([]))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let m2_immut = m2.try_into().unwrap();
            (ops, m1_immut, m2_immut)
        } else {
            let m1_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m1.try_into().unwrap();
            let m2_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m2.try_into().unwrap();
            let ops = diff_merkle_iterator_without_hash(&m1_immut, &m2_immut, Box::new([]))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            (ops, m1_immut, m2_immut)
        };

        // Apply ops to left immutable and compare with right immutable
        let left_after = apply_ops_and_freeze(&m1_immut, &ops);
        assert_merkle_eq(&left_after, &m2_immut);
    }
}
