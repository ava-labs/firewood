// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{
    Children, HashType, KeyProofTrieRoot, KeyRangeProofTrieRoot, KeyValueTrieRoot, PackedPathRef,
    PathBuf, PathComponent, PathGuard, SplitPath, TrieNode, TriePath, ValueDigest,
    tries::{
        debug::{DebugChildren, DebugValue},
        proof::KeyProofTrieNode,
    },
};

/// Errors that can occur when merging a range proof trie with a key-value pair trie.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash, thiserror::Error)]
pub enum RangeProofError {
    /// Disjoint edges occur when we traverse into nodes from both tries with
    /// differing paths.
    #[error(
        "the key-value pair trie diverges from the proof trie at {leading_path}, with a proof node at {proof_path} and key-value pair node at {kvp_path}",
        leading_path = leading_path.display(),
        proof_path = proof_path.display(),
        kvp_path = kvp_path.display(),
    )]
    DisjointEdges {
        /// The leading path to the diverging nodes.
        leading_path: PathBuf,
        /// The path of the proof trie at the diverging node.
        proof_path: PathBuf,
        /// The path of the key-value pair trie at the diverging node.
        kvp_path: PathBuf,
    },
    /// An entire key-value trie was found at the key prefix that was not described
    /// by the proof trie.
    #[error(
        "unexpected key prefix found beginning at {prefix}", prefix = prefix.display()
    )]
    UnexpectedKeyPrefix {
        /// The path where a key prefix was found in the kvp trie but was not in the proof trie.
        prefix: PathBuf,
    },
    /// An unexpected value was found in the key-value pair trie that was not
    /// declared by the proof trie.
    #[error("unexpected value found at {path}", path = path.display())]
    UnexpectedValue {
        /// The path where a value was found in the kvp trie but was not in the proof trie.
        path: PathBuf,
    },
    /// A value in the key-value pair did not match the value declared in the proof trie.
    #[error("incorrect value found at {path}", path = path.display())]
    IncorrectValue {
        /// The path where a value mismatch was found.
        path: PathBuf,
    },
}

/// A root node in a trie formed from a range proof.
///
/// A range proof consists of an optional key proof for the lower bound of the
/// range, an optional key proof for the upper bound of the range, and all key-
/// value pairs within the range. If a bounding proof is not provided, the range
/// is considered unbounded in that direction and the key range includes all
/// keys in that direction.
///
/// A range proof trie is the combination of the three tries formed by the two
/// bounding key proofs and the key-value pairs within the range. During the
/// merge, holes outside of the range are filled in with remote child hashes
/// and any holes within the range that are not filled by the key-value pairs
/// are rejected.
///
/// This trie is iteratively hashed from the leaves to the root, and the
/// resulting hashes are verified at each level and ultimately at the root. If
/// all hashes verify, the range proof is considered valid.
///
/// # Partial vs Complete
///
/// Partial vs Complete refers to whether this node's **children** are well-
/// known or not. A partial node is one where we do not have enough information
/// to be certain that all children edges branching from this node are known. A
/// complete node is one where we do have enough information from the key proof
/// to guarantee that all child edges branching from this node are described and
/// are present, even if those edges lead to remote or partial children.
pub enum RangeProofTrieRoot<'a, P, T: ?Sized> {
    /// A partial node is formed by merging a [`KeyValueTrieRoot`] where we
    /// previously had a `Remote` node in the key proof.
    ///
    /// If a partial node is at the root of the trie, it means that the key
    /// proofs were unbounded in both directions and the trie consists of only
    /// the key-value pairs within the range (which should be all key-value pairs
    /// in the trie).
    Partial(Box<KeyValueTrieRoot<'a, T>>),

    /// A complete node is formed by merging a [`KeyValueTrieRoot`] where we
    /// previously had a `Described` node in the key proof.
    Complete(Box<RangeProofTrieNode<'a, P, T>>),
}

/// A reference to a root node in a trie formed from a range proof.
///
/// This implements [`TrieNode`] where [`RangeProofTrieRoot`] cannot due to
/// it wrapping [`KeyValueTrieRoot`].
pub enum RangeProofTrieRootRef<'a, P, T: ?Sized> {
    /// A reference to a [`RangeProofTrieRoot::Partial`] node.
    Partial(&'a KeyValueTrieRoot<'a, T>),
    /// A reference to a [`RangeProofTrieRoot::Complete`] node.
    Complete(&'a RangeProofTrieNode<'a, P, T>),
}

