// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::sync::Arc;

use firewood_storage::{
    HashedNodeReader, NodeStore, Parentable, ReadableStorage,
    tries::{IterAscending, RangeProofTrieRoot, TrieEdgeIter},
};

use crate::iter::MerkleNodeIter;

pub struct FindNextKeyIterator<'a, P, S, T: ?Sized> {
    // revision_iter: MerkleNodeIter<'a, NodeStore<P, S>>,
    // proof_trie_iter: TrieEdgeIter<'a, IterAscending>,
    revision: &'a NodeStore<P, S>,
    stack: Vec<Frame<'a, T>>,
}

struct Frame<'a, T: ?Sized> {
    revision_node: firewood_storage::SharedNode,
    proof_node: either::Either<
        &'a firewood_storage::tries::KeyValueTrieRoot<'a, T>,
        &'a firewood_storage::tries::RangeProofTrieNode<'a, T>,
    >,
}

impl<'a, P, S, T> FindNextKeyIterator<'a, P, S, T>
where
    P: Parentable,
    S: ReadableStorage,
    T: AsRef<[u8]> + ?Sized,
    NodeStore<P, S>: HashedNodeReader,
{
    pub fn new(revision: &'a NodeStore<P, S>, proof_trie: &'a RangeProofTrieRoot<'a, T>) -> Self {
        todo!();
    }
}
