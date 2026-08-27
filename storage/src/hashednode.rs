// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{
    Children, DefaultHashMode, HashMode, HashType, HashableShunt, IntoSplitPath, Node, Path,
    PathComponent, SplitPath, TrieHash, UnhashedChildError,
};
use smallvec::SmallVec;

/// A [`Node`] that is ready to be hashed: every child of the node is hashed,
/// so the node's hash preimage can be built without silently dropping a child.
#[derive(Debug)]
pub struct HashedNode<'a> {
    node: &'a Node,
    child_hashes: Children<Option<HashType>>,
}

impl<'a> TryFrom<&'a Node> for HashedNode<'a> {
    type Error = UnhashedChildError;

    fn try_from(node: &'a Node) -> Result<Self, Self::Error> {
        let child_hashes = match node {
            Node::Branch(branch) => branch.children_hashes()?,
            Node::Leaf(_) => Children::new(),
        };
        Ok(Self { node, child_hashes })
    }
}

impl<'a> HashedNode<'a> {
    /// Returns the underlying node.
    #[must_use]
    pub const fn node(&self) -> &'a Node {
        self.node
    }

    /// Returns the hashes of the node's children.
    #[must_use]
    pub const fn child_hashes(&self) -> &Children<Option<HashType>> {
        &self.child_hashes
    }

    pub(crate) fn into_parts(self) -> (&'a Node, Children<Option<HashType>>) {
        (self.node, self.child_hashes)
    }
}

impl<'a, P: SplitPath> HashableShunt<'a, P, &'a [PathComponent]> {
    /// Creates a new [`HashableShunt`] from the given `node` at the given `prefix`.
    pub fn from_node(prefix: P, node: HashedNode<'a>) -> Self {
        let (node, child_hashes) = node.into_parts();
        match node {
            Node::Branch(node) => Self::new(
                prefix,
                node.partial_path.as_components(),
                node.value.as_deref().map(ValueDigest::Value),
                child_hashes,
            ),
            Node::Leaf(node) => Self::new(
                prefix,
                node.partial_path.as_components(),
                Some(ValueDigest::Value(&node.value)),
                Children::new(),
            ),
        }
    }
}

/// Returns the hash of `node`, which is at the given `path_prefix`, under the
/// hashing scheme `H`.
#[must_use]
pub fn hash_node<H: HashMode>(node: HashedNode<'_>, path_prefix: &Path) -> HashType {
    H::to_hash(&HashableShunt::from_node(path_prefix.as_components(), node))
}

/// Returns the serialized representation of `node` used as the pre-image
/// when hashing the node under the scheme `H`. The node is at the given
/// `path_prefix`.
#[must_use]
pub fn hash_preimage<H: HashMode>(node: HashedNode<'_>, path_prefix: &Path) -> Box<[u8]> {
    // Key, 3 options, value digest
    #[expect(clippy::arithmetic_side_effects)]
    let est_len =
        node.node().partial_path().len() + path_prefix.len() + 3 + HashType::empty().len();
    let mut buf = Vec::with_capacity(est_len);
    H::write_preimage(
        &HashableShunt::from_node(path_prefix.as_components(), node),
        &mut buf,
    );
    buf.into_boxed_slice()
}

pub trait HasUpdate {
    fn update<T: AsRef<[u8]>>(&mut self, data: T);
}

impl HasUpdate for Vec<u8> {
    fn update<T: AsRef<[u8]>>(&mut self, data: T) {
        self.extend_from_slice(data.as_ref());
    }
}

impl<A: smallvec::Array<Item = u8>> HasUpdate for SmallVec<A> {
    fn update<T: AsRef<[u8]>>(&mut self, data: T) {
        self.extend_from_slice(data.as_ref());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BranchNode, Child, LeafNode, LinearAddress, MaybePersistedNode};

    #[test]
    fn leaf_is_ready_to_hash() {
        let leaf = Node::Leaf(LeafNode {
            partial_path: Path::new(),
            value: Box::from([1, 2, 3]),
        });

        assert!(HashedNode::try_from(&leaf).is_ok());
    }

    #[test]
    fn hashed_children_are_collected() {
        let address_hash = HashType::from([1; 32]);
        let maybe_persisted_hash = HashType::from([2; 32]);
        let address = LinearAddress::new(16).expect("address is aligned");
        let maybe_persisted =
            MaybePersistedNode::from(LinearAddress::new(32).expect("address is aligned"));
        let mut children = Children::new();
        children[PathComponent::ALL[1]] =
            Some(Child::AddressWithHash(address, address_hash.clone()));
        children[PathComponent::ALL[2]] = Some(Child::MaybePersisted(
            maybe_persisted,
            maybe_persisted_hash.clone(),
        ));
        let branch = Node::Branch(Box::new(BranchNode {
            partial_path: Path::new(),
            value: None,
            children,
        }));

        let hashed = HashedNode::try_from(&branch).expect("all children are hashed");

        assert_eq!(
            hashed.child_hashes()[PathComponent::ALL[1]],
            Some(address_hash)
        );
        assert_eq!(
            hashed.child_hashes()[PathComponent::ALL[2]],
            Some(maybe_persisted_hash)
        );
    }

