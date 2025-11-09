// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod try_extend;

pub(crate) use self::try_extend::TryExtend;
use crate::merkle::{Key, Value};
use crate::v2::api;

use firewood_storage::{
    BranchNode, Child, FileIoError, NibblesIterator, Node, PathBuf, PathComponent, PathIterItem,
    SharedNode, TriePathFromUnpackedBytes, TrieReader,
};
use std::cmp::Ordering;
use std::iter::FusedIterator;

/// Represents an ongoing iteration over a node and its children.
pub(crate) enum IterationNode {
    /// This node has not been returned yet.
    Unvisited {
        /// The key (as nibbles) of this node.
        key: Key,
        node: SharedNode,
    },
    /// This node has been returned. Track which child to visit next.
    Visited {
        /// The key (as nibbles) of this node.
        key: Key,
        /// Returns the non-empty children of this node and their positions
        /// in the node's children array.
        children_iter: Box<dyn Iterator<Item = (PathComponent, Child)> + Send>,
    },
}

impl std::fmt::Debug for IterationNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unvisited { key, node } => f
                .debug_struct("Unvisited")
                .field("key", key)
                .field("node", node)
                .finish(),
            Self::Visited {
                key,
                children_iter: _,
            } => f.debug_struct("Visited").field("key", key).finish(),
        }
    }
}

#[derive(Debug)]
pub(crate) enum NodeIterState {
    /// The iterator state is lazily initialized when `poll_next` is called
    /// for the first time. The iteration start key is stored here.
    StartFromKey(Key),
    Iterating {
        /// Each element is a node that will be visited (i.e. returned)
        /// or has been visited but has unvisited children.
        /// On each call to `poll_next` we pop the next element.
        /// If it's unvisited, we visit it.
        /// If it's visited, we push its next child onto this stack.
        iter_stack: Vec<IterationNode>,
    },
}

#[derive(Debug)]
/// An iterator of nodes in order starting from a specific point in the trie.
pub struct MerkleNodeIter<'a, T> {
    state: NodeIterState,
    merkle: &'a T,
}

impl From<Key> for NodeIterState {
    fn from(key: Key) -> Self {
        Self::StartFromKey(key)
    }
}

impl<'a, T: TrieReader> MerkleNodeIter<'a, T> {
    /// Returns a new iterator that will iterate over all the nodes in `merkle`
    /// with keys greater than or equal to `key`.
    pub(super) fn new(merkle: &'a T, key: Key) -> Self {
        Self {
            state: NodeIterState::from(key),
            merkle,
        }
    }
}

impl<T: TrieReader> Iterator for MerkleNodeIter<'_, T> {
    type Item = Result<(Key, SharedNode), FileIoError>;

    fn next(&mut self) -> Option<Self::Item> {
        'outer: loop {
            match &mut self.state {
                NodeIterState::StartFromKey(key) => {
                    match get_iterator_intial_state(self.merkle, key) {
                        Ok(state) => self.state = state,
                        Err(e) => return Some(Err(e)),
                    }
                }
                NodeIterState::Iterating { iter_stack } => {
                    while let Some(mut iter_node) = iter_stack.pop() {
                        match iter_node {
                            IterationNode::Unvisited { key, node } => {
                                match &*node {
                                    Node::Leaf(_) => {}
                                    Node::Branch(branch) => {
                                        // `node` is a branch node. Visit its children next.
                                        iter_stack.push(IterationNode::Visited {
                                            key: key.clone(),
                                            children_iter: Box::new(as_enumerated_children_iter(
                                                branch,
                                            )),
                                        });
                                    }
                                }

                                let key = key_from_nibble_iter(key.iter().copied());
                                return Some(Ok((key, node)));
                            }
                            IterationNode::Visited {
                                ref key,
                                ref mut children_iter,
                            } => {
                                // We returned `node` already. Visit its next child.
                                let Some((pos, child)) = children_iter.next() else {
                                    // We visited all this node's descendants. Go back to its parent.
                                    continue;
                                };

                                let child = match child {
                                    Child::AddressWithHash(addr, _) => {
                                        match self.merkle.read_node(addr) {
                                            Ok(node) => node,
                                            Err(e) => return Some(Err(e)),
                                        }
                                    }
                                    Child::Node(node) => node.clone().into(),
                                    Child::MaybePersisted(maybe_persisted, _) => {
                                        // For MaybePersisted, we need to get the node
                                        match maybe_persisted.as_shared_node(self.merkle) {
                                            Ok(node) => node,
                                            Err(e) => return Some(Err(e)),
                                        }
                                    }
                                };

                                let child_partial_path = child.partial_path().iter().copied();

                                // The child's key is its parent's key, followed by the child's index,
                                // followed by the child's partial path (if any).
                                let child_key: Key = key
                                    .iter()
                                    .copied()
                                    .chain(Some(pos.as_u8()))
                                    .chain(child_partial_path)
                                    .collect();

                                // There may be more children of this node to visit.
                                // Visit it again after visiting its `child`.
                                iter_stack.push(iter_node);

                                iter_stack.push(IterationNode::Unvisited {
                                    key: child_key,
                                    node: child,
                                });

                                continue 'outer;
                            }
                        }
                    }

                    return None;
                }
            }
        }
    }
}

