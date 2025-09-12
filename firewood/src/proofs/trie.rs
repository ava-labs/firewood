// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::convert::Infallible;

use firewood_storage::{BranchNode, Children, HashType, Hashable, ValueDigest, logger::trace};

use crate::{
    proof::{Proof, ProofCollection, ProofError, ProofNode},
    proofs::path::{
        CollectedNibbles, Nibbles, PackedPath, PathNibble, SplitNibbles, SplitPath, WidenedPath,
    },
    range_proof::RangeProof,
    v2::api::{KeyType, ValueType},
};

type EitherProof<'a> = either::Either<RangeProofTrieRoot<'a>, KeyValueTrieRoot<'a>>;

#[derive(Debug)]
pub(crate) struct HashedRangeProof<'a> {
    either: either::Either<HashedRangeProofTrieRoot<'a>, HashedKeyValueTrieRoot<'a>>,
}

#[derive(Debug)]
pub(crate) struct HashedRangeProofRef<'a, 'b> {
    either: either::Either<&'b HashedRangeProofTrieRoot<'a>, &'b HashedKeyValueTrieRoot<'a>>,
}

#[derive(Debug)]
struct HashedRangeProofTrieRoot<'a> {
    computed: HashType,
    leading_path: CollectedNibbles,
    partial_path: WidenedPath<'a>,
    value_digest: Option<ValueDigest<&'a [u8]>>,
    children: Children<Box<HashedRangeProofTrieEdge<'a>>>,
}

#[derive(Debug)]
struct HashedKeyValueTrieRoot<'a> {
    computed: HashType,
    leading_path: CollectedNibbles,
    partial_path: PackedPath<'a>,
    value: Option<&'a [u8]>,
    children: Children<Box<HashedKeyValueTrieRoot<'a>>>,
}

#[derive(Debug)]
enum HashedRangeProofTrieEdge<'a> {
    Distant(HashType),
    Partial(HashType, HashedKeyValueTrieRoot<'a>),
    Complete(HashType, HashedRangeProofTrieRoot<'a>),
}

#[derive(Debug)]
struct RangeProofTrieRoot<'a> {
    partial_path: WidenedPath<'a>,
    value_digest: Option<ValueDigest<&'a [u8]>>,
    children: Children<Box<RangeProofTrieEdge<'a>>>,
}

#[derive(Debug)]
enum RangeProofTrieEdge<'a> {
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

/// A collected of key-value pairs organized into a trie.
#[derive(Debug, PartialEq, Eq)]
struct KeyValueTrieRoot<'a> {
    partial_path: PackedPath<'a>,
    value: Option<&'a [u8]>,
    children: Children<Box<KeyValueTrieRoot<'a>>>,
}

/// A root node in a trie formed from a [`ProofNode`].
#[derive(Debug)]
struct KeyProofTrieRoot<'a> {
    partial_path: WidenedPath<'a>,
    value_digest: Option<ValueDigest<&'a [u8]>>,
    children: Children<Box<KeyProofTrieEdge<'a>>>,
}

/// A node in a trie formed a collection of [`ProofNode`]s.
#[derive(Debug)]
enum KeyProofTrieEdge<'a> {
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

impl<'a> HashedRangeProof<'a> {
    pub const fn computed(&self) -> &HashType {
        match &self.either {
            either::Left(proof) => &proof.computed,
            either::Right(kvp) => &kvp.computed,
        }
    }

    #[expect(unused)]
    pub(crate) const fn leading_path(&self) -> &CollectedNibbles {
        match &self.either {
            either::Left(proof) => &proof.leading_path,
            either::Right(kvp) => &kvp.leading_path,
        }
    }

    #[expect(unused)]
    pub(crate) const fn partial_path(&self) -> either::Either<&WidenedPath<'a>, &PackedPath<'a>> {
        match &self.either {
            either::Left(proof) => either::Left(&proof.partial_path),
            either::Right(kvp) => either::Right(&kvp.partial_path),
        }
    }

