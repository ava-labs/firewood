// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::cmp::Ordering;
use storage::{NodeStore, Parentable, ReadInMemoryNode, ReadableStorage, TrieReader};

use crate::db::BatchOp;
use crate::merkle::{Key, Merkle, Value};
use crate::stream::MerkleKeyValueStream;

enum DiffIteratorState {
    Unknown,
    Matching,
    SavedLeft(Option<Result<(Key, Value), crate::v2::api::Error>>),
    SavedRight(Option<Result<(Key, Value), crate::v2::api::Error>>),
}

struct DiffIterator<'a, T: TrieReader, U: TrieReader> {
    iter1: MerkleKeyValueStream<'a, T>,
    iter2: MerkleKeyValueStream<'a, U>,
    state: DiffIteratorState,
}

fn diff_merkle_iterator<
    'a,
    T1: Parentable + ReadInMemoryNode,
    T2: Parentable + ReadInMemoryNode,
    S1: ReadableStorage,
    S2: ReadableStorage,
>(
    m1: &'a Merkle<NodeStore<T1, S1>>,
    m2: &'a Merkle<NodeStore<T2, S2>>,
    start_key: Key,
) -> DiffIterator<'a, NodeStore<T1, S1>, NodeStore<T2, S2>> {
    let iter1 = m1.key_value_iter_from_key(start_key.clone());
    let iter2 = m2.key_value_iter_from_key(start_key);

    DiffIterator {
        iter1,
        iter2,
        state: DiffIteratorState::Unknown,
    }
}

impl<T: TrieReader, U: TrieReader> Iterator for DiffIterator<'_, T, U> {
    type Item = BatchOp<Key, Value>;

    fn next(&mut self) -> Option<Self::Item> {
        let (left_key, right_key) =
            match std::mem::replace(&mut self.state, DiffIteratorState::Matching) {
                DiffIteratorState::Matching | DiffIteratorState::Unknown => {
                    (self.iter1.next(), self.iter2.next())
                }
                DiffIteratorState::SavedLeft(saved) => (saved, self.iter2.next()),
                DiffIteratorState::SavedRight(saved) => (self.iter1.next(), saved),
            };

        match (left_key, right_key) {
            (Some(Ok((key1, value1))), Some(Ok((key2, value2)))) => {
                match key1.cmp(&key2) {
                    Ordering::Equal => {
                        self.state = DiffIteratorState::Matching;
                        if value1 == value2 {
                            self.next() // Skip matching values, continue to next (tail recursion)
                        } else {
                            Some(BatchOp::Put {
                                key: key2,
                                value: value2,
                            }) // Value changed
                        }
                    }
                    Ordering::Less => {
                        // Save the right key-value for next iteration
                        self.state = DiffIteratorState::SavedRight(Some(Ok((key2, value2))));
                        Some(BatchOp::Delete { key: key1 })
                    }
                    Ordering::Greater => {
                        // Save the left key-value for next iteration
                        self.state = DiffIteratorState::SavedLeft(Some(Ok((key1, value1))));
                        Some(BatchOp::Put {
                            key: key2,
                            value: value2,
                        })
                    }
                }
            }
            (Some(Ok((key, _))), None) => {
                // Key exists in m1 but not m2 - delete it
                Some(BatchOp::Delete { key })
            }
            (None, Some(Ok((key, value)))) => {
                // Key exists in m2 but not m1 - add it
                Some(BatchOp::Put { key, value })
            }
            _ => None, // Both iterators exhausted or error
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use storage::{ImmutableProposal, MemStore, MutableProposal, NodeStore};

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

        let op1 = diff_iter.next().unwrap();
        assert!(
            matches!(op1, BatchOp::Put { key, value } if key == Box::from(b"key1".as_slice()) && value == b"value1")
        );

        let op2 = diff_iter.next().unwrap();
        assert!(
            matches!(op2, BatchOp::Put { key, value } if key == Box::from(b"key2".as_slice()) && value == b"value2")
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

        let op1 = diff_iter.next().unwrap();
        assert!(matches!(op1, BatchOp::Delete { key } if key == Box::from(b"key1".as_slice())));

        let op2 = diff_iter.next().unwrap();
        assert!(matches!(op2, BatchOp::Delete { key } if key == Box::from(b"key2".as_slice())));

        assert!(diff_iter.next().is_none());
    }

    #[test]
    fn test_diff_modifications() {
        let m1 = populate_merkle(create_test_merkle(), &[(b"key1", b"old_value")]);
        let m2 = populate_merkle(create_test_merkle(), &[(b"key1", b"new_value")]);

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([]));

        let op = diff_iter.next().unwrap();
        assert!(
            matches!(op, BatchOp::Put { key, value } if key == Box::from(b"key1".as_slice()) && value == b"new_value")
        );

        assert!(diff_iter.next().is_none());
    }

    #[test]
    fn test_diff_mixed_operations() {
        // m1 has: key1=value1, key2=old_value, key3=value3
        // m2 has: key2=new_value, key4=value4
        // Expected: Delete key1, Put key2=new_value, Delete key3, Put key4=value4

        let m1 = populate_merkle(
            create_test_merkle(),
            &[
                (b"key1", b"value1"),
                (b"key2", b"old_value"),
                (b"key3", b"value3"),
            ],
        );

        let m2 = populate_merkle(
            create_test_merkle(),
            &[(b"key2", b"new_value"), (b"key4", b"value4")],
        );

        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::new([]));

        let op1 = diff_iter.next().unwrap();
        assert!(matches!(op1, BatchOp::Delete { key } if key == Box::from(b"key1".as_slice())));

        let op2 = diff_iter.next().unwrap();
        assert!(
            matches!(op2, BatchOp::Put { key, value } if key == Box::from(b"key2".as_slice()) && value == b"new_value")
        );

        let op3 = diff_iter.next().unwrap();
        assert!(matches!(op3, BatchOp::Delete { key } if key == Box::from(b"key3".as_slice())));

        let op4 = diff_iter.next().unwrap();
        assert!(
            matches!(op4, BatchOp::Put { key, value } if key == Box::from(b"key4".as_slice()) && value == b"value4")
        );

        assert!(diff_iter.next().is_none());
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
                (b"aaa", b"value1"),   // Same
                (b"bbb", b"modified"), // Modified
                (b"ddd", b"value4"),   // Added
            ],
        );

        // Start from key "bbb" - should skip "aaa"
        let mut diff_iter = diff_merkle_iterator(&m1, &m2, Box::from(b"bbb".as_slice()));

        let op1 = diff_iter.next().unwrap();
        assert!(
            matches!(op1, BatchOp::Put { key, value } if key == Box::from(b"bbb".as_slice()) && value == b"modified")
        );

        let op2 = diff_iter.next().unwrap();
        assert!(matches!(op2, BatchOp::Delete { key } if key == Box::from(b"ccc".as_slice())));

        let op3 = diff_iter.next().unwrap();
        assert!(
            matches!(op3, BatchOp::Put { key, value } if key == Box::from(b"ddd".as_slice()) && value == b"value4")
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

        let ops: Vec<_> = diff_iter.collect();

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
}
