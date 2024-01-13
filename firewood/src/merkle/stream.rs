// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::{
    node::{self, Node},
    BranchNode, Merkle, NodeType,
};
use crate::{
    merkle::NodeObjRef,
    shale::{DiskAddress, ShaleStore},
    v2::api,
};
use futures::{stream::FusedStream, Stream};
use std::{collections::VecDeque, task::Poll};
type Key = Box<[u8]>;
type Value = Vec<u8>;

struct BranchIterator {
    // The nibbles of the key at this node.
    key_nibbles: Vec<u8>,
    // Returns the non-empty children of this node
    // and their positions in the node's children array.
    children_iter: Box<dyn Iterator<Item = (DiskAddress, u8)> + Send>,
}

enum IteratorState {
    /// Start iterating at the specified key
    StartAtKey(Key),
    /// Continue iterating after the last node in the `visited_node_path`
    Iterating {
        // Each element is an iterator over a branch node we've visited
        // along our traversal of the key-value pairs in the trie.
        // We pop an iterator off the stack and call next on it to
        // get the next child node to visit. When an iterator is empty,
        // we pop it off the stack and go back up to its parent.
        branch_iter_stack: Vec<BranchIterator>,
    },
}

impl IteratorState {
    fn new() -> Self {
        Self::StartAtKey(vec![].into_boxed_slice())
    }

    fn with_key(key: Key) -> Self {
        Self::StartAtKey(key)
    }
}

/// A MerkleKeyValueStream iterates over keys/values for a merkle trie.
pub struct MerkleKeyValueStream<'a, S, T> {
    key_state: IteratorState,
    merkle_root: DiskAddress,
    merkle: &'a Merkle<S, T>,
}

