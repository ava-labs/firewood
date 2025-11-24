// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{
    db::BatchOp,
    iter::key_from_nibble_iter,
    merkle::{Key, Value},
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
    left_tree: DFSIterator<'a>,
    right_tree: DFSIterator<'a>,
    left_state: Option<(Arc<Node>, Path, Option<TrieHash>)>,
    right_state: Option<(Arc<Node>, Path, Option<TrieHash>)>,
    state: DiffIterationNodeState,
}

impl<'a> DiffMerkleNodeStream<'a> {
    fn new<T: TrieReader + HashedNodeReader, U: TrieReader + HashedNodeReader>(
        left_tree: &'a T,
        right_tree: &'a U,
        start_key: Key,
    ) -> Self {
        Self {
            left_tree: Self::dfs_iter(left_tree),
            right_tree: Self::dfs_iter(right_tree),
            left_state: None,
            right_state: None,
            state: DiffIterationNodeState::TraverseBoth,
            //state: DiffNodeStreamState::from(start_key),
        }
    }

    pub fn dfs_iter<V: TrieReader + HashedNodeReader>(tree: &'a V) -> DFSIterator<'a> {
        let root = tree.root_node();
        let hash = tree.root_hash();

        let mut ret = DFSIterator {
            stack: vec![],
            prev_num_children: 0,
            trie: tree,
        };

        if let Some(root) = root {
            ret.stack.push((root.clone(), Path::default(), hash));
        }
        ret
    }

    pub fn one_step(
        &self,
        left_pre_path: &Path,
        left_node: &triomphe::Arc<Node>,
        left_hash: Option<&TrieHash>,
        right_pre_path: &Path,
        right_node: &triomphe::Arc<Node>,
        right_hash: Option<&TrieHash>,
    ) -> (DiffIterationNodeState, Option<BatchOp<Key, Value>>) {
        // Combine pre_path with node path to get full path.
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

        // Compare the partial path of the current node from the left and right tries.
        match left_full_path.cmp(&right_full_path) {
            Ordering::Less => {
                //println!("Less than: {left_full_path:?} ------ {right_full_path:?}");
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

impl DiffMerkleNodeStream<'_> {
    fn next(&mut self) -> Result<Option<BatchOp<Key, Value>>, firewood_storage::FileIoError> {
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
                    let (next_op, ret) =
                        self.one_step(&left_pre_path, &left_node, left_hash.as_ref(), &right_pre_path, &right_node, right_hash.as_ref());

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
                    let (next_op, ret) =
                        self.one_step(&left_pre_path, &left_node, left_hash.as_ref(), &right_pre_path, &right_node, right_hash.as_ref());

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
                    let (next_op, ret) =
                        self.one_step(&left_pre_path, &left_node, left_hash.as_ref(), &right_pre_path, &right_node, right_hash.as_ref());

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
struct DFSIterator<'a> {
    // The stack of nodes to visit. We store references to avoid ownership issues during iteration.
    trie: &'a dyn TrieReader,
    stack: Vec<(Arc<Node>, Path, Option<TrieHash>)>,
    prev_num_children: usize,
}

impl DFSIterator<'_> {
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
    //use test_case::test_case;

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

    /*
    #[test]
    fn test_diff_empty_mutable_trees() {
        // This is unlikely to happen in practice, but it helps cover the case where
        // hashes do not exist yet.
        let m1 = create_test_merkle();
        let m2 = create_test_merkle();

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([]));
        assert!(diff_iter.next().is_none());
    }
    */

    #[test]
    fn test_diff_empty_trees() {
        let m1 = make_immutable(create_test_merkle());
        let m2 = make_immutable(create_test_merkle());

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([]));
        assert!(diff_iter.next().unwrap().is_none());
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
        assert!(diff_iter.next().unwrap().is_none());
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
            matches!(op1, BatchOp::Put { key, value } if key == Box::from(b"key1".as_slice()) && *value == *b"value1")
        );

        let op2 = diff_iter.next().unwrap().unwrap();
        assert!(
            matches!(op2, BatchOp::Put { key, value } if key == Box::from(b"key2".as_slice()) && *value == *b"value2")
        );

        assert!(diff_iter.next().unwrap().is_none());
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

        assert!(diff_iter.next().unwrap().is_none());
    }

    #[test]
    fn test_diff_modifications() {
        let m1 = populate_merkle(create_test_merkle(), &[(b"key1", b"old_value")]);
        let m2 = populate_merkle(create_test_merkle(), &[(b"key1", b"new_value")]);

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([]));

        let op = diff_iter.next().unwrap().unwrap();
        assert!(
            matches!(op, BatchOp::Put { key, value } if key == Box::from(b"key1".as_slice()) && *value == *b"new_value")
        );

        assert!(diff_iter.next().unwrap().is_none());
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
            matches!(op2, BatchOp::Put { key, value } if key == Box::from(b"key2".as_slice()) && *value == *b"new_value")
        );

        let op3 = diff_iter.next().unwrap().unwrap();
        assert!(matches!(op3, BatchOp::Delete { key } if key == Box::from(b"key3".as_slice())));

        let op4 = diff_iter.next().unwrap().unwrap();
        assert!(
            matches!(op4, BatchOp::Put { key, value } if key == Box::from(b"key4".as_slice()) && *value == *b"value4")
        );

        assert!(diff_iter.next().unwrap().is_none());
    }
}
