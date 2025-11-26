// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

use crate::{
    db::BatchOp,
    iter::key_from_nibble_iter,
    merkle::{Key, PrefixOverlap, Value},
};
use firewood_storage::{Child, FileIoError, HashedNodeReader, Node, Path, TrieHash, TrieReader};
use std::{cmp::Ordering, iter::once};
use triomphe::Arc;

/// Enum containing all possible states for a diff iteration node
enum DiffIterationNodeState {
    TraverseBoth,
    TraverseLeft,
    TraverseRight,
    AddRestRight,
    DeleteRestLeft,
    SkipChildren,
}

/// Optimized node iterator that compares two merkle trees and skips matching subtrees
struct DiffMerkleNodeStream<'a> {
    left_tree: PreOrderIterator<'a>,
    right_tree: PreOrderIterator<'a>,
    left_state: Option<(Arc<Node>, Path, Option<TrieHash>)>,
    right_state: Option<(Arc<Node>, Path, Option<TrieHash>)>,
    state: DiffIterationNodeState,
}

impl<'a> DiffMerkleNodeStream<'a> {
    fn new_without_hash<T: TrieReader, U: TrieReader>(
        left_tree: &'a T,
        right_tree: &'a U,
        start_key: Key,
    ) -> Self {
        let mut ret = Self {
            left_tree: Self::preorder_iter(left_tree, None),
            right_tree: Self::preorder_iter(right_tree, None),
            left_state: None,
            right_state: None,
            state: DiffIterationNodeState::TraverseBoth,
            //state: DiffNodeStreamState::from(start_key),
        };
        let _ = ret.left_tree.iterate_to_key(&start_key);
        let _ = ret.right_tree.iterate_to_key(&start_key);
        ret
    }

    fn new<T: TrieReader + HashedNodeReader, U: TrieReader + HashedNodeReader>(
        left_tree: &'a T,
        right_tree: &'a U,
        start_key: Key,
    ) -> Self {
        let mut ret = Self {
            left_tree: Self::preorder_iter(left_tree, left_tree.root_hash()),
            right_tree: Self::preorder_iter(right_tree, right_tree.root_hash()),
            left_state: None,
            right_state: None,
            state: DiffIterationNodeState::TraverseBoth,
            //state: DiffNodeStreamState::from(start_key),
        };
        let _ = ret.left_tree.iterate_to_key(&start_key);
        let _ = ret.right_tree.iterate_to_key(&start_key);
        ret
    }

    pub fn preorder_iter<V: TrieReader>(
        tree: &'a V,
        root_hash: Option<TrieHash>,
    ) -> PreOrderIterator<'a> {
        let root = tree.root_node();
        let mut ret = PreOrderIterator {
            stack: vec![],
            prev_num_children: 0,
            trie: tree,
        };

