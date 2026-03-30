// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use super::*;
use crate::merkle::ChangeProofCursor;

/// Build a [`PathIterItem`] with the given depth (`key_nibbles` length).
/// Only `key_nibbles.len()` matters to [`ChangeProofCursor`], so the node
/// contents are minimal.
fn cursor_item(depth: usize) -> PathIterItem {
    PathIterItem {
        key_nibbles: (0..depth)
            .map(|_| PathComponent::try_new(0).expect("0 is a valid nibble"))
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

    let hit = cursor.advance_to(0).expect("depth 0 exists");
    assert_eq!(hit.key_nibbles.len(), 0);

    let hit = cursor.advance_to(2).expect("depth 2 exists");
    assert_eq!(hit.key_nibbles.len(), 2);

    let hit = cursor.advance_to(4).expect("depth 4 exists");
    assert_eq!(hit.key_nibbles.len(), 4);
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

    // Skip depths 0 and 2, land on depth 4.
    let hit = cursor.advance_to(4).expect("depth 4 exists");
    assert_eq!(hit.key_nibbles.len(), 4);

    // Depth 5 doesn't exist; cursor stops at depth 6 (>=5) but 6 != 5.
    assert!(cursor.advance_to(5).is_none());

    // Depth 6 is still reachable — the miss didn't consume it.
    let hit = cursor.advance_to(6).expect("depth 6 not consumed by miss");
    assert_eq!(hit.key_nibbles.len(), 6);
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

    // First call returns the item.
    let hit = cursor.advance_to(2).expect("depth 2 exists");
    assert_eq!(hit.key_nibbles.len(), 2);

    // Second call at the same depth returns the same item — idempotent.
    let hit = cursor.advance_to(2).expect("depth 2 still reachable");
    assert_eq!(hit.key_nibbles.len(), 2);
}

#[test]
fn test_cursor_depth_gap() {
    // Large gap simulating trie compression.
    let items = [cursor_item(0), cursor_item(8)];
    let mut cursor = ChangeProofCursor::new(&items);

    // Depth 4 doesn't exist; cursor stops at depth 8 (>=4) but 8 != 4.
    cursor.advance_to(0);
    assert!(cursor.advance_to(4).is_none());

    // Depth 8 is still reachable.
    let hit = cursor
        .advance_to(8)
        .expect("depth 8 not consumed by gap miss");
    assert_eq!(hit.key_nibbles.len(), 8);
}