impl<'a, S: ShaleStore<Node> + Send + Sync, T> FusedStream for MerkleKeyValueStream<'a, S, T> {
    fn is_terminated(&self) -> bool {
        matches!(&self.key_state, IteratorState::Iterating { branch_iter_stack } if branch_iter_stack.is_empty())
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

                let mut branch_iter_stack: Vec<BranchIterator> = vec![];

                // TODO remove
                // branch_iter_stack.push(BranchIterator {
                //     key_nibbles: vec![],
                //     children_iter: Box::new(get_children_iter(
                //         root_node.inner().as_branch().unwrap(),
                //     )),
                // });

                // (disk address, index) for each node we visit along the path to the node
                // with [key], where [index] is the next nibble in the key.
                let mut path_to_key = vec![];

                let node_at_key = merkle
                    .get_node_by_key_with_callbacks(
                        root_node,
                        &key,
                        |node_addr, i| path_to_key.push((node_addr, i)),
                        |_, _| {},
                    )
                    .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                let mut path_to_key: VecDeque<(NodeObjRef<'a>, u8)> = path_to_key
                    .into_iter()
                    .map(|(node, pos)| merkle.get_node(node).map(|node| (node, pos)))
                    .collect::<Result<VecDeque<_>, _>>()
                    .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                let mut key_nibbles_so_far: Vec<u8> = vec![];

                loop {
                    let Some((node, pos)) = path_to_key.pop_front() else {
                        break;
                    };

                    match node.inner() {
                        NodeType::Branch(branch) => {
                            if path_to_key.len() == 0 {
                                // TODO is this right?
                                // This is the last node so there are 2 possibilities:
                                // 1. This node's child is at the [key]
                                // 2. There is no node with [key].

                                if node_at_key.is_none() {
                                    let children_iter = get_children_iter(branch)
                                        .filter(move |(_, child_pos)| child_pos > &pos);

                                    branch_iter_stack.push(BranchIterator {
                                        key_nibbles: key_nibbles_so_far.clone(),
                                        children_iter: Box::new(children_iter),
                                    });
                                }
                            }

                            key_nibbles_so_far.push(pos);
                        }
                        NodeType::Leaf(_) => (),
                        NodeType::Extension(extension) => {
                            // Add the extension node's path to the key nibbles.
                            key_nibbles_so_far.extend(extension.path.iter());

                            if path_to_key.len() == 0 {
                                // This is the last node in the path to the key.
                                if node_at_key.is_some() {
                                    // Get the branch node child
                                    let child = merkle
                                        .get_node(extension.chd())
                                        .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                                    let children_iter =
                                        get_children_iter(child.inner().as_branch().unwrap());

                                    branch_iter_stack.push(BranchIterator {
                                        key_nibbles: key_nibbles_so_far.clone(),
                                        children_iter: Box::new(children_iter),
                                    });
                                }
                            }
                        }
                    }
                }

                self.key_state = IteratorState::Iterating { branch_iter_stack };

                return self.poll_next(_cx);
            }
            IteratorState::Iterating { branch_iter_stack } => {
                loop {
                    let Some(mut branch_iter) = branch_iter_stack.pop() else {
                        return Poll::Ready(None);
                    };

                    // [node_addr] is the next node to visit.
                    // It's the child at index [pos] of [node_iter].
                    let Some((node_addr, pos)) = branch_iter.children_iter.next() else {
                        // We visited all this node's descendants.
                        // Go back to its parent.
                        continue;
                    };

                    let node = merkle
                        .get_node(node_addr)
                        .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                    let mut child_key_nibbles = branch_iter.key_nibbles.clone(); // TODO reduce¸cloning
                    child_key_nibbles.push(pos);

                    branch_iter_stack.push(branch_iter);

                    match node.inner() {
                        NodeType::Branch(branch) => {
                            // [children_iter] returns (child_addr, pos)
                            // for every non-empty child in [node] where
                            // [pos] is the child's index in [node.children].
                            let children_iter = get_children_iter(branch);

                            branch_iter_stack.push(BranchIterator {
                                key_nibbles: child_key_nibbles.clone(), // TODO reduce¸cloning
                                children_iter: Box::new(children_iter),
                            });

                            // If there's a value, return it.
                            if let Some(value) = branch.value.as_ref() {
                                let value = value.to_vec();
                                return Poll::Ready(Some(Ok((
                                    key_from_nibble_iter(child_key_nibbles.into_iter().skip(1)), // skip the sentinel node leading 0
                                    value,
                                ))));
                            }
                        }
                        NodeType::Leaf(leaf) => {
                            let value = leaf.data.to_vec();
                            return Poll::Ready(Some(Ok((
                                key_from_nibble_iter(child_key_nibbles.into_iter().skip(1)), // skip the sentinel node leading 0
                                value,
                            ))));
                        }
                        NodeType::Extension(extension) => {
                            // Follow the extension node to its child, which is a branch.
                            // TODO confirm that an extension node's child is always a branch node.
                            let child = merkle
                                .get_node(extension.chd())
                                .map_err(|e| api::Error::InternalError(Box::new(e)))?;

                            let branch = child.inner().as_branch().unwrap();
                            let children_iter = get_children_iter(branch);

                            // TODO reduce cloning
                            let mut child_key = child_key_nibbles;
                            child_key.extend(extension.path.iter());

                            branch_iter_stack.push(BranchIterator {
                                key_nibbles: child_key,
                                children_iter: Box::new(children_iter),
                            });
                        }
                    };
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
    async fn no_start_key() {
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        for i in (0..256).rev() {
            for j in (0..256).rev() {
                let key = vec![i as u8, j as u8];
                let value = vec![0x00];

                merkle
                    .insert(key.clone(), value.clone(), root.clone())
                    .unwrap();
            }
        }

        let mut stream = merkle.iter(root);

        for i in 0..256 {
            for j in 0..256 {
                let expected_key = vec![i as u8, j as u8];
                let expected_value = vec![0x00];

                println!("i: {}, j: {}", i, j);

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