    #[expect(unused)]
    pub(crate) fn value_digest(&self) -> Option<ValueDigest<&'a [u8]>> {
        match &self.either {
            either::Left(proof) => proof.value_digest.clone(),
            either::Right(kvp) => kvp.value.map(ValueDigest::Value),
        }
    }

    #[expect(unused)]
    pub(crate) fn children(&self) -> Children<HashedRangeProofRef<'a, '_>> {
        match &self.either {
            either::Left(proof) => proof.children.each_ref().map(HashedRangeProofRef::left),
            either::Right(kvp) => kvp.children.each_ref().map(HashedRangeProofRef::right),
        }
    }

    pub fn new<K, V, P>(proof: &'a RangeProof<K, V, P>) -> Result<Self, ProofError>
    where
        K: KeyType,
        V: ValueType,
        P: ProofCollection<Node = ProofNode>,
    {
        let mut path_buf = Vec::new();
        let mut p = PathGuard::new(&mut path_buf);
        Ok(Self {
            either: match RangeProofTrieRoot::new(p.fork(), proof)? {
                either::Left(node) => either::Left(HashedRangeProofTrieRoot::new(p, node)),
                either::Right(node) => either::Right(HashedKeyValueTrieRoot::new(p, node)),
            },
        })
    }
}

impl<'a, 'b> HashedRangeProofRef<'a, 'b> {
    #[expect(clippy::ref_option)]
    fn left(proof: &'b Option<Box<HashedRangeProofTrieEdge<'a>>>) -> Option<Self> {
        proof.as_ref().and_then(|p| match **p {
            HashedRangeProofTrieEdge::Distant(_) => None,
            HashedRangeProofTrieEdge::Partial(_, ref root) => Some(Self {
                either: either::Right(root),
            }),
            HashedRangeProofTrieEdge::Complete(_, ref root) => Some(Self {
                either: either::Left(root),
            }),
        })
    }

    #[expect(clippy::ref_option)]
    fn right(kvp: &'b Option<Box<HashedKeyValueTrieRoot<'a>>>) -> Option<Self> {
        kvp.as_ref().map(|k| Self {
            either: either::Right(k),
        })
    }

    #[expect(unused)]
    pub const fn computed(&self) -> &HashType {
        match &self.either {
            either::Left(proof) => &proof.computed,
            either::Right(kvp) => &kvp.computed,
        }
    }

    #[expect(unused)]
    pub(crate) const fn leading_path(&self) -> &CollectedNibbles {
        match &self.either {
            either::Left(proof) => &proof.leading_path,
            either::Right(kvp) => &kvp.leading_path,
        }
    }

    #[expect(unused)]
    pub(crate) const fn partial_path(&self) -> either::Either<&WidenedPath<'a>, &PackedPath<'a>> {
        match &self.either {
            either::Left(proof) => either::Left(&proof.partial_path),
            either::Right(kvp) => either::Right(&kvp.partial_path),
        }
    }

    #[expect(unused)]
    pub(crate) fn value_digest(&self) -> Option<ValueDigest<&'a [u8]>> {
        match &self.either {
            either::Left(proof) => proof.value_digest.clone(),
            either::Right(kvp) => kvp.value.map(ValueDigest::Value),
        }
    }

    #[expect(unused)]
    pub(crate) fn children(&self) -> Children<HashedRangeProofRef<'a, '_>> {
        match &self.either {
            either::Left(proof) => proof.children.each_ref().map(HashedRangeProofRef::left),
            either::Right(kvp) => kvp.children.each_ref().map(HashedRangeProofRef::right),
        }
    }
}

impl<'a> HashedRangeProofTrieRoot<'a> {
    fn new(mut leading_path: PathGuard<'_>, node: RangeProofTrieRoot<'a>) -> Self {
        let children = {
            let mut nibble = NibbleCounter::new();
            let mut leading_path = leading_path.fork_push(node.partial_path);
            node.children.map(|maybe| {
                let leading_path = leading_path.fork_push(nibble.next());
                maybe.map(|child| HashedRangeProofTrieEdge::new(leading_path, *child))
            })
        };

        Self {
            computed: HashableShunt::new(
                &leading_path,
                node.partial_path,
                node.value_digest.as_ref().map(ValueDigest::as_ref),
                children
                    .each_ref()
                    .map(|maybe| maybe.as_ref().map(HashedRangeProofTrieEdge::hash).cloned()),
            )
            .to_hash(),
            leading_path: leading_path.collect(),
            partial_path: node.partial_path,
            value_digest: node.value_digest,
            children: boxed_children(children),
        }
    }
}

