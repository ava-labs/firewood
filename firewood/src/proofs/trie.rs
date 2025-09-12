// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::{BranchNode, Children, HashType, Hashable, ValueDigest};

use crate::{
    proof::{Proof, ProofCollection, ProofError, ProofNode},
    proofs::path::{Nibbles, PackedPath, RangeProofPath, SplitKey, WidenedPath},
    range_proof::RangeProof,
    v2::api::{KeyType, ValueType},
};

#[derive(Debug)]
struct HashedKeyValueTrieRoot<'a> {
    computed: HashType,
    #[expect(unused)]
    key: PackedPath<'a>,
    #[expect(unused)]
    value: Option<&'a [u8]>,
    #[expect(unused)]
    children: Children<Box<HashedKeyValueTrieRoot<'a>>>,
}

#[derive(Debug)]
pub(crate) struct HashedRangeProofTrieRoot<'a> {
    pub(crate) computed: HashType,
    #[expect(unused)]
    key: RangeProofPath<'a>,
    #[expect(unused)]
    value_digest: Option<ValueDigest<&'a [u8]>>,
    #[expect(unused)]
    children: Children<Box<HashedRangeProofTrieEdge<'a>>>,
}

/// The edge of a trie formed by hashing a range proof.
///
/// The hash stored on the enum variants is the hash discovered from the proof
/// trie. The hash stored within the root is the hash we computed.
#[derive(Debug)]
enum HashedRangeProofTrieEdge<'a> {
    Distant(HashType),
    Partial(HashType, HashedKeyValueTrieRoot<'a>),
    Complete(HashType, HashedRangeProofTrieRoot<'a>),
}

/// A root node within a trie formed by a range proof.
#[derive(Debug)]
struct RangeProofTrieRoot<'a> {
    key: RangeProofPath<'a>,
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
    key: PackedPath<'a>,
    value: Option<&'a [u8]>,
    children: Children<Box<KeyValueTrieRoot<'a>>>,
}

/// A root node in a trie formed from a [`ProofNode`].
struct KeyProofTrieRoot<'a> {
    /// Unlike `KeyValueTrie`, each byte in the slice is a nibble regardless of
    /// the branching factor and those that are out of bounds will error when used.
    key: WidenedPath<'a>,
    value_digest: Option<ValueDigest<&'a [u8]>>,
    children: Children<Box<KeyProofTrieEdge<'a>>>,
}

/// A node in a trie formed a collection of [`ProofNode`]s.
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

impl<'a> HashedRangeProofTrieRoot<'a> {
    pub(crate) fn from_range_proof(
        proof: &'a RangeProof<impl KeyType, impl ValueType, impl ProofCollection<Node = ProofNode>>,
    ) -> Result<Self, ProofError> {
        let root = RangeProofTrieRoot::from_range_proof(proof)?;
        Ok(Self::new(PathGuard::new(&mut Vec::new()), root))
    }

    fn new(mut parent_path: PathGuard<'_>, node: RangeProofTrieRoot<'a>) -> Self {
        let mut nibble = 0_u8;
        let children = node.children.map(|maybe| {
            let this_nibble = nibble;
            nibble = nibble.wrapping_add(1);

            maybe.map(|child| {
                HashedRangeProofTrieEdge::new(parent_path.fork_push(this_nibble), *child)
            })
        });

        Self {
            computed: Shunt::new(
                &parent_path,
                node.key,
                node.value_digest.as_ref().map(ValueDigest::as_ref),
                children
                    .each_ref()
                    .map(|maybe| maybe.as_ref().map(HashedRangeProofTrieEdge::hash).cloned()),
            )
            .to_hash(),
            key: node.key,
            value_digest: node.value_digest,
            children: boxed_children(children),
        }
    }
}

