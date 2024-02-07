// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::{node::Node, BranchNode, Merkle, NodeObjRef, NodeType};
use crate::{
    nibbles::Nibbles,
    shale::{DiskAddress, ShaleStore},
    v2::api,
};
use futures::{stream::FusedStream, Stream, StreamExt};
use std::task::Poll;
use std::{cmp::Ordering, iter::once};

type Key = Box<[u8]>;
type Value = Vec<u8>;

/// Represents an ongoing iteration over a node and its children.
enum IterationNode {
    // This node has not been returned yet.
    Unvisited {
        /// The key (as nibbles) of this node.
        key: Key,
        // The address of the node.
        address: DiskAddress,
    },
    // This node has been returned. Track which child to visit next.
    Visited {
        /// The key (as nibbles) of this node.
        key: Key,
        /// Returns the non-empty children of this node and their positions
        /// in the node's children array.
        children_iter: Box<dyn Iterator<Item = (DiskAddress, u8)> + Send>,
    },
}

enum MerkleNodeStreamState {
    /// The iterator state is lazily initialized when poll_next is called
    /// for the first time. The iteration start key is stored here.
    Uninitialized(Key),
    Initialized {
        /// Each element is a node that will be visited (i.e. returned)
        /// or has been visited but has unvisited children.
        /// On each call to poll_next we pop the next element.
        /// If it's unvisited, we visit it.
        /// If it's visited, we push its next child onto this stack.
        iter_stack: Vec<IterationNode>,
    },
}

impl MerkleNodeStreamState {
    #[allow(dead_code)] // TODO should we remove this function?
    fn new() -> Self {
        Self::Uninitialized(vec![].into_boxed_slice())
    }

    fn with_key(key: Key) -> Self {
        Self::Uninitialized(key)
    }
}

/// Iterates over the nodes in `merkle, whose root is `merkle_root,
/// in order of ascending key. For each, returns the key and the node.
pub struct MerkleNodeStream<'a, S, T> {
    state: MerkleNodeStreamState,
    merkle_root: DiskAddress,
    merkle: &'a Merkle<S, T>,
}

impl<'a, S: ShaleStore<Node> + Send + Sync, T> FusedStream for MerkleNodeStream<'a, S, T> {
    fn is_terminated(&self) -> bool {
        matches!(&self.state, MerkleNodeStreamState::Initialized { iter_stack } if iter_stack.is_empty())
    }
}

impl<'a, S, T> MerkleNodeStream<'a, S, T> {
    /// Returns a new iterator that will iterate over all the nodes in `merkle`.
    #[allow(dead_code)] // TODO should we remove this function?
    pub(super) fn new(merkle: &'a Merkle<S, T>, merkle_root: DiskAddress) -> Self {
        Self {
            state: MerkleNodeStreamState::new(),
            merkle_root,
            merkle,
        }
    }

    /// Returns a new iterator that will iterate over all the nodes in `merkle`
    /// with keys greater than or equal to `key`.
    pub(super) fn from_key(merkle: &'a Merkle<S, T>, merkle_root: DiskAddress, key: Key) -> Self {
        Self {
            state: MerkleNodeStreamState::with_key(key),
            merkle_root,
            merkle,
        }
    }
}

