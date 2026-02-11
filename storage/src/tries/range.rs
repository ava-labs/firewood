// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::path::PathRef;
use crate::tries::TrieEdgeState;
use crate::{Children, HashType, PathBuf, PathComponent, PathGuard, TriePath, ValueDigest};

use super::debug::{DebugChildren, DebugValue};
use super::{
    KeyProofTrieRoot, KeyRangeProofTrieRoot, KeyValueTrieRoot, TrieNode, proof::KeyProofTrieNode,
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
#[derive_where::derive_where(Debug; T: AsRef<[u8]>)]
pub enum RangeProofTrieRoot<'a, T: ?Sized> {
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
    Complete(Box<RangeProofTrieNode<'a, T>>),
}

/// A complete node in a trie formed from a range proof.
///
/// This node is guaranteed to have all children edges branching from it
/// described, even if those edges lead to remote or partial children.
pub struct RangeProofTrieNode<'a, T: ?Sized> {
    /// The path from this node's parent to this node.
    pub partial_path: PathRef<'a>,
    /// This unfortunately erases the type `T` because we may have gotten the
    /// value (or digest) from the proof trie. This mostly doesn't matter since
    /// we always need to copy the value into storage when merging the final,
    /// verified trie into a proposal.
    pub value_digest: Option<ValueDigest<&'a [u8]>>,
    /// The children edges branching from this node. All edges are guaranteed to
    /// be described by the proof trie, but they may lead to remote or partial
    /// children.
    pub children: Children<Option<RangeProofTrieEdge<'a, T>>>,
}

impl<T: AsRef<[u8]> + ?Sized> std::fmt::Debug for RangeProofTrieNode<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RangeProofTrieNode")
            .field("partial_path", &self.partial_path.display())
            .field("value_digest", &DebugValue::new(self.value_digest.as_ref()))
            .field("children", &DebugChildren::new(&self.children))
            .finish()
    }
}

/// An edge from parent to child in a range proof trie.
#[derive_where::derive_where(Debug; T: AsRef<[u8]>)]
pub enum RangeProofTrieEdge<'a, T: ?Sized> {
    /// Local edges are those where we have the child node available locally, but
    /// we cannot be sure that all of the child's edges are fully described by the proof.
    Local {
        /// The node for this edge.
        node: RangeProofTrieRoot<'a, T>,
        /// The hash of this child as known to the parent; not verified.
        hash: HashType,
    },
    /// Remote edges are those where we do not have the child node available locally
    /// and must rely on the proof for the hash of this child and all of its descendants.
    Remote {
        /// The hash of this child as know to the parent.
        hash: HashType,
    },
}

impl<'a, T: AsRef<[u8]> + ?Sized + 'a> RangeProofTrieRoot<'a, T> {
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
        proof: Option<Box<KeyRangeProofTrieRoot<'a>>>,
        kvps: Box<KeyValueTrieRoot<'a, T>>,
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
}

impl<'a, T: AsRef<[u8]> + ?Sized> RangeProofTrieNode<'a, T> {
    fn new(
        mut leading_path: PathGuard<'_>,
        proof: KeyProofTrieRoot<'a>,
        kvps: KeyValueTrieRoot<'a, T>,
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

    fn from_proof(proof: KeyProofTrieRoot<'a>) -> Box<Self> {
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

impl<'a, T: AsRef<[u8]> + ?Sized + 'a> RangeProofTrieEdge<'a, T> {
    fn new(
        leading_path: PathGuard<'_>,
        proof: Option<KeyProofTrieNode<'a>>,
        kvp: Option<Box<KeyValueTrieRoot<'a, T>>>,
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

    const fn node(&self) -> Option<&dyn TrieNode> {
        match self {
            Self::Local { node, .. } => Some(node),
            Self::Remote { .. } => None,
        }
    }

    const fn hash(&'a self) -> &'a HashType {
        match self {
            Self::Local { hash, .. } | Self::Remote { hash } => hash,
        }
    }

    const fn as_edge_state(&self) -> TrieEdgeState<'_> {
        match self {
            Self::Local { node, hash } => TrieEdgeState::LocalChild { node, hash },
            Self::Remote { hash } => TrieEdgeState::RemoteChild { hash },
        }
    }
}

impl<'a, T: AsRef<[u8]> + ?Sized + 'a> TrieNode for RangeProofTrieRoot<'a, T> {
    fn partial_path(&self) -> PathRef<'_> {
        match self {
            Self::Partial(node) => PathRef::Packed(node.partial_path),
            Self::Complete(node) => node.partial_path,
        }
    }

    fn value_digest(&self) -> Option<ValueDigest<&[u8]>> {
        match self {
            Self::Partial(node) => node.value_digest(),
            Self::Complete(node) => node.value_digest.clone(),
        }
    }

    fn child_hash(&self, pc: PathComponent) -> Option<&HashType> {
        match self {
            Self::Partial(node) => node.child_hash(pc),
            Self::Complete(node) => node.children[pc].as_ref().map(RangeProofTrieEdge::hash),
        }
    }

    fn child_node(&self, pc: PathComponent) -> Option<&dyn TrieNode> {
        match self {
            Self::Partial(node) => node.child_node(pc),
            Self::Complete(node) => node.children[pc]
                .as_ref()
                .and_then(RangeProofTrieEdge::node),
        }
    }

    fn child_state(&self, pc: PathComponent) -> Option<TrieEdgeState<'_>> {
        match self {
            Self::Partial(node) => node.child_state(pc),
            Self::Complete(node) => node.children[pc]
                .as_ref()
                .map(RangeProofTrieEdge::as_edge_state),
        }
    }
}

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
