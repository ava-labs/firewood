// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![allow(dead_code)]

use crate::db::BatchOp;
use crate::merkle::{Key, Merkle, Value};
use firewood_storage::TrieReader;
use std::collections::BTreeMap;

/// Simple diff: iterate all key/value pairs from `left` and `right` starting at `start_key`
/// and compute differences in sorted order.
///
/// - Emits Delete for keys present only in `left`
/// - Emits Put for keys present only in `right`
/// - Emits Put for keys present in both with different values
/// - Skips identical pairs
pub fn diff_merkle_simple<'a, T, U>(
    left: &'a Merkle<T>,
    right: &'a Merkle<U>,
    start_key: Key,
) -> impl Iterator<Item = BatchOp<Key, Value>>
where
    T: TrieReader,
    U: TrieReader,
{
    // Collect all key/value pairs from each trie at or after start_key
    let left_map: BTreeMap<Key, Value> = left
        .key_value_iter_from_key(&start_key)
        .map(|res| res.expect("iterator over merkle should not error in simple diff"))
        .collect();

    let right_map: BTreeMap<Key, Value> = right
        .key_value_iter_from_key(start_key)
        .map(|res| res.expect("iterator over merkle should not error in simple diff"))
        .collect();

    let mut ops: Vec<BatchOp<Key, Value>> = Vec::new();
    let mut li = left_map.into_iter().peekable();
    let mut ri = right_map.into_iter().peekable();

    loop {
        match (li.peek(), ri.peek()) {
            (None, None) => break,
            (Some((_lk, _lv)), None) => {
                let (k, _v) = li.next().expect("peek said Some");
                ops.push(BatchOp::Delete { key: k });
            }
            (None, Some((_rk, _rv))) => {
                let (k, v) = ri.next().expect("peek said Some");
                ops.push(BatchOp::Put { key: k, value: v });
            }
            (Some((lk, _lv)), Some((rk, _rv))) => match lk.cmp(rk) {
                std::cmp::Ordering::Less => {
                    let (k, _v) = li.next().expect("peek said Some");
                    ops.push(BatchOp::Delete { key: k });
                }
                std::cmp::Ordering::Greater => {
                    let (k, v) = ri.next().expect("peek said Some");
                    ops.push(BatchOp::Put { key: k, value: v });
                }
                std::cmp::Ordering::Equal => {
                    let (_k_l, v_l) = li.next().expect("peek said Some");
                    let (k_r, v_r) = ri.next().expect("peek said Some");
                    if v_l != v_r {
                        ops.push(BatchOp::Put { key: k_r, value: v_r });
                    }
                }
            },
        }
    }

    ops.into_iter()
}

/// Placeholder type and function for a future optimized structural diff.
/// The iterator body is intentionally left as `todo!()`.
#[derive(Debug)]
pub struct OptimizedDiffIter;

impl Iterator for OptimizedDiffIter {
    type Item = BatchOp<Key, Value>;
    fn next(&mut self) -> Option<Self::Item> {
        todo!("optimized structural diff iterator not yet implemented")
    }
}