impl<'a, S: ShaleStore<Node> + Send + Sync, T> Stream for MerkleNodeStream<'a, S, T> {
    type Item = Result<(Key, NodeObjRef<'a>), api::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        // destructuring is necessary here because we need mutable access to `state`
        // at the same time as immutable access to `merkle`.
        let Self {
            state,
            merkle_root,
            merkle,
        } = &mut *self;

        match state {
            MerkleNodeStreamState::Uninitialized(key) => {
                self.state = get_iterator_intial_state(merkle, *merkle_root, key)?;
                self.poll_next(_cx)
            }
            MerkleNodeStreamState::Initialized { iter_stack } => {
                while let Some(mut iter_node) = iter_stack.pop() {
                    match iter_node {
                        IterationNode::Unvisited { address, key } => {
                            // We haven't returned this node yet.
                            let node = merkle
                                .get_node(address)
                                .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                            match node.inner() {
                                NodeType::Branch(branch) => {
                                    // `node` is a branch node. Visit its children next.
                                    iter_stack.push(IterationNode::Visited {
                                        key: key.clone(),
                                        children_iter: Box::new(get_children_iter(branch)),
                                    });
                                }
                                NodeType::Leaf(_) => {}
                                NodeType::Extension(_) => panic!("extension nodes shouldn't exist"),
                            }

                            return Poll::Ready(Some(Ok((key, node))));
                        }
                        IterationNode::Visited {
                            ref key,
                            ref mut children_iter,
                        } => {
                            // We returned `node` already. Visit its next child.
                            let Some((child_addr, pos)) = children_iter.next() else {
                                // We visited all this node's descendants. Go back to its parent.
                                continue;
                            };

                            let key = key.clone();

                            // There may be more children of this node to visit.
                            iter_stack.push(iter_node);

                            // Visit the child next.
                            let child = merkle
                                .get_node(child_addr)
                                .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                            let partial_path = match child.inner() {
                                NodeType::Branch(branch) => branch.path.iter().copied(),
                                NodeType::Leaf(leaf) => leaf.path.iter().copied(),
                                NodeType::Extension(_) => {
                                    panic!("extension nodes shouldn't exist")
                                }
                            };

                            iter_stack.push(IterationNode::Unvisited {
                                address: child_addr,
                                key: key
                                    .iter()
                                    .copied()
                                    .chain(once(pos))
                                    .chain(partial_path)
                                    .collect(),
                            });
                            return self.poll_next(_cx);
                        }
                    }
                }
                Poll::Ready(None)
            }
        }
    }
}

// TODO comment
fn get_iterator_intial_state<S: ShaleStore<Node> + Send + Sync, T>(
    merkle: &Merkle<S, T>,
    root_node: DiskAddress,
    key: &[u8],
) -> Result<MerkleNodeStreamState, api::Error> {
    // Invariant: `node`'s key is a prefix of `key`.
    let mut node = merkle
        .get_node(root_node)
        .map_err(|e| api::Error::InternalError(Box::new(e)))?;

    // Invariant: This is `node`'s address at the start
    // of each loop iteration.
    let mut node_addr = root_node;

    // Invariant: [matched_key_nibbles] is the key of `node` at the start
    // of each loop iteration.
    let mut matched_key_nibbles = vec![];

    let mut unmatched_key_nibbles = Nibbles::<1>::new(key).into_iter();

    let mut iter_stack: Vec<IterationNode> = vec![];

    loop {
        let Some(nib) = unmatched_key_nibbles.next() else {
            // The invariant tells us `node` is a prefix of `key`.
            // There is no more `key` left so `node` must be at `key`.
            // Visit and return `node` first.
            match &node.inner {
                NodeType::Branch(_) | NodeType::Leaf(_) => {
                    iter_stack.push(IterationNode::Unvisited {
                        address: node_addr,
                        key: matched_key_nibbles.clone().into_boxed_slice(),
                    });
                }
                NodeType::Extension(_) => {
                    panic!("extension nodes shouldn't exist")
                }
            }

            return Ok(MerkleNodeStreamState::Initialized { iter_stack });
        };
        // `nib` is the first nibble after [matched_key_nibbles].

        match &node.inner {
            NodeType::Branch(branch) => {
                // The next nibble in `key` is `nib`,
                // so all children of `node` with a position > `nib`
                // should be visited since they are after `key`.
                iter_stack.push(IterationNode::Visited {
                    key: matched_key_nibbles.clone().into_boxed_slice(),
                    children_iter: Box::new(
                        get_children_iter(branch).filter(move |(_, pos)| *pos > nib),
                    ),
                });

                // Figure out if the child at `nib` is a prefix of `key`.
                // (i.e. if we should run this loop body again)
                #[allow(clippy::indexing_slicing)]
                let child_addr = match branch.children[nib as usize] {
                    Some(c) => c,
                    None => {
                        // There is no child at `nib`.
                        // We'll visit `node`'s first child at index > `nib`
                        // first (if it exists).
                        return Ok(MerkleNodeStreamState::Initialized { iter_stack });
                    }
                };

                matched_key_nibbles.push(nib);

                let child = merkle
                    .get_node(child_addr)
                    .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                let partial_key = match child.inner() {
                    NodeType::Branch(branch) => &branch.path,
                    NodeType::Leaf(leaf) => &leaf.path,
                    NodeType::Extension(_) => {
                        panic!("extension nodes shouldn't exist")
                    }
                };

                let (comparison, new_unmatched_key_nibbles) =
                    is_prefix(partial_key.iter(), unmatched_key_nibbles);
                unmatched_key_nibbles = new_unmatched_key_nibbles;

                match comparison {
                    Ordering::Less => {
                        // `child` is before `key`.
                        return Ok(MerkleNodeStreamState::Initialized { iter_stack });
                    }
                    Ordering::Equal => {
                        // `child` is a prefix of `key`.
                        matched_key_nibbles.extend(partial_key.iter().copied());
                        node = child;
                        node_addr = child_addr;
                    }
                    Ordering::Greater => {
                        // `child` is after `key`.
                        iter_stack.push(IterationNode::Unvisited {
                            address: child_addr,
                            // TODO is there a way to just drain `partial_key`
                            // into `matched_key_nibbles`? Then we could append
                            // each matched nibble as we go instead of using
                            // extend.
                            key: matched_key_nibbles
                                .iter()
                                .copied()
                                .chain(partial_key.iter().copied())
                                .collect(),
                        });

                        return Ok(MerkleNodeStreamState::Initialized { iter_stack });
                    }
                }
            }
            NodeType::Leaf(leaf) => {
                let (comparison, _) = is_prefix(leaf.path.iter(), unmatched_key_nibbles);

                match comparison {
                    Ordering::Less | Ordering::Equal => {}
                    Ordering::Greater => {
                        // The leaf's key > `key`, so we can stop here.
                        iter_stack.push(IterationNode::Unvisited {
                            address: node_addr,
                            key: matched_key_nibbles
                                .iter()
                                .copied()
                                .chain(leaf.path.iter().copied())
                                .collect(),
                        });
                    }
                }
                return Ok(MerkleNodeStreamState::Initialized { iter_stack });
            }
            NodeType::Extension(_) => {
                panic!("extension nodes shouldn't exist")
            }
        };
    }
}

