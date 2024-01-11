// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::{node::Node, BranchNode, ExtNode, LeafNode, Merkle, NodeObjRef, NodeType};
use crate::{
    shale::{DiskAddress, ObjRef, ShaleStore},
    v2::api,
};
use bitflags::iter::Iter;
use futures::{stream::FusedStream, Stream};
use helper_types::{Either, MustUse};
use std::task::Poll;

type Key = Box<[u8]>;
type Value = Vec<u8>;

enum IteratorState<'a> {
    /// Start iterating at the specified key
    StartAtKey(Key),
    /// Continue iterating after the last node in the `visited_node_path`
    Iterating {
        visited_node_path: Vec<(IterationState<'a>)>,
    },
}

impl IteratorState<'_> {
    fn new() -> Self {
        Self::StartAtKey(vec![].into_boxed_slice())
    }

    fn with_key(key: Key) -> Self {
        Self::StartAtKey(key)
    }
}

/// A MerkleKeyValueStream iterates over keys/values for a merkle trie.
pub struct MerkleKeyValueStream<'a, S, T> {
    key_state: IteratorState<'a>,
    merkle_root: DiskAddress,
    merkle: &'a Merkle<S, T>,
}

impl<'a, S: ShaleStore<Node> + Send + Sync, T> FusedStream for MerkleKeyValueStream<'a, S, T> {
    fn is_terminated(&self) -> bool {
        matches!(&self.key_state, IteratorState::Iterating { visited_node_path } if visited_node_path.is_empty())
    }
}

impl<'a, S, T> MerkleKeyValueStream<'a, S, T> {
    pub(super) fn new(merkle: &'a Merkle<S, T>, merkle_root: DiskAddress) -> Self {
        let key_state = IteratorState::new();

        Self {
            merkle,
            key_state,
            merkle_root,
        }
    }