impl<'a> HashedKeyValueTrieRoot<'a> {
    fn new(mut leading_path: PathGuard<'_>, node: KeyValueTrieRoot<'a>) -> Self {
        let children = {
            let mut nibble = NibbleCounter::new();
            let mut leading_path = leading_path.fork_push(node.partial_path);
            node.children.map(|maybe| {
                let leading_path = leading_path.fork_push(nibble.next());
                maybe.map(|child| Self::new(leading_path, *child))
            })
        };

        Self {
            computed: HashableShunt::new(
                &leading_path,
                node.partial_path,
                node.value.map(ValueDigest::Value),
                children
                    .each_ref()
                    .map(|maybe| maybe.as_ref().map(|child| child.computed.clone())),
            )
            .to_hash(),
            leading_path: leading_path.collect(),
            partial_path: node.partial_path,
            value: node.value,
            children: boxed_children(children),
        }
    }
}

impl<'a> HashedRangeProofTrieEdge<'a> {
    fn new(leading_path: PathGuard<'_>, edge: RangeProofTrieEdge<'a>) -> Self {
        match edge {
            RangeProofTrieEdge::Distant(hash) => Self::Distant(hash),
            RangeProofTrieEdge::Partial(hash, node) => {
                Self::Partial(hash, HashedKeyValueTrieRoot::new(leading_path, node))
            }
            RangeProofTrieEdge::Complete(hash, node) => {
                Self::Complete(hash, HashedRangeProofTrieRoot::new(leading_path, node))
            }
        }
    }

    const fn discovered_hash(&self) -> &HashType {
        match self {
            Self::Distant(id) | Self::Partial(id, _) | Self::Complete(id, _) => id,
        }
    }

    const fn computed_hash(&self) -> Option<&HashType> {
        match self {
            Self::Distant(_) => None,
            Self::Partial(_, kvp) => Some(&kvp.computed),
            Self::Complete(_, proof) => Some(&proof.computed),
        }
    }

    const fn hash(&self) -> &HashType {
        match self.computed_hash() {
            Some(hash) => hash,
            None => self.discovered_hash(),
        }
    }
}

impl<'a> RangeProofTrieRoot<'a> {
    const fn empty() -> Self {
        Self {
            partial_path: WidenedPath::new(&[]),
            value_digest: None,
            children: BranchNode::empty_children(),
        }
    }

    fn new<K, V, P>(
        mut leading_path: PathGuard<'_>,
        proof: &'a RangeProof<K, V, P>,
    ) -> Result<EitherProof<'a>, ProofError>
    where
        K: KeyType,
        V: ValueType,
        P: ProofCollection<Node = ProofNode>,
    {
        let kvp = KeyValueTrieRoot::new(leading_path.fork(), proof.key_values())?;

        let lhs = KeyProofTrieRoot::new(proof.start_proof())?;
        let rhs = KeyProofTrieRoot::new(proof.end_proof())?;
        let proof = KeyProofTrieRoot::merge_opt(leading_path.fork(), lhs, rhs)?;

        match (proof, kvp) {
            (None, None) => Ok(either::Left(Self::empty())),
            // no values, return the root as is
            (Some(proof), None) => Ok(either::Left(Self::from_proof_root(proof))),
            // no proof, so we have an unbounded range of key-values
            (None, Some(kvp)) => Ok(either::Right(kvp)),
            (Some(proof), Some(kvp)) => Self::join(leading_path, proof, kvp).map(either::Left),
        }
    }

