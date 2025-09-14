// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::{BranchNode, Children, HashType, ValueDigest, logger::trace};

use crate::{
    proof::{
        DuplicateKeysInProofError, Proof, ProofCollection, ProofError, ProofNode,
        UnexpectedHashError,
    },
    proofs::{
        path::{
            CollectedNibbles, Nibbles, PathGuard, PathNibble, SplitNibbles, SplitPath, WidenedPath,
        },
        trie::{counter::NibbleCounter, iter::Child},
    },
};

/// A root node in a trie formed from a [`ProofNode`].
#[derive(Debug)]
pub(super) struct KeyProofTrieRoot<'a> {
    pub(super) partial_path: WidenedPath<'a>,
    pub(super) value_digest: Option<ValueDigest<&'a [u8]>>,
    pub(super) children: Children<Box<KeyProofTrieEdge<'a>>>,
}

/// A node in a trie formed a collection of [`ProofNode`]s.
#[derive(Debug)]
pub(super) enum KeyProofTrieEdge<'a> {
    /// Remote nodes are the nodes where we only know the ID, as discovered
    /// from a proof node. If we only have the child, we can't infer anything
    /// else about the node.
    Remote(HashType),
    /// Described nodes are proof nodes where we have the data necessary to
    /// reconstruct the hash. The value digest may be a value or a digest. We can
    /// verify the hash of theses nodes using the value or digest, but may not
    /// have the full value.
    Described(HashType, KeyProofTrieRoot<'a>),
}

