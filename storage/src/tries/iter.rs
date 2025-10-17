// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{
    HashType, IntoSplitPath, PathBuf, PathComponent, SplitPath, TrieEdgeState, TrieNode, TriePath,
};

/// A marker type for [`TrieEdgeIter`] that indicates that the iterator traverses
/// the trie in ascending order.
#[derive(Debug)]
pub struct IterAscending;

/// A marker type for [`TrieEdgeIter`] that indicates that the iterator traverses
/// the trie in descending order.
#[derive(Debug)]
pub struct IterDescending;

/// An iterator over the edges in a key-value trie in a specified order.
///
/// Use [`TrieNode::iter_edges`] or [`TrieNode::iter_edges_desc`] to
/// create an instance of this iterator in ascending or descending order,
/// respectively.
#[derive(Debug)]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct TrieEdgeIter<'root, N, V: ?Sized, D> {
    leading_path: PathBuf,
    stack: Vec<Frame<'root, N, V>>,
    marker: std::marker::PhantomData<D>,
}

/// An iterator over the key-value pairs in a key-value trie.
#[derive(Debug)]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct TrieValueIter<'root, N, V: ?Sized, D> {
    edges: TrieEdgeIter<'root, N, V, D>,
}

/// An iterator over the edges along a specified path in a key-value trie
/// terminating at the node corresponding to the path, if it exists; otherwise,
/// terminating at the deepest existing edge along the path.
#[derive(Debug)]
#[must_use = "iterators are lazy and do nothing unless consumed"]
pub struct TriePathIter<'root, P, N, V: ?Sized> {
    needle: P,
    current_path: PathBuf,
    current_edge: Option<TrieEdgeState<'root, N>>,
    marker: std::marker::PhantomData<V>,
}

#[derive(Debug)]
struct Frame<'root, N, V: ?Sized> {
    node: N,
    hash: Option<&'root HashType>,
    leading_path_len: usize,
    children: Option<std::array::IntoIter<PathComponent, { PathComponent::LEN }>>,
    marker: std::marker::PhantomData<V>,
}

impl<'root, N, V, D> TrieEdgeIter<'root, N, V, D>
where
    N: TrieNode<'root, V>,
    V: AsRef<[u8]> + ?Sized + 'root,
{
    /// Creates a new iterator over the given key-value trie.
    pub fn new(root: N, root_hash: Option<&'root HashType>) -> Self {
        let mut this = Self {
            leading_path: PathBuf::new_const(),
            stack: Vec::new(),
            marker: std::marker::PhantomData,
        };
        this.push_frame(None, root, root_hash);
        this
    }

    /// Transforms this iterator into an iterator over the key-value pairs in
    /// the trie.
    pub const fn node_values(self) -> TrieValueIter<'root, N, V, D> {
        TrieValueIter { edges: self }
    }

    fn push_frame(
        &mut self,
        leading_component: Option<PathComponent>,
        node: N,
        hash: Option<&'root HashType>,
    ) {
        let frame = Frame {
            node,
            hash,
            leading_path_len: self.leading_path.len(),
            children: None,
            marker: std::marker::PhantomData,
        };
        self.stack.push(frame);
        self.leading_path.extend(leading_component);
        self.leading_path.extend(node.partial_path().components());
    }
}

