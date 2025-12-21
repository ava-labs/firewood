// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::iter::once;

use firewood_storage::{
    Child, FileIoError, HashedNodeReader, Node, NodeReader, Path, SharedNode, TrieHash,
    firewood_counter,
};
use lender::{Lender, Lending};

use crate::{
    iter::key_from_nibble_iter,
    merkle::{Key, PrefixOverlap},
};

/// Contains all of a node's info that is needed for node comparison in `DiffMerkleNodeStream`.
/// It includes the nodes full path and its hash if available.
#[cfg_attr(not(test), expect(dead_code))]
#[derive(Clone, Debug)]
struct ComparableNodeInfo {
    path: Path,
    node: SharedNode,
    hash: Option<TrieHash>,
}

impl ComparableNodeInfo {
    // Creates a `ComparableNodeInfo` from a `Child`, its pre-path, and the trie that the `Child`
    // is from for reading the node from storage.
    fn new<T: NodeReader>(pre_path: Path, child: &Child, trie: &T) -> Result<Self, FileIoError> {
        let node = child.as_shared_node(trie)?;
        // We need the full path as the diff algorithm compares the full paths of the current
        // nodes of the two tries.
        let mut full_path = pre_path;
        full_path.extend(node.partial_path().iter().copied());

        Ok(Self {
            path: full_path,
            node,
            hash: child.hash().map(|hash| hash.clone().into_triehash()),
        })
    }
}

/// Contains the state required for performing pre-order iteration of a Merkle trie. This includes
/// the `ComparableNodeInfo` of the current node, a reference to a trie that implements the
/// `HashedNodeReader` trait, and a stack that contains the `ComparableNodeInfo` of nodes to be
/// traversed with calls to `next` or `next_internal`.
struct PreOrderIterator<'a, T: HashedNodeReader> {
    node_info: Option<ComparableNodeInfo>,
    trie: &'a T,
    traversal_stack: Vec<ComparableNodeInfo>,
}

/// Implementation for the Lender trait, a lending iterator. A lending iterator rather than a
/// normal iterator is necessary since we want to return a reference to the current
/// `ComparableNodeInfo` that is inside `PreOrderIterator`.
impl<'lend, T: HashedNodeReader> Lending<'lend> for PreOrderIterator<'_, T> {
    type Lend = Result<&'lend ComparableNodeInfo, FileIoError>;
}

impl<T: HashedNodeReader> Lender for PreOrderIterator<'_, T> {
    fn next(&mut self) -> Option<Result<&'_ ComparableNodeInfo, FileIoError>> {
        self.next_internal().transpose()
    }
}