/// A complete node in a trie formed from a range proof.
///
/// This node is guaranteed to have all children edges branching from it
/// described, even if those edges lead to remote or partial children.
pub struct RangeProofTrieNode<'a, P, T: ?Sized> {
    partial_path: P,
    /// This unfortunately erases the type `T` because we may have gotten the
    /// value (or digest) from the proof trie. This mostly doesn't matter since
    /// we always need to copy the value into storage when merging the final,
    /// verified trie into a proposal.
    value_digest: Option<ValueDigest<&'a [u8]>>,
    children: Children<Option<RangeProofTrieEdge<'a, P, T>>>,
}

enum RangeProofTrieEdge<'a, P, T: ?Sized> {
    Local {
        node: RangeProofTrieRoot<'a, P, T>,
        hash: HashType,
    },
    Remote {
        hash: HashType,
    },
}

impl<'root, P: SplitPath + 'root, T: AsRef<[u8]> + ?Sized + 'root> RangeProofTrieRoot<'root, P, T> {
    /// Creates a new range proof trie root from an optional key proof and set
    /// of key-value pairs.
    ///
    /// The key proof root may be [`None`] if the range is unbounded. Otherwise,
    /// the proof should be joined from the lower and upper bounding proofs.
    ///
    /// Merging the proof with the key-value pairs requires that the two tries
    /// share overlapping structure where they intersect, and that all key-value
    /// pairs fall within the bounds of the key proof, if provided.
    ///
    /// The paths of the proofs and the range of the bounds of the key-value
    /// pairs should be verified before calling this function as it will be
    /// impossible to verify them with the resulting trie. This is due to the
    /// the fact that the value digests in the proof may be complete values,
    /// depending on their size and the hashing scheme. Because of this, the
    /// resulting merged trie will not contain enough information to determine
    /// where the value, if complete, came from and whether it was within the
    /// bounds of the range or part of the prooving structure.
    ///
    /// # Errors
    pub fn new(
        proof: Option<Box<KeyRangeProofTrieRoot<'root, P>>>,
        kvps: Box<KeyValueTrieRoot<'root, T>>,
    ) -> Result<Self, RangeProofError> {
        match proof {
            Some(proof) => RangeProofTrieNode::new(
                PathGuard::new(&mut PathBuf::new_const()),
                *proof.into_root(),
                *kvps,
            )
            .map(Self::Complete),
            None => {
                // No proof means unbounded range, so the result is just the
                // key-value trie as a partial node.
                Ok(Self::Partial(kvps))
            }
        }
    }

    /// Returns a reference to this root node that implements [`TrieNode`].
    #[must_use]
    pub const fn as_ref(&self) -> RangeProofTrieRootRef<'_, P, T> {
        match self {
            Self::Partial(node) => RangeProofTrieRootRef::Partial(node),
            Self::Complete(node) => RangeProofTrieRootRef::Complete(node),
        }
    }
}