impl<'root, P, N, V> TriePathIter<'root, P, N, V>
where
    P: SplitPath,
    N: TrieNode<'root, V>,
    V: AsRef<[u8]> + ?Sized + 'root,
{
    /// Creates a new iterator over the edges along the given path in the
    /// specified key-value trie.
    pub const fn new(root: N, root_hash: Option<&'root HashType>, path: P) -> Self {
        let mut this = Self {
            needle: path,
            current_path: PathBuf::new_const(),
            current_edge: None,
            marker: std::marker::PhantomData,
        };
        this.current_edge = Some(TrieEdgeState::from_node(root, root_hash));
        this
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

impl<'root, N, V> Iterator for TrieEdgeIter<'root, N, V, IterAscending>
where
    N: TrieNode<'root, V>,
    V: AsRef<[u8]> + ?Sized + 'root,
{
    type Item = (PathBuf, TrieEdgeState<'root, N>);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(&mut Frame {
            node,
            hash,
            leading_path_len,
            ref mut children,
            marker: _,
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

impl<'root, N, V> Iterator for TrieEdgeIter<'root, N, V, IterDescending>
where
    N: TrieNode<'root, V>,
    V: AsRef<[u8]> + ?Sized + 'root,
{
    type Item = (PathBuf, TrieEdgeState<'root, N>);

    fn next(&mut self) -> Option<Self::Item> {
        while let Some(&mut Frame {
            node,
            hash,
            leading_path_len,
            ref mut children,
            marker: _,
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

impl<'root, N, V> Iterator for TrieValueIter<'root, N, V, IterAscending>
where
    N: TrieNode<'root, V>,
    V: AsRef<[u8]> + ?Sized + 'root,
{
    type Item = (PathBuf, &'root V);

    fn next(&mut self) -> Option<Self::Item> {
        self.edges
            .find_map(|(path, node)| node.value().map(|v| (path, v)))
    }
}

impl<'root, N, V> Iterator for TrieValueIter<'root, N, V, IterDescending>
where
    N: TrieNode<'root, V>,
    V: AsRef<[u8]> + ?Sized + 'root,
{
    type Item = (PathBuf, &'root V);

    fn next(&mut self) -> Option<Self::Item> {
        self.edges
            .find_map(|(path, node)| node.value().map(|v| (path, v)))
    }
}

impl<'root, P, N, V> Iterator for TriePathIter<'root, P, N, V>
where
    P: SplitPath,
    N: TrieNode<'root, V>,
    V: AsRef<[u8]> + ?Sized + 'root,
{
    type Item = (PathBuf, TrieEdgeState<'root, N>);

    fn next(&mut self) -> Option<Self::Item> {
        // qualified path to `Option::take` because rust-analyzer thinks
        // `self.current_edge` is an `Iterator` and displays an error for
        // `self.current_edge.take()` where `rustc` does not
        let edge = Option::take(&mut self.current_edge)?;
        let path = self.current_path.clone();

        let Some(node) = edge.node() else {
            // the current edge is remote, so we cannot descend further
            return Some((path, edge));
        };

        self.current_path.extend(node.partial_path().components());

        if let (None, Some((pc, needle_suffix)), _) = node
            .partial_path()
            .into_split_path()
            .longest_common_prefix(self.needle)
            .split_first_parts()
        {
            // the target path continues beyond the current node, descend
            // into it if there's a child along the path
            self.current_path.push(pc);
            self.current_edge = node.child_state(pc);
            self.needle = needle_suffix;
        }

        Some((path, edge))
    }
}

// auto-derived implementations would require V: Clone which is too much and the
// derive_where crate does not implement this correctly for our use case

impl<N: Copy, V: ?Sized, D> Clone for TrieEdgeIter<'_, N, V, D> {
    fn clone(&self) -> Self {
        Self {
            leading_path: self.leading_path.clone(),
            stack: self.stack.clone(),
            marker: std::marker::PhantomData,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.leading_path.clone_from(&source.leading_path);
        self.stack.clone_from(&source.stack);
    }
}

impl<N: Copy, V: ?Sized, D> Clone for TrieValueIter<'_, N, V, D> {
    fn clone(&self) -> Self {
        Self {
            edges: self.edges.clone(),
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.edges.clone_from(&source.edges);
    }
}

impl<P: Copy, N: Copy, V: ?Sized> Clone for TriePathIter<'_, P, N, V> {
    fn clone(&self) -> Self {
        Self {
            needle: self.needle,
            current_path: self.current_path.clone(),
            current_edge: self.current_edge,
            marker: std::marker::PhantomData,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.needle = source.needle;
        self.current_path.clone_from(&source.current_path);
        self.current_edge = source.current_edge;
    }
}

impl<N: Copy, V: ?Sized> Clone for Frame<'_, N, V> {
    fn clone(&self) -> Self {
        Self {
            node: self.node,
            hash: self.hash,
            leading_path_len: self.leading_path_len,
            children: self.children.clone(),
            marker: std::marker::PhantomData,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        self.node = source.node;
        self.hash = source.hash;
        self.leading_path_len = source.leading_path_len;
        self.children.clone_from(&source.children);
    }
}
