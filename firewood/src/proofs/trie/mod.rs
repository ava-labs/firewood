// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod counter;
mod dump;
mod hashed;
mod iter;
mod keyvalues;
mod merged;
mod proof;
mod shunt;

use firewood_storage::{Children, HashType, ValueDigest, logger::trace};

pub(crate) use self::hashed::HashedRangeProof;
pub use self::iter::MissingKeys;
use crate::{
    proof::{ProofCollection, ProofError, ProofNode, UnexpectedHashError},
    proofs::{
        VerifyRangeProofArguments,
        path::{Nibbles, PathGuard},
        trie::{
            dump::DumpTrie,
            iter::{Child, PreOrderIter},
            keyvalues::KeyValueTrieRoot,
            merged::RangeProofTrieRoot,
            proof::KeyProofTrieRoot,
        },
    },
    v2::api::{KeyType, ValueType},
};

impl<K, V, H> VerifyRangeProofArguments<'_, K, V, H>
where
    K: KeyType,
    V: ValueType,
    H: ProofCollection<Node = ProofNode>,
{
    pub fn verify(self) -> Result<Vec<MissingKeys>, ProofError> {
        macro_rules! trace_trie {
            ($prefix:expr, Some($trie:expr)) => {
                trace!("{} trie\n{}", $prefix, $trie.display());
            };
            ($prefix:expr, $maybe_trie:expr) => {
                if let Some(ref trie) = $maybe_trie {
                    trace_trie!($prefix, Some(trie));
                } else {
                    trace!("{} trie: <empty>", $prefix);
                }
            };
        }

        let mut path = Vec::new();
        let mut path = PathGuard::new(&mut path);

        let kvp = KeyValueTrieRoot::new(path.fork(), self.range_proof.key_values())?;
        trace_trie!("KVP", kvp);

        let kvp_bounds = kvp
            .as_ref()
            .map(|kvp| (kvp.lower_bound(), kvp.upper_bound()));
        trace!("KVP bounds: {kvp_bounds:#?}");
        let kvp_bounds = kvp_bounds.map(|((l, _), (r, _))| (l, r));

        let lower_bound_proof = KeyProofTrieRoot::new(self.range_proof.start_proof())?;
        trace_trie!("Lower bound proof", lower_bound_proof);

        let upper_bound_proof = KeyProofTrieRoot::new(self.range_proof.end_proof())?;
        trace_trie!("Upper bound proof", upper_bound_proof);

        let proof = KeyProofTrieRoot::merge_opt(path.fork(), lower_bound_proof, upper_bound_proof)?;
        trace_trie!("Merged bound proof", proof);

        let proof_bounds = proof
            .as_ref()
            .map(|proof| (proof.lower_bound(), proof.upper_bound()));
        trace!("Proof bounds: {proof_bounds:#?}");

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
        trace_trie!("Combined", Some(root.as_ref()));

        let root = HashedRangeProof::new(path, root)?;
        trace_trie!("Hashed", Some(root.as_ref()));

        // range proofs are not allowed to have holes within the range defined by
        // the lower and upper bound keys; but, they may outside of that range.
        //
        // Callers will be interested in the holes outside of the range in order
        // to continue fetching data if desired. So, we collect them here for
        // both verification and to return to the caller.
        let holes = root
            .as_ref()
            .pre_order_iter()
            .filter_map(|item| match item.node {
                Child::Unhashed(_) | Child::Hashed(_, _) => None,
                Child::Remote(hash) => Some(MissingKeys {
                    depth: item.depth,
                    leading_path: item.leading_path,
                    hash,
                }),
            })
            .collect::<Vec<_>>();
        trace!("Holes: {holes:#?}");
        if let Some((lower_path, upper_path)) = kvp_bounds {
            let bounds = lower_path..=upper_path;
            let missing_keys = holes
                .iter()
                .filter(|hole| bounds.contains(&hole.leading_path))
                .cloned()
                .collect::<Vec<_>>();
            if !missing_keys.is_empty() {
                let (lower_bound, upper_bound) = bounds.into_inner();
                return Err(ProofError::MissingKeys(Box::new(
                    crate::proof::MissingKeys {
                        lower_bound,
                        upper_bound,
                        missing_keys,
                    },
                )));
            }
        }

        if root.computed() != self.expected_root {
            return Err(ProofError::UnexpectedHash(Box::new(UnexpectedHashError {
                key: Box::new([]),
                context: "hashing trie root",
                expected: self.expected_root.clone(),
                actual: root.computed().clone(),
            })));
        }

        Ok(holes)
    }
}

trait TrieNode<'a>: Sized + Copy {
    type Nibbles: Nibbles;
    fn partial_path(self) -> Self::Nibbles;
    fn value_digest(self) -> Option<ValueDigest<&'a [u8]>>;
    fn computed_hash(self) -> Option<HashType>;
    fn children(self) -> Children<Child<Self>>;

    fn display(self) -> DumpTrie<Self> {
        DumpTrie::new(self)
    }

    fn pre_order_iter(self) -> PreOrderIter<Self> {
        PreOrderIter::new(self)
    }
}

impl<'a, L: TrieNode<'a>, R: TrieNode<'a>> TrieNode<'a> for either::Either<L, R> {
    type Nibbles = either::Either<L::Nibbles, R::Nibbles>;

    fn partial_path(self) -> Self::Nibbles {
        match self {
            either::Left(l) => either::Left(l.partial_path()),
            either::Right(r) => either::Right(r.partial_path()),
        }
    }

    fn value_digest(self) -> Option<ValueDigest<&'a [u8]>> {
        either::for_both!(self, this => this.value_digest())
    }

    fn computed_hash(self) -> Option<HashType> {
        either::for_both!(self, this => this.computed_hash())
    }

    fn children(self) -> Children<Child<Self>> {
        match self {
            either::Left(l) => l
                .children()
                .map(|maybe| maybe.map(|child| child.map(either::Left))),
            either::Right(r) => r
                .children()
                .map(|maybe| maybe.map(|child| child.map(either::Right))),
        }
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
