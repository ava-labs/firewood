// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod debug;
mod iter;
mod kvp;
mod proof;
mod range;

pub use self::iter::{IterAscending, IterDescending, TrieEdgeIter, TrieValueIter};
pub use self::kvp::{DuplicateKeyError, HashedKeyValueTrieRoot, KeyValueTrieRoot};
pub use self::proof::{
    FromKeyProofError, KeyProofTrieRoot, KeyRangeProofTrieRoot, MergeKeyProofError,
};
pub use self::range::{
    RangeProofError, RangeProofTrieEdge, RangeProofTrieNode, RangeProofTrieRoot,
};
use crate::path::PathRef;
use crate::{HashType, PathComponent, ValueDigest};

/// The state of an edge from a parent node to a child node in a trie.
#[derive(Clone, Copy, Debug)]
pub enum TrieEdgeState<'a> {
    /// A child node that is fully known locally, along with its hash.
    LocalChild {
        /// The child node at this edge.
        node: &'a dyn TrieNode,
        /// The hash of the child at this edge, as known to the parent. A locally
        /// hashed child implements [`HashedTrieNode`]. It is possible for the
        /// child's computed hash to differ from this hash if the local node has
        /// incomplete information.
        hash: &'a HashType,
    },
    /// A child node that is not known locally, but whose hash is known to the
    /// parent.
    RemoteChild {
        /// The hash of the remote child at this edge, as known to the parent.
        hash: &'a HashType,
    },
    /// A child node that is known locally, but whose hash is not known to the
    /// parent.
    UnhashedChild {
        /// The child node at this edge.
        node: &'a dyn TrieNode,
    },
}

/// A node in a fixed-arity radix trie.
pub trait TrieNode: std::fmt::Debug {
    /// The path from this node's parent to this node.
    fn partial_path(&self) -> PathRef<'_>;

    /// The value or digest stored at this node, if any.
    fn value_digest(&self) -> Option<ValueDigest<&[u8]>>;

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
    fn child_hash(&self, pc: PathComponent) -> Option<&HashType>;

    /// The child node at the given path component, if any.
    ///
    /// See the documentation for [`child_hash`] for more details on the
    /// relationship between these two methods.
    ///
    /// [`child_hash`]: TrieNode::child_hash
    fn child_node(&self, pc: PathComponent) -> Option<&dyn TrieNode>;

    /// A combined view of the child node and its hash at the given path
    /// component, if any.
    ///
    /// This is a combination of [`child_node`] and [`child_hash`], returning
    /// a [`TrieEdgeState`] that describes which of the child node and
    /// hash are known.
    ///
    /// [`child_node`]: TrieNode::child_node
    /// [`child_hash`]: TrieNode::child_hash
    fn child_state(&self, pc: PathComponent) -> Option<TrieEdgeState<'_>> {
        match (self.child_node(pc), self.child_hash(pc)) {
            (Some(node), Some(hash)) => Some(TrieEdgeState::LocalChild { node, hash }),
            (Some(node), None) => Some(TrieEdgeState::UnhashedChild { node }),
            (None, Some(hash)) => Some(TrieEdgeState::RemoteChild { hash }),
            (None, None) => None,
        }
    }

    /// Upcasts this node to a trait object for use in generic contexts where
    /// the concrete type of the node is not known.
    ///
    /// This is a convenience method for cases where the coersion is not done
    /// automatically.
    fn upcast(&self) -> &dyn TrieNode
    where
        Self: Sized,
    {
        self
    }

    /// Returns a breadth-first iterator over the edges in this trie in ascending
    /// order.
    ///
    /// The returned iterator performs a pre-order traversal of the trie, yielding
    /// each edge from parent to child before descending into the child node. The
    /// children of each node are yielded in ascending order by path component.
    fn iter_edges(&self) -> TrieEdgeIter<'_, IterAscending>
    where
        Self: Sized,
    {
        TrieEdgeIter::new(self)
    }

    /// Returns a depth-first iterator over the edges in this trie in descending
    /// order.
    ///
    /// The returned iterator performs a post-order traversal of the trie, yielding
    /// each edge from parent to child after ascending back from the child node.
    /// The children of each node are yielded in descending order by path component.
    fn iter_edges_desc(&self) -> TrieEdgeIter<'_, IterDescending>
    where
        Self: Sized,
    {
        TrieEdgeIter::new(self)
    }

    /// Returns an iterator over each key-value pair in this trie in ascending order.
    fn iter_values(&self) -> TrieValueIter<'_, IterAscending>
    where
        Self: Sized,
    {
        self.iter_edges().node_values()
    }

    /// Returns an iterator over each key-value pair in this trie in descending order.
    fn iter_values_desc(&self) -> TrieValueIter<'_, IterDescending>
    where
        Self: Sized,
    {
        self.iter_edges_desc().node_values()
    }
}

