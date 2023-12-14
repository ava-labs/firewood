// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::{node::Node, BranchNode, Merkle, NodeType, ObjRef};
use crate::{
    shale::{DiskAddress, ShaleStore},
    v2::api,
};
use futures::{stream::FusedStream, Stream};
use helper_types::{Either, MustUse};
use std::task::Poll;

type Key = Box<[u8]>;
type Value = Vec<u8>;

enum IteratorState<'a> {
    /// Start iterating at the specified key
    StartAtKey(Key),
    /// Continue iterating after the given `next_node` and parents
    Iterating { parents: Vec<(ObjRef<'a>, u8)> },
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
        match &self.key_state {
            IteratorState::Iterating { parents } if parents.is_empty() => true,
            _ => false,
        }
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

                let (found_node, mut parents) = merkle
                    .get_node_and_parents_by_key(root_node, &key)
                    .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                if let Some(found_node) = found_node {
                    let value = match found_node.inner() {
                        NodeType::Branch(branch) => branch.value.as_ref(),
                        NodeType::Leaf(leaf) => Some(&leaf.data),
                        NodeType::Extension(_) => None,
                    };

                    let next_result = value.map(|value| {
                        let value = value.to_vec();

                        Ok((std::mem::take(key), value))
                    });

                    parents.push((found_node, 0));

                    self.key_state = IteratorState::Iterating { parents };

                    return Poll::Ready(next_result);
                }

                self.key_state = IteratorState::Iterating { parents };

                self.poll_next(_cx)
            }

            IteratorState::Iterating { parents } => {
                let next = find_next_result(merkle, parents)
                    .map_err(|e| api::Error::InternalError(Box::new(e)))
                    .transpose();

                Poll::Ready(next)
            }
        }
    }
}

enum NodeRef<'a> {
    New(ObjRef<'a>),
    Visited(ObjRef<'a>),
}

#[derive(Debug)]
enum InnerNode<'a> {
    New(&'a NodeType),
    Visited(&'a NodeType),
}

impl<'a> NodeRef<'a> {
    fn inner(&self) -> InnerNode<'_> {
        match self {
            Self::New(node) => InnerNode::New(node.inner()),
            Self::Visited(node) => InnerNode::Visited(node.inner()),
        }
    }

    fn into_node(self) -> ObjRef<'a> {
        match self {
            Self::New(node) => node,
            Self::Visited(node) => node,
        }
    }
}

fn find_next_result<'a, S: ShaleStore<Node>, T>(
    merkle: &'a Merkle<S, T>,
    visited_path: &mut Vec<(ObjRef<'a>, u8)>,
) -> Result<Option<(Key, Value)>, super::MerkleError> {
    let next = find_next_node_with_data(merkle, visited_path)?.map(|(next_node, value)| {
        let partial_path = match next_node.inner() {
            NodeType::Leaf(leaf) => leaf.path.iter().copied(),
            NodeType::Extension(extension) => extension.path.iter().copied(),
            _ => [].iter().copied(),
        };

        let key = key_from_nibble_iter(nibble_iter_from_parents(visited_path).chain(partial_path));

        visited_path.push((next_node, 0));

        (key, value)
    });

    Ok(next)
}