#[allow(unused_variables)]
pub fn diff_merkle_optimized<'a, T, U>(
    left: &'a Merkle<T>,
    right: &'a Merkle<U>,
    start_key: Key,
) -> OptimizedDiffIter
where
    T: TrieReader,
    U: TrieReader,
{
    OptimizedDiffIter
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use firewood_storage::{ImmutableProposal, MemStore, MutableProposal, NodeStore};
    use std::sync::Arc;

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

        let ops: Vec<_> = diff_merkle_simple(&m1, &m2, Box::new([])).collect();
        assert!(ops.is_empty());
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

        let ops: Vec<_> = diff_merkle_simple(&m1, &m2, Box::new([])).collect();
        assert!(ops.is_empty());
    }

    #[test]
    fn test_diff_additions_only() {
        let items = [
            (b"key1".as_slice(), b"value1".as_slice()),
            (b"key2".as_slice(), b"value2".as_slice()),
        ];

        let m1 = make_immutable(create_test_merkle());
        let m2 = populate_merkle(create_test_merkle(), &items);

        let mut ops = diff_merkle_simple(&m1, &m2, Box::new([]));

        let op1 = ops.next().unwrap();
        assert!(matches!(
            op1,
            BatchOp::Put { key, value } if key == Box::from(b"key1".as_slice()) && value.as_ref() == b"value1"
        ));

        let op2 = ops.next().unwrap();
        assert!(matches!(
            op2,
            BatchOp::Put { key, value } if key == Box::from(b"key2".as_slice()) && value.as_ref() == b"value2"
        ));

        assert!(ops.next().is_none());
    }

    #[test]
    fn test_diff_deletions_only() {
        let items = [
            (b"key1".as_slice(), b"value1".as_slice()),
            (b"key2".as_slice(), b"value2".as_slice()),
        ];

        let m1 = populate_merkle(create_test_merkle(), &items);
        let m2 = make_immutable(create_test_merkle());

        let mut ops = diff_merkle_simple(&m1, &m2, Box::new([]));

        let op1 = ops.next().unwrap();
        assert!(matches!(op1, BatchOp::Delete { key } if key == Box::from(b"key1".as_slice())));

        let op2 = ops.next().unwrap();
        assert!(matches!(op2, BatchOp::Delete { key } if key == Box::from(b"key2".as_slice())));

        assert!(ops.next().is_none());
    }

    #[test]
    fn test_diff_modifications() {
        let m1 = populate_merkle(create_test_merkle(), &[(b"key1", b"old_value")]);
        let m2 = populate_merkle(create_test_merkle(), &[(b"key1", b"new_value")]);

        let mut ops = diff_merkle_simple(&m1, &m2, Box::new([]));

        let op = ops.next().unwrap();
        assert!(matches!(
            op,
            BatchOp::Put { key, value } if key == Box::from(b"key1".as_slice()) && value.as_ref() == b"new_value"
        ));

        assert!(ops.next().is_none());
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

        let mut ops = diff_merkle_simple(&m1, &m2, Box::new([]));

        let op1 = ops.next().unwrap();
        assert!(matches!(op1, BatchOp::Delete { key } if key == Box::from(b"key1".as_slice())));

        let op2 = ops.next().unwrap();
        assert!(matches!(
            op2,
            BatchOp::Put { key, value } if key == Box::from(b"key2".as_slice()) && value.as_ref() == b"new_value"
        ));

        let op3 = ops.next().unwrap();
        assert!(matches!(op3, BatchOp::Delete { key } if key == Box::from(b"key3".as_slice())));

        let op4 = ops.next().unwrap();
        assert!(matches!(
            op4,
            BatchOp::Put { key, value } if key == Box::from(b"key4".as_slice()) && value.as_ref() == b"value4"
        ));

        assert!(ops.next().is_none());
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
        let mut ops = diff_merkle_simple(&m1, &m2, Box::from(b"bbb".as_slice()));

        let op1 = ops.next().unwrap();
        assert!(
            matches!(op1, BatchOp::Put { ref key, ref value } if **key == *b"bbb" && **value == *b"modified"),
            "Expected first operation to be Put bbb=modified, got: {op1:?}",
        );

        let op2 = ops.next().unwrap();
        assert!(matches!(op2, BatchOp::Delete { key } if key == Box::from(b"ccc".as_slice())));

        let op3 = ops.next().unwrap();
        assert!(matches!(
            op3,
            BatchOp::Put { key, value } if key == Box::from(b"ddd".as_slice()) && value.as_ref() == b"value4"
        ));

        assert!(ops.next().is_none());
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

        let ops: Vec<_> = diff_merkle_simple(&m1, &m2, Box::new([])).collect();

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