impl<T: TrieReader> FusedIterator for MerkleNodeIter<'_, T> {}

/// Returns the initial state for an iterator over the given `merkle` which starts at `key`.
pub(crate) fn get_iterator_intial_state<T: TrieReader>(
    merkle: &T,
    key: &[u8],
) -> Result<NodeIterState, FileIoError> {
    let Some(root) = merkle.root_node() else {
        // This merkle is empty.
        return Ok(NodeIterState::Iterating { iter_stack: vec![] });
    };
    let mut node = root;

    // Invariant: `matched_key_nibbles` is the path before `node`'s
    // partial path at the start of each loop iteration.
    let mut matched_key_nibbles = vec![];

    let mut unmatched_key_nibbles = NibblesIterator::new(key);

    let mut iter_stack: Vec<IterationNode> = vec![];

    loop {
        // See if `node`'s key is a prefix of `key`.
        let partial_path = node.partial_path();

        let (comparison, new_unmatched_key_nibbles) =
            compare_partial_path(partial_path.iter(), unmatched_key_nibbles);
        unmatched_key_nibbles = new_unmatched_key_nibbles;

        matched_key_nibbles.extend(partial_path.iter());

        match comparison {
            Ordering::Less => {
                // `node` is before `key`. It shouldn't be visited
                // and neither should its descendants.
                return Ok(NodeIterState::Iterating { iter_stack });
            }
            Ordering::Greater => {
                // `node` is after `key`. Visit it first.
                iter_stack.push(IterationNode::Unvisited {
                    key: Box::from(matched_key_nibbles),
                    node,
                });
                return Ok(NodeIterState::Iterating { iter_stack });
            }
            Ordering::Equal => match &*node {
                Node::Leaf(_) => {
                    iter_stack.push(IterationNode::Unvisited {
                        key: matched_key_nibbles.clone().into_boxed_slice(),
                        node,
                    });
                    return Ok(NodeIterState::Iterating { iter_stack });
                }
                Node::Branch(branch) => {
                    let Some(next_unmatched_key_nibble) = unmatched_key_nibbles.next() else {
                        // There is no more key to traverse.
                        iter_stack.push(IterationNode::Unvisited {
                            key: matched_key_nibbles.clone().into_boxed_slice(),
                            node,
                        });

                        return Ok(NodeIterState::Iterating { iter_stack });
                    };
                    let next_unmatched_key_nibble =
                        PathComponent::try_new(next_unmatched_key_nibble).expect("valid nibble");

                    // There is no child at `next_unmatched_key_nibble`.
                    // We'll visit `node`'s first child at index > `next_unmatched_key_nibble`
                    // first (if it exists).
                    iter_stack.push(IterationNode::Visited {
                        key: matched_key_nibbles.clone().into_boxed_slice(),
                        children_iter: Box::new(
                            as_enumerated_children_iter(branch)
                                .filter(move |(pos, _)| *pos > next_unmatched_key_nibble),
                        ),
                    });

                    let child = &branch.children[next_unmatched_key_nibble];
                    node = match child {
                        None => return Ok(NodeIterState::Iterating { iter_stack }),
                        Some(Child::AddressWithHash(addr, _)) => merkle.read_node(*addr)?,
                        Some(Child::Node(node)) => (*node).clone().into(), // TODO can we avoid cloning this?
                        Some(Child::MaybePersisted(maybe_persisted, _)) => {
                            // For MaybePersisted, we need to get the node
                            maybe_persisted.as_shared_node(merkle)?
                        }
                    };

                    matched_key_nibbles.push(next_unmatched_key_nibble.as_u8());
                }
            },
        }
    }
}

