// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::v2::api::HashKey;
use nix::errno::Errno;
use storage::{BranchNode, TrieHash};
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

#[derive(Clone, Debug)]
pub struct ProofNode {
    /// The key this node is at.
    pub key: Box<[u8]>, // TODO danlaine: should this be generic?
    /// None if the node has no value.
    /// The value associated with `key` if the value is < 32 bytes.
    /// The hash of the value if it's >= than 32 bytes.
    pub value_digest: Option<Box<[u8]>>, // TODO danlaine: should this be generic?
    /// The hash of each child, or None if the child does not exist.
    pub child_hashes: [Option<TrieHash>; BranchNode::MAX_CHILDREN],
}

/// A proof that a given key-value pair either exists or does not exist in a trie.
#[derive(Clone, Debug)]
pub struct Proof(pub Box<[ProofNode]>);

impl Proof {
    /// Returns the value associated with the given `key` in the trie revision
    /// with the given `root_hash`. If the key does not exist in the trie, returns `None`.
    /// Returns an error if the proof is invalid or doesn't prove the key-value pair for
    /// the given `root_hash`.
    pub fn verify<K: AsRef<[u8]>>(
        &self,
        _key: K,
        _root_hash: HashKey,
    ) -> Result<Option<Vec<u8>>, ProofError> {
        todo!()
    }

    // TODO danlaine: This should go somewhere else.
    // Proof code should be separate from range proof code.
    // pub fn verify_range_proof<K, V>(
    //     &self,
    //     _root_hash: HashKey,
    //     _first_key: K,
    //     _last_key: K,
    //     keys: Vec<K>,
    //     vals: Vec<V>,
    // ) -> Result<bool, ProofError>
    // where
    //     K: AsRef<[u8]>,
    //     V: AsRef<[u8]>,
    // {
    //     if keys.len() != vals.len() {
    //         return Err(ProofError::InconsistentProofData);
    //     }

    //     // Ensure the received batch is monotonic increasing and contains no deletions
    //     #[allow(clippy::indexing_slicing)]
    //     if !keys.windows(2).all(|w| w[0].as_ref() < w[1].as_ref()) {
    //         return Err(ProofError::NonMonotonicIncreaseRange);
    //     }

    //     // create an empty merkle trie in memory
    //     todo!();
    // }
}