enum MerkleKeyValueStreamState<'a, S, T> {
    /// The iterator state is lazily initialized when poll_next is called
    /// for the first time. The iteration start key is stored here.
    Uninitialized(Key),
    /// The iterator works by iterating over the nodes in the merkle trie
    /// and returning the key-value pairs for nodes that have values.
    Initialized {
        node_iter: MerkleNodeStream<'a, S, T>,
    },
}

impl<'a, S, T> MerkleKeyValueStreamState<'a, S, T> {
    /// Returns a new iterator that will iterate over all the key-value pairs in `merkle`.
    #[allow(dead_code)] // TODO should we remove this function?
    fn new() -> Self {
        Self::Uninitialized(vec![].into_boxed_slice())
    }

    /// Returns a new iterator that will iterate over all the key-value pairs in `merkle`
    /// with keys greater than or equal to `key`.
    fn with_key(key: Key) -> Self {
        Self::Uninitialized(key)
    }
}

pub struct MerkleKeyValueStream<'a, S, T> {
    state: MerkleKeyValueStreamState<'a, S, T>,
    merkle_root: DiskAddress,
    merkle: &'a Merkle<S, T>,
}

impl<'a, S: ShaleStore<Node> + Send + Sync, T> FusedStream for MerkleKeyValueStream<'a, S, T> {
    fn is_terminated(&self) -> bool {
        matches!(&self.state, MerkleKeyValueStreamState::Initialized { node_iter } if node_iter.is_terminated())
    }
}

impl<'a, S, T> MerkleKeyValueStream<'a, S, T> {
    pub(super) fn new(merkle: &'a Merkle<S, T>, merkle_root: DiskAddress) -> Self {
        Self {
            state: MerkleKeyValueStreamState::new(),
            merkle_root,
            merkle,
        }
    }

    pub(super) fn from_key(merkle: &'a Merkle<S, T>, merkle_root: DiskAddress, key: Key) -> Self {
        Self {
            state: MerkleKeyValueStreamState::with_key(key),
            merkle_root,
            merkle,
        }
    }
}