impl<'a> HashedRangeProofTrieEdge<'a> {
    fn new(parent_path: PathGuard<'_>, edge: RangeProofTrieEdge<'a>) -> Self {
        match edge {
            RangeProofTrieEdge::Distant(hash) => Self::Distant(hash),
            RangeProofTrieEdge::Partial(hash, root) => {
                Self::Partial(hash, HashedKeyValueTrieRoot::new(parent_path, root))
            }
            RangeProofTrieEdge::Complete(hash, root) => {
                Self::Complete(hash, HashedRangeProofTrieRoot::new(parent_path, root))
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

impl<'a> HashedKeyValueTrieRoot<'a> {
    fn new(mut parent_path: PathGuard<'_>, node: KeyValueTrieRoot<'a>) -> Self {
        let mut nibble = 0_u8;
        let children = node.children.map(|maybe| {
            let this_nibble = nibble;
            nibble = nibble.wrapping_add(1);

            maybe.map(|child| Self::new(parent_path.fork_push(this_nibble), *child))
        });

        let child_hashes = children
            .each_ref()
            .map(|maybe| maybe.as_ref().map(|child| child.computed.clone()));

        HashedKeyValueTrieRoot {
            computed: Shunt::new(
                &parent_path,
                node.key,
                node.value.map(ValueDigest::Value),
                child_hashes,
            )
            .to_hash(),
            key: node.key,
            value: node.value,
            children: boxed_children(children),
        }
    }
}

impl<'a> RangeProofTrieRoot<'a> {
    fn from_range_proof(
        proof: &'a RangeProof<impl KeyType, impl ValueType, impl ProofCollection<Node = ProofNode>>,
    ) -> Result<Self, ProofError> {
        let start_proof = KeyProofTrieRoot::from_proof(proof.start_proof())?;
        let end_proof = KeyProofTrieRoot::from_proof(proof.end_proof())?;

        let key_proof = KeyProofTrieRoot::merge_proof_roots(start_proof, end_proof)?
            .unwrap_or_else(KeyProofTrieRoot::empty);

        let kvp = KeyValueTrieRoot::from_pairs(proof.key_values())?
            .unwrap_or_else(KeyValueTrieRoot::empty);

        Self::join(key_proof, kvp)
    }

    /// Recursively joins a proof trie with a key-value trie.
    ///
    /// The key-value trie must be a subset of the proof trie and must not introduce
    /// any new children to discovered [`KeyProofTrieRoot`] nodes. However, key-
    /// value nodes may introduce any number of nodes that fill in a
    /// [`KeyProofTrieEdge::Remote`] node.
    fn join(proof: KeyProofTrieRoot<'a>, kvp: KeyValueTrieRoot<'a>) -> Result<Self, ProofError> {
        // provide kvp first so SplitKey uses the PackedKey for the common prefix
        let split = SplitKey::new(kvp.key, proof.key);

        match (
            split.rhs_suffix.split_first(),
            split.lhs_suffix.split_first(),
        ) {
            // The proof path diverges from the kvp path. This is not allowed
            // because it would introduce a new node where the proof trie
            // indicates there is none.
            ((Some(child_nibble), _), _) => Err(ProofError::NodeNotInTrie {
                parent: proof.key.with_prefix().to_packed_key(),
                child_nibble,
                child: kvp.key.with_prefix().to_packed_key(),
            }),
            // The kvp path diverges from the proof path. We can merge the kvp
            // with the child of the proof node at the next nibble; but only
            // if the proof describes a child at that nibble.
            ((None, _), (Some(kvp_nibble), kvp_key)) => {
                Self::from_parent_child(split.common_prefix, proof, kvp, kvp_nibble, kvp_key)
            }

            // Both keys are identical, we can merge the nodes directly
            // but only if the value digest matches the value on the kvp node
            ((None, _), (None, _)) => Self::from_deep_merge(split.common_prefix, proof, kvp),
        }
    }

    fn from_parent_child(
        key: PackedPath<'a>,
        proof: KeyProofTrieRoot<'a>,
        kvp: KeyValueTrieRoot<'a>,
        kvp_nibble: u8,
        kvp_key: PackedPath<'a>,
    ) -> Result<Self, ProofError> {
        let mut kvp_children = BranchNode::empty_children();
        #[expect(
            clippy::indexing_slicing,
            reason = "we trust the values from PackedPath"
        )]
        {
            kvp_children[kvp_nibble as usize] = Some(KeyValueTrieRoot {
                key: kvp_key,
                value: kvp.value,
                children: kvp.children,
            });
        }

        let mut nibble = 0_u8;
        let children = merge_array(proof.children, kvp_children, |maybe_proof, maybe_kvp| {
            let this_nibble = nibble;
            nibble = nibble.wrapping_add(1);
            RangeProofTrieEdge::merge_trie_edge(
                proof.key,
                this_nibble,
                maybe_proof.map(|v| *v),
                maybe_kvp,
            )
        })?;

        Ok(Self {
            key: key.into(),
            value_digest: proof.value_digest,
            children: boxed_children(children),
        })
    }

    fn from_deep_merge(
        key: PackedPath<'a>,
        proof: KeyProofTrieRoot<'a>,
        kvp: KeyValueTrieRoot<'a>,
    ) -> Result<Self, ProofError> {
        crate::proof::verify_opt_value_digest(kvp.value, proof.value_digest)?;

        let mut nibble = 0_u8;
        let children = merge_array(proof.children, kvp.children, |maybe_proof, maybe_kvp| {
            let this_nibble = nibble;
            nibble = nibble.wrapping_add(1);
            RangeProofTrieEdge::merge_trie_edge(
                proof.key,
                this_nibble,
                maybe_proof.map(|v| *v),
                maybe_kvp.map(|v| *v),
            )
        })?;

        Ok(Self {
            key: key.into(),
            value_digest: kvp.value.map(ValueDigest::Value),
            children: boxed_children(children),
        })
    }

    fn from_proof_root(proof: KeyProofTrieRoot<'a>) -> Result<Self, ProofError> {
        Ok(Self {
            key: proof.key.into(),
            value_digest: proof.value_digest,
            children: boxed_children(merge_array(
                proof.children,
                BranchNode::empty_children::<std::convert::Infallible>(),
                |child, None| match child {
                    None => Ok(None),
                    Some(child) => match *child {
                        KeyProofTrieEdge::Remote(hash) => {
                            Ok(Some(RangeProofTrieEdge::Distant(hash)))
                        }
                        KeyProofTrieEdge::Described(id, root) => {
                            match Self::from_proof_root(root) {
                                Ok(root) => Ok(Some(RangeProofTrieEdge::Complete(id, root))),
                                Err(e) => Err(e),
                            }
                        }
                    },
                },
            )?),
        })
    }
}

impl<'a> RangeProofTrieEdge<'a> {
    fn merge_trie_edge(
        parent_key: WidenedPath<'a>,
        this_nibble: u8,
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
                parent: parent_key.with_prefix().to_packed_key(),
                child_nibble: this_nibble,
                child: kvp.key.with_prefix().to_packed_key(),
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
                RangeProofTrieRoot::from_proof_root(root)?,
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
                RangeProofTrieRoot::join(proof, kvp)?,
            ))),
        }
    }
}

