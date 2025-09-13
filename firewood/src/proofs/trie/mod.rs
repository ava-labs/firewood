// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod counter;
mod hashed;
mod keyvalues;
mod merged;
mod proof;
mod shunt;

use firewood_storage::Path;

use crate::{
    proof::{ProofCollection, ProofError, ProofNode, UnexpectedHashError},
    proofs::{
        VerifyRangeProofArguments,
        path::{Nibbles, PackedPath, PathGuard, WidenedPath},
        trie::{keyvalues::KeyValueTrieRoot, merged::RangeProofTrieRoot, proof::KeyProofTrieRoot},
    },
    v2::api::{KeyType, ValueType},
};

pub(crate) use self::hashed::HashedRangeProof;

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

impl<K, V, H> VerifyRangeProofArguments<'_, K, V, H>
where
    K: KeyType,
    V: ValueType,
    H: ProofCollection<Node = ProofNode>,
{
    pub fn verify(self) -> Result<(), ProofError> {
        let mut path = Vec::new();
        let mut path = PathGuard::new(&mut path);

        let kvp = KeyValueTrieRoot::new(path.fork(), self.range_proof.key_values())?;
        let kvp_bounds = kvp
            .as_ref()
            .map(|kvp| (kvp.lower_bound(), kvp.upper_bound()));

        let lower_bound_proof = KeyProofTrieRoot::new(self.range_proof.start_proof())?;
        let upper_bound_proof = KeyProofTrieRoot::new(self.range_proof.end_proof())?;
        let proof = KeyProofTrieRoot::merge_opt(path.fork(), lower_bound_proof, upper_bound_proof)?;
        let proof_bounds = proof
            .as_ref()
            .map(|proof| (proof.lower_bound(), proof.upper_bound()));

        let root = match (proof, kvp) {
            (None, None) => Ok(either::Left(RangeProofTrieRoot::empty())),
            // no values, return the root as is
            (Some(proof), None) => Ok(either::Left(RangeProofTrieRoot::from_proof_root(proof))),
            // no proof, so we have an unbounded range of key-values
            (None, Some(kvp)) => Ok(either::Right(kvp)),
            (Some(proof), Some(kvp)) => {
                RangeProofTrieRoot::join(path.fork(), proof, kvp).map(either::Left)
            }
        }?;

        let root = HashedRangeProof::new(path, root)?;

        if root.computed() != self.expected_root {
            return Err(ProofError::UnexpectedHash(Box::new(UnexpectedHashError {
                key: Box::new([]),
                context: "hashing trie root",
                expected: self.expected_root.clone(),
                actual: root.computed().clone(),
            })));
        }

        Ok(())
    }
}