#[derive(Debug)]
/// An iterator of key-value pairs in order starting from a specific point in the trie.
pub struct MerkleKeyValueIter<'a, T> {
    iter: MerkleNodeIter<'a, T>,
}

impl<'a, T: TrieReader> From<&'a T> for MerkleKeyValueIter<'a, T> {
    fn from(merkle: &'a T) -> Self {
        Self {
            iter: MerkleNodeIter::new(merkle, Box::new([])),
        }
    }
}

impl<'a, T: TrieReader> MerkleKeyValueIter<'a, T> {
    /// Construct a [`MerkleKeyValueIter`] that will iterate over all the key-value pairs in `merkle`
    /// starting from a particular key
    pub fn from_key<K: AsRef<[u8]>>(merkle: &'a T, key: K) -> Self {
        Self {
            iter: MerkleNodeIter::new(merkle, key.as_ref().into()),
        }
    }
}

impl<T: TrieReader> Iterator for MerkleKeyValueIter<'_, T> {
    type Item = Result<(Key, Value), api::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.find_map(|result| {
            result
                .map(|(key, node)| {
                    match &*node {
                        Node::Branch(branch) => {
                            let Some(value) = branch.value.as_ref() else {
                                // This node doesn't have a value to return.
                                // Continue to the next node.
                                return None;
                            };
                            Some((key, value.clone()))
                        }
                        Node::Leaf(leaf) => Some((key, leaf.value.clone())),
                    }
                })
                .map_err(Into::into)
                .transpose()
        })
    }
}

impl<T: TrieReader> FusedIterator for MerkleKeyValueIter<'_, T> {}

#[derive(Debug)]
enum PathIteratorState<'a> {
    Iterating {
        /// The key, as nibbles, of the node at `address`, without the
        /// node's partial path (if any) at the end.
        /// Invariant: If this node has a parent, the parent's key is a
        /// prefix of the key we're traversing to.
        /// Note the node at `address` may not have a key which is a
        /// prefix of the key we're traversing to.
        matched_key: Vec<u8>,
        unmatched_key: NibblesIterator<'a>,
        node: SharedNode,
    },
    Exhausted,
}

#[derive(Debug)]
/// An iterator of nodes on the path from the root to either a key, or the node immediately
/// preceding it in the lexicographic traversal order (i.e. the node for which any descendant's
/// key is an immediate predecessor to `key`).
pub struct PathIterator<'a, T: TrieReader> {
    merkle: &'a T,
    state: PathIteratorState<'a>,
}

impl<T: TrieReader> PathIterator<'_, T> {
    pub fn from_key(merkle: &T, key: &[u8]) -> Result<Self, FileIoError> {
        if let Some(root) = merkle.root_node() {
            Ok(Self {
                merkle,
                state: PathIteratorState::Iterating {
                    matched_key: vec![],
                    unmatched_key: NibblesIterator::new(key),
                    node: root,
                },
            })
        } else {
            Ok(Self {
                merkle,
                state: PathIteratorState::Exhausted,
            })
        }
    }
}

