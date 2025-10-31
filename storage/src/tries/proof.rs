// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{
    Children, HashType, Hashable, IntoSplitPath, PathBuf, PathComponent, PathGuard, SplitPath,
    TrieEdgeState, TrieNode, TriePath, ValueDigest,
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

/// An error indicating that merging two key proof tries failed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
pub enum MergeKeyProofError {
    /// The two key proofs are from disjoint tries.
    #[error(
        "the two key proofs are from disjoint tries starting at {leading_path}, with lower bound {lower_path} and upper bound {upper_path}",
        leading_path = leading_path.display(),
        lower_path = lower_path.display(),
        upper_path = upper_path.display(),
    )]
    DisjointEdges {
        /// The path leading to the point where the two tries diverge.
        leading_path: PathBuf,
        /// The path of the lower bound proof.
        lower_path: PathBuf,
        /// The path of the upper bound proof.
        upper_path: PathBuf,
    },
    /// The two key proofs contain a child node that is only present in one of
    /// the two tries.
    #[error("only one of the two key proofs contains a child starting at {path}", path = path.display())]
    DisjointChildren {
        /// The path of the child node that is only present in one of the two tries.
        path: PathBuf,
    },
    /// The two key proofs contain a child node with differing hashes.
    #[error("the two nodes at {path} have differing hashes", path = path.display())]
    DifferentHash {
        /// The path of the node with differing hashes.
        path: PathBuf,
        /// The hash of the node in the lower bound proof.
        lower_hash: HashType,
        /// The hash of the node in the upper bound proof.
        upper_hash: HashType,
    },
    /// The two key proofs both contain the same key with different values.
    #[error(
        "the two key proofs both contain the key {key_path} with different values",
        key_path = key_path.display(),
    )]
    DuplicateKeys {
        /// The path of the duplicate key.
        key_path: PathBuf,
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

/// A key range proof is a combination of zero, one, or two key proofs.
///
/// The key proofs represent the lower and upper bounds of the range, if they
/// exist. A key range proof trie is formed by merging the two bounding key
/// proofs into a single trie, which is later merged with the key-value pairs
/// within the range to form a complete range proof trie.
///
/// If there are zero bounding key proofs, the range is unbounded in both
/// directions. In which case, this trie is empty as there was no information
/// to include.
///
/// If there is only one bounding key proof, the range is unbounded in the
/// direction opposite the provided bound. In which case, this trie is simply
/// the provided key proof trie.
///
/// When there are two bounding key proofs, the range is inclusively bounded
/// in both directions. In which case, this trie is the merging of the two
/// proof tries. Upon merging, the two tries must overlap at the root and every
/// node along the path until the two paths diverge.
///
/// When merging the key-value pairs into this trie, key-values must be within
/// the bounds set by the two key proofs, if they exist.
#[derive(Debug)]
pub struct KeyRangeProofTrieRoot<'a, P> {
    root: Box<KeyProofTrieRoot<'a, P>>,
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

impl<'root, P: SplitPath + 'root> KeyProofTrieRoot<'root, P> {
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
    pub fn new<T, N>(proof: &'root T) -> Result<Option<Box<Self>>, FromKeyProofError>
    where
        T: AsRef<[N]> + ?Sized,
        N: Hashable<FullPath<'root>: IntoSplitPath<Path = P>> + 'root,
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
    fn new_tail_node<N>(node: &'root N) -> Box<Self>
    where
        N: Hashable<FullPath<'root>: IntoSplitPath<Path = P>>,
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
        parent: &'root N,
    ) -> Result<Box<Self>, FromKeyProofError>
    where
        N: Hashable<FullPath<'root>: IntoSplitPath<Path = P>>,
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

    fn merge_opt(
        leading_path: PathGuard<'_>,
        lower: Option<Box<Self>>,
        upper: Option<Box<Self>>,
    ) -> Result<Option<Box<Self>>, MergeKeyProofError> {
        match (lower, upper) {
            (None, None) => Ok(None),
            (Some(root), None) | (None, Some(root)) => Ok(Some(root)),
            (Some(lower), Some(upper)) => Self::merge(leading_path, lower, upper).map(Some),
        }
    }

    /// Iteratively merges two key proofs into a single key proof.
    ///
    /// The two proofs must overlap at the root and every `Described` node along
    /// the path until the two paths diverge.
    fn merge(
        mut leading_path: PathGuard<'_>,
        mut lower: Box<Self>,
        #[expect(clippy::boxed_local)] mut upper: Box<Self>,
    ) -> Result<Box<Self>, MergeKeyProofError> {
        if !lower.partial_path.path_eq(&upper.partial_path) {
            return Err(MergeKeyProofError::DisjointEdges {
                leading_path: leading_path.cloned(),
                lower_path: lower.partial_path.as_component_slice().into_owned(),
                upper_path: upper.partial_path.as_component_slice().into_owned(),
            });
        }

        leading_path.extend(lower.partial_path.components());

        if lower.value_digest != upper.value_digest {
            return Err(MergeKeyProofError::DuplicateKeys {
                key_path: leading_path.cloned(),
            });
        }

        lower.children = std::mem::take(&mut lower.children).merge(
            std::mem::take(&mut upper.children),
            |pc, lower, upper| {
                KeyProofTrieNode::merge_opt(leading_path.fork_append(pc), lower, upper)
            },
        )?;

        Ok(lower)
    }
}

impl<'root, P: SplitPath + 'root> KeyRangeProofTrieRoot<'root, P> {
    /// Constructs a key range proof trie from the optional lower and upper
    /// bounding key proof tries.
    ///
    /// # Errors
    ///
    /// - [`MergeKeyProofError::DisjointEdges`] if both bounding key proofs are
    ///   provided but do not overlap at the root or every `Described` node along
    ///   the path until the two paths diverge.
    ///
    /// - [`MergeKeyProofError::DisjointChildren`] if both bounding key proofs are
    ///   provided but contain a child node that is only present in one of the two
    ///   tries.
    ///
    /// - [`MergeKeyProofError::DifferentHash`] if both bounding key proofs are
    ///   provided but contain a child node with differing hashes.
    ///
    /// - [`MergeKeyProofError::DuplicateKeys`] if both bounding key proofs are
    ///   provided but both contain the same key with different values.
    pub fn new(
        lower: Option<Box<KeyProofTrieRoot<'root, P>>>,
        upper: Option<Box<KeyProofTrieRoot<'root, P>>>,
    ) -> Result<Option<Self>, MergeKeyProofError> {
        KeyProofTrieRoot::merge_opt(PathGuard::new(&mut PathBuf::new_const()), lower, upper)
            .map(|root| root.map(|root| Self { root }))
    }

    /// Returns the root key proof trie that was created by merging the two
    /// bounding key proofs.
    #[must_use]
    pub const fn root(&self) -> &KeyProofTrieRoot<'root, P> {
        &self.root
    }
}

