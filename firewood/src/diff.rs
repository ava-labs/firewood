// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use storage::{
    FileIoError, Node, NodeStore, Parentable, ReadInMemoryNode, ReadableStorage, RootReader, TrieHash, TrieReader
};

use crate::{
    db::{Batch, BatchOp},
    manager::RevisionManager,
    merkle::{Key, Merkle, Value},
    stream::MerkleNodeStream,
};

#[allow(dead_code)]
pub(crate) fn diff(
    revmgr: &RevisionManager,
    old: TrieHash,
    new: TrieHash,
    start: Option<Key>,
    limit: Option<usize>,
) -> Result<Batch<Key, Value>, crate::v2::api::Error> {
    let old = revmgr.revision(old)?;
    let new = revmgr.revision(new)?;
    Ok(old.diff(&new, start, limit)?)
}

/// Trait for computing differences between two trie states.
pub trait Diffable {
    /// Computes the difference between this trie state and another, returning a batch of key-value changes.
    fn diff<TN, SN>(
        &self,
        new: &NodeStore<TN, SN>,
        start: Option<Key>,
        limit: Option<usize>,
    ) -> Result<Batch<Key, Value>, FileIoError>
    where
        Self: TrieReader,
        NodeStore<TN, SN>: TrieReader,
        TN: ReadInMemoryNode + Parentable,
        SN: ReadableStorage;
}

impl<T, S> Diffable for NodeStore<T, S>
where
    NodeStore<T, S>: TrieReader,
    T: ReadInMemoryNode + Parentable,
    S: ReadableStorage,
{
    fn diff<TN, SN>(
        &self,
        new: &NodeStore<TN, SN>,
        start: Option<Key>,
        limit: Option<usize>,
    ) -> Result<Batch<Key, Value>, FileIoError>
    where
        NodeStore<TN, SN>: TrieReader,
        TN: ReadInMemoryNode + Parentable,
        SN: ReadableStorage,
    {
        let mut diff = Vec::new();

        let old_root = self.root_node();
        let new_root = new.root_node();

        match (old_root, new_root) {
            (None, None) => {}
            (Some(_), Some(_)) => {
                // the hard part: start walking the two trees at the same time, but start at the given key
                let start = start.unwrap_or_default();
                let left = Merkle::from(self);
                let right = Merkle::from(new);
                let mut left_stream = left.path_iter(&start)?;
                let mut right_stream = right.path_iter(&start)?;

                let left_item = left_stream.next();
                let right_item = right_stream.next();
                while left_item.is_some() || right_item.is_some() {
                    match (left_item, right_item) {
                        (Some(left_item), Some(right_item)) => {
                            match *left_item?.node {
                                Node::Leaf(ref _left_leaf) => {
                                    match *right_item?.node {
                                        Node::Leaf(ref _right_leaf) => {
                                            todo!()
                                        }
                                        Node::Branch(ref _right_branch) => {
                                            todo!()
                                        }
                                    }
                                }
                                Node::Branch(ref _left_branch) => {
                                    match *right_item?.node {
                                        Node::Branch(ref _right_branch) => {
                                            // if the branches have the same hash, then jump to the next nibble
                                            todo!()
                                        }
                                        Node::Leaf(ref _right_leaf) => {
                                            todo!()
                                        }
                                    }
                                }
                            }
                        }
                        (Some(_), None) => {
                            todo!()
                        }
                        (None, Some(_)) => {
                            todo!()
                        }
                        (None, None) => {
                            break;
                        }
                    }
                }
            }
            (None, Some(_)) => {
                // the diff consists of everything from the new root
                MerkleNodeStream::new(new, start.unwrap_or_default())
                    .take(limit.unwrap_or(usize::MAX))
                    .try_for_each(|item| -> Result<(), FileIoError> {
                        let item = item?;
                        match *item.1 {
                            Node::Leaf(ref leaf) => {
                                diff.push(BatchOp::Put {
                                    key: item.0,
                                    value: leaf.value.to_vec(),
                                });
                            }
                            Node::Branch(ref branch) => {
                                // TODO: if no value exists, then the limit should not be applied
                                if let Some(value) = &branch.value {
                                    diff.push(BatchOp::Put {
                                        key: item.0,
                                        value: value.to_vec(),
                                    })
                                }
                            }
                        }
                        Ok(())
                    })?;
            }
            (Some(_), None) => {
                MerkleNodeStream::new(self, start.unwrap_or_default())
                    .take(limit.unwrap_or(usize::MAX))
                    .try_for_each(|item| -> Result<(), FileIoError> {
                        let item = item?;
                        match *item.1 {
                            Node::Leaf(ref _leaf) => {
                                diff.push(BatchOp::Delete { key: item.0 });
                            }
                            Node::Branch(ref branch) => {
                                // TODO: if no value exists, then the limit should not be applied
                                if let Some(_value) = &branch.value {
                                    diff.push(BatchOp::Delete { key: item.0 })
                                }
                            }
                        }
                        Ok(())
                    })?;
            }
        }
        Ok(diff)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::sync::Arc;

    use crate::{db::BatchOp, merkle::Merkle};

    use super::*;
    use storage::{ImmutableProposal, MemStore, MutableProposal, NodeStore};

    fn nodestore_from_batch(
        batch: Batch<Key, Value>,
    ) -> Result<NodeStore<Arc<ImmutableProposal>, MemStore>, Box<dyn std::error::Error>> {
        let nodestore = NodeStore::new_empty_proposal(MemStore::new(vec![]).into());
        let mut merkle = Merkle::from(nodestore);
        for op in batch {
            match op {
                BatchOp::Put { key, value } => {
                    merkle.insert(&key, value.into_boxed_slice())?;
                }
                BatchOp::Delete { key } => {
                    merkle.remove(&key)?;
                }
                BatchOp::DeleteRange { prefix } => {
                    merkle.remove_prefix(&prefix)?;
                }
            }
        }
        let mutable: NodeStore<MutableProposal, MemStore> = merkle.into_inner();
        Ok(mutable.try_into()?)
    }

    fn test_row() -> BatchOp<Key, Value> {
        BatchOp::Put {
            key: Box::from([0x42]),
            value: vec![0x00],
        }
    }

    #[test]
    fn empty_diff() -> Result<(), Box<dyn std::error::Error>> {
        let old = NodeStore::new_empty_committed(MemStore::new(vec![]).into())?;
        let new = NodeStore::new_empty_committed(MemStore::new(vec![]).into())?;

        let diff = old.diff(&new, None, None)?;
        assert_eq!(diff.len(), 0);
        Ok(())
    }
    #[test]
    fn half_empty() -> Result<(), Box<dyn std::error::Error>> {
        let old = NodeStore::new_empty_committed(MemStore::new(vec![]).into())?;
        let new = nodestore_from_batch(vec![test_row()])?;

        // forward, everything is inserted
        let diff = old.diff(&new, None, None)?;
        assert_eq!(diff.len(), 1);
        assert_eq!(*diff.first().unwrap(), test_row());

        // reversed, everything is deleted
        let diff = new.diff(&old, None, None)?;
        assert_eq!(
            diff,
            vec![BatchOp::Delete {
                key: Box::from([0x42])
            }]
        );

        // forward, start at 0x43
        let diff = old.diff(&new, Some(Box::from([0x43])), None)?;
        assert_eq!(diff, vec![]);

        // reversed, start at 0x43
        let diff = new.diff(&old, Some(Box::from([0x43])), None)?;
        assert_eq!(diff, vec![]);

        Ok(())
    }
}