impl<'a> KeyProofTrieRoot<'a> {
    /// Constructs a trie root from a slice of proof nodes.
    ///
    /// Each node in the slice must be a strict prefix of the following node. And,
    /// each child node must be referenced by its parent.
    pub(super) fn new<P>(nodes: &'a Proof<P>) -> Result<Option<Self>, ProofError>
    where
        P: ProofCollection<Node = ProofNode>,
    {
        nodes
            .as_ref()
            .iter()
            .rev()
            .try_fold(None::<Self>, |child, parent| match child {
                // each node in the slice must be a strict prefix of the following node
                Some(child) => child.new_parent_node(parent).map(Some),
                None => Ok(Some(Self::new_tail_node(parent))),
            })
    }

    pub fn lower_bound(&self) -> (CollectedNibbles, &KeyProofTrieRoot<'_>) {
        let mut path = CollectedNibbles::empty();
        let mut this = self;
        loop {
            path.extend(this.partial_path);
            let Some((nibble, child)) = this.children.iter().enumerate().find_map(|(i, child)| {
                child
                    .as_deref()
                    .and_then(KeyProofTrieEdge::root)
                    .map(|child| (i as u8, child))
            }) else {
                return (path, this);
            };
            path.extend(PathNibble(nibble));
            this = child;
        }
    }

    pub fn upper_bound(&self) -> (CollectedNibbles, &KeyProofTrieRoot<'_>) {
        let mut path = CollectedNibbles::empty();
        let mut this = self;
        loop {
            path.extend(this.partial_path);
            let Some((nibble, child)) =
                this.children
                    .iter()
                    .enumerate()
                    .rev()
                    .find_map(|(i, child)| {
                        child
                            .as_deref()
                            .and_then(KeyProofTrieEdge::root)
                            .map(|child| (i as u8, child))
                    })
            else {
                return (path, this);
            };
            path.extend(PathNibble(nibble));
            this = child;
        }
    }

    fn new_tail_node(node: &'a ProofNode) -> Self {
        let partial_path = WidenedPath::new(node.key.as_ref());
        trace!(
            "KeyProofTrieRoot: creating tail node for key {}",
            hex::encode(partial_path.bytes_iter().collect::<Vec<_>>()),
        );

        Self {
            partial_path,
            value_digest: node.value_digest.as_ref().map(ValueDigest::as_ref),
            // A tail node is allowed to have children; they will all be remote.
            // However, a tail node with children is also only valid if the proof
            // is an exclusion proof on some key that would be a child of this node.
            children: super::boxed_children(
                node.child_hashes
                    .clone()
                    .map(|maybe| maybe.map(KeyProofTrieEdge::Remote)),
            ),
        }
    }

    /// Creates a new trie root by making this node a child of the given parent.
    ///
    /// The parent key must be a strict prefix of this node's key, and the parent
    /// must reference this node in its children by hash (the hash is not verified
    /// here).
    fn new_parent_node(self, parent: &'a ProofNode) -> Result<Self, ProofError> {
        let parent_path = WidenedPath::new(&parent.key);
        trace!(
            "KeyProofTrieRoot: adding parent for key {}; parent: {}",
            hex::encode(self.partial_path.bytes_iter().collect::<Vec<_>>()),
            hex::encode(parent_path.bytes_iter().collect::<Vec<_>>()),
        );

        let split = SplitPath::new(parent_path, self.partial_path);

        let ((None, _), (Some(nibble), partial_path)) = (
            split.lhs_suffix.split_first(),
            split.rhs_suffix.split_first(),
        ) else {
            return Err(ProofError::ShouldBePrefixOfNextKey {
                parent: parent_path.bytes_iter().collect(),
                child: self.partial_path.bytes_iter().collect(),
            });
        };

        let mut with_self = BranchNode::empty_children();
        with_self
            .get_mut(nibble.0 as usize)
            .ok_or_else(|| {
                ProofError::ChildIndexOutOfBounds(self.partial_path.nibbles_iter().collect())
            })?
            .replace(self);

        let children =
            super::merge_array(parent.child_hashes.clone(), with_self, |hash, this| match (
                hash, this,
            ) {
                (None, None) => Ok(None),
                (None, Some(this)) => Err(ProofError::NodeNotInTrie {
                    parent: parent_path.bytes_iter().collect(),
                    child: this.partial_path.bytes_iter().collect(),
                }),
                (Some(hash), None) => Ok(Some(KeyProofTrieEdge::Remote(hash))),
                (Some(hash), Some(this)) => Ok(Some(KeyProofTrieEdge::Described(
                    hash,
                    Self {
                        partial_path,
                        ..this
                    },
                ))),
            })?;

        Ok(Self {
            partial_path: parent_path,
            value_digest: parent.value_digest.as_ref().map(ValueDigest::as_ref),
            children: super::boxed_children(children),
        })
    }

    /// Merges two trie roots, as returned by [`Self::new`].
    ///
    /// Every node in both tries must be equal by hash. [`KeyProofTrieEdge::Described`]
    /// nodes in both tries must have the same key and value digest. The resulting
    /// trie will contain all nodes from both tries and resolve any
    /// [`KeyProofTrieEdge::Remote`] nodes to the corresponding
    /// [`KeyProofTrieEdge::Described`] if one is present in the opposite trie.
    ///
    /// Children that are present in both tries will be merged recursively. Nodes
    /// that are in both tries must have the same key and value digest.
    pub(super) fn merge_opt(
        leading_path: PathGuard<'_>,
        lhs: Option<Self>,
        rhs: Option<Self>,
    ) -> Result<Option<Self>, ProofError> {
        match (lhs, rhs) {
            (Some(lhs), Some(rhs)) => Self::merge(leading_path, lhs, rhs).map(Some),
            (Some(root), None) | (None, Some(root)) => Ok(Some(root)),
            (None, None) => Ok(None),
        }
    }

    fn merge(mut leading_path: PathGuard<'_>, lhs: Self, rhs: Self) -> Result<Self, ProofError> {
        trace!(
            "KeyProofTrieRoot: merging two proof roots; leading_path: {}, lhs: {}, rhs: {}",
            leading_path.display(),
            lhs.partial_path.display(),
            rhs.partial_path.display(),
        );

        // at this level, keys must be identical
        if lhs.partial_path != rhs.partial_path {
            return Err(ProofError::DisjointEdges {
                lower: (&leading_path)
                    .join(lhs.partial_path)
                    .bytes_iter()
                    .collect(),
                upper: (&leading_path)
                    .join(rhs.partial_path)
                    .bytes_iter()
                    .collect(),
            });
        }

        let mut leading_path = leading_path.fork_push(lhs.partial_path);

        if lhs.value_digest != rhs.value_digest {
            return Err(ProofError::DuplicateKeysInProof(
                DuplicateKeysInProofError {
                    key: leading_path.bytes_iter().collect(),
                    value1: format!("{:?}", lhs.value_digest),
                    value2: format!("{:?}", rhs.value_digest),
                },
            ));
        }

        let mut nibble = NibbleCounter::new();
        let children = super::merge_array(lhs.children, rhs.children, |lhs, rhs| {
            KeyProofTrieEdge::new(
                leading_path.fork_push(nibble.next()),
                lhs.map(|v| *v),
                rhs.map(|v| *v),
            )
        })?;

        Ok(Self {
            children: super::boxed_children(children),
            ..lhs
        })
    }
}

impl KeyProofTrieEdge<'_> {
    fn new(
        leading_path: PathGuard<'_>,
        lhs: Option<Self>,
        rhs: Option<Self>,
    ) -> Result<Option<Self>, ProofError> {
        match (lhs, rhs) {
            (None, None) => Ok(None),
            // TODO: this should be unreachable or an error, but regardless will
            // cause the hash to mismatch later down the line
            (Some(child), None) | (None, Some(child)) => Ok(Some(child)),
            (Some(Self::Remote(lhs)), Some(Self::Remote(rhs))) => {
                if lhs == rhs {
                    Ok(Some(Self::Remote(lhs)))
                } else {
                    Err(ProofError::UnexpectedHash(Box::new(UnexpectedHashError {
                        key: leading_path.bytes_iter().collect(),
                        context: "merging two remote edges with different hashes",
                        expected: lhs,
                        actual: rhs,
                    })))
                }
            }
            (Some(Self::Remote(hash)), Some(Self::Described { 0: id, 1: root }))
            | (Some(Self::Described { 0: id, 1: root }), Some(Self::Remote(hash))) => {
                if hash == id {
                    Ok(Some(Self::Described(id, root)))
                } else {
                    Err(ProofError::UnexpectedHash(Box::new(UnexpectedHashError {
                        key: leading_path.join(root.partial_path).bytes_iter().collect(),
                        context: "merging a remote edge with a described edge with different hashes",
                        expected: hash,
                        actual: id,
                    })))
                }
            }
            (
                Some(Self::Described { 0: l_id, 1: l_root }),
                Some(Self::Described { 0: r_id, 1: r_root }),
            ) => {
                if l_id == r_id {
                    KeyProofTrieRoot::merge(leading_path, l_root, r_root)
                        .map(|root| Some(Self::Described(l_id, root)))
                } else {
                    Err(ProofError::UnexpectedHash(Box::new(UnexpectedHashError {
                        key: leading_path
                            .join(l_root.partial_path)
                            .bytes_iter()
                            .collect(),
                        context: "merging two described edges with different hashes",
                        expected: l_id,
                        actual: r_id,
                    })))
                }
            }
        }
    }

    const fn root(&self) -> Option<&KeyProofTrieRoot<'_>> {
        match self {
            Self::Remote(_) => None,
            Self::Described(_, root) => Some(root),
        }
    }
}

impl<'a> super::TrieNode<'a> for &'a KeyProofTrieRoot<'a> {
    type Nibbles = WidenedPath<'a>;

    fn partial_path(self) -> Self::Nibbles {
        self.partial_path
    }

    fn value_digest(self) -> Option<ValueDigest<&'a [u8]>> {
        self.value_digest.clone()
    }

    fn computed_hash(self) -> Option<HashType> {
        None
    }

    fn children(self) -> Children<Child<Self>> {
        self.children.each_ref().map(|child| {
            child.as_deref().map(|child| match child {
                KeyProofTrieEdge::Remote(hash) => Child::Remote(hash.clone()),
                KeyProofTrieEdge::Described(hash, node) => Child::Hashed(hash.clone(), node),
            })
        })
    }
}
