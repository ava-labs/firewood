// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::{Children, HashType, ValueDigest};

use crate::{
    proof::ProofError,
    proofs::{
        path::{CollectedNibbles, Nibbles, PackedPath, PathGuard, WidenedPath},
        trie::{
            counter::NibbleCounter,
            iter::Child,
            keyvalues::KeyValueTrieRoot,
            merged::{EitherProof, RangeProofTrieEdge, RangeProofTrieRoot},
            shunt::HashableShunt,
        },
    },
};

/// A hashed trie over a range proof.
///
/// A range proof is constructed over zero, one, or two proof nodes and a collection
/// of many key-value pairs. This structure represents the hashed version of that
/// trie and can be used to verify the integrity of the range proof.
///
/// If there were zero proof nodes, the trie consists solely of the key-value pairs. If
/// the root hash verifies, then the trie is complete and valid. However, if the hash
/// does not verify then the trie is incomplete and invalid and it is not possible to
/// determine which nodes are missing or wrong.
///
/// If there was one or two proof nodes, they were first merged into a single proof trie
/// that describes as many branches of the trie as possible; but, only along the paths
/// to the lower and/or upper bound keys. Distant branches of the trie are only known
/// by their hashes. This merged trie is then combined with the key-value pairs to form
/// the complete range proof trie. If this trie's root hash verifies, then the proof
/// is complete and valid. However, the trie may still be incomplete as some branches
/// may be distant and only their parents have been verified.
#[derive(Debug)]
pub(crate) struct HashedRangeProof<'a> {
    either: either::Either<HashedRangeProofTrieRoot<'a>, HashedKeyValueTrieRoot<'a>>,
}

/// A reference to a trie node in a [`HashedRangeProof`].
#[derive(Debug, Clone, Copy)]
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

impl<'a> HashedRangeProof<'a> {
    pub(super) fn new(
        leading_path: PathGuard<'_>,
        root: EitherProof<'a>,
    ) -> Result<Self, ProofError> {
        Ok(Self {
            either: match root {
                either::Left(node) => {
                    either::Left(HashedRangeProofTrieRoot::new(leading_path, node))
                }
                either::Right(node) => {
                    either::Right(HashedKeyValueTrieRoot::new(leading_path, node))
                }
            },
        })
    }

    pub const fn as_ref(&self) -> HashedRangeProofRef<'a, '_> {
        HashedRangeProofRef {
            either: match self.either {
                either::Left(ref proof) => either::Left(proof),
                either::Right(ref kvp) => either::Right(kvp),
            },
        }
    }

    pub const fn computed(&self) -> &HashType {
        self.as_ref().computed()
    }

    #[expect(unused)]
    pub const fn leading_path(&self) -> &CollectedNibbles {
        self.as_ref().leading_path()
    }

    #[expect(unused)]
    pub const fn partial_path(&self) -> either::Either<&WidenedPath<'a>, &PackedPath<'a>> {
        self.as_ref().partial_path()
    }

    #[expect(unused)]
    pub fn value_digest(&self) -> Option<ValueDigest<&'a [u8]>> {
        self.as_ref().value_digest()
    }

    #[expect(unused)]
    pub fn children(&self) -> Children<HashedRangeProofRef<'a, '_>> {
        self.as_ref().children()
    }
}

impl<'a, 'b> HashedRangeProofRef<'a, 'b> {
    fn left(proof: Option<&'b HashedRangeProofTrieEdge<'a>>) -> Option<Self> {
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

    fn right(kvp: Option<&'b HashedKeyValueTrieRoot<'a>>) -> Option<Self> {
        kvp.as_ref().map(|k| Self {
            either: either::Right(k),
        })
    }

    pub const fn computed(self) -> &'b HashType {
        match self.either {
            either::Left(proof) => &proof.computed,
            either::Right(kvp) => &kvp.computed,
        }
    }

    pub const fn leading_path(self) -> &'b CollectedNibbles {
        match self.either {
            either::Left(proof) => &proof.leading_path,
            either::Right(kvp) => &kvp.leading_path,
        }
    }

    pub const fn partial_path(self) -> either::Either<&'b WidenedPath<'a>, &'b PackedPath<'a>> {
        match self.either {
            either::Left(proof) => either::Left(&proof.partial_path),
            either::Right(kvp) => either::Right(&kvp.partial_path),
        }
    }

    pub fn value_digest(self) -> Option<ValueDigest<&'a [u8]>> {
        match self.either {
            either::Left(proof) => proof.value_digest.clone(),
            either::Right(kvp) => kvp.value.map(ValueDigest::Value),
        }
    }

    pub fn children(self) -> Children<Self> {
        match &self.either {
            either::Left(proof) => proof
                .children
                .each_ref()
                .map(Option::as_deref)
                .map(Self::left),
            either::Right(kvp) => kvp
                .children
                .each_ref()
                .map(Option::as_deref)
                .map(Self::right),
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

impl<'a> super::TrieNode<'a> for HashedRangeProofRef<'a, 'a> {
    type Nibbles = either::Either<WidenedPath<'a>, PackedPath<'a>>;

    fn partial_path(self) -> Self::Nibbles {
        match self.either {
            either::Left(proof) => either::Left(proof.partial_path),
            either::Right(kvp) => either::Right(kvp.partial_path),
        }
    }

    fn value_digest(self) -> Option<ValueDigest<&'a [u8]>> {
        match self.either {
            either::Left(proof) => proof.value_digest.clone(),
            either::Right(kvp) => kvp.value.map(ValueDigest::Value),
        }
    }

    fn computed_hash(self) -> Option<HashType> {
        match self.either {
            either::Left(proof) => Some(proof.computed.clone()),
            either::Right(kvp) => Some(kvp.computed.clone()),
        }
    }

    fn children(self) -> Children<Child<Self>> {
        match self.either {
            either::Left(proof) => proof.children.each_ref().map(|maybe| {
                maybe.as_deref().map(|child| match child {
                    HashedRangeProofTrieEdge::Distant(hash) => Child::Remote(hash.clone()),
                    HashedRangeProofTrieEdge::Partial(hash, root) => Child::Hashed(
                        hash.clone(),
                        Self {
                            either: either::Right(root),
                        },
                    ),
                    HashedRangeProofTrieEdge::Complete(hash, root) => Child::Hashed(
                        hash.clone(),
                        Self {
                            either: either::Left(root),
                        },
                    ),
                })
            }),
            either::Right(kvp) => kvp.children.each_ref().map(|maybe| {
                maybe.as_deref().map(|child| {
                    Child::Hashed(
                        child.computed.clone(),
                        Self {
                            either: either::Right(child),
                        },
                    )
                })
            }),
        }
    }
}
