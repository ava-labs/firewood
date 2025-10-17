// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod debug;
mod iter;
mod kvp;
mod proof;
mod range;

use crate::{HashType, IntoSplitPath, PathComponent, SplitPath};

pub use self::iter::{IterAscending, IterDescending, TrieEdgeIter, TriePathIter, TrieValueIter};
pub use self::kvp::{DuplicateKeyError, HashedKeyValueTrieRoot, KeyValueTrieRoot};
pub use self::proof::{
    FromKeyProofError, KeyProofTrieRoot, KeyRangeProofTrieRoot, MergeKeyProofError,
};
pub use self::range::{RangeProofTrieNode, RangeProofTrieRoot, RangeProofTrieRootRef};

/// The state of an edge from a parent node to a child node in a trie.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrieEdgeState<'root, N> {
    /// A child node that is fully known locally, along with its hash.
    LocalChild {
        /// The child node at this edge.
        node: N,
        /// The hash of the child at this edge, as known to the parent. A locally
        /// hashed child implements [`HashedTrieNode`]. It is possible for the
        /// child's computed hash to differ from this hash if the local node has
        /// incomplete information.
        hash: &'root HashType,
    },
    /// A child node that is not known locally, but whose hash is known to the
    /// parent.
    RemoteChild {
        /// The hash of the remote child at this edge, as known to the parent.
        hash: &'root HashType,
    },
    /// A child node that is known locally, but whose hash is not known to the
    /// parent.
    UnhashedChild {
        /// The child node at this edge.
        node: N,
    },
}

/// A node in a fixed-arity radix trie.
pub trait TrieNode<'root, V: AsRef<[u8]> + ?Sized + 'root>: Copy + 'root {
    /// The type of path from this node's parent to this node.
    type PartialPath: IntoSplitPath + 'root;

    /// The path from this node's parent to this node.
    fn partial_path(self) -> Self::PartialPath;

    /// The value stored at this node, if any.
    fn value(self) -> Option<&'root V>;

    /// The node-local hash of the child at the given path component, if any.
    ///
    /// This *may* be different from the child's computed hash the child has
    /// missing information.
    ///
    /// A trie node may also have a child node without knowing its hash, in which
    /// case this returns [`None`], but [`child_node`] will return [`Some`].
    ///
    /// A trie node may also know the hash of a child without having a reference
    /// to the child's node. In this case, this will return [`Some`], but
    /// [`child_node`] return [`None`]. For example, this occurs in the proof
    /// trie where a proof follows a linear path down the trie and only includes
    /// the hashes of sibling nodes that branch off the path.
    ///
    /// [`child_node`]: TrieNode::child_node
    fn child_hash(self, pc: PathComponent) -> Option<&'root HashType>;

    /// The child node at the given path component, if any.
    ///
    /// See the documentation for [`child_hash`] for more details on the
    /// relationship between these two methods.
    ///
    /// [`child_hash`]: TrieNode::child_hash
    fn child_node(self, pc: PathComponent) -> Option<Self>;

    /// A combined view of the child node and its hash at the given path
    /// component, if any.
    ///
    /// This is a combination of [`child_node`] and [`child_hash`], returning
    /// a [`TrieEdgeState`] that describes which of the child node and
    /// hash are known.
    ///
    /// [`child_node`]: TrieNode::child_node
    /// [`child_hash`]: TrieNode::child_hash
    fn child_state(self, pc: PathComponent) -> Option<TrieEdgeState<'root, Self>> {
        match (self.child_node(pc), self.child_hash(pc)) {
            (Some(node), Some(hash)) => Some(TrieEdgeState::LocalChild { node, hash }),
            (Some(node), None) => Some(TrieEdgeState::UnhashedChild { node }),
            (None, Some(hash)) => Some(TrieEdgeState::RemoteChild { hash }),
            (None, None) => None,
        }
    }

    /// Returns an iterator over the edges along a specified path in this trie
    /// terminating at the node corresponding to the path, if it exists;
    /// otherwise, terminating at the deepest existing edge along the path.
    ///
    /// The returned iterator yields each edge along the path as a tuple where
    /// the first element is full path to the edge, inclusive of the edge's
    /// leading path components, and the second element is the edge state.
    fn iter_path<P: SplitPath>(self, path: P) -> TriePathIter<'root, P, Self, V> {
        TriePathIter::new(self, None, path)
    }

    /// Returns a breadth-first iterator over the edges in this trie in ascending
    /// order.
    ///
    /// The returned iterator performs a pre-order traversal of the trie, yielding
    /// each edge from parent to child before descending into the child node. The
    /// children of each node are yielded in ascending order by path component.
    fn iter_edges(self) -> TrieEdgeIter<'root, Self, V, IterAscending> {
        TrieEdgeIter::new(self, None)
    }

    /// Returns a depth-first iterator over the edges in this trie in descending
    /// order.
    ///
    /// The returned iterator performs a post-order traversal of the trie, yielding
    /// each edge from parent to child after ascending back from the child node.
    /// The children of each node are yielded in descending order by path component.
    fn iter_edges_desc(self) -> TrieEdgeIter<'root, Self, V, IterDescending> {
        TrieEdgeIter::new(self, None)
    }

    /// Returns an iterator over each key-value pair in this trie in ascending order.
    fn iter_values(self) -> TrieValueIter<'root, Self, V, IterAscending> {
        self.iter_edges().node_values()
    }

    /// Returns an iterator over each key-value pair in this trie in descending order.
    fn iter_values_desc(self) -> TrieValueIter<'root, Self, V, IterDescending> {
        self.iter_edges_desc().node_values()
    }
}

