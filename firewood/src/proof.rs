// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.
use crate::v2::api::HashKey;
use nix::errno::Errno;
use sha3::{Digest, Sha3_256};
use storage::{BranchNode, Node, Path, TrieHash};
use thiserror::Error;

use crate::{db::DbError, merkle::MerkleError};

#[derive(Debug, Error)]
pub enum ProofError {
    #[error("decoding error")]
    DecodeError(#[from] bincode::Error),
    #[error("no such node")]
    NoSuchNode,
    #[error("proof node missing")]
    ProofNodeMissing,
    #[error("inconsistent proof data")]
    InconsistentProofData,
    #[error("non-monotonic range increase")]
    NonMonotonicIncreaseRange,
    #[error("invalid data")]
    InvalidData,
    #[error("invalid proof")]
    InvalidProof,
    #[error("invalid edge keys")]
    InvalidEdgeKeys,
    #[error("node insertion error")]
    NodesInsertionError,
    #[error("node not in trie")]
    NodeNotInTrie,
    #[error("invalid node {0:?}")]
    InvalidNode(#[from] MerkleError),
    #[error("empty range")]
    EmptyRange,
    #[error("fork left")]
    ForkLeft,
    #[error("fork right")]
    ForkRight,
    #[error("system error: {0:?}")]
    SystemError(Errno),
    #[error("invalid root hash")]
    InvalidRootHash,
}

impl From<DbError> for ProofError {
    fn from(d: DbError) -> ProofError {
        match d {
            DbError::InvalidParams => ProofError::InvalidProof,
            DbError::Merkle(e) => ProofError::InvalidNode(e),
            DbError::System(e) => ProofError::SystemError(e),
            DbError::KeyNotFound => ProofError::InvalidEdgeKeys,
            DbError::CreateError => ProofError::NoSuchNode,
            // TODO: fix better by adding a new error to ProofError
            #[allow(clippy::unwrap_used)]
            DbError::IO(e) => {
                ProofError::SystemError(nix::errno::Errno::from_raw(e.raw_os_error().unwrap()))
            }
            DbError::InvalidProposal => ProofError::InvalidProof,
        }
    }
}

/*
type ProofNode struct {
    Key Key
    // Nothing if this is an intermediate node.
    // The value in this node if its length < [HashLen].
    // The hash of the value in this node otherwise.
    ValueOrHash maybe.Maybe[[]byte]
    Children    map[byte]ids.ID
}
*/

// TODO danlaine: This should proabbly go somewhere else.
// We will want to make the hasher generic.
/// Returns the value digest for of this node, or None if it has no value.
pub fn _value_digest(node: &Node) -> Option<Box<[u8]>> {
    let value = match node {
        Node::Branch(branch) => match &branch.value {
            Some(value) => value,
            None => return None.into(),
        },
        Node::Leaf(leaf) => &leaf.value,
    };

    if value.len() < 32 {
        Some(value.clone())
    } else {
        let hash = Sha3_256::digest(value)
            .into_iter()
            .collect::<Vec<u8>>()
            .into_boxed_slice();
        Some(hash)
    }
}

#[derive(Clone, Debug)]
pub struct ProofNode {
    path: Path,
    value_digest: Option<Box<[u8]>>,
    children: [Option<TrieHash>; BranchNode::MAX_CHILDREN],
}

/// A proof that a key does/doesn't exist in a given trie.
#[derive(Clone, Debug)]
pub struct Proof(pub Vec<ProofNode>);

impl Proof {
    pub fn verify(&self, _path: Path, _root_hash: TrieHash) -> Result<bool, ProofError> {
        // First check whether the proof is syntactically valid.
        todo!()
    }

    pub fn extend(&mut self, other: Proof) {
        self.0.extend(other.0)
    }

    pub fn verify_range_proof<K, V>(
        &self,
        _root_hash: HashKey,
        _first_key: K,
        _last_key: K,
        keys: Vec<K>,
        vals: Vec<V>,
    ) -> Result<bool, ProofError>
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        if keys.len() != vals.len() {
            return Err(ProofError::InconsistentProofData);
        }

        // Ensure the received batch is monotonic increasing and contains no deletions
        #[allow(clippy::indexing_slicing)]
        if !keys.windows(2).all(|w| w[0].as_ref() < w[1].as_ref()) {
            return Err(ProofError::NonMonotonicIncreaseRange);
        }

        // create an empty merkle trie in memory
        todo!();
    }
}