    /// Recursively joins a proof trie with a key-value trie.
    ///
    /// The key-value trie must be a subset of the proof trie and must not introduce
    /// any new children to discovered [`KeyProofTrieRoot`] nodes. However, key-
    /// value nodes may introduce any number of nodes that fill in a
    /// [`KeyProofTrieEdge::Remote`] node.
    fn join(
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
        let children = merge_array(children, kvp_child, |maybe_proof, maybe_kvp| {
            RangeProofTrieEdge::new(
                leading_path.fork_push(nibble.next()),
                maybe_proof.map(|v| *v),
                maybe_kvp,
            )
        })?;

        Ok(Self {
            partial_path,
            value_digest,
            children: boxed_children(children),
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
        let children = merge_array(proof_children, kvp_children, |maybe_proof, maybe_kvp| {
            RangeProofTrieEdge::new(
                leading_path.fork_push(nibble.next()),
                maybe_proof.map(|v| *v),
                maybe_kvp.map(|v| *v),
            )
        })?;

        Ok(Self {
            partial_path,
            value_digest: value.map(ValueDigest::Value),
            children: boxed_children(children),
        })
    }

    fn from_proof_root(proof: KeyProofTrieRoot<'a>) -> Self {
        Self {
            partial_path: proof.partial_path,
            value_digest: proof.value_digest,
            children: boxed_children(
                merge_array(
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

impl<'a> KeyValueTrieRoot<'a> {
    const fn leaf(key: &'a [u8], value: &'a [u8]) -> Self {
        Self {
            partial_path: PackedPath::new(key),
            value: Some(value),
            children: BranchNode::empty_children(),
        }
    }

    fn new<K, V>(
        mut leading_path: PathGuard<'_>,
        pairs: &'a [(K, V)],
    ) -> Result<Option<Self>, ProofError>
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        match pairs {
            [] => Ok(None),
            [(k, v)] => Ok(Some(Self::leaf(k.as_ref(), v.as_ref()))),
            many => {
                let mid = many.len() / 2;
                let (lhs, rhs) = many.split_at(mid);
                let lhs = Self::new(leading_path.fork(), lhs)?;
                let rhs = Self::new(leading_path.fork(), rhs)?;
                Self::merge_root(leading_path, lhs, rhs)
            }
        }
    }

    fn merge_root(
        leading_path: PathGuard<'_>,
        lhs: Option<Self>,
        rhs: Option<Self>,
    ) -> Result<Option<Self>, ProofError> {
        match (lhs, rhs) {
            (None, None) => Ok(None),
            (Some(root), None) | (None, Some(root)) => Ok(Some(root)),
            (Some(lhs), Some(rhs)) => Self::merge(leading_path, lhs, rhs).map(Some),
        }
    }

    fn merge(leading_path: PathGuard<'_>, lhs: Self, rhs: Self) -> Result<Self, ProofError> {
        let split = SplitPath::new(lhs.partial_path, rhs.partial_path);
        match (
            split.lhs_suffix.split_first(),
            split.rhs_suffix.split_first(),
        ) {
            ((Some(lhs_nibble), lhs_path), (Some(rhs_nibble), rhs_path)) => {
                trace!(
                    "KeyValueTrieRoot: merging two diverging keys; leading_path: {}, {split}",
                    leading_path.display(),
                );
                // both keys diverge at some point after the common prefix, we
                // need a new parent node with no value and both nodes as children
                Ok(Self::from_siblings(
                    split.common_prefix,
                    lhs_nibble,
                    Self {
                        partial_path: lhs_path,
                        ..lhs
                    },
                    rhs_nibble,
                    Self {
                        partial_path: rhs_path,
                        ..rhs
                    },
                ))
            }
            ((None, _), (Some(nibble), partial_path)) => {
                trace!(
                    "KeyValueTrieRoot: merging where lhs is a prefix of rhs; leading_path: {}, {split}",
                    leading_path.display(),
                );
                // lhs is a strict prefix of rhs, so lhs becomes the parent of rhs
                lhs.merge_child(
                    leading_path,
                    nibble,
                    Self {
                        partial_path,
                        ..rhs
                    },
                )
            }
            ((Some(nibble), partial_path), (None, _)) => {
                trace!(
                    "KeyValueTrieRoot: merging where rhs is a prefix of lhs; leading_path: {}, {split}",
                    leading_path.display(),
                );
                // rhs is a strict prefix of lhs, so rhs becomes the parent of lhs
                rhs.merge_child(
                    leading_path,
                    nibble,
                    Self {
                        partial_path,
                        ..lhs
                    },
                )
            }
            ((None, _), (None, _)) => {
                trace!(
                    "KeyValueTrieRoot: merging where both keys are identical; leading_path: {}, {split}",
                    leading_path.display(),
                );
                // both keys are identical, this is invalid if they both have
                // values, otherwise we can merge their children
                Self::deep_merge(leading_path, lhs, rhs)
            }
        }
    }

    /// Deeply merges two nodes.
    ///
    /// Used when the keys are identical. Errors if both nodes have values
    /// and they are not equal.
    fn deep_merge(
        mut leading_path: PathGuard<'_>,
        lhs: Self,
        rhs: Self,
    ) -> Result<Self, ProofError> {
        leading_path.extend(lhs.partial_path.nibbles_iter());

        let value = match (lhs.value, rhs.value) {
            (Some(lhs), Some(rhs)) if lhs == rhs => Some(lhs),
            (Some(value1), Some(value2)) => {
                return Err(ProofError::DuplicateKeysInProof {
                    key: leading_path.bytes_iter().collect(),
                    value1: hex::encode(value1),
                    value2: hex::encode(value2),
                });
            }
            (Some(v), None) | (None, Some(v)) => Some(v),
            (None, None) => None,
        };

        let mut nibble = NibbleCounter::new();
        let children = merge_array(lhs.children, rhs.children, |lhs, rhs| {
            Self::merge_root(
                leading_path.fork_push(nibble.next()),
                lhs.map(|v| *v),
                rhs.map(|v| *v),
            )
        })?;

        Ok(Self {
            partial_path: lhs.partial_path,
            value,
            children: boxed_children(children),
        })
    }

    fn from_siblings(
        partial_path: PackedPath<'a>,
        lhs_nibble: PathNibble,
        lhs: Self,
        rhs_nibble: PathNibble,
        rhs: Self,
    ) -> Self {
        #![expect(clippy::indexing_slicing)]
        debug_assert_ne!(lhs_nibble, rhs_nibble);
        let mut children = BranchNode::empty_children();
        children[lhs_nibble.0 as usize] = Some(Box::new(lhs));
        children[rhs_nibble.0 as usize] = Some(Box::new(rhs));

        Self {
            partial_path,
            value: None,
            children,
        }
    }

    fn merge_child(
        self,
        mut leading_path: PathGuard<'_>,
        nibble: PathNibble,
        child: Self,
    ) -> Result<Self, ProofError> {
        #![expect(clippy::indexing_slicing)]

        leading_path.extend(self.partial_path.nibbles_iter());
        leading_path.push(nibble.0);

        let mut children = self.children;
        children[nibble.0 as usize] = Some(Box::new(match children[nibble.0 as usize].take() {
            Some(existing) => Self::merge(leading_path, *existing, child)?,
            None => child,
        }));

        Ok(Self { children, ..self })
    }
}

impl<'a> KeyProofTrieRoot<'a> {
    /// Constructs a trie root from a slice of proof nodes.
    ///
    /// Each node in the slice must be a strict prefix of the following node. And,
    /// each child node must be referenced by its parent.
    fn new<P>(nodes: &'a Proof<P>) -> Result<Option<Self>, ProofError>
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
            children: boxed_children(
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

        let children = merge_array(parent.child_hashes.clone(), with_self, |hash, this| match (
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
            children: boxed_children(children),
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
    fn merge_opt(
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
            return Err(ProofError::DuplicateKeysInProof {
                key: leading_path.bytes_iter().collect(),
                value1: format!("{:?}", lhs.value_digest),
                value2: format!("{:?}", rhs.value_digest),
            });
        }

        let mut nibble = NibbleCounter::new();
        let children = merge_array(lhs.children, rhs.children, |lhs, rhs| {
            KeyProofTrieEdge::new(
                leading_path.fork_push(nibble.next()),
                lhs.map(|v| *v),
                rhs.map(|v| *v),
            )
        })?;

        Ok(Self {
            children: boxed_children(children),
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
                    Err(ProofError::UnexpectedHash {
                        key: leading_path.bytes_iter().collect(),
                        context: "merging two remote edges with different hashes",
                        expected: lhs,
                        actual: rhs,
                    })
                }
            }
            (Some(Self::Remote(hash)), Some(Self::Described { 0: id, 1: root }))
            | (Some(Self::Described { 0: id, 1: root }), Some(Self::Remote(hash))) => {
                if hash == id {
                    Ok(Some(Self::Described(id, root)))
                } else {
                    Err(ProofError::UnexpectedHash {
                        key: leading_path.join(root.partial_path).bytes_iter().collect(),
                        context: "merging a remote edge with a described edge with different hashes",
                        expected: hash,
                        actual: id,
                    })
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
                    Err(ProofError::UnexpectedHash {
                        key: leading_path
                            .join(l_root.partial_path)
                            .bytes_iter()
                            .collect(),
                        context: "merging two described edges with different hashes",
                        expected: l_id,
                        actual: r_id,
                    })
                }
            }
        }
    }
}

struct HashableShunt<'a, P1, P2> {
    parent_nibbles: P1,
    partial_path: P2,
    value: Option<ValueDigest<&'a [u8]>>,
    child_hashes: Children<HashType>,
}

impl<'a, P1: Nibbles, P2: Nibbles> HashableShunt<'a, P1, P2> {
    const fn new(
        parent_nibbles: P1,
        partial_path: P2,
        value: Option<ValueDigest<&'a [u8]>>,
        child_hashes: Children<HashType>,
    ) -> Self {
        Self {
            parent_nibbles,
            partial_path,
            value,
            child_hashes,
        }
    }

    fn to_hash(&self) -> HashType {
        firewood_storage::Preimage::to_hash(self)
    }
}

impl<P1: Nibbles, P2: Nibbles> std::fmt::Debug for HashableShunt<'_, P1, P2> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shunt")
            .field("parent_nibbles", &self.parent_nibbles.display())
            .field("partial_path", &self.partial_path.display())
            .field(
                "value",
                &self.value.as_ref().map(|v| v.as_ref().map(hex::encode)),
            )
            .field("child_hashes", &self.child_hashes)
            .finish()
    }
}

impl<P1: Nibbles, P2: Nibbles> Hashable for HashableShunt<'_, P1, P2> {
    fn parent_prefix_path(&self) -> impl Iterator<Item = u8> + Clone {
        self.parent_nibbles.nibbles_iter()
    }

    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone {
        self.partial_path.nibbles_iter()
    }

    fn value_digest(&self) -> Option<ValueDigest<&[u8]>> {
        self.value.clone()
    }

    fn children(&self) -> Children<HashType> {
        self.child_hashes.clone()
    }
}

struct PathGuard<'a> {
    nibbles: &'a mut Vec<u8>,
    reset_len: usize,
}

impl Drop for PathGuard<'_> {
    fn drop(&mut self) {
        self.nibbles.truncate(self.reset_len);
    }
}

impl<'a> PathGuard<'a> {
    const fn new(nibbles: &'a mut Vec<u8>) -> Self {
        let reset_len = nibbles.len();
        Self { nibbles, reset_len }
    }

    const fn fork(&mut self) -> PathGuard<'_> {
        PathGuard::new(self.nibbles)
    }

    fn fork_push(&mut self, nibbles: impl Nibbles) -> PathGuard<'_> {
        let mut this = self.fork();
        this.extend(nibbles.nibbles_iter());
        this
    }
}

