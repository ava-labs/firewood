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
use firewood_storage::{BranchNode, Child, FileIoError, LeafNode, Node, Path, PathComponent, SharedNode};
use std::collections::VecDeque;

#[derive(Debug)]
pub struct OptimizedDiffIter<'a, T: TrieReader, U: TrieReader> {
    left: &'a Merkle<T>,
    right: &'a Merkle<U>,
    start_key: Key,
    stack: Vec<Frame<'a, T, U>>,
    pending: VecDeque<BatchOp<Key, Value>>,
    right_prefix_iter: Option<RightEnum<'a, U>>,
}

#[derive(Debug)]
struct RightEnum<'a, U: TrieReader> {
    iter: crate::iter::MerkleKeyValueIter<'a, U>,
    prefix: Key,
}

#[derive(Debug)]
enum Frame<'a, T: TrieReader, U: TrieReader> {
    Pair { prefix: Path, left: SharedNode, right: SharedNode },
    LeftOnly { prefix: Path, node: SharedNode },
    RightOnlyPrefix { prefix: Path },
    _Phantom(std::marker::PhantomData<(&'a T, &'a U)>),
}

impl<'a, T: TrieReader, U: TrieReader> OptimizedDiffIter<'a, T, U> {
    fn new(left: &'a Merkle<T>, right: &'a Merkle<U>, start_key: Key) -> Self {
        let mut stack = Vec::new();
        // Initialize with root frame
        match (left.root(), right.root()) {
            (None, None) => {}
            (Some(ln), None) => stack.push(Frame::LeftOnly {
                prefix: Path::new(),
                node: ln,
            }),
            (None, Some(rn)) => stack.push(Frame::RightOnlyPrefix {
                prefix: rn.partial_path().clone(),
            }),
            (Some(ln), Some(rn)) => stack.push(Frame::Pair {
                prefix: Path::new(),
                left: ln,
                right: rn,
            }),
        }
        Self {
            left,
            right,
            start_key,
            stack,
            pending: VecDeque::new(),
            right_prefix_iter: None,
        }
    }

    fn key_from_path(path: &Path) -> Key {
        path.bytes_iter().collect::<Vec<_>>().into_boxed_slice()
    }

    fn append_nibble(mut base: Path, nib: u8) -> Path {
        base.extend(std::iter::once(nib));
        base
    }

    fn lcp_len(a: &Path, b: &Path) -> usize {
        let min_len = a.len().min(b.len());
        for i in 0..min_len {
            if a[i] != b[i] {
                return i;
            }
        }
        min_len
    }

    fn trim_node_prefix(node: &SharedNode, prefix_len: usize) -> SharedNode {
        match &**node {
            Node::Leaf(leaf) if prefix_len > 0 => {
                let skip = prefix_len.min(leaf.partial_path.len());
                let trimmed_path = Path::from_nibbles_iterator(
                    leaf.partial_path.iter().copied().skip(skip),
                );
                SharedNode::new(Node::Leaf(LeafNode {
                    partial_path: trimmed_path,
                    value: leaf.value.clone(),
                }))
            }
            Node::Branch(branch) if prefix_len > 0 => {
                let skip = prefix_len.min(branch.partial_path.len());
                let trimmed_path =
                    Path::from_nibbles_iterator(branch.partial_path.iter().copied().skip(skip));
                SharedNode::new(Node::Branch(Box::new(BranchNode {
                    partial_path: trimmed_path,
                    value: branch.value.clone(),
                    children: branch.children.clone(),
                })))
            }
            _ => node.clone(),
        }
    }

    fn resolve_child_shared<V: TrieReader>(
        reader: &V,
        child: &Child,
    ) -> Result<SharedNode, FileIoError> {
        match child {
            Child::Node(node) => Ok(SharedNode::new(node.clone())),
            Child::AddressWithHash(addr, _) => reader.read_node(*addr),
            Child::MaybePersisted(maybe, _) => maybe.as_shared_node(reader),
        }
    }

