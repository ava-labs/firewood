// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::{Children, HashType, ValueDigest};

use crate::{
    proof::{ProofCollection, ProofError, ProofNode},
    proofs::{
        path::{CollectedNibbles, Nibbles, PackedPath, PathGuard, WidenedPath},
        trie::{
            counter::NibbleCounter,
            keyvalues::KeyValueTrieRoot,
            merged::{RangeProofTrieEdge, RangeProofTrieRoot},
            shunt::HashableShunt,
        },
    },
    range_proof::RangeProof,
    v2::api::{KeyType, ValueType},
};

pub(super) type EitherProof<'a> = either::Either<RangeProofTrieRoot<'a>, KeyValueTrieRoot<'a>>;

#[derive(Debug)]
pub(crate) struct HashedRangeProof<'a> {
    either: either::Either<HashedRangeProofTrieRoot<'a>, HashedKeyValueTrieRoot<'a>>,
}

#[derive(Debug)]
pub(crate) struct HashedRangeProofRef<'a, 'b> {
    either: either::Either<&'b HashedRangeProofTrieRoot<'a>, &'b HashedKeyValueTrieRoot<'a>>,
}

#[derive(Debug)]
pub(super) struct HashedRangeProofTrieRoot<'a> {
    computed: HashType,
    leading_path: CollectedNibbles,
    partial_path: WidenedPath<'a>,
    value_digest: Option<ValueDigest<&'a [u8]>>,
    children: Children<Box<HashedRangeProofTrieEdge<'a>>>,
}

#[derive(Debug)]
pub(super) struct HashedKeyValueTrieRoot<'a> {
    computed: HashType,
    leading_path: CollectedNibbles,
    partial_path: PackedPath<'a>,
    value: Option<&'a [u8]>,
    children: Children<Box<HashedKeyValueTrieRoot<'a>>>,
}

#[derive(Debug)]
pub(super) enum HashedRangeProofTrieEdge<'a> {
    Distant(HashType),
    Partial(HashType, HashedKeyValueTrieRoot<'a>),
    Complete(HashType, HashedRangeProofTrieRoot<'a>),
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
            children: super::boxed_children(children),
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
            children: super::boxed_children(children),
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
