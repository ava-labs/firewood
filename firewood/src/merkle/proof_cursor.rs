// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Forward-only cursor that pairs proof nodes with their corresponding
//! proposal trie nodes for verification. Given a proof node at some
//! depth, the cursor finds the proposal node at the same depth so
//! their values and child hashes can be compared. Both sequences are
//! ordered by depth, so a single forward scan suffices.

use std::cmp::Ordering;

use firewood_storage::PathIterItem;

pub(crate) struct ProofCursor<'a> {
    path: &'a [PathIterItem],
}

impl<'a> ProofCursor<'a> {
    pub(crate) fn new(path: &'a [PathIterItem]) -> Self {
        debug_assert!(
            path.is_sorted_by_key(|item| item.key_nibbles.len()),
            "path items must be in ascending depth order"
        );
        Self { path }
    }

    /// Advance past nodes shallower than `depth`, return the node
    /// at exactly `depth` if one exists.
    pub(crate) fn advance_to(&mut self, depth: usize) -> Option<&'a PathIterItem> {
        while let Some((item, rest)) = self.path.split_first() {
            match item.key_nibbles.len().cmp(&depth) {
                Ordering::Less => self.path = rest,
                Ordering::Equal => return Some(item),
                Ordering::Greater => return None,
            }
        }
        None
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used)]
mod tests {
    use super::*;
    use firewood_storage::{LeafNode, Node, Path, PathComponent, SharedNode};

    fn path_item_at_depth(depth: usize) -> PathIterItem {
        PathIterItem {
            key_nibbles: (0..depth)
                .map(|_| PathComponent::try_new(0).unwrap())
                .collect(),
            node: SharedNode::new(Node::Leaf(LeafNode {
                partial_path: Path::new(),
                value: Box::default(),
            })),
            next_nibble: None,
        }
    }

    #[test]
    fn empty_path() {
        let mut cursor = ProofCursor::new(&[]);
        assert!(cursor.advance_to(0).is_none());
        assert!(cursor.advance_to(5).is_none());
    }

    #[test]
    fn exact_match() {
        let items = [
            path_item_at_depth(0),
            path_item_at_depth(2),
            path_item_at_depth(4),
        ];
        let mut cursor = ProofCursor::new(&items);
        assert_eq!(cursor.advance_to(0).unwrap().key_nibbles.len(), 0);
        assert_eq!(cursor.advance_to(2).unwrap().key_nibbles.len(), 2);
        assert_eq!(cursor.advance_to(4).unwrap().key_nibbles.len(), 4);
        assert!(cursor.advance_to(6).is_none());
    }

    #[test]
    fn skip_and_miss() {
        let items = [
            path_item_at_depth(0),
            path_item_at_depth(2),
            path_item_at_depth(4),
            path_item_at_depth(6),
        ];
        let mut cursor = ProofCursor::new(&items);
        assert_eq!(cursor.advance_to(4).unwrap().key_nibbles.len(), 4);
        assert!(cursor.advance_to(5).is_none());
        assert_eq!(cursor.advance_to(6).unwrap().key_nibbles.len(), 6);
    }

    #[test]
    fn repeated_depth() {
        let items = [path_item_at_depth(2)];
        let mut cursor = ProofCursor::new(&items);
        assert_eq!(cursor.advance_to(2).unwrap().key_nibbles.len(), 2);
        assert_eq!(cursor.advance_to(2).unwrap().key_nibbles.len(), 2);
    }

    #[test]
    fn depth_gap() {
        let items = [path_item_at_depth(0), path_item_at_depth(8)];
        let mut cursor = ProofCursor::new(&items);
        cursor.advance_to(0);
        assert!(cursor.advance_to(4).is_none());
        assert_eq!(cursor.advance_to(8).unwrap().key_nibbles.len(), 8);
    }

    #[test]
    fn first_item_too_deep() {
        let items = [path_item_at_depth(4), path_item_at_depth(6)];
        let mut cursor = ProofCursor::new(&items);
        assert!(cursor.advance_to(2).is_none());
        assert_eq!(cursor.advance_to(4).unwrap().key_nibbles.len(), 4);
    }

    #[test]
    fn single_item_miss() {
        let items = [path_item_at_depth(3)];
        let mut cursor = ProofCursor::new(&items);
        assert!(cursor.advance_to(5).is_none());
    }

    #[test]
    fn advance_after_exhaustion() {
        let items = [path_item_at_depth(2)];
        let mut cursor = ProofCursor::new(&items);
        assert_eq!(cursor.advance_to(2).unwrap().key_nibbles.len(), 2);
        assert!(cursor.advance_to(4).is_none());
        assert!(cursor.advance_to(6).is_none());
    }
}