    fn process_frame(&mut self, frame: Frame<'a, T, U>) {
        match frame {
            Frame::LeftOnly { prefix, node } => {
                // delete entire left-only subtree under (prefix + node.partial_path)
                let mut p = prefix.clone();
                p.extend(node.partial_path().iter().copied());
                let prefix_key = Self::key_from_path(&p);
                self.pending
                    .push_back(BatchOp::DeleteRange { prefix: prefix_key });
            }
            Frame::RightOnlyPrefix { prefix } => {
                // enumerate puts from right-only subtree at prefix
                let prefix_key = Self::key_from_path(&prefix);
                let iter = self
                    .right
                    .key_value_iter_from_key(prefix_key.clone())
                    .into_iter();
                self.right_prefix_iter = Some(RightEnum { iter, prefix: prefix_key });
            }
            Frame::Pair { prefix, left, right } => {
                // Align partial paths by LCP
                let lpp = left.partial_path().clone();
                let rpp = right.partial_path().clone();
                let lcp = Self::lcp_len(&lpp, &rpp);
                let mut p2 = prefix.clone();
                p2.extend(lpp.iter().copied().take(lcp));
                let lrem_len = lpp.len() - lcp;
                let rrem_len = rpp.len() - lcp;

                match (lrem_len == 0, rrem_len == 0) {
                    (true, true) => {
                        // same node position; handle value then children
                        let v_l = left.value().map(|v| v.to_vec().into_boxed_slice());
                        let v_r = right.value().map(|v| v.to_vec().into_boxed_slice());
                        match (v_l, v_r) {
                            (Some(_), None) => {
                                let key = Self::key_from_path(&p2);
                                self.pending.push_back(BatchOp::Delete { key });
                            }
                            (None, Some(vr)) => {
                                let key = Self::key_from_path(&p2);
                                self.pending.push_back(BatchOp::Put { key, value: vr });
                            }
                            (Some(vl), Some(vr)) if vl != vr => {
                                let key = Self::key_from_path(&p2);
                                self.pending.push_back(BatchOp::Put { key, value: vr });
                            }
                            _ => {}
                        }

                        // children
                        match (&*left, &*right) {
                            (Node::Branch(bl), Node::Branch(br)) => {
                                // Collect child tasks and push in reverse order
                                let mut tasks: Vec<Frame<'a, T, U>> = Vec::new();
                                for pc in PathComponent::ALL {
                                    let lch = &bl.children[pc];
                                    let rch = &br.children[pc];
                                    match (lch, rch) {
                                        (Some(lc), None) => {
                                            // Left-only subtree
                                            // Read child header to find partial path
                                            let lchild = Self::resolve_child_shared(self.left.nodestore(), lc)
                                                .expect("left child read");
                                            let mut child_prefix = p2.clone();
                                            child_prefix = Self::append_nibble(child_prefix, pc.as_u8());
                                            tasks.push(Frame::LeftOnly {
                                                prefix: child_prefix,
                                                node: lchild,
                                            });
                                        }
                                        (None, Some(rc)) => {
                                            // Right-only subtree: enumerate puts
                                            let rchild = Self::resolve_child_shared(self.right.nodestore(), rc)
                                                .expect("right child read");
                                            let mut child_prefix = p2.clone();
                                            child_prefix = Self::append_nibble(child_prefix, pc.as_u8());
                                            child_prefix
                                                .extend(rchild.partial_path().iter().copied());
                                            tasks.push(Frame::RightOnlyPrefix { prefix: child_prefix });
                                        }
                                        (Some(lc), Some(rc)) => {
                                            // Both present; if hashes equal, skip; else descend
                                            let hl = lc.hash();
                                            let hr = rc.hash();
                                            if hl.is_some() && hr.is_some() && hl == hr {
                                                // skip subtree
                                            } else {
                                                let lchild = Self::resolve_child_shared(self.left.nodestore(), lc)
                                                    .expect("left child read");
                                                let rchild = Self::resolve_child_shared(self.right.nodestore(), rc)
                                                    .expect("right child read");
                                                let mut child_prefix = p2.clone();
                                                child_prefix =
                                                    Self::append_nibble(child_prefix, pc.as_u8());
                                                tasks.push(Frame::Pair {
                                                    prefix: child_prefix,
                                                    left: lchild,
                                                    right: rchild,
                                                });
                                            }
                                        }
                                        (None, None) => {}
                                    }
                                }
                                // push in reverse to process ascending
                                for t in tasks.into_iter().rev() {
                                    self.stack.push(t);
                                }
                            }
                            (Node::Leaf(_), Node::Leaf(_)) => {
                                // nothing more to do
                            }
                            // Branch vs Leaf: children only on the branch side
                            (Node::Branch(bl), Node::Leaf(_)) => {
                                // Everything under left branch is left-only
                                // Delete entire subtree under prefix p2||child
                                let mut tasks: Vec<Frame<'a, T, U>> = Vec::new();
                                for (pc, opt) in bl.children.iter() {
                                    if let Some(child) = opt {
                                        let lchild = Self::resolve_child_shared(self.left.nodestore(), child)
                                            .expect("left child read");
                                        let mut child_prefix = p2.clone();
                                        child_prefix = Self::append_nibble(child_prefix, pc.as_u8());
                                        tasks.push(Frame::LeftOnly {
                                            prefix: child_prefix,
                                            node: lchild,
                                        });
                                    }
                                }
                                for t in tasks.into_iter().rev() {
                                    self.stack.push(t);
                                }
                            }
                            (Node::Leaf(_), Node::Branch(br)) => {
                                // Everything under right branch is right-only
                                let mut tasks: Vec<Frame<'a, T, U>> = Vec::new();
                                for (pc, opt) in br.children.iter() {
                                    if let Some(child) = opt {
                                        let rchild = Self::resolve_child_shared(self.right.nodestore(), child)
                                            .expect("right child read");
                                        let mut child_prefix = p2.clone();
                                        child_prefix = Self::append_nibble(child_prefix, pc.as_u8());
                                        child_prefix
                                            .extend(rchild.partial_path().iter().copied());
                                        tasks.push(Frame::RightOnlyPrefix { prefix: child_prefix });
                                    }
                                }
                                for t in tasks.into_iter().rev() {
                                    self.stack.push(t);
                                }
                            }
                        }
                    }
                    (true, false) => {
                        // Right is deeper below; descend on right remainder's first nibble
                        let r_next = rpp[lcp];
                        match &*left {
                            Node::Branch(bl) => {
                                let pc = PathComponent::try_new(r_next).expect("pc in bounds");
                                match &bl.children[pc] {
                                    Some(lc) => {
                                        // both exist under this child; trim right by 1 nibble and compare
                                        let lchild = Self::resolve_child_shared(self.left.nodestore(), lc)
                                            .expect("left child read");
                                        let rtrim = Self::trim_node_prefix(&right, 1);
                                        let mut child_prefix = p2.clone();
                                        child_prefix = Self::append_nibble(child_prefix, pc.as_u8());
                                        self.stack.push(Frame::Pair {
                                            prefix: child_prefix,
                                            left: lchild,
                                            right: rtrim,
                                        });
                                    }
                                    None => {
                                        // right-only subtree under P2 || Rrem
                                        let mut pref = p2.clone();
                                        pref.extend(rpp.iter().copied().skip(lcp));
                                        self.stack.push(Frame::RightOnlyPrefix { prefix: pref });
                                    }
                                }
                            }
                            // Left is leaf at P2; delete value at P2 (handled above), and enumerate right-only below
                            _ => {
                                let mut pref = p2.clone();
                                pref.extend(rpp.iter().copied().skip(lcp));
                                self.stack.push(Frame::RightOnlyPrefix { prefix: pref });
                            }
                        }
                    }
                    (false, true) => {
                        // Left is deeper below; symmetric
                        let l_next = lpp[lcp];
                        match &*right {
                            Node::Branch(br) => {
                                let pc = PathComponent::try_new(l_next).expect("pc in bounds");
                                match &br.children[pc] {
                                    Some(rc) => {
                                        let rchild = Self::resolve_child_shared(self.right.nodestore(), rc)
                                            .expect("right child read");
                                        let ltrim = Self::trim_node_prefix(&left, 1);
                                        let mut child_prefix = p2.clone();
                                        child_prefix = Self::append_nibble(child_prefix, pc.as_u8());
                                        self.stack.push(Frame::Pair {
                                            prefix: child_prefix,
                                            left: ltrim,
                                            right: rchild,
                                        });
                                    }
                                    None => {
                                        // left-only subtree under P2 || Lrem
                                        let mut pref = p2.clone();
                                        pref.extend(lpp.iter().copied().skip(lcp));
                                        // We need the node header to compute exact partial path; use left trimmed
                                        let ltrim = Self::trim_node_prefix(&left, 0);
                                        self.stack.push(Frame::LeftOnly { prefix: pref, node: ltrim });
                                    }
                                }
                            }
                            _ => {
                                let mut pref = p2.clone();
                                pref.extend(lpp.iter().copied().skip(lcp));
                                let ltrim = Self::trim_node_prefix(&left, 0);
                                self.stack.push(Frame::LeftOnly { prefix: pref, node: ltrim });
                            }
                        }
                    }
                    (false, false) => {
                        // Diverged at next nibble; left-only delete range and right-only puts
                        let mut pref_l = p2.clone();
                        pref_l.extend(lpp.iter().copied().skip(lcp));
                        let ltrim = Self::trim_node_prefix(&left, 0);
                        self.stack.push(Frame::LeftOnly { prefix: pref_l, node: ltrim });

                        let mut pref_r = p2.clone();
                        pref_r.extend(rpp.iter().copied().skip(lcp));
                        self.stack.push(Frame::RightOnlyPrefix { prefix: pref_r });
                    }
                }
            }
            Frame::_Phantom(_) => {}
        }
    }
}

