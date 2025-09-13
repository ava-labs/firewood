// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::convert::Infallible;

use firewood_storage::{BranchNode, Children, HashType, ValueDigest};

use crate::{
    proof::ProofError,
    proofs::{
        path::{Nibbles, PackedPath, PathGuard, PathNibble, SplitNibbles, SplitPath, WidenedPath},
        trie::{
            counter::NibbleCounter,
            keyvalues::KeyValueTrieRoot,
            proof::{KeyProofTrieEdge, KeyProofTrieRoot},
        },
    },
};

pub(super) type EitherProof<'a> = either::Either<RangeProofTrieRoot<'a>, KeyValueTrieRoot<'a>>;

#[derive(Debug)]
pub(super) struct RangeProofTrieRoot<'a> {
    pub(super) partial_path: WidenedPath<'a>,
    pub(super) value_digest: Option<ValueDigest<&'a [u8]>>,
    pub(super) children: Children<Box<RangeProofTrieEdge<'a>>>,
}

#[derive(Debug)]
pub(super) enum RangeProofTrieEdge<'a> {
    /// Distant edge nodes are the nodes we discovered from the proof trie but
    /// do not have any additional information. We only know that it exists with
    /// the hash which we need to calculate and verify its parent node's hash.
    Distant(HashType),
    /// A partial node is like a described node and also like a distant edge. It
    /// was born out of merging a kvp trie into the proof node where we had a
    /// [`KeyProofTrieEdge::Remote`]. We know the hash of the root, but the root
    /// may or may not be complete. We will not know until we compute the hash.
    Partial(HashType, KeyValueTrieRoot<'a>),
    /// Complete nodes are nodes that were [`KeyProofTrieEdge::Described`] in
    /// the proof trie. This means we know how many and which children are present
    /// in the trie on this node as well as all of the hashes for those children.
    Complete(HashType, RangeProofTrieRoot<'a>),
}

impl<'a> RangeProofTrieRoot<'a> {
    pub(super) const fn empty() -> Self {
        Self {
            partial_path: WidenedPath::new(&[]),
            value_digest: None,
            children: BranchNode::empty_children(),
        }
    }

    /// Recursively joins a proof trie with a key-value trie.
    ///
    /// The key-value trie must be a subset of the proof trie and must not introduce
    /// any new children to discovered [`KeyProofTrieRoot`] nodes. However, key-
    /// value nodes may introduce any number of nodes that fill in a
    /// [`KeyProofTrieEdge::Remote`] node.
    pub(super) fn join(
        leading_path: PathGuard<'_>,
        proof: KeyProofTrieRoot<'a>,
        kvp: KeyValueTrieRoot<'a>,
    ) -> Result<Self, ProofError> {
        let split = SplitPath::new(proof.partial_path, kvp.partial_path);

        match (
            split.lhs_suffix.split_first(),
            split.rhs_suffix.split_first(),
        ) {
            // The proof path diverges from the kvp path. This is not allowed
            // because it would introduce a new node where the proof trie
            // indicates there is none.
            ((Some(_), _), _) => Err(ProofError::NodeNotInTrie {
                parent: (&leading_path)
                    .join(proof.partial_path)
                    .bytes_iter()
                    .collect(),
                child: (&leading_path)
                    .join(kvp.partial_path)
                    .bytes_iter()
                    .collect(),
            }),
            // The kvp path diverges from the proof path. We can merge the kvp
            // with the child of the proof node at the next nibble; but only
            // if the proof describes a child at that nibble.
            ((None, _), (Some(child_nibble), child_partial_path)) => Self::from_parent_child(
                leading_path,
                split.common_prefix,
                proof.value_digest,
                proof.children,
                child_nibble,
                KeyValueTrieRoot {
                    partial_path: child_partial_path,
                    ..kvp
                },
            ),
            // Both keys are identical, we can merge the nodes directly
            // but only if the value digest matches the value on the kvp node
            ((None, _), (None, _)) => Self::from_deep_merge(
                leading_path,
                split.common_prefix,
                proof.value_digest,
                kvp.value,
                proof.children,
                kvp.children,
            ),
        }
    }

    fn from_parent_child(
        mut leading_path: PathGuard<'_>,
        partial_path: WidenedPath<'a>,
        value_digest: Option<ValueDigest<&'a [u8]>>,
        children: Children<Box<KeyProofTrieEdge<'a>>>,
        child_nibble: PathNibble,
        child: KeyValueTrieRoot<'a>,
    ) -> Result<Self, ProofError> {
        #![expect(clippy::indexing_slicing)]

        let mut kvp_child = BranchNode::empty_children();
        kvp_child[child_nibble.0 as usize].replace(child);

        let mut nibble = NibbleCounter::new();
        leading_path.extend(partial_path.nibbles_iter());
        let children = super::merge_array(children, kvp_child, |maybe_proof, maybe_kvp| {
            RangeProofTrieEdge::new(
                leading_path.fork_push(nibble.next()),
                maybe_proof.map(|v| *v),
                maybe_kvp,
            )
        })?;

        Ok(Self {
            partial_path,
            value_digest,
            children: super::boxed_children(children),
        })
    }

    fn from_deep_merge(
        mut leading_path: PathGuard<'_>,
        partial_path: WidenedPath<'a>,
        value_digest: Option<ValueDigest<&'a [u8]>>,
        value: Option<&'a [u8]>,
        proof_children: Children<Box<KeyProofTrieEdge<'a>>>,
        kvp_children: Children<Box<KeyValueTrieRoot<'a>>>,
    ) -> Result<Self, ProofError> {
        crate::proof::verify_opt_value_digest(value, value_digest)?;

        let mut nibble = NibbleCounter::new();
        leading_path.extend(partial_path.nibbles_iter());
        let children =
            super::merge_array(proof_children, kvp_children, |maybe_proof, maybe_kvp| {
                RangeProofTrieEdge::new(
                    leading_path.fork_push(nibble.next()),
                    maybe_proof.map(|v| *v),
                    maybe_kvp.map(|v| *v),
                )
            })?;

        Ok(Self {
            partial_path,
            value_digest: value.map(ValueDigest::Value),
            children: super::boxed_children(children),
        })
    }

    pub(super) fn from_proof_root(proof: KeyProofTrieRoot<'a>) -> Self {
        Self {
            partial_path: proof.partial_path,
            value_digest: proof.value_digest,
            children: super::boxed_children(
                super::merge_array(
                    proof.children,
                    BranchNode::empty_children::<Infallible>(),
                    |child, None| match child {
                        None => Ok(None),
                        Some(child) => match *child {
                            KeyProofTrieEdge::Remote(hash) => {
                                Ok(Some(RangeProofTrieEdge::Distant(hash)))
                            }
                            KeyProofTrieEdge::Described(id, root) => Ok(Some(
                                RangeProofTrieEdge::Complete(id, Self::from_proof_root(root)),
                            )),
                        },
                    },
                )
                .unwrap_or_else(|inf: Infallible| match inf {}),
            ),
        }
    }
}

impl<'a> RangeProofTrieEdge<'a> {
    fn new(
        leading_path: PathGuard<'_>,
        proof: Option<KeyProofTrieEdge<'a>>,
        kvp: Option<KeyValueTrieRoot<'a>>,
    ) -> Result<Option<Self>, ProofError> {
        match (proof, kvp) {
            (None, None) => Ok(None),
            // The proof does not describe a child at this nibble,
            // but the kvp trie does. This is not allowed because
            // it would introduce a new node where the proof trie
            // indicates there is none.
            (None, Some(kvp)) => Err(ProofError::NodeNotInTrie {
                parent: leading_path.bytes_iter().collect(),
                child: leading_path.join(kvp.partial_path).bytes_iter().collect(),
            }),
            // The proof describes a distant edge node at this nibble,
            // and the kvp trie doesn't have a child here. We can
            // carry over the distant edge node.
            (Some(KeyProofTrieEdge::Remote(id)), None) => Ok(Some(Self::Distant(id))),
            // The proof describes a recursively verifiable node at this
            // nibble, but the kvp trie doesn't have a child here. We
            // can recursively create a `RangeProofRoot` from the
            // proof root without needing to merge in any kvp nodes.
            (Some(KeyProofTrieEdge::Described(id, root)), None) => Ok(Some(Self::Complete(
                id,
                RangeProofTrieRoot::from_proof_root(root),
            ))),
            // The proof describes a distant edge node at this nibble,
            // and the kvp trie has a child here. We can store the
            // KeyValueTrieRoot on the partial node and will detect
            // if it is incomplete later when we compute the hash.
            (Some(KeyProofTrieEdge::Remote(id)), Some(kvp)) => Ok(Some(Self::Partial(id, kvp))),
            // The proof describes a node and its children for this
            // nibble. We need to recursively join the two nodes.
            (Some(KeyProofTrieEdge::Described(id, proof)), Some(kvp)) => Ok(Some(Self::Complete(
                id,
                RangeProofTrieRoot::join(leading_path, proof, kvp)?,
            ))),
        }
    }
}

impl<'a> super::TrieNode<'a>
    for either::Either<&'a RangeProofTrieRoot<'a>, &'a KeyValueTrieRoot<'a>>
{
    type Nibbles = either::Either<WidenedPath<'a>, PackedPath<'a>>;

    fn partial_path(self) -> Self::Nibbles {
        match self {
            either::Left(root) => either::Left(root.partial_path),
            either::Right(root) => either::Right(root.partial_path),
        }
    }

    fn value_digest(self) -> Option<ValueDigest<&'a [u8]>> {
        match self {
            either::Left(root) => root.value_digest.clone(),
            either::Right(root) => root.value_digest(),
        }
    }

    fn computed_hash(self) -> Option<HashType> {
        None
    }

    fn children(self) -> Children<super::Child<Self>> {
        match self {
            either::Left(root) => root.children.each_ref().map(|maybe| {
                maybe.as_deref().map(|child| match child {
                    RangeProofTrieEdge::Distant(hash) => super::Child::Remote(hash.clone()),
                    RangeProofTrieEdge::Partial(hash, node) => {
                        super::Child::Hashed(hash.clone(), either::Right(node))
                    }
                    RangeProofTrieEdge::Complete(hash, node) => {
                        super::Child::Hashed(hash.clone(), either::Left(node))
                    }
                })
            }),
            either::Right(root) => root
                .children()
                .map(|maybe| maybe.map(|child| child.map(either::Right))),
        }
    }
}