impl std::ops::Deref for PathGuard<'_> {
    type Target = Vec<u8>;

    fn deref(&self) -> &Self::Target {
        self.nibbles
    }
}

impl std::ops::DerefMut for PathGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.nibbles
    }
}

impl Nibbles for PathGuard<'_> {
    fn nibbles_iter(&self) -> impl Iterator<Item = u8> + Clone + '_ {
        self.nibbles.iter().copied()
    }

    fn len(&self) -> usize {
        self.nibbles.len()
    }

    fn is_empty(&self) -> bool {
        self.nibbles.is_empty()
    }
}

struct NibbleCounter(u8);

impl NibbleCounter {
    const fn new() -> Self {
        Self(0)
    }

    fn next(&mut self) -> PathNibble {
        let this = self.0;
        self.0 = self
            .0
            .wrapping_add(1)
            .clamp(0, const { (BranchNode::MAX_CHILDREN - 1) as u8 });
        PathNibble(this)
    }
}

fn merge_array<T, U, V, E, const N: usize>(
    lhs: [T; N],
    rhs: [U; N],
    mut merge: impl FnMut(T, U) -> Result<Option<V>, E>,
) -> Result<[Option<V>; N], E> {
    let mut output = [const { None::<V> }; N];
    for (slot, (l, r)) in output.iter_mut().zip(lhs.into_iter().zip(rhs)) {
        *slot = merge(l, r)?;
    }
    Ok(output)
}

fn boxed_children<T, const N: usize>(children: [Option<T>; N]) -> [Option<Box<T>>; N] {
    children.map(|maybe| maybe.map(Box::new))
}