impl<T: TrieReader> Iterator for PathIterator<'_, T> {
    type Item = Result<PathIterItem, FileIoError>;

    fn next(&mut self) -> Option<Self::Item> {
        // Invariant: `node_key` is `node`'s key without its
        // partial path (if any) at the end at the start of each call.
        let (merkle, state) = (&self.merkle, &mut self.state);
        match state {
            PathIteratorState::Exhausted => None,
            PathIteratorState::Iterating {
                matched_key,
                unmatched_key,
                node,
            } => loop {
                match node.as_ref() {
                    Node::Leaf(node) => {
                        let (partial, is_leaf_key_prefix_of_query_key) = {
                            let partial = node.partial_path.iter().copied();
                            let mut iter_copy = unmatched_key.clone();
                            let (_, new_unmatched) =
                                compare_partial_path(partial.clone(), iter_copy);
                            (partial, new_unmatched.len() == 0)
                        };

                        let key: PathBuf = matched_key.iter().copied().chain(partial).collect();
                        let next_nibble = is_leaf_key_prefix_of_query_key
                            .then_some(unmatched_key.next().and_then(|n| n.try_into().ok()))
                            .flatten();

                        return Some(Ok(PathIterItem {
                            key_nibbles: key,
                            node: node.clone().into(),
                            next_nibble,
                        }));
                    }
                    Node::Branch(node) => {
                        // If `node` matches `unmatched_key` up until `next_unmatched_key_nibble`,
                        // then we construct 2 merges as follows:
                        //   Path A: root to node (inode)
                        //   Path B: root to predecessor of next_unmatched_key_nibble (jnode)
                        // if exists.
                        // Then this function is done.
                        let (node_key, is_node_key_prefix_of_query_key) = {
                            let partial = node.partial_path.iter().copied();
                            let mut iter_copy = unmatched_key.clone();
                            let (_, new_unmatched) =
                                compare_partial_path(partial.clone(), iter_copy);
                            (
                                matched_key.iter().copied().chain(partial).collect(),
                                new_unmatched.len() == 0,
                            )
                        };

                        if is_node_key_prefix_of_query_key {
                            // `node` is a predecessor of `key` or equal to it
                            // It needs to be returned first
                            let next_unmatched_key_nibble =
                                PathComponent::try_new(unmatched_key.next().expect("nibble")).expect("valid nibble");
                            let saved_node = node.clone().into();
                            let saved_node_key = node_key.clone();
                            if let Some(Some(child)) =
                                node.children.get(next_unmatched_key_nibble).copied()
                            {
                                *state = PathIteratorState::Iterating {
                                    matched_key: matched_key
                                        .iter()
                                        .copied()
                                        .chain(node.partial_path.iter().copied())
                                        .chain(Some(next_unmatched_key_nibble.as_u8()))
                                        .collect(),
                                    unmatched_key: unmatched_key.clone(),
                                    node: match child {
                                        Child::Node(child) => child.clone().into(),
                                        Child::AddressWithHash(addr, _) => merkle.read_node(addr).expect("read node"),
                                        Child::MaybePersisted(maybe, _) => maybe.as_shared_node(merkle).expect("maybe persisted"),
                                    },
                                };

                                return Some(Ok(PathIterItem {
                                    key_nibbles: saved_node_key,
                                    node: saved_node,
                                    next_nibble: Some(next_unmatched_key_nibble),
                                }));
                            }

                            // Child doesn't exist -> find predecessor
                            // loop over node.children in reverse order to find j
                            let j = node
                                .children
                                .clone()
                                .into_iter()
                                .filter_map(|(pos, child)| child.map(|_| pos))
                                .rev()
                                .find(|pos| *pos < next_unmatched_key_nibble);
                            match j {
                                None => {
                                    // No predecessor exists -> inode is the predecessor for the entire subtree.
                                    *state = PathIteratorState::Exhausted;
                                    return Some(Ok(PathIterItem {
                                        key_nibbles: node_key,
                                        node: node.clone().into(),
                                        next_nibble: Some(next_unmatched_key_nibble),
                                    }));
                                }
                                Some(j) => {
                                    // Walk into the subtree on `j` until it reaches that subtree's successor.
                                    let mut node = match &node.children[j] {
                                        Some(Child::Node(child)) => child.clone().into(),
                                        Some(Child::AddressWithHash(addr, _)) => merkle.read_node(*addr).expect("read node"),
                                        Some(Child::MaybePersisted(maybe, _)) => maybe.as_shared_node(merkle).expect("maybe persisted"),
                                        None => unreachable!("j is always set to a child that exists"),
                                    };

                                    loop {
                                        match node.as_ref() {
                                            Node::Leaf(leaf) => {
                                                *state = PathIteratorState::Exhausted;
                                                return Some(Ok(PathIterItem {
                                                    key_nibbles: matched_key
                                                        .iter()
                                                        .copied()
                                                        .chain(node_key.iter().copied())
                                                        .chain(std::iter::once(j.as_u8()))
                                                        .chain(leaf.partial_path.iter().copied())
                                                        .collect(),
                                                    node,
                                                    next_nibble: None,
                                                }));
                                            }
                                            Node::Branch(branch) => {
                                                // if this branch has a value, then we can return it immediately
                                                if branch.value.is_some() {
                                                    let saved_node = node.clone();
                                                    let exact_key = matched_key
                                                        .iter()
                                                        .copied()
                                                        .chain(node_key.iter().copied())
                                                        .chain(std::iter::once(j.as_u8()))
                                                        .chain(branch.partial_path.iter().copied())
                                                        .collect();
                                                    *state = PathIteratorState::Iterating {
                                                        matched_key: exact_key.clone().into_vec(),
                                                        unmatched_key: NibblesIterator::new(&[]),
                                                        node,
                                                    };
                                                    return Some(Ok(PathIterItem {
                                                        key_nibbles: exact_key,
                                                        node: saved_node,
                                                        next_nibble: None,
                                                    }));
                                                }

                                                // If there is no value in this branch, we want the last child in the subtree.
                                                // loop over branch.children in reverse order to find the last child.
                                                if let Some((pos, child)) = branch
                                                    .children
                                                    .clone()
                                                    .into_iter()
                                                    .filter_map(|(pos, child)| child.map(|child| (pos, child)))
                                                    .rev()
                                                    .next()
                                                {
                                                    let saved_node = node.clone();
                                                    let node_key = matched_key
                                                        .iter()
                                                        .copied()
                                                        .chain(node_key.iter().copied())
                                                        .chain(std::iter::once(j.as_u8()))
                                                        .chain(branch.partial_path.iter().copied())
                                                        .collect();

                                                    let child = match child {
                                                        Child::Node(child) => child.clone().into(),
                                                        Child::AddressWithHash(addr, _) => merkle.read_node(addr).expect("read node"),
                                                        Child::MaybePersisted(maybe, _) => maybe.as_shared_node(merkle).expect("maybe persisted"),
                                                    };

                                                    matched_key.extend(node_key.iter().copied());
                                                    matched_key.push(pos.as_u8());
                                                    *node = child;

                                                    Some(Ok(PathIterItem {
                                                        key_nibbles: node_key,
                                                        node: saved_node,
                                                        next_nibble: Some(pos),
                                                    }))
                                                } else {
                                                    unreachable!("the branch at `node_key` must have at least one child");
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                            // `node` does not match `unmatched_key_nibbles`. This implies that `node` is either
                            // lexicographically smaller or larger than the key.
                            // Continuing reading path for predecessor of `key`.
                            let saved_node = node.clone().into();
                            let next_unmatched_key_nibble =
                                PathComponent::try_new(unmatched_key.next().expect("nibble")).expect("valid nibble");
                            let saved_key = matched_key
                                .iter()
                                .copied()
                                .chain(node.partial_path.iter().copied())
                                .collect();

                            let (comparison, new_unmatched) = compare_partial_path(
                                node.partial_path.iter(),
                                unmatched_key.clone(),
                            );
                            let child_result = match comparison {
                                Ordering::Less => {
                                    // Found the predecessor. Keep walking down the predecessor.
                                    // The predecessor will either be a matching child, if exists, or
                                    // j = largest child that is < `next_unmatched_key_nibble`.
                                    unmatched_key.clone().next();

                                    let mut pos = next_unmatched_key_nibble;
                                    'lp: loop {
                                        pos = PathComponent::try_new(pos.as_u8().saturating_sub(1)).expect("valid nibble");
                                        if pos.as_u8() == 0 {
                                            break 'lp;
                                        }
                                        if let Some(Some(child)) = node.children.get(pos).copied() {
                                            break Some((pos, child));
                                        }
                                    }
                                }
                                Ordering::Greater => {
                                    // Next step to find the predecessor of `node`.
                                    // j = largest child that is < `next_unmatched_key_nibble`.
                                    let mut pos = PathComponent::try_new(0x10).expect("valid nibble");
                                    'lp: loop {
                                        pos = PathComponent::try_new(pos.as_u8().saturating_sub(1)).expect("valid nibble");
                                        if pos.as_u8() == 0 {
                                            break 'lp;
                                        }
                                        if let Some(Some(child)) = node.children.get(pos).copied() {
                                            break Some((pos, child));
                                        }
                                    }
                                }
                                Ordering::Equal => unreachable!("covered in is_node_key_prefix_of_query_key"),
                            };

                            if let Some((j, child)) = child_result {
                                // Walk into the subtree on `j` until it reaches that subtree's successor.
                                let child = match child {
                                    Child::Node(child) => child.clone().into(),
                                    Child::AddressWithHash(addr, _) => merkle.read_node(addr).expect("read node"),
                                    Child::MaybePersisted(maybe, _) => maybe.as_shared_node(merkle).expect("maybe persisted"),
                                };

                                matched_key.push(j.as_u8());
                                *unmatched_key = new_unmatched;
                                *node = child;

                                Some(Ok(PathIterItem {
                                    key_nibbles: saved_key,
                                    node: saved_node,
                                    next_nibble: Some(next_unmatched_key_nibble),
                                }))
                            } else {
                                // Predecessor does not exist.
                                *state = PathIteratorState::Exhausted;
                                Some(Ok(PathIterItem {
                                    key_nibbles: saved_key,
                                    node: saved_node,
                                    next_nibble: Some(next_unmatched_key_nibble),
                                }))
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Takes in an iterator over a node's partial path and an iterator over the
/// unmatched portion of a key.
/// The first returned element is:
/// * [`Ordering::Less`] if the node is before the key.
/// * [`Ordering::Equal`] if the node is a prefix of the key.
/// * [`Ordering::Greater`] if the node is after the key.
///
/// The second returned element is the unmatched portion of the key after the
/// partial path has been matched.
fn compare_partial_path<'a, I1, I2>(
    partial_path_iter: I1,
    mut unmatched_key_nibbles_iter: I2,
) -> (Ordering, I2)
where
    I1: Iterator<Item = &'a u8>,
    I2: Iterator<Item = u8>,
{
    for next_partial_path_nibble in partial_path_iter {
        let Some(next_key_nibble) = unmatched_key_nibbles_iter.next() else {
            return (Ordering::Greater, unmatched_key_nibbles_iter);
        };

        match next_partial_path_nibble.cmp(&next_key_nibble) {
            Ordering::Less => return (Ordering::Less, unmatched_key_nibbles_iter),
            Ordering::Greater => return (Ordering::Greater, unmatched_key_nibbles_iter),
            Ordering::Equal => {}
        }
    }

    (Ordering::Equal, unmatched_key_nibbles_iter)
}

/// Returns an iterator that returns (`pos`,`child`) for each non-empty child of `branch`,
/// where `pos` is the position of the child in `branch`'s children array.
fn as_enumerated_children_iter(
    branch: &BranchNode,
) -> impl Iterator<Item = (PathComponent, Child)> + use<> {
    branch
        .children
        .clone()
        .into_iter()
        .filter_map(|(pos, child)| child.map(|child| (pos, child)))
}

#[cfg(feature = "branch_factor_256")]
fn key_from_nibble_iter<Iter: Iterator<Item = u8>>(nibbles: Iter) -> Key {
    nibbles.collect()
}

#[cfg(not(feature = "branch_factor_256"))]
fn key_from_nibble_iter<Iter: Iterator<Item = u8>>(mut nibbles: Iter) -> Key {
    let mut data = Vec::with_capacity(nibbles.size_hint().0 / 2);

    while let (Some(hi), Some(lo)) = (nibbles.next(), nibbles.next()) {
        let byte = hi
            .checked_shl(4)
            .and_then(|v| v.checked_add(lo))
            .expect("Nibble overflow while constructing byte");
        data.push(byte);
    }

    data.into_boxed_slice()
}
