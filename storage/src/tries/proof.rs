// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{
    Children, HashType, Hashable, IntoSplitPath, PathBuf, PathComponent, SplitPath, TrieEdgeState,
    TrieNode, TriePath, ValueDigest,
};

/// An error indicating that a slice of proof nodes is invalid.
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
pub enum FromKeyProofError {
    /// The parent node's path is not a strict prefix the node that follows it.
    #[error(
        "parent node {parent_path} precedes child node {child_path} but is not a strict prefix of it",
        parent_path = parent_path.display(),
        child_path = child_path.display(),
    )]
    InvalidChildPath {
        /// The path of the parent node.
        parent_path: PathBuf,
        /// The path of the following child node.
        child_path: PathBuf,
    },
    /// The parent node does not reference the child node at the path component
    /// leading to the child node.
    #[error(
        "child node {child_path} is not reachable from parent node {parent_path}",
        parent_path = parent_path.display(),
        child_path = child_path.display(),
    )]
    MissingChild {
        /// The path of the parent node.
        parent_path: PathBuf,
        /// The path of the following child node.
        child_path: PathBuf,
    },
}

/// A root node in a trie formed from a key proof.
///
/// A proof trie follows a linear path from the root to a terminal node, and
/// includes the necessary information to calculate the hash of each node along
/// that path.
///
/// In the proof, each node will include the value or value digest at that node,
/// depending on what is required by the hasher. Additionally, the hashes of each
/// child node that branches off the node along the path are included.
#[derive(Debug)]
pub struct KeyProofTrieRoot<'a, P> {
    partial_path: P,
    value_digest: Option<ValueDigest<&'a [u8]>>,
    children: Children<Option<KeyProofTrieNode<'a, P>>>,
}

#[derive(Debug)]
enum KeyProofTrieNode<'a, P> {
    /// Described nodes are proof nodes where we have the data necessary to
    /// reconstruct the hash. The value digest may be a value or a digest. We can
    /// verify the hash of theses nodes using the value or digest, but may not
    /// have the full value.
    Described {
        node: Box<KeyProofTrieRoot<'a, P>>,
        hash: HashType,
    },
    /// Remote nodes are the nodes where we only know the ID, as discovered
    /// from a proof node. If we only have the child, we can't infer anything
    /// else about the node.
    Remote { hash: HashType },
}

impl<'a, P: SplitPath> KeyProofTrieRoot<'a, P> {
    /// Constructs a trie root from a slice of proof nodes.
    ///
    /// Each node in the slice must be a strict prefix of the following node. And,
    /// each child node must be referenced by its parent (i.e., the parent must
    /// indicate a child at the path component leading to the child). The hash
    /// is not verified here.
    ///
    /// # Errors
    ///
    /// - [`FromKeyProofError::InvalidChildPath`] if any node's path is not a strict
    ///   prefix of the following node's path.
    /// - [`FromKeyProofError::MissingChild`] if any parent node does not reference
    ///   the following child node at the path component leading to the child.
    pub fn new<T, N>(proof: &'a T) -> Result<Option<Box<Self>>, FromKeyProofError>
    where
        T: AsRef<[N]> + ?Sized,
        N: Hashable<FullPath<'a>: IntoSplitPath<Path = P>> + 'a,
    {
        proof
            .as_ref()
            .iter()
            .rev()
            .try_fold(None::<Box<Self>>, |parent, node| match parent {
                None => Ok(Some(Self::new_tail_node(node))),
                Some(p) => p.new_parent_node(node).map(Some),
            })
    }

    /// Creates a new trie root from the tail node of a proof.
    fn new_tail_node<N>(node: &'a N) -> Box<Self>
    where
        N: Hashable<FullPath<'a>: IntoSplitPath<Path = P>>,
    {
        Box::new(Self {
            partial_path: node.full_path().into_split_path(),
            value_digest: node.value_digest(),
            children: node
                .children()
                .map(|_, child| child.map(|hash| KeyProofTrieNode::Remote { hash })),
        })
    }

    /// Creates a new trie root by making this node a child of the given parent.
    ///
    /// The parent key must be a strict prefix of this node's key, and the parent
    /// must reference this node in its children by hash (the hash is not verified
    /// here).
    fn new_parent_node<N>(
        mut self: Box<Self>,
        parent: &'a N,
    ) -> Result<Box<Self>, FromKeyProofError>
    where
        N: Hashable<FullPath<'a>: IntoSplitPath<Path = P>>,
    {
        match parent
            .full_path()
            .into_split_path()
            .longest_common_prefix(self.partial_path)
            .split_first_parts()
        {
            (None, Some((pc, child_path)), parent_path) => {
                let mut parent = Self::new_tail_node(parent);
                if let Some(KeyProofTrieNode::Remote { hash }) = parent.children.take(pc) {
                    self.partial_path = child_path;
                    parent.partial_path = parent_path;
                    parent.children[pc] = Some(KeyProofTrieNode::Described { node: self, hash });
                    Ok(parent)
                } else {
                    Err(FromKeyProofError::MissingChild {
                        parent_path: parent.partial_path.as_component_slice().into_owned(),
                        child_path: self.partial_path.as_component_slice().into_owned(),
                    })
                }
            }
            _ => Err(FromKeyProofError::InvalidChildPath {
                parent_path: parent.full_path().as_component_slice().into_owned(),
                child_path: self.partial_path.as_component_slice().into_owned(),
            }),
        }
    }
}

impl<'a, P: IntoSplitPath + 'a> KeyProofTrieNode<'a, P> {
    const fn hash(&self) -> &HashType {
        match self {
            KeyProofTrieNode::Described { hash, .. } | KeyProofTrieNode::Remote { hash } => hash,
        }
    }

    const fn node(&self) -> Option<&KeyProofTrieRoot<'a, P>> {
        match self {
            KeyProofTrieNode::Described { node, .. } => Some(node),
            KeyProofTrieNode::Remote { .. } => None,
        }
    }

    const fn as_edge_state(&self) -> TrieEdgeState<'_, KeyProofTrieRoot<'a, P>> {
        match self {
            KeyProofTrieNode::Described { node, hash } => TrieEdgeState::LocalChild { node, hash },
            KeyProofTrieNode::Remote { hash } => TrieEdgeState::RemoteChild { hash },
        }
    }
}

impl<'a, P: SplitPath + 'a> TrieNode<ValueDigest<&'a [u8]>> for KeyProofTrieRoot<'a, P> {
    type PartialPath<'b>
        = P
    where
        Self: 'b;

    fn partial_path(&self) -> Self::PartialPath<'_> {
        self.partial_path
    }

    fn value(&self) -> Option<&ValueDigest<&'a [u8]>> {
        self.value_digest.as_ref()
    }

    fn child_hash(&self, pc: PathComponent) -> Option<&HashType> {
        self.children[pc].as_ref().map(KeyProofTrieNode::hash)
    }

    fn child_node(&self, pc: PathComponent) -> Option<&Self> {
        self.children[pc].as_ref().and_then(KeyProofTrieNode::node)
    }

    fn child_state(&self, pc: PathComponent) -> Option<super::TrieEdgeState<'_, Self>> {
        self.children[pc]
            .as_ref()
            .map(KeyProofTrieNode::as_edge_state)
    }
}