impl<'root, P: SplitPath + 'root> KeyProofTrieNode<'root, P> {
    const fn hash(&'root self) -> &'root HashType {
        match self {
            KeyProofTrieNode::Described { hash, .. } | KeyProofTrieNode::Remote { hash } => hash,
        }
    }

    const fn node(&'root self) -> Option<&'root KeyProofTrieRoot<'root, P>> {
        match self {
            KeyProofTrieNode::Described { node, .. } => Some(node),
            KeyProofTrieNode::Remote { .. } => None,
        }
    }

    const fn as_edge_state(&'root self) -> TrieEdgeState<'root, &'root KeyProofTrieRoot<'root, P>> {
        match self {
            KeyProofTrieNode::Described { node, hash } => TrieEdgeState::LocalChild {
                node: &**node,
                hash,
            },
            KeyProofTrieNode::Remote { hash } => TrieEdgeState::RemoteChild { hash },
        }
    }

    fn merge_opt(
        leading_path: PathGuard<'_>,
        lower: Option<Self>,
        upper: Option<Self>,
    ) -> Result<Option<Self>, MergeKeyProofError> {
        match (lower, upper) {
            (None, None) => Ok(None),
            (Some(_), None) | (None, Some(_)) => Err(MergeKeyProofError::DisjointChildren {
                path: leading_path.cloned(),
            }),
            (Some(lower), Some(upper)) => Self::merge(leading_path, lower, upper).map(Some),
        }
    }

    fn merge(
        leading_path: PathGuard<'_>,
        lower: Self,
        upper: Self,
    ) -> Result<Self, MergeKeyProofError> {
        match (lower, upper) {
            (
                Self::Remote { hash: lower_hash }
                | Self::Described {
                    node: _,
                    hash: lower_hash,
                },
                Self::Remote { hash: upper_hash }
                | Self::Described {
                    node: _,
                    hash: upper_hash,
                },
            ) if lower_hash != upper_hash => Err(MergeKeyProofError::DifferentHash {
                path: leading_path.cloned(),
                lower_hash,
                upper_hash,
            }),
            (edge, Self::Remote { .. }) | (Self::Remote { .. }, edge) => Ok(edge),
            (Self::Described { node: lower, hash }, Self::Described { node: upper, .. }) => {
                KeyProofTrieRoot::merge(leading_path, lower, upper)
                    .map(|node| Self::Described { node, hash })
            }
        }
    }
}

impl<'root, P> TrieNode<'root, ValueDigest<&'root [u8]>> for &'root KeyProofTrieRoot<'root, P>
where
    P: SplitPath + 'root,
{
    type PartialPath = P;

    fn partial_path(self) -> Self::PartialPath {
        self.partial_path
    }

    fn value(self) -> Option<&'root ValueDigest<&'root [u8]>> {
        self.value_digest.as_ref()
    }

    fn child_hash(self, pc: PathComponent) -> Option<&'root HashType> {
        self.children[pc].as_ref().map(KeyProofTrieNode::hash)
    }

    fn child_node(self, pc: PathComponent) -> Option<Self> {
        self.children[pc].as_ref().and_then(KeyProofTrieNode::node)
    }

    fn child_state(self, pc: PathComponent) -> Option<super::TrieEdgeState<'root, Self>> {
        self.children[pc]
            .as_ref()
            .map(KeyProofTrieNode::as_edge_state)
    }
}
