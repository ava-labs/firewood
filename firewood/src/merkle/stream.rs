// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::{node::Node, BranchNode, Merkle, NodeType, ObjRef};
use crate::{
    shale::{DiskAddress, ShaleStore},
    v2::api,
};
use futures::Stream;
use std::{ops::Not, task::Poll};

pub(super) enum IteratorState<'a> {
    /// Start iterating at the beginning of the trie,
    /// returning the lowest key/value pair first
    StartAtBeginning,
    /// Start iterating at the specified key
    StartAtKey(Vec<u8>),
    /// Continue iterating after the given `next_node` and parents
    Iterating {
        visit_last: bool,
        next_result: Option<(Vec<u8>, Vec<u8>)>,
        parents: Vec<(ObjRef<'a>, u8)>,
    },
}
impl IteratorState<'_> {
    pub(super) fn new<K: AsRef<[u8]>>(starting: Option<K>) -> Self {
        match starting {
            None => Self::StartAtBeginning,
            Some(key) => Self::StartAtKey(key.as_ref().to_vec()),
        }
    }
}

// The default state is to start at the beginning
impl<'a> Default for IteratorState<'a> {
    fn default() -> Self {
        Self::StartAtBeginning
    }
}

/// A MerkleKeyValueStream iterates over keys/values for a merkle trie.
pub struct MerkleKeyValueStream<'a, S, T> {
    pub(super) key_state: IteratorState<'a>,
    pub(super) merkle_root: DiskAddress,
    pub(super) merkle: &'a Merkle<S, T>,
}

impl<'a, S: ShaleStore<Node> + Send + Sync, T> Stream for MerkleKeyValueStream<'a, S, T> {
    type Item = Result<(Vec<u8>, Vec<u8>), api::Error>;

    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let MerkleKeyValueStream {
            key_state,
            merkle_root,
            merkle,
        } = &mut *self;

        match key_state {
            IteratorState::StartAtBeginning => {
                let root = merkle
                    .get_node(*merkle_root)
                    .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                // always put the sentinal node in parents
                let mut parents = vec![(root, 0)];

                let next_result = find_next_result(merkle, &mut parents, true)
                    .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                let (next_result, visit_last) = next_result
                    .map(|(next, visit)| (Some(next), visit))
                    .unwrap_or_default();

                self.key_state = IteratorState::Iterating {
                    visit_last,
                    next_result,
                    parents,
                };

                self.poll_next(_cx)
            }

            IteratorState::StartAtKey(key) => {
                let root_node = merkle
                    .get_node(*merkle_root)
                    .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                let (next_result, parents) = {
                    let (found_node, parents) = {
                        let mut parents = vec![];

                        let found_node = merkle
                            .get_node_by_key_with_callbacks(
                                root_node,
                                &key,
                                |addr, nib| {
                                    parents.push((addr, nib));
                                },
                                |_, _| {},
                            )
                            .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                        (found_node, parents)
                    };

                    let mut parents = parents
                        .into_iter()
                        .map(|(addr, nib)| merkle.get_node(addr).map(|node| (node, nib)))
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                    // means the keys definitely match
                    if let Some(found_node) = found_node {
                        let (value, visit_last) = match found_node.inner() {
                            NodeType::Extension(_) => unreachable!(),

                            NodeType::Branch(branch) => {
                                let value = branch.value.as_ref().map(|v| v.to_vec());
                                let visit_last = true;
                                parents.push((found_node, 0));

                                (value, visit_last)
                            }

                            NodeType::Leaf(leaf) => {
                                let value = leaf.data.to_vec();
                                let visit_last = false;

                                (Some(value), visit_last)
                            }
                        };

                        let next_result = value.map(|value| (std::mem::take(key), value));

                        self.key_state = IteratorState::Iterating {
                            visit_last,
                            next_result,
                            parents,
                        };

                        return self.poll_next(_cx);
                    }

                    let key_from_parents = key_from_nibble_iter(nibble_iter_from_parents(&parents));
                    let mut visit_last = key == &key_from_parents;

                    if key.as_slice() < key_from_parents.as_slice() {
                        let _ = parents.pop();
                        visit_last = true;
                    }

                    let next_result = find_next_result(merkle, &mut parents, visit_last)
                        .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                    (next_result, parents)
                };

                let (next_result, visit_last) = next_result
                    .map(|(next, visit)| (Some(next), visit))
                    .unwrap_or_default();

                self.key_state = IteratorState::Iterating {
                    visit_last,
                    next_result,
                    parents,
                };

                self.poll_next(_cx)
            }

            IteratorState::Iterating {
                visit_last,
                next_result,
                parents,
            } => {
                let Some((next_key, next_value)) = next_result.take() else {
                    return Poll::Ready(None);
                };

                let next = find_next_result(merkle, parents, *visit_last)
                    .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                (*next_result, *visit_last) = next
                    .map(|(next, visit)| (Some(next), visit))
                    .unwrap_or_default();

                Poll::Ready(Some(Ok((next_key, next_value))))
            }
        }
    }
}

