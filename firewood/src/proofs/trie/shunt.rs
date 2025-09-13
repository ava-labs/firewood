// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::{Children, HashType, Hashable, ValueDigest};

use crate::proofs::path::Nibbles;

/// A shunt for a hasheable trie that we can use to compute the hash of a node
/// using component parts.
pub(super) struct HashableShunt<'a, P1, P2> {
    parent_nibbles: P1,
    partial_path: P2,
    value: Option<ValueDigest<&'a [u8]>>,
    child_hashes: Children<HashType>,
}

impl<'a, P1: Nibbles, P2: Nibbles> HashableShunt<'a, P1, P2> {
    pub(super) const fn new(
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

    pub(super) fn to_hash(&self) -> HashType {
        firewood_storage::Preimage::to_hash(self)
    }
}

impl<P1: Nibbles, P2: Nibbles> std::fmt::Debug for HashableShunt<'_, P1, P2> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HashableShunt")
            .field("parent_nibbles", &self.parent_nibbles.display())
            .field("partial_path", &self.partial_path.display())
            .field(
                "value",
                &self.value.as_ref().map(|v| v.as_ref().map(hex::encode)),
            )
            .field("child_hashes", &self.child_hashes)
            .field("hash", &self.to_hash())
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