/// A merkleized node in a fixed-arity radix trie.
pub trait HashedTrieNode<'root, V: AsRef<[u8]> + ?Sized + 'root>: TrieNode<'root, V> {
    /// The computed hash of this node.
    fn computed(self) -> &'root HashType;
}

impl<'root, N> TrieEdgeState<'root, N> {
    const fn from_node(node: N, hash: Option<&'root HashType>) -> Self {
        match hash {
            Some(hash) => TrieEdgeState::LocalChild { node, hash },
            None => TrieEdgeState::UnhashedChild { node },
        }
    }

    fn value<V: AsRef<[u8]> + ?Sized + 'root>(self) -> Option<&'root V>
    where
        N: TrieNode<'root, V>,
    {
        self.node().and_then(TrieNode::value)
    }

    /// Maps the child node using the given function, preserving the edge state.
    #[must_use]
    pub fn map_node<O>(self, f: impl FnOnce(N) -> O) -> TrieEdgeState<'root, O> {
        match self {
            TrieEdgeState::LocalChild { node, hash } => TrieEdgeState::LocalChild {
                node: f(node),
                hash,
            },
            TrieEdgeState::RemoteChild { hash } => TrieEdgeState::RemoteChild { hash },
            TrieEdgeState::UnhashedChild { node } => TrieEdgeState::UnhashedChild { node: f(node) },
        }
    }

    /// Returns `true` if this edge state represents a local child node with a known hash.
    #[must_use]
    pub fn is_local(self) -> bool {
        matches!(self, TrieEdgeState::LocalChild { .. })
    }

    /// Returns `true` if this edge state represents a remote child node with only a known hash.
    #[must_use]
    pub fn is_remote(self) -> bool {
        matches!(self, TrieEdgeState::RemoteChild { .. })
    }

    /// Returns `true` if this edge state represents a local child node without a known hash.
    #[must_use]
    pub fn is_unhashed(self) -> bool {
        matches!(self, TrieEdgeState::UnhashedChild { .. })
    }

    /// Returns the child node if it is known locally.
    #[must_use]
    pub fn node(self) -> Option<N> {
        match self {
            TrieEdgeState::LocalChild { node, .. } | TrieEdgeState::UnhashedChild { node } => {
                Some(node)
            }
            TrieEdgeState::RemoteChild { .. } => None,
        }
    }

    /// Returns the hash of the child node if it is known.
    #[must_use]
    pub fn hash(self) -> Option<&'root HashType> {
        match self {
            TrieEdgeState::LocalChild { hash, .. } | TrieEdgeState::RemoteChild { hash } => {
                Some(hash)
            }
            TrieEdgeState::UnhashedChild { .. } => None,
        }
    }
}