impl<'root, P: SplitPath + 'root, T: AsRef<[u8]> + ?Sized> RangeProofTrieNode<'root, P, T> {
    fn new(
        mut leading_path: PathGuard<'_>,
        proof: KeyProofTrieRoot<'root, P>,
        kvps: KeyValueTrieRoot<'root, T>,
    ) -> Result<Box<Self>, RangeProofError> {
        if !proof.partial_path().path_eq(&kvps.partial_path()) {
            return Err(RangeProofError::DisjointEdges {
                leading_path: leading_path.cloned(),
                proof_path: proof.partial_path().as_component_slice().into_owned(),
                kvp_path: kvps.partial_path().as_component_slice().into_owned(),
            });
        }

        leading_path.extend(proof.partial_path().components());

        let value = unwrap_value(&leading_path, proof.value_digest, kvps.value)?;

        let children = proof.children.merge(kvps.children, |pc, proof, kvp| {
            RangeProofTrieEdge::new(leading_path.fork_append(pc), proof, kvp)
        })?;

        Ok(Box::new(Self {
            partial_path: proof.partial_path,
            value_digest: value,
            children,
        }))
    }

    fn from_proof(proof: KeyProofTrieRoot<'root, P>) -> Box<Self> {
        Box::new(Self {
            partial_path: proof.partial_path,
            value_digest: proof.value_digest,
            children: proof.children.map(|_, child| {
                child.map(|proof_edge| match proof_edge {
                    KeyProofTrieNode::Described { node, hash } => RangeProofTrieEdge::Local {
                        node: RangeProofTrieRoot::Complete(RangeProofTrieNode::from_proof(*node)),
                        hash,
                    },
                    KeyProofTrieNode::Remote { hash } => RangeProofTrieEdge::Remote { hash },
                })
            }),
        })
    }
}

impl<'root, P: SplitPath + 'root, T: AsRef<[u8]> + ?Sized + 'root> RangeProofTrieEdge<'root, P, T> {
    fn new(
        leading_path: PathGuard<'_>,
        proof: Option<KeyProofTrieNode<'root, P>>,
        kvp: Option<Box<KeyValueTrieRoot<'root, T>>>,
    ) -> Result<Option<Self>, RangeProofError> {
        match (proof, kvp) {
            (None, None) => Ok(None),
            (None, Some(_)) => Err(RangeProofError::UnexpectedKeyPrefix {
                prefix: leading_path.cloned(),
            }),
            (Some(proof_edge), None) => match proof_edge {
                KeyProofTrieNode::Described { node, hash } => {
                    // the trie continues down a path where only the proof trie
                    // contains information
                    Ok(Some(RangeProofTrieEdge::Local {
                        node: RangeProofTrieRoot::Complete(RangeProofTrieNode::from_proof(*node)),
                        hash,
                    }))
                }
                KeyProofTrieNode::Remote { hash } => Ok(Some(RangeProofTrieEdge::Remote { hash })),
            },
            (Some(proof_edge), Some(kvp_node)) => match proof_edge {
                KeyProofTrieNode::Described { node, hash } => {
                    RangeProofTrieNode::new(leading_path, *node, *kvp_node).map(|node| {
                        Some(RangeProofTrieEdge::Local {
                            node: RangeProofTrieRoot::Complete(node),
                            hash,
                        })
                    })
                }
                KeyProofTrieNode::Remote { hash } => Ok(Some(RangeProofTrieEdge::Local {
                    node: RangeProofTrieRoot::Partial(kvp_node),
                    hash,
                })),
            },
        }
    }

    const fn node(&'root self) -> Option<RangeProofTrieRootRef<'root, P, T>> {
        match self {
            Self::Local { node, .. } => Some(node.as_ref()),
            Self::Remote { .. } => None,
        }
    }

    const fn hash(&'root self) -> &'root HashType {
        match self {
            Self::Local { hash, .. } | Self::Remote { hash } => hash,
        }
    }
}

impl<P: SplitPath, T: AsRef<[u8]> + ?Sized> std::fmt::Debug for RangeProofTrieRoot<'_, P, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Partial(node) => f.debug_tuple("Partial").field(node).finish(),
            Self::Complete(node) => f.debug_tuple("Complete").field(node).finish(),
        }
    }
}

impl<P: SplitPath, T: AsRef<[u8]> + ?Sized> std::fmt::Debug for RangeProofTrieRootRef<'_, P, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Partial(node) => f.debug_tuple("Partial").field(node).finish(),
            Self::Complete(node) => f.debug_tuple("Complete").field(node).finish(),
        }
    }
}

impl<P: SplitPath, T: AsRef<[u8]> + ?Sized> std::fmt::Debug for RangeProofTrieNode<'_, P, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RangeProofTrieNode")
            .field("partial_path", &self.partial_path.display())
            .field("value_digest", &DebugValue::new(self.value_digest.as_ref()))
            .field("children", &DebugChildren::new(&self.children))
            .finish()
    }
}