impl<'a, S: ShaleStore<Node> + Send + Sync, T> Stream for MerkleKeyValueStream<'a, S, T> {
    type Item = Result<(Key, Value), api::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        // destructuring is necessary here because we need mutable access to `key_state`
        // at the same time as immutable access to `merkle`
        let Self {
            state,
            merkle_root,
            merkle,
        } = &mut *self;

        match state {
            MerkleKeyValueStreamState::Uninitialized(key) => {
                // TODO how to not clone here?
                let iter = MerkleNodeStream::from_key(merkle, *merkle_root, key.clone());
                self.state = MerkleKeyValueStreamState::Initialized { node_iter: iter };
                self.poll_next(_cx)
            }
            MerkleKeyValueStreamState::Initialized { node_iter: iter } => {
                match iter.poll_next_unpin(_cx) {
                    Poll::Ready(node) => match node {
                        Some(Ok((key, node))) => {
                            let key = key_from_nibble_iter(key.iter().copied().skip(1));

                            match node.inner() {
                                NodeType::Branch(branch) => {
                                    let Some(value) = branch.value.as_ref() else {
                                        return self.poll_next(_cx);
                                    };

                                    let value = value.to_vec();
                                    Poll::Ready(Some(Ok((key, value))))
                                }
                                NodeType::Leaf(leaf) => {
                                    let value = leaf.data.to_vec();
                                    Poll::Ready(Some(Ok((key, value))))
                                }
                                NodeType::Extension(_) => panic!("extension nodes shouldn't exist"),
                            }
                        }
                        Some(Err(e)) => Poll::Ready(Some(Err(e))),
                        None => Poll::Ready(None),
                    },
                    Poll::Pending => Poll::Pending,
                }
            }
        }
    }
}

/// Takes in an iterator over a node's partial path and an iterator over the
/// unmatched portion of a key.
/// The first returned element is:
/// * [Ordering::Less] if the node is before the key.
/// * [Ordering::Equal] if the node is a prefix of the key.
/// * [Ordering::Greater] if the node is after the key.
/// The second returned element is the unmatched portion of the key after the
/// partial path has been matched.
fn is_prefix<'a, I1, I2>(
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

/// Returns an iterator that returns (child_addr, pos) for each non-empty child of `branch`,
/// where `pos` is the position of the child in `branch`'s children array.
fn get_children_iter(branch: &BranchNode) -> impl Iterator<Item = (DiskAddress, u8)> {
    branch
        .children
        .into_iter()
        .enumerate()
        .filter_map(|(pos, child_addr)| child_addr.map(|child_addr| (child_addr, pos as u8)))
}

fn key_from_nibble_iter<Iter: Iterator<Item = u8>>(mut nibbles: Iter) -> Key {
    let mut data = Vec::with_capacity(nibbles.size_hint().0 / 2);

    while let (Some(hi), Some(lo)) = (nibbles.next(), nibbles.next()) {
        data.push((hi << 4) + lo);
    }

    data.into_boxed_slice()
}

// CAUTION: only use with nibble iterators
trait IntoBytes: Iterator<Item = u8> {
    fn nibbles_into_bytes(&mut self) -> Vec<u8> {
        let mut data = Vec::with_capacity(self.size_hint().0 / 2);

        while let (Some(hi), Some(lo)) = (self.next(), self.next()) {
            data.push((hi << 4) + lo);
        }

        data
    }
}
impl<T: Iterator<Item = u8>> IntoBytes for T {}

#[cfg(test)]
use super::tests::create_test_merkle;

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used)]
mod tests {
    use crate::nibbles::Nibbles;

    use super::*;
    use futures::StreamExt;
    use test_case::test_case;

    #[tokio::test]
    async fn iterate_empty() {
        let merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();
        let stream = merkle.iter_from(root, b"x".to_vec().into_boxed_slice());
        check_stream_is_done(stream).await;
    }