    pub(super) fn from_key(merkle: &'a Merkle<S, T>, merkle_root: DiskAddress, key: Key) -> Self {
        let key_state = IteratorState::with_key(key);

        Self {
            merkle,
            key_state,
            merkle_root,
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
            key_state,
            merkle_root,
            merkle,
        } = &mut *self;

        match key_state {
            IteratorState::StartAtKey(key) => {
                let root_node = merkle
                    .get_node(*merkle_root)
                    .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                // traverse the trie along each nibble until we find a node with the [key].
                // TODO: merkle.iter_by_key(key) will simplify this entire code-block.
                let visited_node_path = {
                    let mut visited_node_path = vec![];

                    // [key_node] is the node with [key], if it exists.
                    let key_node = merkle
                        .get_node_by_key_with_callbacks(
                            root_node,
                            &key,
                            |node_addr, i| visited_node_path.push((node_addr, i)),
                            |_, _| {},
                        )
                        .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                    let num_elts = visited_node_path.len();
                    let visited_node_path = visited_node_path
                        .into_iter()
                        .enumerate()
                        .map(|(index, node_and_pos)| {
                            let (node, pos) = node_and_pos;
                            merkle.get_node(node).map(|node| match node.inner() {
                                NodeType::Branch(_) => IterationState::Branch(
                                    BranchIterationState::VisitedChildren(node, pos),
                                ),
                                NodeType::Leaf(_) => {
                                    if key_node.is_some() {
                                        // This must be the last node on the path since it's a leaf.
                                        // Since we found a node with the key, this must be the node.
                                        IterationState::Leaf(LeafIterationState::New(node))
                                    } else {
                                        // TODO what do we do in the else statement?
                                        todo!()
                                    }
                                }
                                NodeType::Extension(_) => {
                                    if index == num_elts - 1 {
                                        IterationState::Extension(
                                            ExtensionIterationState::VisitedSelf(node),
                                        )
                                    } else {
                                        IterationState::Extension(ExtensionIterationState::New(
                                            node,
                                        ))
                                    }
                                }
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                    visited_node_path
                };

                self.key_state = IteratorState::Iterating { visited_node_path };

                self.poll_next(_cx)
            }

            IteratorState::Iterating { visited_node_path } => {
                let next = find_next_result(merkle, visited_node_path)
                    .map_err(|e| api::Error::InternalError(Box::new(e)))
                    .transpose();

                Poll::Ready(next)
            }
        }
    }
}

enum LeafIterationState<'a> {
    // This leaf's key-value pair hasn't been returned by the iterator.
    New(NodeObjRef<'a>),
}

enum BranchIterationState<'a> {
    // This branch's key-value pair hasn't been returned by the iterator.
    New(NodeObjRef<'a>),
    // This branch's key-value pair (if any) has been returned by the iterator
    // but we haven't visited any of its children.
    VisitedSelf(NodeObjRef<'a>),
    // This branch's key-value pair (if any) has been returned by the iterator
    // and we've visited its children up to and including the one at the given index.
    VisitedChildren(NodeObjRef<'a>, u8),
}

enum ExtensionIterationState<'a> {
    New(NodeObjRef<'a>),
    VisitedSelf(NodeObjRef<'a>),
}

enum IterationState<'a> {
    Leaf(LeafIterationState<'a>),
    Branch(BranchIterationState<'a>),
    Extension(ExtensionIterationState<'a>),
}

enum VisitableNodeObjRef<'a> {
    New(NodeObjRef<'a>),
    Visited(NodeObjRef<'a>),
}

#[derive(Debug)]
enum VisitableNode<'a> {
    New(&'a NodeType),
    Visited(&'a NodeType),
}

impl<'a> VisitableNodeObjRef<'a> {
    fn inner(&self) -> VisitableNode<'_> {
        match self {
            Self::New(node) => VisitableNode::New(node.inner()),
            Self::Visited(node) => VisitableNode::Visited(node.inner()),
        }
    }

    fn into_node(self) -> NodeObjRef<'a> {
        match self {
            Self::New(node) => node,
            Self::Visited(node) => node,
        }
    }
}

fn find_next_result<'a, S: ShaleStore<Node>, T>(
    merkle: &'a Merkle<S, T>,
    visited_path: &mut Vec<IterationState<'a>>,
) -> Result<Option<(Key, Value)>, super::MerkleError> {
    let next = find_next_node_with_data(merkle, visited_path)?.map(|(next_node)| {
        // TODO uncomment
        // let partial_path = match next_node.inner() {
        //     NodeType::Leaf(leaf) => leaf.path.iter().copied(),
        //     NodeType::Extension(extension) => extension.path.iter().copied(),
        //     _ => [].iter().copied(),
        // };

        // let key = key_from_nibble_iter(nibble_iter_from_parents(visited_path).chain(partial_path));
        let key: Box<[u8]> = Box::new([]); // TODO implement and uncomment above

        let value = match next_node.inner() {
            NodeType::Leaf(leaf) => leaf.data.to_vec(),
            NodeType::Branch(branch) => branch.value.as_ref().unwrap().to_vec(),
            _ => unreachable!(), // TODO is this right?
        };

        (key, value)
    });

    Ok(next)
}

fn find_next_node_with_data<'a, S: ShaleStore<Node>, T>(
    merkle: &'a Merkle<S, T>,
    visited_path: &mut Vec<IterationState<'a>>,
) -> Result<Option<NodeObjRef<'a>>, super::MerkleError> {
    let Some(mut node) = visited_path.pop() else {
        return Ok(None);
    };

    loop {
        match node {
            IterationState::Leaf(iter_state) => match iter_state {
                LeafIterationState::New(node_ref) => {
                    return Ok(Some(node_ref));
                }
            },
            IterationState::Branch(iter_state) => match iter_state {
                BranchIterationState::New(node_ref) => {
                    // We haven't returned this node's key-value yet
                    let branch_node = node_ref.inner().as_branch().unwrap();

                    if let Some(value) = branch_node.value.as_ref() {
                        return Ok(Some(node_ref));
                    }

                    node = IterationState::Branch(BranchIterationState::VisitedSelf(node_ref));
                }
                BranchIterationState::VisitedSelf(node_ref) => {
                    // We've returned this node's key-value, but we haven't visited any of its children
                    let branch_node = node_ref.inner().as_branch().unwrap();

                    // Find the first child
                    let mut children = get_children_iter(branch_node);

                    if let Some(first_child_and_pos) = children.next() {
                        // Get the first child
                        let (child_addr, child_pos) = first_child_and_pos;
                        let child = merkle.get_node(child_addr)?;

                        // Push updated parent (branch) onto the stack
                        visited_path.push(IterationState::Branch(
                            BranchIterationState::VisitedChildren(node_ref, child_pos),
                        ));

                        // Push child onto the stack
                        node = match child.inner() {
                            NodeType::Branch(_) => {
                                IterationState::Branch(BranchIterationState::New(child))
                            }
                            NodeType::Leaf(_) => {
                                IterationState::Leaf(LeafIterationState::New(child))
                            }
                            NodeType::Extension(_) => {
                                IterationState::Extension(ExtensionIterationState::New(child))
                            }
                        }
                    } else {
                        // There are no children, so we're done with this node.
                        let Some(next_parent) = visited_path.pop() else {
                            return Ok(None);
                        };
                        node = next_parent;
                    }
                }
                BranchIterationState::VisitedChildren(node_ref, handled_child_pos) => {
                    let branch_node = node_ref.inner().as_branch().unwrap();

                    // Find the next unhandled child
                    let mut children = get_children_iter(branch_node)
                        .filter(|(_, child_pos)| child_pos > &handled_child_pos);

                    if let Some(next_child_and_pos) = children.next() {
                        // Get the next child
                        let (child_addr, child_pos) = next_child_and_pos;
                        let child = merkle.get_node(child_addr)?;

                        // Push updated parent (branch) onto the stack
                        visited_path.push(IterationState::Branch(
                            BranchIterationState::VisitedChildren(node_ref, child_pos),
                        ));

                        // Push child onto the stack
                        node = match child.inner() {
                            NodeType::Branch(_) => {
                                IterationState::Branch(BranchIterationState::New(child))
                            }
                            NodeType::Leaf(_) => {
                                IterationState::Leaf(LeafIterationState::New(child))
                            }
                            NodeType::Extension(_) => {
                                IterationState::Extension(ExtensionIterationState::New(child))
                            }
                        }
                    } else {
                        // There are no more children, so we're done with this node.
                        let Some(next_parent) = visited_path.pop() else {
                            return Ok(None);
                        };
                        node = next_parent;
                    }
                }
            },
            IterationState::Extension(iter_state) => match iter_state {
                ExtensionIterationState::New(node_ref) => {
                    let child = node_ref.inner().as_extension().unwrap().chd();

                    // Push updated parent (extension) onto the stack
                    visited_path.push(IterationState::Extension(
                        ExtensionIterationState::VisitedSelf(node_ref),
                    ));

                    // Push child onto the stack
                    let child = merkle.get_node(child)?;

                    node = match child.inner() {
                        NodeType::Branch(_) => {
                            IterationState::Branch(BranchIterationState::New(child))
                        }
                        NodeType::Leaf(_) => IterationState::Leaf(LeafIterationState::New(child)),
                        NodeType::Extension(_) => {
                            IterationState::Extension(ExtensionIterationState::New(child))
                        }
                    }
                }
                ExtensionIterationState::VisitedSelf(node_ref) => {
                    // TODO can we remove this?
                    let extension_node = node_ref.inner().as_extension().unwrap();

                    // Push child onto the stack
                    let child = merkle.get_node(extension_node.chd())?;

                    node = match child.inner() {
                        NodeType::Branch(_) => {
                            IterationState::Branch(BranchIterationState::New(child))
                        }
                        NodeType::Leaf(_) => IterationState::Leaf(LeafIterationState::New(child)),
                        NodeType::Extension(_) => {
                            IterationState::Extension(ExtensionIterationState::New(child))
                        }
                    }
                }
            },
        }
    }
}

fn get_children_iter(branch: &BranchNode) -> impl Iterator<Item = (DiskAddress, u8)> {
    branch
        .children
        .into_iter()
        .enumerate()
        .filter_map(|(pos, child_addr)| child_addr.map(|child_addr| (child_addr, pos as u8)))
}

/// This function is a little complicated because we need to be able to early return from the parent
/// when we return `false`. `MustUse` forces the caller to check the inner value of `Result::Ok`.
/// It also replaces `node`
fn next_node<'a, S, T, Iter>(
    merkle: &'a Merkle<S, T>,
    mut children: Iter,
    parents: &mut Vec<(NodeObjRef<'a>, u8)>,
    node: &mut VisitableNodeObjRef<'a>,
    pos: &mut u8,
) -> Result<MustUse<bool>, super::MerkleError>
where
    Iter: Iterator<Item = (DiskAddress, u8)>,
    S: ShaleStore<Node>,
{
    if let Some((child_addr, child_pos)) = children.next() {
        let child = merkle.get_node(child_addr)?;

        *pos = child_pos;
        let node = std::mem::replace(node, VisitableNodeObjRef::New(child));
        parents.push((node.into_node(), *pos));
    } else {
        let Some((next_parent, next_pos)) = parents.pop() else {
            return Ok(false.into());
        };

        *node = VisitableNodeObjRef::Visited(next_parent);
        *pos = next_pos;
    }

    Ok(true.into())
}

/// create an iterator over the key-nibbles from all parents _excluding_ the sentinal node.
fn nibble_iter_from_parents<'a>(parents: &'a [(NodeObjRef, u8)]) -> impl Iterator<Item = u8> + 'a {
    parents
        .iter()
        .skip(1) // always skip the sentinal node
        .flat_map(|(parent, child_nibble)| match parent.inner() {
            NodeType::Branch(_) => Either::Left(std::iter::once(*child_nibble)),
            NodeType::Extension(extension) => Either::Right(extension.path.iter().copied()),
            NodeType::Leaf(leaf) => Either::Right(leaf.path.iter().copied()),
        })
}

fn key_from_nibble_iter<Iter: Iterator<Item = u8>>(mut nibbles: Iter) -> Key {
    let mut data = Vec::with_capacity(nibbles.size_hint().0 / 2);

    while let (Some(hi), Some(lo)) = (nibbles.next(), nibbles.next()) {
        data.push((hi << 4) + lo);
    }

    data.into_boxed_slice()
}

mod helper_types {
    use std::ops::Not;

    /// Enums enable stack-based dynamic-dispatch as opposed to heap-based `Box<dyn Trait>`.
    /// This helps us with match arms that return different types that implement the same trait.
    /// It's possible that [rust-lang/rust#63065](https://github.com/rust-lang/rust/issues/63065) will make this unnecessary.
    ///
    /// And this can be replaced by the `either` crate from crates.io if we ever need more functionality.
    pub(super) enum Either<T, U> {
        Left(T),
        Right(U),
    }

    impl<T, U> Iterator for Either<T, U>
    where
        T: Iterator,
        U: Iterator<Item = T::Item>,
    {
        type Item = T::Item;

        fn next(&mut self) -> Option<Self::Item> {
            match self {
                Self::Left(left) => left.next(),
                Self::Right(right) => right.next(),
            }
        }
    }

    #[must_use]
    pub(super) struct MustUse<T>(T);

    impl<T> From<T> for MustUse<T> {
        fn from(t: T) -> Self {
            Self(t)
        }
    }

    impl<T: Not> Not for MustUse<T> {
        type Output = T::Output;

        fn not(self) -> Self::Output {
            self.0.not()
        }
    }
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
    async fn start_at_key_greater_than_all_others() {
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

/*
error[E0382]: use of moved value
   --> firewood/src/merkle/stream.rs:288:36
    |
288 |             IterationState::Branch(iter_state) => match iter_state {
    |                                    ^^^^^^^^^^ value moved here, in previous iteration of loop
    |
note: these 3 reinitializations might get skipped
   --> firewood/src/merkle/stream.rs:285:21
    |
285 |                     node = next_parent;
    |                     ^^^^
...
340 |                         node = next_parent;
    |                         ^^^^
...
381 |                         node = next_parent;
    |                         ^^^^
    = note: move occurs because value has type `BranchIterationState<'_>`, which does not implement the `Copy` trait
help: borrow this binding in the pattern to avoid moving the value
    |
288 |             IterationState::Branch(ref iter_state) => match iter_state {
    |                                    +++

*/
