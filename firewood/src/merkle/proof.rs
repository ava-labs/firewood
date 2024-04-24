// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::HashMap;

use crate::v2::api::HashKey;
use aiofut::AioError;
use nix::errno::Errno;
use sha3::Digest;
use thiserror::Error;

use crate::{
    db::DbError,
    merkle::MerkleError,
    merkle_util::{DataStoreError, InMemoryMerkle},
};

#[derive(Debug, Error)]
pub enum ProofError {
    #[error("aio error: {0:?}")]
    AioError(AioError),
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

impl From<DataStoreError> for ProofError {
    fn from(d: DataStoreError) -> ProofError {
        match d {
            DataStoreError::InsertionError => ProofError::NodesInsertionError,
            DataStoreError::RootHashError => ProofError::InvalidRootHash,
            _ => ProofError::InvalidProof,
        }
    }
}

impl From<DbError> for ProofError {
    fn from(d: DbError) -> ProofError {
        match d {
            DbError::Aio(e) => ProofError::AioError(e),
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

/// A proof that a single key is present
///
/// The generic N represents the storage for the node
#[derive(Clone, Debug)]
pub struct Proof<N>(pub HashMap<HashKey, N>);

/// `SubProof` contains the value or the hash of a node that maps
/// to a single proof step. If reaches an end step during proof verification,
/// the `SubProof` should be the `Value` variant.

#[derive(Debug)]
#[allow(dead_code)] // TODO use or remove this type
enum SubProof {
    Value(Vec<u8>),
    Hash(HashKey),
}

impl<N: AsRef<[u8]> + Send> Proof<N> {
    /// verify_proof checks merkle proofs. The given proof must contain the value for
    /// key in a trie with the given root hash. VerifyProof returns an error if the
    /// proof contains invalid trie nodes or the wrong value.
    ///
    /// The generic N represents the storage for the node
    pub fn verify<K: AsRef<[u8]>>(
        &self,
        _key: K,
        _root_hash: HashKey,
    ) -> Result<Option<Vec<u8>>, ProofError> {
        todo!()
    }

    pub fn extend(&mut self, other: Proof<N>) {
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

    /// proofToPath converts a merkle proof to trie node path. The main purpose of
    /// this function is recovering a node path from the merkle proof stream. All
    /// necessary nodes will be resolved and leave the remaining as hashnode.
    ///
    /// The given edge proof is allowed to be an existent or non-existent proof.
    fn _proof_to_path<K, T>(
        &self,
        _key: K,
        _root_hash: HashKey,
        _in_mem_merkle: &mut InMemoryMerkle,
        _allow_non_existent_node: bool,
    ) -> Result<Option<Vec<u8>>, ProofError>
    where
        K: AsRef<[u8]>,
    {
        todo!()
    }
}

fn _generate_subproof_hash(encoded: &[u8]) -> Result<HashKey, ProofError> {
    match encoded.len() {
        0..=31 => {
            let sub_hash = sha3::Keccak256::digest(encoded).into();
            Ok(sub_hash)
        }

        32 => {
            let sub_hash = encoded
                .try_into()
                .expect("slice length checked in match arm");

            Ok(sub_hash)
        }

        len => Err(ProofError::DecodeError(Box::new(
            bincode::ErrorKind::Custom(format!("invalid proof length: {len}")),
        ))),
    }
}

fn _generate_subproof(encoded: &[u8]) -> Result<SubProof, ProofError> {
    Ok(SubProof::Hash(_generate_subproof_hash(encoded)?))
}