fn find_next_node_with_data<'a, S: ShaleStore<Node>, T>(
    merkle: &'a Merkle<S, T>,
    visited_path: &mut Vec<(ObjRef<'a>, u8)>,
) -> Result<Option<(ObjRef<'a>, Vec<u8>)>, super::MerkleError> {
    use InnerNode::*;

    let Some((visited_parent, visited_pos)) = visited_path.pop() else {
        return Ok(None);
    };

    let mut node = NodeRef::Visited(visited_parent);
    let mut pos = visited_pos;
    let mut first_loop = true;

    loop {
        match node.inner() {
            New(NodeType::Leaf(leaf)) => {
                let value = leaf.data.to_vec();
                return Ok(Some((node.into_node(), value)));
            }

            Visited(NodeType::Leaf(_)) | Visited(NodeType::Extension(_)) => {
                let Some((next_parent, next_pos)) = visited_path.pop() else {
                    return Ok(None);
                };

                node = NodeRef::Visited(next_parent);
                pos = next_pos;
            }

            New(NodeType::Extension(extension)) => {
                let child = merkle.get_node(extension.chd())?;

                pos = 0;
                visited_path.push((node.into_node(), pos));

                node = NodeRef::New(child);
            }

            Visited(NodeType::Branch(branch)) => {
                let compare_op = if first_loop {
                    <u8 as PartialOrd>::ge
                } else {
                    <u8 as PartialOrd>::gt
                };

                let children = get_children_iter(branch)
                    .filter(move |(_, child_pos)| compare_op(child_pos, &pos));

                let next_node_success =
                    next_node(merkle, children, visited_path, &mut node, &mut pos)?;

                if !next_node_success {
                    return Ok(None);
                }
            }

            New(NodeType::Branch(branch)) => {
                if let Some(value) = branch.value.as_ref() {
                    let value = value.to_vec();
                    return Ok(Some((node.into_node(), value)));
                }

                let children = get_children_iter(branch);

                let next_node_success =
                    next_node(merkle, children, visited_path, &mut node, &mut pos)?;

                if !next_node_success {
                    return Ok(None);
                }
            }
        }

        first_loop = false;
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
    parents: &mut Vec<(ObjRef<'a>, u8)>,
    node: &mut NodeRef<'a>,
    pos: &mut u8,
) -> Result<MustUse<bool>, super::MerkleError>
where
    Iter: Iterator<Item = (DiskAddress, u8)>,
    S: ShaleStore<Node>,
{
    if let Some((child_addr, child_pos)) = children.next() {
        let child = merkle.get_node(child_addr)?;

        *pos = child_pos;
        let node = std::mem::replace(node, NodeRef::New(child));
        parents.push((node.into_node(), *pos));
    } else {
        let Some((next_parent, next_pos)) = parents.pop() else {
            return Ok(false.into());
        };

        *node = NodeRef::Visited(next_parent);
        *pos = next_pos;
    }

    Ok(true.into())
}

/// create an iterator over the key-nibbles from all parents _excluding_ the sentinal node.
fn nibble_iter_from_parents<'a>(parents: &'a [(ObjRef, u8)]) -> impl Iterator<Item = u8> + 'a {
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
    /// It's possible that https://github.com/rust-lang/rust/issues/63065 will make this unnecessary.
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

#[cfg(test)]
use super::tests::create_test_merkle;

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use test_case::test_case;

    #[tokio::test]
    async fn iterate_empty() {
        let merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();
        let mut it = merkle.iter_from(root, b"x".to_vec().into_boxed_slice());
        let next = it.next().await;
        assert!(next.is_none());
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

        let mut it = match start {
            Some(start) => merkle.iter_from(root, start.to_vec().into_boxed_slice()),
            None => merkle.iter(root),
        };

        // we iterate twice because we should get a None then start over
        for k in start.map(|r| r[0]).unwrap_or_default()..=u8::MAX {
            let next = it.next().await.map(|kv| {
                let (k, v) = kv.unwrap();
                assert_eq!(&*k, &*v);
                k
            });

            assert_eq!(next, Some(vec![k].into_boxed_slice()));
        }

        assert!(it.next().await.is_none());
    }

    #[tokio::test]
    async fn fused_empty() {
        let merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();
        let mut it = merkle.iter(root);
        assert!(it.next().await.is_none());
        assert!(it.is_terminated());
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

        let mut it = merkle.iter(root);

        for kv in key_values.iter() {
            let next = it.next().await.unwrap().unwrap();
            assert_eq!(&*next.0, &*next.1);
            assert_eq!(&next.1, kv);
        }

        assert!(it.next().await.is_none());
        assert!(it.is_terminated());
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

        let done = stream.next().await;

        assert!(done.is_none());
    }
}
