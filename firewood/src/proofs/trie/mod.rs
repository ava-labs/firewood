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

use std::ops::RangeBounds;

use firewood_storage::{Children, HashType, IntoHashType, ValueDigest, logger::trace};

pub(crate) use self::hashed::HashedRangeProof;
pub use self::iter::MissingKeys;
use crate::{
    proof::{ProofCollection, ProofError, ProofNode, UnexpectedHashError},
    proofs::{
        CollectedNibbles, VerifyRangeProofArguments,
        path::{Nibbles, PackedPath, PathGuard},
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

#[derive(Debug)]
struct KeyBounds {
    lower: Option<CollectedNibbles>,
    upper: Option<CollectedNibbles>,
}

type ProofBoundFn<'a, 'b> =
    fn(&'a KeyProofTrieRoot<'b>) -> (CollectedNibbles, &'a KeyProofTrieRoot<'b>);

impl KeyBounds {
    const fn unbounded() -> Self {
        Self {
            lower: None,
            upper: None,
        }
    }

    fn edge_bound<'a, 'b>(
        bound: Option<PackedPath<'_>>,
        proof: Option<&'a KeyProofTrieRoot<'b>>,
        proof_bound_fn: ProofBoundFn<'a, 'b>,
        is_valid: fn(&PackedPath<'_>, &CollectedNibbles) -> bool,
    ) -> Result<Option<CollectedNibbles>, ProofError> {
        match (bound, proof) {
            // no proof, edge is unbounded
            (_, None) => Ok(None),
            // only a proof, so bounded by the proof
            (None, Some(proof)) => {
                let (proof_bound, _) = proof_bound_fn(proof);
                Ok(Some(proof_bound))
            }
            // both a bound and a proof, so bounded by the tighter of the two
            (Some(bound), Some(proof)) => {
                let (proof_bound, _) = proof_bound_fn(proof);
                if !is_valid(&bound, &proof_bound) {
                    trace!("Invalid bound: {bound:?} vs proof bound: {proof_bound:?}");
                    return Err(ProofError::InvalidRange);
                }
                Ok(Some(bound.collect()))
            }
        }
    }
}

impl RangeBounds<CollectedNibbles> for KeyBounds {
    fn start_bound(&self) -> std::ops::Bound<&CollectedNibbles> {
        match self.lower.as_ref() {
            Some(bound) => std::ops::Bound::Included(bound),
            None => std::ops::Bound::Unbounded,
        }
    }

    fn end_bound(&self) -> std::ops::Bound<&CollectedNibbles> {
        match self.upper.as_ref() {
            Some(bound) => std::ops::Bound::Included(bound),
            None => std::ops::Bound::Unbounded,
        }
    }
}

impl<K, V, H> VerifyRangeProofArguments<'_, K, V, H>
where
    K: KeyType,
    V: ValueType,
    H: ProofCollection<Node = ProofNode>,
{
    /// Shrink the bounds, if applicable, and return the resulting bounds, if any.
    ///
    /// - If there is no proof for the respective bound, then the edge is unbounded.
    /// - If there is a proof for the respective bound, then its bound must be
    ///   within the bounds specified in the parameters. However, if the bound
    ///   is smaller than the specified bound (but still within it), then the
    ///   smaller bound is used.
    /// - Proof bounds may be truncated at the node where the proof ends. In this
    ///   case, the proof bound must be a strict prefix of the specified bound.
    /// - Additionally, the key value pairs must be within the bounds of the
    ///   proof, which may be smaller than the specified bounds.
    ///
    /// The resulting bounds are inclusive in their respective directions.
    fn key_bounds(
        &self,
        kvp: Option<&KeyValueTrieRoot<'_>>,
        left_proof: Option<&KeyProofTrieRoot<'_>>,
        right_proof: Option<&KeyProofTrieRoot<'_>>,
    ) -> Result<KeyBounds, ProofError> {
        let mut bounds = KeyBounds::unbounded();
        bounds.lower = KeyBounds::edge_bound(
            self.lower_bound.map(PackedPath::new),
            left_proof,
            KeyProofTrieRoot::lower_bound,
            // if the proof bound is a strict prefix of the specified bound, then it will
            // be "less than" the bound but only if it is a strict prefix is this valid
            |bound, proof| bound <= proof || proof.is_prefix_of(bound),
        )?;
        bounds.upper = KeyBounds::edge_bound(
            self.upper_bound.map(PackedPath::new),
            right_proof,
            KeyProofTrieRoot::upper_bound,
            |bound, proof| bound >= proof,
        )?;

        if let (Some(lower), Some(upper)) = (bounds.lower.as_ref(), bounds.upper.as_ref())
            && lower > upper
        {
            return Err(ProofError::InvalidRange);
        }

        if let Some(kvp) = kvp {
            let (lower_bound, _) = kvp.lower_bound();
            let (upper_bound, _) = kvp.upper_bound();
            if bounds.contains(&lower_bound) && bounds.contains(&upper_bound) {
                trace!(
                    "kvp bounds {lower_bound:?}, {upper_bound:?} within proof bounds {bounds:?}"
                );
                bounds.lower = Some(lower_bound);
                bounds.upper = Some(upper_bound);
            } else {
                trace!(
                    "kvp bounds {lower_bound:?}, {upper_bound:?} not within proof bounds {bounds:?}"
                );
                return Err(ProofError::StateFromOutsideOfRange);
            }
        }

        Ok(bounds)
    }

    pub fn verify(self) -> Result<Vec<MissingKeys>, ProofError> {
        let mut path = Vec::new();
        let mut path = PathGuard::new(&mut path);

        let kvp = KeyValueTrieRoot::new(path.fork(), self.range_proof.key_values())?;
        trace_trie!("KVP", kvp);

        let lower_bound_proof = KeyProofTrieRoot::new(self.range_proof.start_proof())?;
        trace_trie!("Lower bound proof", lower_bound_proof);
        let upper_bound_proof = KeyProofTrieRoot::new(self.range_proof.end_proof())?;
        trace_trie!("Upper bound proof", upper_bound_proof);

        let key_bounds = self.key_bounds(
            kvp.as_ref(),
            lower_bound_proof.as_ref(),
            upper_bound_proof.as_ref(),
        )?;
        trace!("Bounds: {key_bounds:#?}");

        let proof = KeyProofTrieRoot::merge_opt(path.fork(), lower_bound_proof, upper_bound_proof)?;
        trace_trie!("Merged bound proof", proof);

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

        let missing_keys = holes
            .iter()
            .filter(|hole| key_bounds.contains(&hole.leading_path))
            .cloned()
            .collect::<Vec<_>>();
        if !missing_keys.is_empty() {
            // let (lower_bound, upper_bound) = bounds.into_inner();
            return Err(ProofError::MissingKeys(Box::new(
                crate::proof::MissingKeys {
                    lower_bound: key_bounds.lower.unwrap_or_default(),
                    upper_bound: key_bounds.upper.unwrap_or_default(),
                    missing_keys,
                },
            )));
        }

        if root.computed() != self.expected_root {
            return Err(ProofError::UnexpectedHash(Box::new(UnexpectedHashError {
                key: Box::new([]),
                context: "hashing trie root",
                expected: self.expected_root.clone().into_hash_type(),
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
