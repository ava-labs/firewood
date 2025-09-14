// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::{array::IntoIter as ArrayIter, iter::Enumerate};

use firewood_storage::{BranchNode, HashType};

use crate::proofs::{
    CollectedNibbles,
    path::{Nibbles, PathNibble},
    trie::TrieNode,
};

/// A hole in a range proof where keys are missing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MissingKeys {
    /// The depth in the trie where keys are missing.
    pub depth: usize,
    /// The leading path to the missing keys.
    pub leading_path: CollectedNibbles,
    /// The hash of the node rooted at `leading_path` where the missing keys would be.
    pub hash: HashType,
}

pub(super) enum Child<T> {
    Unhashed(T),
    Hashed(HashType, T),
    Remote(HashType),
}

impl<T> Child<T> {
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Child<U> {
        match self {
            Child::Unhashed(node) => Child::Unhashed(f(node)),
            Child::Hashed(hash, node) => Child::Hashed(hash, f(node)),
            Child::Remote(hash) => Child::Remote(hash),
        }
    }
}

pub(super) struct PreOrderItem<T> {
    pub depth: usize,
    pub leading_path: CollectedNibbles,
    pub node: Child<T>,
}

struct PreOrderState<T> {
    depth: usize,
    leading_path: CollectedNibbles,
    node: T,
    children: Option<Enumerate<ArrayIter<Option<Child<T>>, { BranchNode::MAX_CHILDREN }>>>,
    parent: Option<Box<PreOrderState<T>>>,
}

pub(super) struct PreOrderIter<T> {
    stack: Option<Box<PreOrderState<T>>>,
}

impl<'a, T: TrieNode<'a>> PreOrderIter<T> {
    pub fn new(root: T) -> Self {
        Self {
            stack: Some(Box::new(PreOrderState {
                depth: 0,
                leading_path: CollectedNibbles::empty(),
                node: root,
                children: None,
                parent: None,
            })),
        }
    }
}

impl<'a, T: TrieNode<'a>> Iterator for PreOrderIter<T> {
    type Item = PreOrderItem<T>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut outer_this = self.stack.take();
        while let Some(mut this) = outer_this.take() {
            let Some(children) = this.children.as_mut() else {
                // this only happens on the first iteration so that we can yield the root node
                let item = PreOrderItem {
                    depth: this.depth,
                    leading_path: this.leading_path.clone(),
                    node: Child::Unhashed(this.node),
                };
                this.children = Some(this.node.children().into_iter().enumerate());
                this.leading_path.extend(this.node.partial_path());
                self.stack = Some(this);
                return Some(item);
            };

            let Some((nibble, child)) = children.by_ref().find_map(find_nibble) else {
                // exhausted all children, traverse back up
                outer_this = this.parent.take();
                continue;
            };

            let depth = this.depth;
            let leading_path = this.leading_path.by_ref().join(nibble).collect();
            self.stack = match &child {
                // push nodes onto the stack to traverse their children on the next iteration
                Child::Unhashed(node) | Child::Hashed(_, node) => Some(Box::new(PreOrderState {
                    depth: depth.saturating_add(1),
                    leading_path: leading_path.clone(),
                    node: *node,
                    children: Some(node.children().into_iter().enumerate()),
                    parent: Some(this),
                })),
                // remote children cannot be traversed any further
                Child::Remote(_) => Some(this),
            };

            return Some(PreOrderItem {
                depth,
                leading_path,
                node: child,
            });
        }
        None
    }
}

fn find_nibble<T>((nibble, child): (usize, Option<Child<T>>)) -> Option<(PathNibble, Child<T>)> {
    child.map(|child| (PathNibble(nibble as u8), child))
}