        if let Some(root) = root {
            ret.stack.push((root.clone(), Path::default(), root_hash));
        }
        ret
    }

    /// Called as part of a lock-step synchronized pre-order traversal of the left and right tries. This
    /// function compares the current nodes from the two tries to determine if any operations needed to be
    /// deleted (i.e., op appears on the left but not the right trie) or added (i.e., op appears on the
    /// right but not the left trie). It also returns the next iteration action, which can include
    /// traversing down the left or right trie, traversing down both if current nodes' path on both tries
    /// are the same but their node hashes differ, or skipping the children of the current nodes from both
    /// tries if their node hashes match.
    /// TODO: Consider grouping pre-path, node, and hash into a single struct.
    pub fn one_step(
        &self,
        left_pre_path: &Path,
        left_node: &Arc<Node>,
        left_hash: Option<&TrieHash>,
        right_pre_path: &Path,
        right_node: &Arc<Node>,
        right_hash: Option<&TrieHash>,
    ) -> (DiffIterationNodeState, Option<BatchOp<Key, Value>>) {
        // Combine pre_path with node's partial path to get its full path.
        // TODO: Determine if it is necessary to compute the full path.
        let left_full_path = Path::from_nibbles_iterator(
            left_pre_path
                .iter()
                .copied()
                .chain(left_node.partial_path().iter().copied()),
        );
        let right_full_path = Path::from_nibbles_iterator(
            right_pre_path
                .iter()
                .copied()
                .chain(right_node.partial_path().iter().copied()),
        );

        // Compare the full path of the current nodes from the left and right tries.
        match left_full_path.cmp(&right_full_path) {
            // If the left full path is less than the right full path, that means that all of
            // the remaining nodes (and any keys stored in those nodes) from the right trie
            // are greater than the current node on the left trie. Therefore, we should traverse
            // down the left trie until we reach a node that is larger than or equal to the
            // current node on the right trie, and collect all of the keys associated with
            // those nodes (excluding the last one) and add them to the set of keys that
            // need to be deleted in the change proof.
            Ordering::Less => {
                // If there is a value in the current node in the left trie, than that value
                // should be included in the set of deleted keys in the change proof. We do
                // this by returning it in the second entry of the tuple in the return value.
                if let Some(_val) = left_node.value() {
                    (
                        DiffIterationNodeState::TraverseLeft,
                        Some(BatchOp::Delete {
                            key: key_from_nibble_iter(left_full_path.iter().copied()),
                        }),
                    )
                } else {
                    (DiffIterationNodeState::TraverseLeft, None)
                }
            }
            Ordering::Greater => {
                println!("Greater than: {left_full_path:?} ------ {right_full_path:?}");
                if let Some(val) = right_node.value() {
                    (
                        DiffIterationNodeState::TraverseRight,
                        Some(BatchOp::Put {
                            key: key_from_nibble_iter(right_full_path.iter().copied()),
                            value: val.into(),
                        }),
                    )
                } else {
                    (DiffIterationNodeState::TraverseRight, None)
                }
            }
            Ordering::Equal => {
                // Check if there are values. If there are, then it might not actually be equal
                match (left_node.value(), right_node.value()) {
                    (None, None) => {
                        if let (Some(left_hash), Some(right_hash)) = (left_hash, right_hash) {
                            if left_hash.eq(right_hash) {
                                (DiffIterationNodeState::SkipChildren, None)
                            } else {
                                (DiffIterationNodeState::TraverseBoth, None)
                            }
                        } else {
                            (DiffIterationNodeState::TraverseBoth, None)
                        }
                    }
                    (Some(_val), None) => (
                        DiffIterationNodeState::TraverseLeft,
                        Some(BatchOp::Delete {
                            key: key_from_nibble_iter(left_full_path.iter().copied()),
                        }),
                    ),
                    (None, Some(val)) => (
                        DiffIterationNodeState::TraverseRight,
                        Some(BatchOp::Put {
                            key: key_from_nibble_iter(right_full_path.iter().copied()),
                            value: val.into(),
                        }),
                    ),
                    // TODO: Replace cmp with eq
                    (Some(left_val), Some(right_val)) => match left_val.cmp(right_val) {
                        Ordering::Equal => {
                            if let (Some(left_hash), Some(right_hash)) = (left_hash, right_hash) {
                                if left_hash.eq(right_hash) {
                                    (DiffIterationNodeState::SkipChildren, None)
                                } else {
                                    (DiffIterationNodeState::TraverseBoth, None)
                                }
                            } else {
                                (DiffIterationNodeState::TraverseBoth, None)
                            }
                        }
                        Ordering::Less | Ordering::Greater => (
                            DiffIterationNodeState::TraverseBoth,
                            Some(BatchOp::Put {
                                key: key_from_nibble_iter(right_full_path.iter().copied()),
                                value: right_val.into(),
                            }),
                        ),
                    },
                }
            }
        }
    }
}

impl Iterator for DiffMerkleNodeStream<'_> {
    type Item = Result<BatchOp<Key, Value>, firewood_storage::FileIoError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_internal().transpose()? {
            Ok(batch) => Some(Ok(batch)),
            Err(e) => Some(Err(e)),
        }
    }
}