impl<'a> KeyValueTrieRoot<'a> {
    const fn empty() -> Self {
        Self {
            key: PackedPath::new(&[]),
            value: None,
            children: BranchNode::empty_children(),
        }
    }

    const fn new(key: &'a [u8], value: &'a [u8]) -> Self {
        Self {
            key: PackedPath::new(key),
            value: Some(value),
            children: BranchNode::empty_children(),
        }
    }

    fn from_pairs<K, V>(pairs: &'a [(K, V)]) -> Result<Option<Self>, ProofError>
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        match pairs {
            [] => Ok(None),
            [(k, v)] => Ok(Some(Self::new(k.as_ref(), v.as_ref()))),
            many => {
                let mid = many.len() / 2;
                let (lhs, rhs) = many.split_at(mid);
                let lhs = Self::from_pairs(lhs)?;
                let rhs = Self::from_pairs(rhs)?;
                Merge::merge(lhs, rhs)
            }
        }
    }

    fn merge(lhs: Self, rhs: Self) -> Result<Self, ProofError> {
        let split = SplitKey::new(lhs.key, rhs.key);
        match (
            split.lhs_suffix.split_first(),
            split.rhs_suffix.split_first(),
        ) {
            ((Some(lhs_nibble), lhs_key), (Some(rhs_nibble), rhs_key)) => Ok(
                // both keys diverge at some point after the common prefix, we
                // need a new parent node with no value and both nodes as children
                Self::from_siblings(
                    lhs,
                    rhs,
                    split.common_prefix,
                    lhs_nibble,
                    lhs_key,
                    rhs_nibble,
                    rhs_key,
                ),
            ),
            ((None, _), (Some(rhs_nibble), rhs_key)) => {
                // lhs is a strict prefix of rhs, so lhs becomes the parent of rhs
                Self::from_parent_child(lhs, rhs, rhs_nibble, rhs_key)
            }
            ((Some(lhs_nibble), lhs_key), (None, _)) => {
                // rhs is a strict prefix of lhs, so rhs becomes the parent of lhs
                Self::from_parent_child(rhs, lhs, lhs_nibble, lhs_key)
            }
            ((None, _), (None, _)) => {
                // both keys are identical, this is invalid if they both have
                // values, otherwise we can merge their children
                Self::deep_merge(lhs, rhs)
            }
        }
    }