    #[test_case(Some(&[u8::MIN]); "Starting at first key")]
    #[test_case(None; "No start specified")]
    #[test_case(Some(&[128u8]); "Starting in middle")]
    #[test_case(Some(&[u8::MAX]); "Starting at last key")]
    #[tokio::test]
    async fn iterate_many(start: Option<&[u8]>) {
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        // insert all values from u8::MIN to u8::MAX, with the key and value the same
        for k in u8::MIN..=u8::MAX {
            merkle.insert([k], vec![k], root).unwrap();
        }

        let mut stream = match start {
            Some(start) => merkle.iter_from(root, start.to_vec().into_boxed_slice()),
            None => merkle.iter(root),
        };

        // we iterate twice because we should get a None then start over
        #[allow(clippy::indexing_slicing)]
        for k in start.map(|r| r[0]).unwrap_or_default()..=u8::MAX {
            let next = stream.next().await.map(|kv| {
                let (k, v) = kv.unwrap();
                assert_eq!(&*k, &*v);
                k
            });

            assert_eq!(next, Some(vec![k].into_boxed_slice()));
        }

        check_stream_is_done(stream).await;
    }

    #[tokio::test]
    async fn fused_empty() {
        let merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();
        check_stream_is_done(merkle.iter(root)).await;
    }

    #[tokio::test]
    async fn table_test() {
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        // Insert key-values in reverse order to ensure iterator
        // doesn't just return the keys in insertion order.
        for i in (0..=u8::MAX).rev() {
            for j in (0..=u8::MAX).rev() {
                let key = vec![i, j];
                let value = vec![i, j];

                merkle.insert(key, value, root).unwrap();
            }
        }

        // Test with no start key
        let mut stream = merkle.iter(root);
        for i in 0..=u8::MAX {
            for j in 0..=u8::MAX {
                let expected_key = vec![i, j];
                let expected_value = vec![i, j];

                assert_eq!(
                    stream.next().await.unwrap().unwrap(),
                    (expected_key.into_boxed_slice(), expected_value),
                    "i: {}, j: {}",
                    i,
                    j,
                );
            }
        }
        check_stream_is_done(stream).await;

        // Test with start key
        for i in 0..=u8::MAX {
            let mut stream = merkle.iter_from(root, vec![i].into_boxed_slice());
            for j in 0..=u8::MAX {
                let expected_key = vec![i, j];
                let expected_value = vec![i, j];
                assert_eq!(
                    stream.next().await.unwrap().unwrap(),
                    (expected_key.into_boxed_slice(), expected_value),
                    "i: {}, j: {}",
                    i,
                    j,
                );
            }
            if i == u8::MAX {
                check_stream_is_done(stream).await;
            } else {
                assert_eq!(
                    stream.next().await.unwrap().unwrap(),
                    (vec![i + 1, 0].into_boxed_slice(), vec![i + 1, 0]),
                    "i: {}",
                    i,
                );
            }
        }
    }

    #[tokio::test]
    async fn fused_full() {
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        let last = vec![0x00, 0x00, 0x00];

        let mut key_values = vec![vec![0x00], vec![0x00, 0x00], last.clone()];

        // branchs with paths (or extensions) will be present as well as leaves with siblings
        for kv in u8::MIN..=u8::MAX {
            let mut last = last.clone();
            last.push(kv);
            key_values.push(last);
        }

        for kv in key_values.iter() {
            merkle.insert(kv, kv.clone(), root).unwrap();
        }

        let mut stream = merkle.iter(root);

        for kv in key_values.iter() {
            let next = stream.next().await.unwrap().unwrap();
            assert_eq!(&*next.0, &*next.1);
            assert_eq!(&next.1, kv);
        }

        check_stream_is_done(stream).await;
    }

    #[tokio::test]
    async fn root_with_empty_data() {
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        let key = vec![].into_boxed_slice();
        let value = vec![0x00];

        merkle.insert(&key, value.clone(), root).unwrap();

        let mut stream = merkle.iter(root);

        assert_eq!(stream.next().await.unwrap().unwrap(), (key, value));
    }

