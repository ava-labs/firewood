// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::{BranchNode, Children};

use crate::{
    proof::{DuplicateKeysInProofError, ProofError},
    proofs::{
        path::{
            CollectedNibbles, Nibbles, PackedPath, PathGuard, PathNibble, SplitNibbles, SplitPath,
        },
        trie::{counter::NibbleCounter, iter::Child},
    },
};

/// A collected of key-value pairs organized into a trie.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct KeyValueTrieRoot<'a> {
    pub(super) partial_path: PackedPath<'a>,
    pub(super) value: Option<&'a [u8]>,
    pub(super) children: Children<Box<KeyValueTrieRoot<'a>>>,
}

impl<'a> KeyValueTrieRoot<'a> {
    const fn leaf(key: &'a [u8], value: &'a [u8]) -> Self {
        Self {
            partial_path: PackedPath::new(key),
            value: Some(value),
            children: BranchNode::empty_children(),
        }
    }

    pub(super) fn new<K, V>(
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

    #[expect(unused)]
    fn new_v2<K, V>(
        mut leading_path: PathGuard<'_>,
        pairs: &'a [(K, V)],
    ) -> Result<Option<Self>, ProofError>
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        // we need to ensure that the keys are sorted so that we can efficiently build a trie.
        if !pairs.iter().is_sorted_by_key(|(k, _)| k.as_ref()) {
            return Err(ProofError::NonMonotonicIncreaseRange);
        }

        let mut bitmap = crate::proofs::bitmap::ChildrenMap::empty();

        todo!();
    }

    pub fn lower_bound(&self) -> (CollectedNibbles, &Self) {
        let mut path = CollectedNibbles::empty();
        let mut this = self;
        loop {
            path.extend(this.partial_path);
            let Some((nibble, node)) = this
                .children
                .iter()
                .enumerate()
                .find_map(|(nibble, child)| child.as_deref().map(|child| (nibble as u8, child)))
            else {
                return (path, this);
            };
            path.extend(PathNibble(nibble));
            this = node;
        }
    }

    pub fn upper_bound(&self) -> (CollectedNibbles, &Self) {
        let mut path = CollectedNibbles::empty();
        let mut this = self;
        loop {
            path.extend(this.partial_path);
            let Some((nibble, node)) = this
                .children
                .iter()
                .enumerate()
                .rev()
                .find_map(|(nibble, child)| child.as_deref().map(|child| (nibble as u8, child)))
            else {
                break (path, this);
            };
            path.extend(PathNibble(nibble));
            this = node;
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
                return Err(ProofError::DuplicateKeysInProof(
                    DuplicateKeysInProofError {
                        key: leading_path.bytes_iter().collect(),
                        value1: hex::encode(value1),
                        value2: hex::encode(value2),
                    },
                ));
            }
            (Some(v), None) | (None, Some(v)) => Some(v),
            (None, None) => None,
        };

        let mut nibble = NibbleCounter::new();
        let children = super::merge_array(lhs.children, rhs.children, |lhs, rhs| {
            Self::merge_root(
                leading_path.fork_push(nibble.next()),
                lhs.map(|v| *v),
                rhs.map(|v| *v),
            )
        })?;

        Ok(Self {
            partial_path: lhs.partial_path,
            value,
            children: super::boxed_children(children),
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

impl<'a> super::TrieNode<'a> for &'a KeyValueTrieRoot<'a> {
    type Nibbles = PackedPath<'a>;

    fn partial_path(self) -> Self::Nibbles {
        self.partial_path
    }

    fn value_digest(self) -> Option<firewood_storage::ValueDigest<&'a [u8]>> {
        self.value.map(firewood_storage::ValueDigest::Value)
    }

    fn computed_hash(self) -> Option<firewood_storage::HashType> {
        None
    }

    fn children(self) -> Children<Child<Self>> {
        self.children
            .each_ref()
            .map(|child| child.as_deref().map(Child::Unhashed))
    }
}