    /// Deeply merges two nodes.
    ///
    /// Used when the keys are identical. Errors if both nodes have values
    /// and they are not equal.
    fn deep_merge(lhs: Self, rhs: Self) -> Result<Self, ProofError> {
        let value = match (lhs.value, rhs.value) {
            (Some(lhs), Some(rhs)) if lhs == rhs => Some(lhs),
            (Some(value1), Some(value2)) => {
                return Err(ProofError::DuplicateKeysInProof {
                    key: lhs.key.with_prefix().to_packed_key(),
                    value1: value1.into(),
                    value2: value2.into(),
                });
            }
            (Some(v), None) | (None, Some(v)) => Some(v),
            (None, None) => None,
        };

        let children = merge_array(lhs.children, rhs.children, Merge::merge)?;

        Ok(Self {
            key: lhs.key,
            value,
            children,
        })
    }

    fn from_siblings(
        lhs: Self,
        rhs: Self,
        common_prefix: PackedPath<'a>,
        lhs_nibble: u8,
        lhs_key: PackedPath<'a>,
        rhs_nibble: u8,
        rhs_key: PackedPath<'a>,
    ) -> Self {
        #![expect(
            clippy::indexing_slicing,
            reason = "we trust the values from PackedPath"
        )]

        debug_assert_ne!(lhs_nibble, rhs_nibble);

        let mut children = BranchNode::empty_children();

        children[lhs_nibble as usize] = Some(Self {
            key: lhs_key,
            value: lhs.value,
            children: lhs.children,
        });

        children[rhs_nibble as usize] = Some(Self {
            key: rhs_key,
            value: rhs.value,
            children: rhs.children,
        });

        Self {
            key: common_prefix,
            value: None,
            children: boxed_children(children),
        }
    }

    fn from_parent_child(
        parent: Self,
        child: Self,
        child_nibble: u8,
        child_key: PackedPath<'a>,
    ) -> Result<Self, ProofError> {
        #![expect(
            clippy::indexing_slicing,
            reason = "we trust the values from PackedPath"
        )]

        let child = Self {
            key: child_key,
            value: child.value,
            children: child.children,
        };

        let mut children = parent.children;
        children[child_nibble as usize] =
            Some(Box::new(match children[child_nibble as usize].take() {
                Some(existing) => Self::merge(*existing, child)?,
                None => child,
            }));

        Ok(Self {
            key: parent.key,
            value: parent.value,
            children,
        })
    }
}

impl<'a> KeyProofTrieRoot<'a> {
    const fn empty() -> Self {
        Self {
            key: WidenedPath::new(&[]),
            value_digest: None,
            children: BranchNode::empty_children(),
        }
    }

    /// Constructs a trie root from a slice of proof nodes.
    ///
    /// Each node in the slice must be a strict prefix of the following node. And,
    /// each child node must be referenced by its parent.
    fn from_proof(
        nodes: &'a Proof<impl ProofCollection<Node = ProofNode>>,
    ) -> Result<Option<Self>, ProofError> {
        nodes
            .as_ref()
            .iter()
            .rev()
            .try_fold(None::<Self>, |child, parent| match child {
                // each node in the slice must be a strict prefix of the following node
                Some(child) => child.with_parent(parent).map(Some),
                None => Ok(Some(Self::tail_node(parent))),
            })
    }