impl<'a, T: TrieReader, U: TrieReader> Iterator for OptimizedDiffIter<'a, T, U> {
    type Item = BatchOp<Key, Value>;
    fn next(&mut self) -> Option<Self::Item> {
        // Drain any in-progress right-only prefix iterator first
        if let Some(right_enum) = &mut self.right_prefix_iter {
            while let Some(res) = right_enum.iter.next() {
                match res {
                    Ok((k, v)) => {
                        if k.starts_with(right_enum.prefix.as_ref()) {
                            return Some(BatchOp::Put { key: k, value: v });
                        } else {
                            // prefix escaped; end this prefix stream
                            self.right_prefix_iter = None;
                            break;
                        }
                    }
                    Err(_) => {
                        // treat errors as end of stream
                        self.right_prefix_iter = None;
                        break;
                    }
                }
            }
            // exhausted
            self.right_prefix_iter = None;
        }

        if let Some(op) = self.pending.pop_front() {
            return Some(op);
        }

        while let Some(frame) = self.stack.pop() {
            self.process_frame(frame);

            // Flush any immediate op
            if let Some(op) = self.pending.pop_front() {
                return Some(op);
            }

            // Or if a right-only iter has been set up, emit from it
            if self.right_prefix_iter.is_some() {
                return self.next();
            }
        }
        None
    }
}

