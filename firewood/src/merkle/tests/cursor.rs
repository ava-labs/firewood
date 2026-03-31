// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::*;
use crate::merkle::ChangeProofCursor;

fn cursor_item(depth: usize) -> PathIterItem {
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
fn test_cursor_empty_path() {
    let mut cursor = ChangeProofCursor::new(&[]);
    assert!(cursor.advance_to(0).is_none());
    assert!(cursor.advance_to(5).is_none());
}

#[test]
fn test_cursor_exact_match() {
    let items = [cursor_item(0), cursor_item(2), cursor_item(4)];
    let mut cursor = ChangeProofCursor::new(&items);
    assert_eq!(cursor.advance_to(0).unwrap().key_nibbles.len(), 0);
    assert_eq!(cursor.advance_to(2).unwrap().key_nibbles.len(), 2);
    assert_eq!(cursor.advance_to(4).unwrap().key_nibbles.len(), 4);
}

#[test]
fn test_cursor_skip_and_miss() {
    let items = [
        cursor_item(0),
        cursor_item(2),
        cursor_item(4),
        cursor_item(6),
    ];
    let mut cursor = ChangeProofCursor::new(&items);
    assert_eq!(cursor.advance_to(4).unwrap().key_nibbles.len(), 4);
    assert!(cursor.advance_to(5).is_none());
    assert_eq!(cursor.advance_to(6).unwrap().key_nibbles.len(), 6);
}

#[test]
fn test_cursor_past_end() {
    let items = [cursor_item(0), cursor_item(2)];
    let mut cursor = ChangeProofCursor::new(&items);
    assert!(cursor.advance_to(0).is_some());
    assert!(cursor.advance_to(2).is_some());
    assert!(cursor.advance_to(4).is_none());
}

#[test]
fn test_cursor_repeated_depth() {
    let items = [cursor_item(2)];
    let mut cursor = ChangeProofCursor::new(&items);
    assert_eq!(cursor.advance_to(2).unwrap().key_nibbles.len(), 2);
    assert_eq!(cursor.advance_to(2).unwrap().key_nibbles.len(), 2);
}

#[test]
fn test_cursor_depth_gap() {
    let items = [cursor_item(0), cursor_item(8)];
    let mut cursor = ChangeProofCursor::new(&items);
    cursor.advance_to(0);
    assert!(cursor.advance_to(4).is_none());
    assert_eq!(cursor.advance_to(8).unwrap().key_nibbles.len(), 8);
}