    /// Merges two trie roots, as returned by [`Self::from_proof`].
    ///
    /// Every node in both tries must be equal by hash. [`KeyProofTrieEdge::Described`]
    /// nodes in both tries must have the same key and value digest. The resulting
    /// trie will contain all nodes from both tries and resolve any
    /// [`KeyProofTrieEdge::Remote`] nodes to the corresponding
    /// [`KeyProofTrieEdge::Described`] if one is present in the opposite trie.
    ///
    /// Children that are present in both tries will be merged recursively. Nodes
    /// that are in both tries must have the same key and value digest.
    fn merge_proof_roots(lhs: Option<Self>, rhs: Option<Self>) -> Result<Option<Self>, ProofError> {
        match (lhs, rhs) {
            (Some(lhs), Some(rhs)) => Self::merge_from_proof(lhs, rhs).map(Some),
            (Some(root), None) | (None, Some(root)) => Ok(Some(root)),
            (None, None) => Ok(None),
        }
    }

    fn merge_from_proof(lhs: Self, rhs: Self) -> Result<Self, ProofError> {
        // at this level, keys must be identical
        if lhs.key != rhs.key {
            return Err(ProofError::DisjointEdges {
                lower: lhs.key.with_prefix().to_packed_key(),
                upper: rhs.key.with_prefix().to_packed_key(),
            });
        }

        if lhs.value_digest != rhs.value_digest {
            return Err(ProofError::DuplicateKeysInProof {
                key: lhs.key.with_prefix().to_packed_key(),
                value1: AsRef::<[u8]>::as_ref(&lhs.value_digest.unwrap_or(ValueDigest::Value(&[])))
                    .into(),
                value2: AsRef::<[u8]>::as_ref(&rhs.value_digest.unwrap_or(ValueDigest::Value(&[])))
                    .into(),
            });
        }

        let parent_path = lhs.key;
        let mut child_nibble = 0_u8;
        let children = merge_array(lhs.children, rhs.children, |lhs, rhs| {
            let nibble = child_nibble;
            child_nibble = child_nibble.wrapping_add(1);
            KeyProofTrieEdge::merge_from_proof(
                parent_path,
                nibble,
                lhs.map(|v| *v),
                rhs.map(|v| *v),
            )
        })?;

        Ok(Self {
            key: parent_path,
            value_digest: lhs.value_digest,
            children: boxed_children(children),
        })
    }

    fn tail_node(node: &'a ProofNode) -> Self {
        Self {
            key: WidenedPath::new(node.key.as_ref()),
            value_digest: node.value_digest.as_ref().map(ValueDigest::as_ref),
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
    /// must reference this node in its children by hash.
    fn with_parent(self, parent: &'a ProofNode) -> Result<Self, ProofError> {
        let parent_key = WidenedPath::new(&parent.key);
        let split = SplitKey::new(parent_key, self.key);

        let ((None, _), (Some(rhs_nibble), rhs_key)) = (
            split.lhs_suffix.split_first(),
            split.rhs_suffix.split_first(),
        ) else {
            return Err(ProofError::ShouldBePrefixOfNextKey {
                parent: parent_key.with_prefix().to_packed_key(),
                child: self.key.with_prefix().to_packed_key(),
            });
        };

        let mut with_self = BranchNode::empty_children();
        with_self
            .get_mut(rhs_nibble as usize)
            .ok_or_else(|| ProofError::ChildIndexOutOfBounds(self.key.nibbles_iter().collect()))?
            .replace(self);

        let children = merge_array(
            parent.child_hashes.each_ref().map(Option::as_ref),
            with_self,
            |hash, this| match (hash, this) {
                (None, None) => Ok(None),
                (None, Some(child)) => Err(ProofError::NodeNotInTrie {
                    parent: parent_key.with_prefix().to_packed_key(),
                    child_nibble: rhs_nibble,
                    child: child.key.with_prefix().to_packed_key(),
                }),
                (Some(hash), None) => Ok(Some(KeyProofTrieEdge::Remote(hash.clone()))),
                (Some(hash), Some(this)) => Ok(Some(KeyProofTrieEdge::Described(
                    hash.clone(),
                    KeyProofTrieRoot {
                        key: rhs_key,
                        value_digest: this.value_digest,
                        children: this.children,
                    },
                ))),
            },
        )?;

        Ok(Self {
            key: parent_key,
            value_digest: parent.value_digest.as_ref().map(ValueDigest::as_ref),
            children: boxed_children(children),
        })
    }
}

impl KeyProofTrieEdge<'_> {
    fn merge_from_proof(
        parent_path: WidenedPath<'_>,
        child_nibble: u8,
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
                        key: firewood_storage::padded_packed_path(
                            parent_path
                                .with_prefix()
                                .nibbles_iter()
                                .chain(Some(child_nibble)),
                        ),
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
                        key: root.key.with_prefix().to_packed_key(),
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
                    KeyProofTrieRoot::merge_from_proof(l_root, r_root)
                        .map(|root| Some(Self::Described(l_id, root)))
                } else {
                    Err(ProofError::UnexpectedHash {
                        key: l_root.key.with_prefix().to_packed_key(),
                        expected: l_id,
                        actual: r_id,
                    })
                }
            }
        }
    }
}