impl<'a, T: HashedNodeReader> PreOrderIterator<'a, T> {
    /// Create a pre-order iterator for the trie that starts at `start_key`.
    #[cfg_attr(not(test), expect(dead_code))]
    fn new(trie: &'a T, start_key: &Key) -> Result<PreOrderIterator<'a, T>, FileIoError> {
        let mut preorder = Self {
            node_info: None,
            traversal_stack: vec![],
            trie,
        };
        // If the root node is not None, then push a `ComparableNodeInfo` for the root onto
        // the traversal stack. It will be used on the first call to `next` or `next_internal`.
        // Because we already have the root node, we create its `ComparableNodeInfo` directly
        // instead of using `ComparableNodeInfo::new` as we don't need to create a `SharedNode`
        // from a `Child`. The full path of the root node is just its partial path since it has
        // no pre-path.
        if let Some(root) = trie.root_node() {
            preorder.traversal_stack.push(ComparableNodeInfo {
                path: root.partial_path().clone(),
                node: root,
                hash: trie.root_hash().map(|hash| hash.clone().into_triehash()),
            });
        }
        preorder.iterate_to_key(start_key)?;
        Ok(preorder)
    }

    /// In a textbook implementation of pre-order traversal we would normally pop off the next node
    /// from the traversal stack and push its children onto the stack before returning. However, that
    /// would be inefficient if we want to skip traversing a node's children if its hash matches that
    /// of a node in a different trie, as we would then need to pop its children off the traversal
    /// stack.
    ///
    /// This implementation flips the order such that we don't push a node's children onto the
    /// traversal stack until the subseqent call to `next` or `next_internal`. This is done by saving
    /// the `ComparableNodeInfo` of the current node in `node_info` and keeping that available for the
    /// next call. Skipping traversal of the children then just involves setting `node_info` to None.
    fn next_internal(&mut self) -> Result<Option<&ComparableNodeInfo>, FileIoError> {
        firewood_counter!(
            "firewood.change_proof.next",
            "number of next calls to calculate a change proof"
        )
        .increment(1);

        // Take the info of the current node (which will soon be replaced), and check if it is a
        // branch. If it is, add its children onto the traversal stack.
        if let Some(prev_node_info) = self.node_info.take()
            && let Node::Branch(branch) = &*prev_node_info.node
        {
            // Since a stack is LIFO and we want to perform pre-order traversal, we pushed the
            // children in reverse order.
            for (path_comp, child) in branch.children.iter_present().rev() {
                // Generate the pre-path for this child, and push it onto the traversal stack.
                let child_pre_path = Path::from_nibbles_iterator(
                    prev_node_info
                        .path
                        .iter()
                        .copied()
                        .chain(once(path_comp.as_u8())),
                );
                self.traversal_stack.push(ComparableNodeInfo::new(
                    child_pre_path,
                    child,
                    self.trie,
                )?);
            }
        }

        // Pop a `ComparableNodeInfo` from the traversal stack and set it as the current node's
        // info. If the stack is empty, then return None and the iteration is complete. Otherwise
        // return a reference to the current node's info.
        self.node_info = self.traversal_stack.pop();
        Ok(self.node_info.as_ref())
    }

    /// Calling `skip_children` will clear the current node's info, which will cause the children
    /// of the current node to not be added to the traversal stack on the subsequent `next` or
    /// `next_internal` call. This will effectively cause the traversal to skip the children of
    /// the current node.
    #[cfg_attr(not(test), expect(dead_code))]
    fn skip_children(&mut self) {
        self.node_info = None;
    }

    /// Modify the iterator to skip all keys prior to the specified key.
    fn iterate_to_key(&mut self, key: &Key) -> Result<(), FileIoError> {
        // Function is a no-op if the key is empty.
        if key.is_empty() {
            return Ok(());
        }
        // Keep iterating until we have reached the start key. Only traverse branches that can
        // contain the start key.
        loop {
            // Push the children that can contain the keys that are larger than or equal to the
            // the start key onto the traversal stack.
            if let Some(prev_node_info) = self.node_info.take()
                && let Node::Branch(branch) = &*prev_node_info.node
            {
                // Create a reverse iterator on the children that includes the child's pre-path.
                let mut reversed_children_with_pre_path =
                    branch
                        .children
                        .iter_present()
                        .rev()
                        .map(|(path_comp, child)| {
                            (
                                child,
                                Path::from_nibbles_iterator(
                                    prev_node_info
                                        .path
                                        .iter()
                                        .copied()
                                        .chain(once(path_comp.as_u8())),
                                ),
                            )
                        });

                for (child, child_pre_path) in reversed_children_with_pre_path.by_ref() {
                    // We only need to traverse this child if its pre-path is a prefix of the key (including
                    // being equal to the key) or is lexicographically larger than the key.
                    let child_key = key_from_nibble_iter(child_pre_path.iter().copied());
                    let path_overlap = PrefixOverlap::from(key, child_key.as_ref());
                    let unique_node = path_overlap.unique_b;
                    if unique_node.is_empty() || child_key > *key {
                        self.traversal_stack.push(ComparableNodeInfo::new(
                            child_pre_path,
                            child,
                            self.trie,
                        )?);
                        // Once we have found the first child that should be traversed, we can break out of
                        // the loop where we we add the rest of the children without the above test.
                        break;
                    }
                }

                // Add the rest of the children (if any) to the traversal stack.
                for node_info in reversed_children_with_pre_path.map(|(child, child_pre_path)| {
                    ComparableNodeInfo::new(child_pre_path, child, self.trie)
                }) {
                    self.traversal_stack.push(node_info?);
                }
            }

            // Pop the next node's info from the stack
            if let Some(node_info) = self.traversal_stack.pop() {
                // Since pre-order traversal of a trie iterates through the nodes in lexicographical
                // order, we can stop the traversal once we see a node key that is larger than or
                // equal to the key. We stop the traversal by pushing the current `ComparableNodeInfo`
                // back to the stack. Calling `next` or `next_internal` will process this node.
                let node_key = key_from_nibble_iter(node_info.path.iter().copied());
                if node_key >= *key {
                    self.traversal_stack.push(node_info);
                    return Ok(());
                }

                // If this node is a leaf, then we don't need to save its info to `node_info` as
                // it has no children to traverse.
                if let Node::Branch(_branch) = &*node_info.node {
                    // Check if this node's path is a prefix of the key. If it is not (`unique_node`
                    // is not empty), then this node's children cannot be larger than or equal to
                    // the key, and we don't need to include them on the traversal stack.
                    let path_overlap = PrefixOverlap::from(key, node_key.as_ref());
                    let unique_node = path_overlap.unique_b;
                    if unique_node.is_empty() {
                        self.node_info = Some(node_info);
                    }
                }
            } else {
                // Traversal stack is empty. This means the key is lexicographically larger than
                // all of the keys in the trie. Calling `next` or `next_internal` will return None.
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::arithmetic_side_effects,
    clippy::type_complexity
)]
mod tests {
    use crate::{
        db::BatchOp,
        iter::key_from_nibble_iter,
        merkle::{Key, Merkle, changes::PreOrderIterator},
    };

    use firewood_storage::{
        HashedNodeReader, ImmutableProposal, MemStore, MutableProposal, NodeStore, SeededRng,
    };
    use lender::Lender;
    use std::{collections::HashSet, sync::Arc};

    fn create_test_merkle() -> Merkle<NodeStore<MutableProposal, MemStore>> {
        let memstore = MemStore::new(vec![]);
        let nodestore = NodeStore::new_empty_proposal(Arc::new(memstore));
        Merkle::from(nodestore)
    }

    fn make_immutable(
        merkle: Merkle<NodeStore<MutableProposal, MemStore>>,
    ) -> Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> {
        merkle.try_into().unwrap()
    }

    fn random_key_from_hashset<'a>(
        rng: &'a SeededRng,
        set: &'a HashSet<Vec<u8>>,
    ) -> Option<Vec<u8>> {
        let len = set.len();
        if len == 0 {
            return None;
        }
        // Generate a random index within the bounds of the set's size
        let index = rng.random_range(0..len);
        // Use the iterator's nth method to get the element at that index
        set.iter().nth(index).cloned()
    }

    fn gen_delete_key(
        rng: &SeededRng,
        committed_keys: &HashSet<Vec<u8>>,
        seen_keys: &mut HashSet<Vec<u8>>,
    ) -> Option<Vec<u8>> {
        let key_opt = random_key_from_hashset(rng, committed_keys);
        // Just skip the key if it has been seen. Otherwise return it and add the key
        // to seen keys.
        key_opt
            .filter(|del_key| !seen_keys.contains(del_key))
            .inspect(|del_key| {
                seen_keys.insert(del_key.clone());
            })
    }

    fn gen_random_keys(
        rng: &SeededRng,
        committed_keys: &HashSet<Vec<u8>>,
        num_keys: usize,
        start_val: usize,
    ) -> (Vec<BatchOp<Vec<u8>, Box<[u8]>>>, usize) {
        const CHANCE_DELETE: usize = 2;
        let mut seen_keys = std::collections::HashSet::new();
        let mut i = 0;
        let mut batch = Vec::new();

        while batch.len() < num_keys {
            if !committed_keys.is_empty() && rng.random_range(1..=100) <= CHANCE_DELETE {
                let del_key = gen_delete_key(rng, committed_keys, &mut seen_keys);
                if let Some(key) = del_key {
                    batch.push(BatchOp::Delete { key });
                    continue;
                }
                // If we couldn't generate a delete key, then just fall through and create
                // a BatchOp::Put.
            }
            let key_len = rng.random_range(1..=32);
            let key: Vec<u8> = (0..key_len).map(|_| rng.random()).collect();
            // Only add if key is unique
            if seen_keys.insert(key.clone()) {
                batch.push(BatchOp::Put {
                    key,
                    value: Box::from(format!("value{}", i + start_val).as_bytes()),
                });
            }
            i += 1;
        }
        (batch, i + start_val)
    }

    #[test]
    fn test_preorder_iterator() {
        let rng = firewood_storage::SeededRng::from_env_or_random();
        let (batch, _) = gen_random_keys(&rng, &HashSet::new(), 1000, 0);

        // Keep a sorted copy of the batch.
        let mut batch_sorted = batch.clone();
        batch_sorted.sort_by_key(|op| op.key().clone());

        // Insert batch into a merkle trie.
        let mut merkle = create_test_merkle();
        for item in &batch {
            // All of the ops should be Puts
            merkle
                .insert(
                    item.key(),
                    item.value().unwrap().to_vec().into_boxed_slice(),
                )
                .unwrap();
        }
        let merkle = make_immutable(merkle);
        assert!(merkle.nodestore().root_hash().is_some());

        // Check if the sorted batch and the pre-order traversal have identical values.
        let mut preorder_it = PreOrderIterator::new(merkle.nodestore(), &Key::default()).unwrap();
        let mut batch_sorted_it = batch_sorted.clone().into_iter();
        while let Some(node_info) = preorder_it.next() {
            let node_info = node_info.unwrap();
            assert!(node_info.hash.is_some());
            if let Some(val) = node_info.node.value() {
                let key = key_from_nibble_iter(node_info.path.iter().copied());
                let batch_sorted_item = batch_sorted_it.next().unwrap();
                assert!(
                    *key == *batch_sorted_item.key().as_slice()
                        && *val == **batch_sorted_item.value().unwrap()
                );
            }
        }
        assert!(batch_sorted_it.next().is_none());

        // Second test where we pick a random key from the sorted batch as the start key, and check
        // the sorted batch and pre-order traversal have identical values when using that start key.
        let mut index = rng.random_range(0..batch_sorted.len());
        let start_key = batch_sorted
            .get(index)
            .unwrap()
            .key()
            .clone()
            .into_boxed_slice();
        let mut preorder_it = PreOrderIterator::new(merkle.nodestore(), &start_key).unwrap();
        while let Some(node_info) = preorder_it.next() {
            let node_info = node_info.unwrap();
            if let Some(val) = node_info.node.value() {
                let key = key_from_nibble_iter(node_info.path.iter().copied());
                let batch_sorted_item = batch_sorted.get(index).unwrap();
                assert!(
                    *key == *batch_sorted_item.key().as_slice()
                        && *val == **batch_sorted_item.value().unwrap()
                );
                index += 1;
            }
        }
        assert!(index == batch_sorted.len());

        // Third test that just skips the children after calling `next` once. This should skip all of
        // the children of the root node, causing the next call to `next` to return None.
        let mut preorder_it = PreOrderIterator::new(merkle.nodestore(), &Key::default()).unwrap();
        let _ = preorder_it.next().unwrap();
        preorder_it.skip_children();
        assert!(preorder_it.next().is_none());

        // Fourth test that uses the Iterator trait to count the number of nodes in the trie
        let preorder_it = PreOrderIterator::new(merkle.nodestore(), &Key::default()).unwrap();
        assert!(preorder_it.count() > 0);
    }
}