    #[tokio::test]
    async fn get_branch_and_leaf() {
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        let first_leaf = &[0x00, 0x00];
        let second_leaf = &[0x00, 0x0f];
        let branch = &[0x00];

        merkle
            .insert(first_leaf, first_leaf.to_vec(), root)
            .unwrap();
        merkle
            .insert(second_leaf, second_leaf.to_vec(), root)
            .unwrap();

        merkle.insert(branch, branch.to_vec(), root).unwrap();

        let mut stream = merkle.iter(root);

        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            (branch.to_vec().into_boxed_slice(), branch.to_vec())
        );

        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            (first_leaf.to_vec().into_boxed_slice(), first_leaf.to_vec())
        );

        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            (
                second_leaf.to_vec().into_boxed_slice(),
                second_leaf.to_vec()
            )
        );
    }

    #[tokio::test]
    async fn start_at_key_not_in_trie() {
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        let first_key = 0x00;
        let intermediate = 0x80;

        assert!(first_key < intermediate);

        let key_values = vec![
            vec![first_key],
            vec![intermediate, intermediate],
            vec![intermediate, intermediate, intermediate],
        ];
        assert!(key_values[0] < key_values[1]);
        assert!(key_values[1] < key_values[2]);

        for key in key_values.iter() {
            merkle.insert(key, key.to_vec(), root).unwrap();
        }

        let mut stream = merkle.iter_from(root, vec![intermediate].into_boxed_slice());

        let first_expected = key_values[1].as_slice();
        let first = stream.next().await.unwrap().unwrap();

        assert_eq!(&*first.0, &*first.1);
        assert_eq!(first.1, first_expected);

        let second_expected = key_values[2].as_slice();
        let second = stream.next().await.unwrap().unwrap();

        assert_eq!(&*second.0, &*second.1);
        assert_eq!(second.1, second_expected);

        check_stream_is_done(stream).await;
    }

    #[tokio::test]
    async fn start_at_key_on_branch_with_no_value() {
        let sibling_path = 0x00;
        let branch_path = 0x0f;
        let children = 0..=0x0f;

        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        children.clone().for_each(|child_path| {
            let key = vec![sibling_path, child_path];

            merkle.insert(&key, key.clone(), root).unwrap();
        });

        let mut keys: Vec<_> = children
            .map(|child_path| {
                let key = vec![branch_path, child_path];

                merkle.insert(&key, key.clone(), root).unwrap();

                key
            })
            .collect();

        keys.sort();

        let start = keys.iter().position(|key| key[0] == branch_path).unwrap();
        let keys = &keys[start..];

        let mut stream = merkle.iter_from(root, vec![branch_path].into_boxed_slice());

        for key in keys {
            let next = stream.next().await.unwrap().unwrap();

            assert_eq!(&*next.0, &*next.1);
            assert_eq!(&*next.0, key);
        }

        check_stream_is_done(stream).await;
    }

    #[tokio::test]
    async fn start_at_key_on_branch_with_value() {
        let sibling_path = 0x00;
        let branch_path = 0x0f;
        let branch_key = vec![branch_path];

        let children = (0..=0xf).map(|val| (val << 4) + val); // 0x00, 0x11, ... 0xff

        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        merkle
            .insert(&branch_key, branch_key.clone(), root)
            .unwrap();

        children.clone().for_each(|child_path| {
            let key = vec![sibling_path, child_path];

            merkle.insert(&key, key.clone(), root).unwrap();
        });

        let mut keys: Vec<_> = children
            .map(|child_path| {
                let key = vec![branch_path, child_path];

                merkle.insert(&key, key.clone(), root).unwrap();

                key
            })
            .chain(Some(branch_key.clone()))
            .collect();

        keys.sort();

        let start = keys.iter().position(|key| key == &branch_key).unwrap();
        let keys = &keys[start..];

        let mut stream = merkle.iter_from(root, branch_key.into_boxed_slice());

        for key in keys {
            let next = stream.next().await.unwrap().unwrap();

            assert_eq!(&*next.0, &*next.1);
            assert_eq!(&*next.0, key);
        }

        check_stream_is_done(stream).await;
    }

    #[tokio::test]
    async fn start_at_key_on_extension() {
        let missing = 0x0a;
        let children = (0..=0x0f).filter(|x| *x != missing);
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        let keys: Vec<_> = children
            .map(|child_path| {
                let key = vec![child_path];

                merkle.insert(&key, key.clone(), root).unwrap();

                key
            })
            .collect();

        let keys = &keys[(missing as usize)..];

        let mut stream = merkle.iter_from(root, vec![missing].into_boxed_slice());

        for key in keys {
            let next = stream.next().await.unwrap().unwrap();

            assert_eq!(&*next.0, &*next.1);
            assert_eq!(&*next.0, key);
        }

        check_stream_is_done(stream).await;
    }

    #[tokio::test]
    async fn start_at_key_overlapping_with_extension_but_greater() {
        let start_key = 0x0a;
        let shared_path = 0x09;
        // 0x0900, 0x0901, ... 0x0a0f
        // path extension is 0x090
        let children = (0..=0x0f).map(|val| vec![shared_path, val]);

        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        children.for_each(|key| {
            merkle.insert(&key, key.clone(), root).unwrap();
        });

        let stream = merkle.iter_from(root, vec![start_key].into_boxed_slice());

        check_stream_is_done(stream).await;
    }

    #[tokio::test]
    async fn start_at_key_overlapping_with_extension_but_smaller() {
        let start_key = 0x00;
        let shared_path = 0x09;
        // 0x0900, 0x0901, ... 0x0a0f
        // path extension is 0x090
        let children = (0..=0x0f).map(|val| vec![shared_path, val]);

        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        let keys: Vec<_> = children
            .map(|key| {
                merkle.insert(&key, key.clone(), root).unwrap();
                key
            })
            .collect();

        let mut stream = merkle.iter_from(root, vec![start_key].into_boxed_slice());

        for key in keys {
            let next = stream.next().await.unwrap().unwrap();

            assert_eq!(&*next.0, &*next.1);
            assert_eq!(&*next.0, key);
        }

        check_stream_is_done(stream).await;
    }

    #[tokio::test]
    async fn start_at_key_between_siblings() {
        let missing = 0xaa;
        let children = (0..=0xf)
            .map(|val| (val << 4) + val) // 0x00, 0x11, ... 0xff
            .filter(|x| *x != missing);
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        let keys: Vec<_> = children
            .map(|child_path| {
                let key = vec![child_path];

                merkle.insert(&key, key.clone(), root).unwrap();

                key
            })
            .collect();

        let keys = &keys[((missing >> 4) as usize)..];

        let mut stream = merkle.iter_from(root, vec![missing].into_boxed_slice());

        for key in keys {
            let next = stream.next().await.unwrap().unwrap();

            assert_eq!(&*next.0, &*next.1);
            assert_eq!(&*next.0, key);
        }

        check_stream_is_done(stream).await;
    }

    #[tokio::test]
    async fn start_at_key_greater_than_all_others_leaf() {
        let key = vec![0x00];
        let greater_key = vec![0xff];
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();
        merkle.insert(key.clone(), key, root).unwrap();
        let stream = merkle.iter_from(root, greater_key.into_boxed_slice());

        check_stream_is_done(stream).await;
    }

    #[tokio::test]
    async fn start_at_key_greater_than_all_others_branch() {
        let greatest = 0xff;
        let children = (0..=0xf)
            .map(|val| (val << 4) + val) // 0x00, 0x11, ... 0xff
            .filter(|x| *x != greatest);
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        let keys: Vec<_> = children
            .map(|child_path| {
                let key = vec![child_path];

                merkle.insert(&key, key.clone(), root).unwrap();

                key
            })
            .collect();

        let keys = &keys[((greatest >> 4) as usize)..];

        let mut stream = merkle.iter_from(root, vec![greatest].into_boxed_slice());

        for key in keys {
            let next = stream.next().await.unwrap().unwrap();

            assert_eq!(&*next.0, &*next.1);
            assert_eq!(&*next.0, key);
        }

        check_stream_is_done(stream).await;
    }

    async fn check_stream_is_done<S>(mut stream: S)
    where
        S: FusedStream + Unpin,
    {
        assert!(stream.next().await.is_none());
        assert!(stream.is_terminated());
    }

    #[test]
    fn remaining_bytes() {
        let data = &[1];
        let nib: Nibbles<'_, 0> = Nibbles::<0>::new(data);
        let mut it = nib.into_iter();
        assert_eq!(it.nibbles_into_bytes(), data.to_vec());
    }

    #[test]
    fn remaining_bytes_off() {
        let data = &[1];
        let nib: Nibbles<'_, 0> = Nibbles::<0>::new(data);
        let mut it = nib.into_iter();
        it.next();
        assert_eq!(it.nibbles_into_bytes(), vec![]);
    }
}
