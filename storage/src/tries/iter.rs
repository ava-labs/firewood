// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{HashType, PathBuf, PathComponent, TriePath, ValueDigest};

use super::{TrieEdgeState, TrieNode};

/// A marker type for [`TrieEdgeIter`] that indicates that the iterator traverses
/// the trie in ascending order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IterAscending {}

/// A marker type for [`TrieEdgeIter`] that indicates that the iterator traverses
/// the trie in descending order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IterDescending {}

/// An iterator over the edges in a key-value trie in a specified order.
///
/// Use [`TrieNode::iter_edges`] or [`TrieNode::iter_edges_desc`] to
/// create an instance of this iterator in ascending or descending order,
/// respectively.
#[derive(Debug, Clone)]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct TrieEdgeIter<'a, D> {
    leading_path: PathBuf,
    stack: Vec<Frame<'a>>,
    marker: std::marker::PhantomData<D>,
}

/// An iterator over the key-value pairs in a key-value trie.
#[derive(Debug, Clone)]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct TrieValueIter<'a, D> {
    edges: TrieEdgeIter<'a, D>,
}

#[derive(Debug, Clone)]
struct Frame<'a> {
    node: &'a dyn TrieNode,
    hash: Option<&'a HashType>,
    leading_path_len: usize,
    children: Option<std::array::IntoIter<PathComponent, { PathComponent::LEN }>>,
}

impl<'a, D> TrieEdgeIter<'a, D> {
    /// Creates a new iterator over the given key-value trie.
    pub fn new(root: &'a dyn TrieNode) -> Self {
        let mut this = Self {
            leading_path: PathBuf::new_const(),
            stack: Vec::new(),
            marker: std::marker::PhantomData,
        };
        this.push_frame(None, root, None);
        this
    }

    /// Transforms this iterator into an iterator over the key-value pairs in
    /// the trie.
    pub const fn node_values(self) -> TrieValueIter<'a, D> {
        TrieValueIter { edges: self }
    }

    fn push_frame(
        &mut self,
        leading_component: Option<PathComponent>,
        node: &'a dyn TrieNode,
        hash: Option<&'a HashType>,
    ) {
        let frame = Frame {
            node,
            hash,
            leading_path_len: self.leading_path.len(),
            children: None,
        };
        self.stack.push(frame);
        self.leading_path.extend(leading_component);
        self.leading_path.extend(node.partial_path().components());
    }
}

/// Both iterators share this logic to descend into a node's children.
///
/// The passed in `children_iter` should be an iterator over the indices into
/// the children array in the desired order (e.g. ascending or descending).
macro_rules! descend {
    (
        $self:expr,
        $node:expr,
        $children_iter:expr
    ) => {
        if let Some((pc, state)) =
            $children_iter.find_map(|pc| $node.child_state(pc).map(|state| (pc, state)))
        {
            match state {
                TrieEdgeState::LocalChild { node, hash } => {
                    $self.push_frame(Some(pc), node, Some(hash));
                }
                TrieEdgeState::RemoteChild { hash } => {
                    let mut path = $self.leading_path.clone();
                    path.push(pc);
                    return Some((path, TrieEdgeState::RemoteChild { hash }));
                }
                TrieEdgeState::UnhashedChild { node } => {
                    $self.push_frame(Some(pc), node, None);
                }
            }

            continue;
        }
    };
}

impl<'a> Iterator for TrieEdgeIter<'a, IterAscending> {
    type Item = (PathBuf, TrieEdgeState<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(&mut Frame {
            node,
            hash,
            leading_path_len,
            ref mut children,
        }) = self.stack.last_mut()
        {
            // ascending iterator yields the node before iterating its children
            let mut do_yield = false;

            let children = children.get_or_insert_with(|| {
                do_yield = true;
                PathComponent::ALL.into_iter()
            });

            if do_yield {
                return Some((
                    self.leading_path.clone(),
                    TrieEdgeState::from_node(node, hash),
                ));
            }

            descend!(self, node, children);

            // we've exhausted this node's children, so pop its frame
            self.stack.pop();
            self.leading_path.truncate(leading_path_len);
        }

        None
    }
}

impl<'a> Iterator for TrieEdgeIter<'a, IterDescending> {
    type Item = (PathBuf, TrieEdgeState<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(&mut Frame {
            node,
            hash,
            leading_path_len,
            ref mut children,
        }) = self.stack.last_mut()
        {
            // descending iterator yields the node after iterating its children
            let children = children.get_or_insert_with(|| PathComponent::ALL.into_iter());

            descend!(self, node, children.rev());

            // clone the path before we pop the frame
            let leading_path = self.leading_path.clone();

            // we've exhausted this node's children, so pop its frame and yield the node
            self.stack.pop();
            self.leading_path.truncate(leading_path_len);

            return Some((leading_path, TrieEdgeState::from_node(node, hash)));
        }

        None
    }
}

impl<'a> Iterator for TrieValueIter<'a, IterAscending> {
    type Item = (PathBuf, ValueDigest<&'a [u8]>);

    fn next(&mut self) -> Option<Self::Item> {
        self.edges.find_map(|(path, node)| match node {
            TrieEdgeState::LocalChild { node, hash: _ } | TrieEdgeState::UnhashedChild { node } => {
                Some((path, node.value_digest()?))
            }
            TrieEdgeState::RemoteChild { .. } => None,
        })
    }
}

impl<'a> Iterator for TrieValueIter<'a, IterDescending> {
    type Item = (PathBuf, ValueDigest<&'a [u8]>);

    fn next(&mut self) -> Option<Self::Item> {
        self.edges.find_map(|(path, node)| match node {
            TrieEdgeState::LocalChild { node, hash: _ } | TrieEdgeState::UnhashedChild { node } => {
                Some((path, node.value_digest()?))
            }
            TrieEdgeState::RemoteChild { .. } => None,
        })
    }
}