impl<T: TrieNode + ?Sized> TrieNode for &T {
    fn partial_path(&self) -> PathRef<'_> {
        (**self).partial_path()
    }

    fn value_digest(&self) -> Option<ValueDigest<&[u8]>> {
        (**self).value_digest()
    }

    fn child_hash(&self, pc: PathComponent) -> Option<&HashType> {
        (**self).child_hash(pc)
    }

    fn child_node(&self, pc: PathComponent) -> Option<&dyn TrieNode> {
        (**self).child_node(pc)
    }

    fn child_state(&self, pc: PathComponent) -> Option<TrieEdgeState<'_>> {
        (**self).child_state(pc)
    }
}

/// A merkleized node in a fixed-arity radix trie.
pub trait HashedTrieNode: TrieNode {
    /// The computed hash of this node.
    fn computed(&self) -> &HashType;
}

impl<'a> TrieEdgeState<'a> {
    fn from_node(node: &'a dyn TrieNode, hash: Option<&'a HashType>) -> Self {
        match hash {
            Some(hash) => TrieEdgeState::LocalChild { node, hash },
            None => TrieEdgeState::UnhashedChild { node },
        }
    }

    /// Returns `true` if this edge state represents a local child node with a known hash.
    #[must_use]
    pub const fn is_local(&self) -> bool {
        matches!(self, TrieEdgeState::LocalChild { .. })
    }

    /// Returns `true` if this edge state represents a remote child node with only a known hash.
    #[must_use]
    pub const fn is_remote(&self) -> bool {
        matches!(self, TrieEdgeState::RemoteChild { .. })
    }

    /// Returns `true` if this edge state represents a local child node without a known hash.
    #[must_use]
    pub const fn is_unhashed(&self) -> bool {
        matches!(self, TrieEdgeState::UnhashedChild { .. })
    }

    /// Returns the child node if it is known locally.
    #[must_use]
    pub const fn node(&'a self) -> Option<&'a dyn TrieNode> {
        match self {
            TrieEdgeState::LocalChild { node, .. } | TrieEdgeState::UnhashedChild { node } => {
                Some(node)
            }
            TrieEdgeState::RemoteChild { .. } => None,
        }
    }

    /// Returns the hash of the child node if it is known.
    #[must_use]
    pub const fn hash(&self) -> Option<&HashType> {
        match self {
            TrieEdgeState::LocalChild { hash, .. } | TrieEdgeState::RemoteChild { hash } => {
                Some(hash)
            }
            TrieEdgeState::UnhashedChild { .. } => None,
        }
    }
}

impl<L: TrieNode, R: TrieNode> TrieNode for either::Either<L, R> {
    fn partial_path(&self) -> PathRef<'_> {
        either::for_both!(self, this => this.partial_path())
    }

    fn value_digest(&self) -> Option<ValueDigest<&[u8]>> {
        either::for_both!(self, this => this.value_digest())
    }

    fn child_hash(&self, pc: PathComponent) -> Option<&HashType> {
        either::for_both!(self, this => this.child_hash(pc))
    }

    fn child_node(&self, pc: PathComponent) -> Option<&dyn TrieNode> {
        either::for_both!(self, this => this.child_node(pc))
    }

    fn child_state(&self, pc: PathComponent) -> Option<TrieEdgeState<'_>> {
        either::for_both!(self, this => this.child_state(pc))
    }
}