enum ParentNode<'a> {
    New(ObjRef<'a>),
    Visited(ObjRef<'a>),
}

#[derive(Debug)]
enum InnerNode<'a> {
    New(&'a NodeType),
    Visited(&'a NodeType),
}

impl<'a> ParentNode<'a> {
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

type NextResult = Option<((Vec<u8>, Vec<u8>), bool)>;

fn find_next_result<'a, S: ShaleStore<Node>, T>(
    merkle: &'a Merkle<S, T>,
    parents: &mut Vec<(ObjRef<'a>, u8)>,
    visit_last: bool,
) -> Result<NextResult, super::MerkleError> {
    let next = find_next_node_with_data(merkle, parents, visit_last)?.map(|next| {
        let (next_node, value) = next;
        let node_path_iter = match next_node.inner() {
            NodeType::Leaf(leaf) => leaf.path.iter().copied(),
            _ => [].iter().copied(),
        };

        let key = key_from_nibble_iter(nibble_iter_from_parents(parents).chain(node_path_iter));
        let visit_last = matches!(next_node.inner(), NodeType::Branch(_));

        if visit_last {
            parents.push((next_node, 0));
        }

        ((key, value), visit_last)
    });

    Ok(next)
}

fn find_next_node_with_data<'a, S: ShaleStore<Node>, T>(
    merkle: &'a Merkle<S, T>,
    visited_parents: &mut Vec<(ObjRef<'a>, u8)>,
    visit_last: bool,
) -> Result<Option<(ObjRef<'a>, Vec<u8>)>, super::MerkleError> {
    use InnerNode::*;

    let Some((visited_parent, visited_pos)) = visited_parents.pop() else {
        return Ok(None);
    };

    let mut node = ParentNode::Visited(visited_parent);
    let mut pos = visited_pos;

    loop {
        match node.inner() {
            // TODO: find a better way to handle this impossible case
            Visited(NodeType::Leaf(_)) => unreachable!(),

            New(NodeType::Leaf(leaf)) => {
                let value = leaf.data.to_vec();
                return Ok(Some((node.into_node(), value)));
            }

            Visited(NodeType::Extension(_)) => {
                let Some((next_parent, next_pos)) = visited_parents.pop() else {
                    return Ok(None);
                };

                node = ParentNode::Visited(next_parent);
                pos = next_pos;
            }

            New(NodeType::Extension(extension)) => {
                let child = merkle.get_node(extension.chd())?;

                pos = 0;
                visited_parents.push((node.into_node(), pos));

                node = ParentNode::New(child);
            }

            Visited(NodeType::Branch(branch)) => {
                let compare_op = if visit_last {
                    <u8 as PartialOrd>::ge
                } else {
                    <u8 as PartialOrd>::gt
                };

                let children = get_children_iter(branch)
                    .filter(move |(_, child_pos)| compare_op(child_pos, &pos));

                let next_child_success =
                    next_child(merkle, children, visited_parents, &mut node, &mut pos)?;

                if !next_child_success {
                    return Ok(None);
                }
            }

            New(NodeType::Branch(branch)) => {
                if let Some(value) = branch.value.as_ref() {
                    let value = value.to_vec();
                    return Ok(Some((node.into_node(), value)));
                }

                let children = get_children_iter(branch);

                let next_child_success =
                    next_child(merkle, children, visited_parents, &mut node, &mut pos)?;

                if !next_child_success {
                    return Ok(None);
                }
            }
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

#[must_use]
struct MustUse<T>(T);

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

fn next_child<'a, S, T, Iter>(
    merkle: &'a Merkle<S, T>,
    mut children: Iter,
    parents: &mut Vec<(ObjRef<'a>, u8)>,
    node: &mut ParentNode<'a>,
    pos: &mut u8,
) -> Result<MustUse<bool>, super::MerkleError>
where
    Iter: Iterator<Item = (DiskAddress, u8)>,
    S: ShaleStore<Node>,
{
    if let Some((child_addr, child_pos)) = children.next() {
        let child = merkle.get_node(child_addr)?;

        *pos = child_pos;
        let node = std::mem::replace(node, ParentNode::New(child));
        parents.push((node.into_node(), *pos));
    } else {
        let Some((next_parent, next_pos)) = parents.pop() else {
            return Ok(false.into());
        };

        *node = ParentNode::Visited(next_parent);
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
            NodeType::Branch(_) => vec![*child_nibble],
            NodeType::Extension(extension) => extension.path.to_vec(),
            NodeType::Leaf(leaf) => leaf.path.to_vec(),
        })
}

fn key_from_nibble_iter<Iter: Iterator<Item = u8>>(mut nibbles: Iter) -> Vec<u8> {
    let mut data = Vec::with_capacity(nibbles.size_hint().0);

    while let (Some(hi), Some(lo)) = (nibbles.next(), nibbles.next()) {
        data.push((hi << 4) + lo);
    }

    data
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
        let mut it = merkle.get_iter(Some(b"x"), root).unwrap();
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

        let mut it = merkle.get_iter(start, root).unwrap();
        // we iterate twice because we should get a None then start over
        for k in start.map(|r| r[0]).unwrap_or_default()..=u8::MAX {
            let next = it.next().await.map(|kv| {
                let (k, v) = kv.unwrap();
                assert_eq!(k, v);
                k
            });

            assert_eq!(next, Some(vec![k]));
        }

        assert!(it.next().await.is_none());
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

        let mut stream = merkle.get_iter(None::<&[u8]>, root).unwrap();

        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            (branch.to_vec(), branch.to_vec())
        );

        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            (first_leaf.to_vec(), first_leaf.to_vec())
        );

        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            (second_leaf.to_vec(), second_leaf.to_vec())
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

        let mut stream = merkle.get_iter(Some([intermediate]), root).unwrap();

        let first_expected = key_values[1].as_slice();
        let first = stream.next().await.unwrap().unwrap();

        assert_eq!(first.0, first.1);
        assert_eq!(first.0, first_expected);

        let second_expected = key_values[2].as_slice();
        let second = stream.next().await.unwrap().unwrap();

        assert_eq!(second.0, second.1);
        assert_eq!(second.0, second_expected);

        let done = stream.next().await;

        assert!(done.is_none());
    }
}