pub fn diff_merkle_optimized<'a, T, U>(
    left: &'a Merkle<T>,
    right: &'a Merkle<U>,
    start_key: Key,
) -> OptimizedDiffIter<'a, T, U>
where
    T: TrieReader,
    U: TrieReader,
{
    OptimizedDiffIter::new(left, right, start_key)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use firewood_storage::{ImmutableProposal, MemStore, MutableProposal, NodeStore};
    use std::sync::Arc;
    use test_case::test_case;

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

    fn apply_ops_and_freeze(
        base: &Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>>,
        ops: &[BatchOp<Key, Value>],
    ) -> Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> {
        let mut fork = base.fork().unwrap();
        for op in ops {
            match op {
                BatchOp::Put { key, value } => {
                    fork.insert(key, value.clone()).unwrap();
                }
                BatchOp::Delete { key } => {
                    fork.remove(key).unwrap();
                }
                BatchOp::DeleteRange { prefix } => {
                    fork.remove_prefix(prefix).unwrap();
                }
            }
        }
        fork.try_into().unwrap()
    }

    fn assert_merkle_eq<L, R>(
        left: &Merkle<L>,
        right: &Merkle<R>,
    ) where
        L: TrieReader,
        R: TrieReader,
    {
        let mut l = crate::iter::MerkleKeyValueIter::from(left.nodestore());
        let mut r = crate::iter::MerkleKeyValueIter::from(right.nodestore());
        loop {
            match (l.next(), r.next()) {
                (None, None) => break,
                (Some(Ok((lk, lv))), Some(Ok((rk, rv)))) => {
                    assert_eq!(lk, rk, "keys differ");
                    assert_eq!(lv, rv, "values differ");
                }
                (None, Some(Ok((rk, _)))) => panic!("Missing key in result: {rk:x?}"),
                (Some(Ok((lk, _))), None) => panic!("Extra key in result: {lk:x?}"),
                (Some(Err(e)), _) | (_, Some(Err(e))) => panic!("iteration error: {e:?}"),
            }
        }
    }

    #[test]
    fn test_structural_diff_random() {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};

        let base_seed = 0xBEEF_F00D_CAFE_BABE_u64;
        let rounds = 5usize;
        let items = 500usize;
        let modify = 150usize;
        for round in 0..rounds {
            let seed = base_seed ^ (round as u64);
            let mut rng = StdRng::seed_from_u64(seed);

            // Unique base
            let mut base: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(items);
            let mut seen = std::collections::HashSet::new();
            while base.len() < items {
                let klen = rng.random_range(1..=32);
                let vlen = rng.random_range(1..=64);
                let key: Vec<u8> = (0..klen).map(|_| rng.random()).collect();
                if seen.insert(key.clone()) {
                    let val: Vec<u8> = (0..vlen).map(|_| rng.random()).collect();
                    base.push((key, val));
                }
            }

            let mut left = create_test_merkle();
            let mut right = create_test_merkle();
            for (k, v) in &base {
                left.insert(k, v.clone().into_boxed_slice()).unwrap();
                right.insert(k, v.clone().into_boxed_slice()).unwrap();
            }

            // Modify right
            for _ in 0..modify {
                let idx = rng.random_range(0..base.len());
                match rng.random_range(0..3) {
                    0 => {
                        // delete
                        right.remove(&base[idx].0).ok();
                    }
                    1 => {
                        // update existing
                        if right.get_value(&base[idx].0).unwrap().is_some() {
                            let newlen = rng.random_range(1..=64);
                            let newval: Vec<u8> = (0..newlen).map(|_| rng.random()).collect();
                            right.insert(&base[idx].0, newval.into_boxed_slice()).unwrap();
                        }
                    }
                    _ => {
                        // insert
                        let klen = rng.random_range(1..=32);
                        let vlen = rng.random_range(1..=64);
                        let key: Vec<u8> = (0..klen).map(|_| rng.random()).collect();
                        let val: Vec<u8> = (0..vlen).map(|_| rng.random()).collect();
                        right.insert(&key, val.into_boxed_slice()).unwrap();
                    }
                }
            }

            // Freeze
            let left_imm = make_immutable(left);
            let right_imm = make_immutable(right);

            // Compute diff ops using simple diff
            let ops: Vec<_> = diff_merkle_simple(&left_imm, &right_imm, Box::new([])).collect();

            // Apply operations to left and verify equals right
            let after = apply_ops_and_freeze(&left_imm, &ops);
            assert_merkle_eq(&after, &right_imm);
        }
    }

    // example of running this test with a specific seed and parameters:
    // FIREWOOD_TEST_SEED=14805530293320947613 cargo test --features logger diff::tests::diff_random_with_deletions
    #[test_case(false, false, 500)]
    #[test_case(false, true, 500)]
    #[test_case(true, false, 500)]
    #[test_case(true, true, 500)]
    #[allow(clippy::indexing_slicing, clippy::cast_precision_loss)]
    fn diff_random_with_deletions(trie1_mutable: bool, trie2_mutable: bool, num_items: usize) {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};

        // Read FIREWOOD_TEST_SEED from environment or use default seed
        let seed = std::env::var("FIREWOOD_TEST_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(14805530293320947613);
        let mut rng = StdRng::seed_from_u64(seed);

        // Generate random key-value pairs, ensuring uniqueness
        let mut items: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        let mut seen_keys = std::collections::HashSet::new();

        while items.len() < num_items {
            let key_len = rng.random_range(1..=32);
            let value_len = rng.random_range(1..=64);

            let key: Vec<u8> = (0..key_len).map(|_| rng.random()).collect();

            // Only add if key is unique
            if seen_keys.insert(key.clone()) {
                let value: Vec<u8> = (0..value_len).map(|_| rng.random()).collect();
                items.push((key, value));
            }
        }

        // Create two identical merkles
        let mut m1 = create_test_merkle();
        let mut m2 = create_test_merkle();

        for (key, value) in &items {
            m1.insert(key, value.clone().into_boxed_slice()).unwrap();
            m2.insert(key, value.clone().into_boxed_slice()).unwrap();
        }

        // Pick two different random indices to delete (if possible)
        if !items.is_empty() {
            let delete_idx1 = rng.random_range(0..items.len());
            m1.remove(&items[delete_idx1].0).unwrap();
        }
        if items.len() > 1 {
            let mut delete_idx2 = rng.random_range(0..items.len());
            // ensure different index
            while items.len() > 1 && delete_idx2 == 0 { // it's okay if equal when len==1
                delete_idx2 = rng.random_range(0..items.len());
            }
            m2.remove(&items[delete_idx2].0).unwrap();
        }

        // Compute ops and immutable views according to mutability flags
        let (ops, m1_immut, m2_immut): (
            Vec<BatchOp<Box<[u8]>, Box<[u8]>>>,
            Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>>,
            Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>>,
        ) = if trie1_mutable && trie2_mutable {
            let ops = diff_merkle_simple(&m1, &m2, Box::new([])).collect();
            let m1_immut = m1.try_into().unwrap();
            let m2_immut = m2.try_into().unwrap();
            (ops, m1_immut, m2_immut)
        } else if trie1_mutable && !trie2_mutable {
            let m2_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m2.try_into().unwrap();
            let ops = diff_merkle_simple(&m1, &m2_immut, Box::new([])).collect();
            let m1_immut = m1.try_into().unwrap();
            (ops, m1_immut, m2_immut)
        } else if !trie1_mutable && trie2_mutable {
            let m1_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m1.try_into().unwrap();
            let ops = diff_merkle_simple(&m1_immut, &m2, Box::new([])).collect();
            let m2_immut = m2.try_into().unwrap();
            (ops, m1_immut, m2_immut)
        } else {
            let m1_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m1.try_into().unwrap();
            let m2_immut: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>> =
                m2.try_into().unwrap();
            let ops = diff_merkle_simple(&m1_immut, &m2_immut, Box::new([])).collect();
            (ops, m1_immut, m2_immut)
        };

        // Apply ops to left immutable and compare with right immutable
        let left_after = apply_ops_and_freeze(&m1_immut, &ops);
        assert_merkle_eq(&left_after, &m2_immut);
    }

    #[test]
    #[ignore]
    fn diff_large_random_stress() {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};

        // Default parameters (can be overridden by env vars)
        let default_items = 5000usize;
        let default_modify = 1000usize; // modifies or inserts
        let seed = std::env::var("FIREWOOD_STRESS_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0xD1FF_C0DE_BAAD_F00D);
        let total_items = std::env::var("FIREWOOD_STRESS_ITEMS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(default_items);
        let total_modify = std::env::var("FIREWOOD_STRESS_MODIFY")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(default_modify);

        let mut rng = StdRng::seed_from_u64(seed);

        // Build base unique keys and values
        let mut base: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(total_items);
        let mut seen = std::collections::HashSet::new();
        while base.len() < total_items {
            let klen: usize = rng.random_range(1..=32);
            let vlen: usize = rng.random_range(1..=64);
            let key: Vec<u8> = (0..klen).map(|_| rng.random()).collect();
            if !seen.insert(key.clone()) {
                continue;
            }
            let val: Vec<u8> = (0..vlen).map(|_| rng.random()).collect();
            base.push((key, val));
        }

        // Left and right start identical
        let mut left = create_test_merkle();
        let mut right = create_test_merkle();
        for (k, v) in &base {
            left.insert(k, v.clone().into_boxed_slice()).unwrap();
            right.insert(k, v.clone().into_boxed_slice()).unwrap();
        }

        // Make modifications on the right: mix of deletes, updates, and inserts
        for _ in 0..total_modify {
            match rng.random_range(0..100) {
                0..=24 => {
                    // delete 25%
                    let idx = rng.random_range(0..base.len());
                    right.remove(&base[idx].0).ok();
                }
                25..=74 => {
                    // update 50%
                    let idx = rng.random_range(0..base.len());
                    let newlen = rng.random_range(1..=64);
                    let newval: Vec<u8> = (0..newlen).map(|_| rng.random()).collect();
                    right
                        .insert(&base[idx].0, newval.into_boxed_slice())
                        .unwrap();
                }
                _ => {
                    // insert 25%
                    let klen = rng.random_range(1..=32);
                    let vlen = rng.random_range(1..=64);
                    let key: Vec<u8> = (0..klen).map(|_| rng.random()).collect();
                    let val: Vec<u8> = (0..vlen).map(|_| rng.random()).collect();
                    right.insert(&key, val.into_boxed_slice()).unwrap();
                }
            }
        }

        // Freeze to immutable for diff
        let left = make_immutable(left);
        let right = make_immutable(right);

        // Compute diff ops using simple diff
        let ops = diff_merkle_optimized(&left, &right, Box::new([])).collect::<Vec<_>>();

        // Apply ops to left
        let left_after = apply_ops_and_freeze(&left, &ops);

        // Verify left_after == right by comparing full key/value iteration
        assert_merkle_eq(&left_after, &right);
    }
}