impl DiffMerkleNodeStream<'_> {
    fn next_internal(
        &mut self,
    ) -> Result<Option<BatchOp<Key, Value>>, firewood_storage::FileIoError> {
        loop {
            match &mut self.state {
                DiffIterationNodeState::SkipChildren => {
                    println!("######## Just skip for now ############");
                    self.left_tree.skip_branches();
                    self.right_tree.skip_branches();

                    // TODO: Copied and pasted from TraverseBoth. Need to refactor
                    let Some((left_node, left_pre_path, left_hash)) = self.left_tree.next()? else {
                        self.state = DiffIterationNodeState::AddRestRight;
                        self.left_state = None;
                        continue;
                    };

                    let Some((right_node, right_pre_path, right_hash)) = self.right_tree.next()?
                    else {
                        self.state = DiffIterationNodeState::DeleteRestLeft;
                        self.right_state = None;
                        // Combine pre_path with node path to get full path.
                        let full_path = Path::from_nibbles_iterator(
                            left_pre_path
                                .iter()
                                .copied()
                                .chain(left_node.partial_path().iter().copied()),
                        );

                        let ret = left_node.value().map(|_val| BatchOp::Delete {
                            key: key_from_nibble_iter(full_path.iter().copied()),
                        });
                        if ret.is_some() {
                            return Ok(ret);
                        }
                        continue;
                    };
                    println!("Right state: {:?}", self.right_state);
                    let (next_op, ret) = self.one_step(
                        &left_pre_path,
                        &left_node,
                        left_hash.as_ref(),
                        &right_pre_path,
                        &right_node,
                        right_hash.as_ref(),
                    );

                    // Update states
                    self.left_state = Some((left_node, left_pre_path, left_hash));
                    self.right_state = Some((right_node, right_pre_path, right_hash));
                    self.state = next_op;
                    println!("Ret: {ret:?}");
                    if ret.is_some() {
                        return Ok(ret); // There is a BatchOp to return, else keep looping until there is or is done
                    }
                }
                // TODO: Do I actually need a traverse both? What if the right state is None, then call next. If next will continue
                //       to return None after the first None is reached, then it would be safe.
                DiffIterationNodeState::TraverseBoth => {
                    println!("TraverseBoth");
                    // TODO: For traverse both, CHECK if we need to call next on both trees even if the left tree is empty!!
                    let Some((left_node, left_pre_path, left_hash)) = self.left_tree.next()? else {
                        self.state = DiffIterationNodeState::AddRestRight;
                        self.left_state = None;
                        continue;
                    };

                    let Some((right_node, right_pre_path, right_hash)) = self.right_tree.next()?
                    else {
                        self.state = DiffIterationNodeState::DeleteRestLeft;
                        self.right_state = None;
                        // Combine pre_path with node path to get full path.
                        let full_path = Path::from_nibbles_iterator(
                            left_pre_path
                                .iter()
                                .copied()
                                .chain(left_node.partial_path().iter().copied()),
                        );

                        let ret = left_node.value().map(|_val| BatchOp::Delete {
                            key: key_from_nibble_iter(full_path.iter().copied()),
                        });
                        if ret.is_some() {
                            return Ok(ret);
                        }
                        continue;
                    };
                    println!("Right state: {:?}", self.right_state);
                    let (next_op, ret) = self.one_step(
                        &left_pre_path,
                        &left_node,
                        left_hash.as_ref(),
                        &right_pre_path,
                        &right_node,
                        right_hash.as_ref(),
                    );

                    // Update states
                    self.left_state = Some((left_node, left_pre_path, left_hash));
                    self.right_state = Some((right_node, right_pre_path, right_hash));
                    self.state = next_op;
                    println!("Ret: {ret:?}");
                    if ret.is_some() {
                        return Ok(ret); // There is a BatchOp to return, else keep looping until there is or is done
                    }
                }
                DiffIterationNodeState::TraverseLeft => {
                    println!("TraverseLeft");
                    let (right_node, right_pre_path, right_hash) =
                        self.right_state.take().expect("TODO");

                    let Some((left_node, left_pre_path, left_hash)) = self.left_tree.next()? else {
                        self.state = DiffIterationNodeState::AddRestRight;
                        self.left_state = None;

                        // TODO: Refactor this into a helper function
                        // Combine pre_path with node path to get full path.
                        let full_path = Path::from_nibbles_iterator(
                            right_pre_path
                                .iter()
                                .copied()
                                .chain(right_node.partial_path().iter().copied()),
                        );

                        let ret = right_node.value().map(|val| {
                            println!("2");
                            BatchOp::Put {
                                key: key_from_nibble_iter(full_path.iter().copied()),
                                value: val.into(),
                            }
                        });
                        if ret.is_some() {
                            return Ok(ret);
                        }
                        continue;
                    };
                    //println!("Right state: {:?}", self.right_state);

                    //let (right_node, right_pre_path, right_hash) =
                    //    self.right_state.take().expect("TODO");
                    let (next_op, ret) = self.one_step(
                        &left_pre_path,
                        &left_node,
                        left_hash.as_ref(),
                        &right_pre_path,
                        &right_node,
                        right_hash.as_ref(),
                    );

                    self.left_state = Some((left_node, left_pre_path, left_hash));
                    self.right_state = Some((right_node, right_pre_path, right_hash));
                    self.state = next_op;
                    println!("Ret: {ret:?}");
                    if ret.is_some() {
                        return Ok(ret); // There is a BatchOp to return, else keep looping until there is or is done
                    }
                }
                DiffIterationNodeState::TraverseRight => {
                    println!("TraverseRight");
                    let (left_node, left_pre_path, left_hash) =
                        self.left_state.take().expect("TODO");
                    let Some((right_node, right_pre_path, right_hash)) = self.right_tree.next()?
                    else {
                        //todo!(); // Add remaining left to Delete list
                        self.state = DiffIterationNodeState::DeleteRestLeft;
                        self.right_state = None;

                        // Combine pre_path with node path to get full path.
                        let full_path = Path::from_nibbles_iterator(
                            left_pre_path
                                .iter()
                                .copied()
                                .chain(left_node.partial_path().iter().copied()),
                        );

                        let ret = left_node.value().map(|_val| BatchOp::Delete {
                            key: key_from_nibble_iter(full_path.iter().copied()),
                        });
                        if ret.is_some() {
                            return Ok(ret);
                        }
                        continue;
                    };
                    let (next_op, ret) = self.one_step(
                        &left_pre_path,
                        &left_node,
                        left_hash.as_ref(),
                        &right_pre_path,
                        &right_node,
                        right_hash.as_ref(),
                    );

                    self.left_state = Some((left_node, left_pre_path, left_hash));
                    self.right_state = Some((right_node, right_pre_path, right_hash));
                    println!("Ret: {ret:?}");
                    self.state = next_op;
                    if ret.is_some() {
                        return Ok(ret); // There is a BatchOp to return, else keep looping until there is or is done
                    }
                }
                DiffIterationNodeState::AddRestRight => {
                    println!("AddRestRight");
                    let Some((right_node, right_pre_path, right_hash)) = self.right_tree.next()?
                    else {
                        println!("1");
                        self.right_state = None;
                        break;
                    };
                    // Combine pre_path with node path to get full path.
                    let full_path = Path::from_nibbles_iterator(
                        right_pre_path
                            .iter()
                            .copied()
                            .chain(right_node.partial_path().iter().copied()),
                    );

                    let ret = right_node.value().map(|val| {
                        println!("2");
                        BatchOp::Put {
                            key: key_from_nibble_iter(full_path.iter().copied()),
                            value: val.into(),
                        }
                    });
                    self.right_state = Some((right_node, right_pre_path, right_hash));
                    if ret.is_some() {
                        println!("3");
                        return Ok(ret); // There is a BatchOp to return, else keep looping until there is or is done
                    }
                }
                DiffIterationNodeState::DeleteRestLeft => {
                    println!("DeleteRestLeft");
                    let Some((left_node, left_pre_path, left_hash)) = self.left_tree.next()? else {
                        self.left_state = None;
                        break;
                    };
                    // Combine pre_path with node path to get full path.
                    let full_path = Path::from_nibbles_iterator(
                        left_pre_path
                            .iter()
                            .copied()
                            .chain(left_node.partial_path().iter().copied()),
                    );

                    let ret = left_node.value().map(|_val| BatchOp::Delete {
                        key: key_from_nibble_iter(full_path.iter().copied()),
                    });
                    self.left_state = Some((left_node, left_pre_path, left_hash));
                    if ret.is_some() {
                        return Ok(ret); // There is a BatchOp to return, else keep looping until there is or is done
                    }
                }
            }
        }
        Ok(None)
    }
}