impl<P: SplitPath, T: AsRef<[u8]> + ?Sized> std::fmt::Debug for RangeProofTrieEdge<'_, P, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local { node, hash } => f
                .debug_struct("Local")
                .field("node", node)
                .field("hash", hash)
                .finish(),
            Self::Remote { hash } => f.debug_struct("Remote").field("hash", hash).finish(),
        }
    }
}

impl<'root, P, T> TrieNode<'root, [u8]> for RangeProofTrieRootRef<'root, P, T>
where
    P: SplitPath + 'root,
    T: AsRef<[u8]> + ?Sized + 'root,
{
    type PartialPath = either::Either<PackedPathRef<'root>, P>;

    fn partial_path(self) -> Self::PartialPath {
        match self {
            Self::Partial(node) => either::Left(node.partial_path()),
            Self::Complete(node) => either::Right(node.partial_path),
        }
    }

    fn value(self) -> Option<&'root [u8]> {
        match self {
            Self::Partial(node) => node.value().map(AsRef::as_ref),
            Self::Complete(node) => node
                .value_digest
                .as_ref()
                // NB: we only yield the value if it is complete and not the digest. Iterators at
                // this point only want the actual value.
                .and_then(ValueDigest::value)
                .copied(),
        }
    }

    fn child_hash(self, pc: crate::PathComponent) -> Option<&'root HashType> {
        match self {
            Self::Partial(node) => node.child_hash(pc),
            Self::Complete(node) => node.children[pc].as_ref().map(RangeProofTrieEdge::hash),
        }
    }

    fn child_node(self, pc: crate::PathComponent) -> Option<Self> {
        match self {
            Self::Partial(node) => node.child_node(pc).map(RangeProofTrieRootRef::Partial),
            Self::Complete(node) => node.children[pc]
                .as_ref()
                .and_then(RangeProofTrieEdge::node),
        }
    }

    fn child_state(self, pc: PathComponent) -> Option<super::TrieEdgeState<'root, Self>> {
        match self {
            Self::Partial(node) => node
                .child_state(pc)
                .map(|state| state.map_node(RangeProofTrieRootRef::Partial)),
            Self::Complete(node) => node.children[pc].as_ref().map(|edge| match edge {
                RangeProofTrieEdge::Local { node, hash } => super::TrieEdgeState::LocalChild {
                    node: node.as_ref(),
                    hash,
                },
                RangeProofTrieEdge::Remote { hash } => super::TrieEdgeState::RemoteChild { hash },
            }),
        }
    }
}

impl<P: Copy, T: ?Sized> Clone for RangeProofTrieRootRef<'_, P, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<P: Copy, T: ?Sized> Copy for RangeProofTrieRootRef<'_, P, T> {}

fn unwrap_value<'a, T: AsRef<[u8]> + ?Sized + 'a>(
    leading_path: &[PathComponent],
    proof_value: Option<ValueDigest<&'a [u8]>>,
    kvp_value: Option<&'a T>,
) -> Result<Option<ValueDigest<&'a [u8]>>, RangeProofError> {
    match (proof_value, kvp_value) {
        (None, None) => Ok(None),
        (Some(vd), None) => {
            // it is okay for the proof to have a value when the kvp trie does not
            // since the proof trie needs to prove beyond just the range of kvps.
            // because of this, we might encounter a proof node with a value
            // without a corresponding kvp value. Depending on the hashing
            // scheme, this might be a complete value or just a digest.
            Ok(Some(vd))
        }
        (None, Some(_)) => Err(RangeProofError::UnexpectedValue {
            path: leading_path.into(),
        }),
        (Some(vd), Some(value)) if vd == ValueDigest::Value(value.as_ref()) => {
            // drop the value digest from the proof since we know we have the full value
            // from the key-values. However, we must still wrap with a ValueDigest for
            // the structure of the trie.
            Ok(Some(ValueDigest::Value(value.as_ref())))
        }
        (Some(_), Some(_)) => Err(RangeProofError::IncorrectValue {
            path: leading_path.into(),
        }),
    }
}