    #[test]
    fn unhashed_child_is_rejected() {
        let mut children = Children::new();
        children[PathComponent::ALL[7]] = Some(Child::Node(Node::Leaf(LeafNode {
            partial_path: Path::new(),
            value: Box::from([1]),
        })));
        let branch = Node::Branch(Box::new(BranchNode {
            partial_path: Path::new(),
            value: None,
            children,
        }));

        let Err(error) = HashedNode::try_from(&branch) else {
            panic!("unhashed child should be rejected");
        };
        assert_eq!(
            error,
            UnhashedChildError {
                index: PathComponent::ALL[7]
            }
        );
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// A `ValueDigest` is either a node's value or the hash of its value.
pub enum ValueDigest<T> {
    /// The node's value.
    Value(T),
    /// For MerkleDB hashing, the digest is the hash of the value if it is 32
    /// bytes or longer. (Unused by the Ethereum scheme, which never stores a
    /// value as a hash, but the variant exists unconditionally so both schemes
    /// share one data type.)
    Hash(HashType),
}

impl<T: AsRef<[u8]>> ValueDigest<T> {
    /// Verifies that the value or hash matches the expected value.
    pub fn verify(&self, expected: impl AsRef<[u8]>) -> bool {
        match self {
            Self::Value(got_value) => {
                // This proof proves that `key` maps to `got_value`.
                got_value.as_ref() == expected.as_ref()
            }
            Self::Hash(got_hash) => {
                use sha2::{Digest, Sha256};
                // This proof proves that `key` maps to a value
                // whose hash is `got_hash`. `HashType` implements
                // `PartialEq<TrieHash>`, so compare without re-wrapping.
                *got_hash == TrieHash::from(Sha256::digest(expected.as_ref()))
            }
        }
    }

    /// Returns a `ValueDigest` that borrows from this one.
    pub fn as_ref(&self) -> ValueDigest<&[u8]> {
        match self {
            Self::Value(v) => ValueDigest::Value(v.as_ref()),
            Self::Hash(h) => ValueDigest::Hash(h.clone()),
        }
    }

    /// Returns the inner bytes if this digest carries a value, or `None` if
    /// it carries only a hash. The Ethereum scheme never produces a `Hash`
    /// digest, so under that scheme this function always returns `Some`.
    pub fn value(&self) -> Option<&[u8]> {
        match self {
            Self::Value(v) => Some(v.as_ref()),
            Self::Hash(_) => None,
        }
    }

    /// Convert the value to a hash if it is not already a hash, under the
    /// scheme `H`.
    ///
    /// Under the MerkleDB scheme, a value of 32 bytes or more is replaced by
    /// its SHA-256 hash; shorter values pass through unchanged. The Ethereum
    /// scheme never hashes values, so this is the identity there.
    ///
    /// The capping is the MerkleDB scheme's behavior, selected by the scheme
    /// `H` rather than the compile-time default.
    pub fn make_hash<H: HashMode>(&self) -> ValueDigest<&[u8]> {
        match self.as_ref() {
            ValueDigest::Value(v) if v.len() >= 32 && !H::ALGORITHM.is_ethereum() => {
                use sha2::{Digest, Sha256};
                ValueDigest::Hash(HashType::from(TrieHash::from(Sha256::digest(v))))
            }

            ValueDigest::Value(v) => ValueDigest::Value(v),

            ValueDigest::Hash(v) => ValueDigest::Hash(v),
        }
    }

    /// Maps the value inside this `ValueDigest` to another value.
    pub fn map<O>(self, f: impl FnOnce(T) -> O) -> ValueDigest<O> {
        match self {
            Self::Value(v) => ValueDigest::Value(f(v)),
            Self::Hash(h) => ValueDigest::Hash(h),
        }
    }
}

impl<T: AsRef<[u8]>> AsRef<[u8]> for ValueDigest<T> {
    fn as_ref(&self) -> &[u8] {
        match self {
            Self::Value(v) => v.as_ref(),
            Self::Hash(h) => h.as_ref(),
        }
    }
}

/// A node in the trie that can be hashed.
pub trait Hashable: std::fmt::Debug {
    /// The type of the leading path.
    type LeadingPath<'a>: IntoSplitPath + 'a
    where
        Self: 'a;

    /// The type of the partial path.
    type PartialPath<'a>: IntoSplitPath + 'a
    where
        Self: 'a;

    /// The type of the full path.
    type FullPath<'a>: IntoSplitPath + 'a
    where
        Self: 'a;

    /// The full path of this node's parent where each byte is a nibble.
    fn parent_prefix_path(&self) -> Self::LeadingPath<'_>;
    /// The partial path of this node where each byte is a nibble.
    fn partial_path(&self) -> Self::PartialPath<'_>;
    /// The full path of this node including the parent's prefix where each byte is a nibble.
    fn full_path(&self) -> Self::FullPath<'_>;
    /// The node's value or hash.
    fn value_digest(&self) -> Option<ValueDigest<&[u8]>>;
    /// Each element is a child's index and hash.
    /// Yields 0 elements if the node is a leaf.
    fn children(&self) -> Children<Option<HashType>>;
}

/// A preimage of a hash.
pub trait Preimage: std::fmt::Debug {
    /// Returns the hash of this preimage.
    fn to_hash(&self) -> HashType;

    /// Write this hash preimage to `buf`.
    fn write(&self, buf: &mut impl HasUpdate);
}

/// A single blanket implementation that delegates to the compile-selected
/// [`DefaultHashMode`].
///
/// Threading a generic `H: HashMode` through these call sites (so a caller can
/// pick the scheme at runtime) is deferred to a follow-up PR.
impl<T: Hashable> Preimage for T {
    fn to_hash(&self) -> HashType {
        DefaultHashMode::to_hash(self)
    }

    fn write(&self, buf: &mut impl HasUpdate) {
        DefaultHashMode::write_preimage(self, buf);
    }
}