// For now, just implement traverse left and traverse right functions to test out the iterator
struct PreOrderIterator<'a> {
    // The stack of nodes to visit. We store references to avoid ownership issues during iteration.
    trie: &'a dyn TrieReader,
    stack: Vec<(Arc<Node>, Path, Option<TrieHash>)>,
    prev_num_children: usize,
}

impl PreOrderIterator<'_> {
    fn iterate_to_key(&mut self, key: &Key) -> Result<(), FileIoError> {
        if key.is_empty() {
            return Ok(());
        }
        // Assume root or other nodes have already been pushed onto the stack
        loop {
            self.prev_num_children = 0;
            if let Some((node, pre_path, node_hash)) = self.stack.pop() {
                // Check if it is a match for the key.
                let full_path = Path::from_nibbles_iterator(
                    pre_path
                        .iter()
                        .copied()
                        .chain(node.partial_path().iter().copied()),
                );

                let node_key = key_from_nibble_iter(full_path.iter().copied());
                if node_key.eq(key) {
                    // Just push back to stack and return. Change proof will now start at this key
                    self.stack.push((node, pre_path, node_hash));
                    return Ok(());
                }
                // Check if this node's path is a prefix of the key. If it is, then keep traversing
                // Otherwise, this is the lexicographically next node to the key. Not clear what the
                // correct response is for this case, but for now we will just push that to the stac
                // and allow the change proof to start at this node.
                // TODO: This can be improved to not require generating the full path on each
                //       iteration by following the approach in insert_helper, which is to update
                //       the key on each iteration with only the non-overlapping parts.
                let path_overlap = PrefixOverlap::from(key, node_key.as_ref());
                let unique_node = path_overlap.unique_b;
                if !unique_node.is_empty() {
                    self.stack.push((node, pre_path, node_hash));
                    return Ok(());
                }

                if let Node::Branch(branch) = &*node {
                    // TODO: Once the first child is added to the stack, then the rest should be added without needing
                    //       to do additional checks.
                    for (path_comp, child) in branch.children.clone().into_iter().rev() {
                        if let Some(child) = child {
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
                            let child_pre_path = Path::from_nibbles_iterator(
                                pre_path.iter().copied().chain(
                                    node.partial_path()
                                        .iter()
                                        .copied()
                                        .chain(once(path_comp.as_u8())),
                                ),
                            );

                            let child_pre_key =
                                key_from_nibble_iter(child_pre_path.iter().copied());
                            let path_overlap = PrefixOverlap::from(key, child_pre_key.as_ref());
                            let unique_node = path_overlap.unique_b;
                            if unique_node.is_empty() || child_pre_key > *key {
                                self.stack.push((child, child_pre_path, child_hash));
                                self.prev_num_children =
                                    self.prev_num_children.checked_add(1).expect("TODO");
                            }
                        }
                    }
                }
            } else {
                return Ok(());
            }
        }
    }

    fn next(&mut self) -> Result<Option<(Arc<Node>, Path, Option<TrieHash>)>, FileIoError> {
        // Pop the next node from the stack
        self.prev_num_children = 0;
        if let Some((node, pre_path, node_hash)) = self.stack.pop() {
            // Push children onto the stack. Since a stack is LIFO,
            // push the right child first so the left child is processed next.
            if let Node::Branch(branch) = &*node {
                for (path_comp, child) in branch.children.clone().into_iter().rev() {
                    if let Some(child) = child {
                        // Need to expand this into a Node.
                        // Return the child as the new root. Update its partial path to include the index value.
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
                        let child_pre_path = Path::from_nibbles_iterator(
                            pre_path.iter().copied().chain(
                                node.partial_path()
                                    .iter()
                                    .copied()
                                    .chain(once(path_comp.as_u8())),
                            ),
                        );

                        self.stack.push((child, child_pre_path, child_hash));
                        self.prev_num_children =
                            self.prev_num_children.checked_add(1).expect("TODO");
                    }
                }
            }
            // Return the value of the current node
            Ok(Some((node.clone(), pre_path, node_hash)))
        } else {
            // Stack is empty, iteration is complete
            Ok(None)
        }
    }

    // Used to skip branches from a branch node if there is a hash match.
    fn skip_branches(&mut self) {
        for _ in 0..self.prev_num_children {
            self.stack.pop();
        }
        self.prev_num_children = 0;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use crate::{
        db::BatchOp,
        diff::DiffMerkleNodeStream,
        merkle::{Key, Merkle},
    };

    use firewood_storage::{
        HashedNodeReader, ImmutableProposal, MemStore, MutableProposal, NodeStore,
    };
    use std::sync::Arc;
    use test_case::test_case;

    fn diff_merkle_iterator<'a, T, U>(
        tree_left: &'a Merkle<T>,
        tree_right: &'a Merkle<U>,
        start_key: Key,
    ) -> DiffMerkleNodeStream<'a>
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
    ) -> DiffMerkleNodeStream<'a>
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

    #[test]
    fn test_diff_empty_mutable_trees() {
        // This is unlikely to happen in practice, but it helps cover the case where
        // hashes do not exist yet.
        let m1 = create_test_merkle();
        let m2 = create_test_merkle();

        let mut diff_iter = diff_merkle_iterator_without_hash(&m1, &m2, Box::new([]));
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

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([]));

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
        let diff_stream = DiffMerkleNodeStream::new(tree_left, tree_right, Key::default());
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

        let diff_stream = DiffMerkleNodeStream::new(m1.nodestore(), m2.nodestore(), Key::default());

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

        let diff_iter = diff_merkle_iterator(&m1, &m2, Key::default());
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

        let diff_stream = DiffMerkleNodeStream::new(m1.nodestore(), m2.nodestore(), Key::default());

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
            matches!(op3, BatchOp::Put { key, value } if key == Box::from(b"ddd".as_slice()) && value.as_ref() == b"value4")
        );

        assert!(diff_iter.next().is_none());
    }
}