struct Shunt<'a, P> {
    parent_nibbles: &'a [u8],
    key: P,
    value: Option<ValueDigest<&'a [u8]>>,
    child_hashes: Children<HashType>,
}

impl<'a, P> Shunt<'a, P>
where
    P: Nibbles<'a> + Copy,
{
    const fn new(
        parent_nibbles: &'a [u8],
        key: P,
        value: Option<ValueDigest<&'a [u8]>>,
        child_hashes: Children<HashType>,
    ) -> Self {
        Self {
            parent_nibbles,
            key,
            value,
            child_hashes,
        }
    }

    fn to_hash(&self) -> HashType {
        firewood_storage::Preimage::to_hash(self)
    }
}

impl<'a, P: Nibbles<'a> + Copy> std::fmt::Debug for Shunt<'a, P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        struct ParentPath<'a>(&'a [u8]);

        impl std::fmt::Debug for ParentPath<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                super::path::display_nibbles(f, self.0.iter().copied())
            }
        }

        struct DebugNibbles<P>(P);

        impl<'a, P: Nibbles<'a> + Copy> std::fmt::Debug for DebugNibbles<P> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                super::path::display_nibbles(f, self.0.nibbles_iter())
            }
        }

        f.debug_struct("Shunt")
            .field("parent_path", &ParentPath(self.parent_nibbles))
            .field("key", &DebugNibbles(self.key))
            .field("value", &self.value.as_ref().map(hex::encode))
            .field("child_hashes", &self.child_hashes)
            .finish()
    }
}

impl<'a, P: Nibbles<'a> + Copy> Hashable for Shunt<'a, P> {
    fn parent_prefix_path(&self) -> impl Iterator<Item = u8> + Clone {
        self.parent_nibbles.iter().copied()
    }

    fn partial_path(&self) -> impl Iterator<Item = u8> + Clone {
        self.key.nibbles_iter()
    }

    fn value_digest(&self) -> Option<ValueDigest<&[u8]>> {
        self.value.clone()
    }

    fn children(&self) -> Children<HashType> {
        self.child_hashes.clone()
    }
}

trait Merge: Sized {
    type Output;
    type Error;
    fn merge(lhs: Self, rhs: Self) -> Result<Option<Self::Output>, Self::Error>;
}

impl<T: Merge<Output = T>> Merge for Option<T> {
    type Output = T;
    type Error = T::Error;

    fn merge(lhs: Self, rhs: Self) -> Result<Option<Self::Output>, Self::Error> {
        match (lhs, rhs) {
            (Some(l), Some(r)) => T::merge(l, r),
            (Some(l), None) => Ok(Some(l)),
            (None, Some(r)) => Ok(Some(r)),
            (None, None) => Ok(None),
        }
    }
}

impl<T: Merge> Merge for Box<T> {
    type Output = Box<T::Output>;
    type Error = T::Error;

    fn merge(lhs: Self, rhs: Self) -> Result<Option<Self::Output>, Self::Error> {
        #[inline]
        fn boxed<T>(t: Option<T>) -> Option<Box<T>> {
            t.map(Box::new)
        }

        T::merge(*lhs, *rhs).map(boxed)
    }
}

impl<'a> Merge for KeyValueTrieRoot<'a> {
    type Output = KeyValueTrieRoot<'a>;
    type Error = ProofError;

    fn merge(lhs: Self, rhs: Self) -> Result<Option<Self::Output>, Self::Error> {
        KeyValueTrieRoot::merge(lhs, rhs).map(Some)
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

    fn fork_push(&mut self, nibble: u8) -> PathGuard<'_> {
        let mut this = self.fork();
        this.push(nibble);
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
