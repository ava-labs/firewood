// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod counter;
mod hashed;
mod keyvalues;
mod merged;
mod proof;
mod shunt;

use std::borrow::Cow;

use firewood_storage::{Children, HashType, ValueDigest, logger::trace};

use crate::{
    proof::{ProofCollection, ProofError, ProofNode, UnexpectedHashError},
    proofs::{
        VerifyRangeProofArguments,
        path::{Nibbles, PathGuard, PathNibble},
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
        if let Some(ref trie) = kvp {
            trace!("KVP trie\n{}", trie.display());
        } else {
            trace!("KVP trie: <empty>");
        }

        let kvp_bounds = kvp
            .as_ref()
            .map(|kvp| (kvp.lower_bound(), kvp.upper_bound()));
        trace!("KVP bounds: {kvp_bounds:#?}");

        let lower_bound_proof = KeyProofTrieRoot::new(self.range_proof.start_proof())?;
        if let Some(ref trie) = lower_bound_proof {
            trace!("Lower bound proof trie\n{}", trie.display());
        } else {
            trace!("Lower bound proof trie: <empty>");
        }

        let upper_bound_proof = KeyProofTrieRoot::new(self.range_proof.end_proof())?;
        if let Some(ref trie) = upper_bound_proof {
            trace!("Upper bound proof trie\n{}", trie.display());
        } else {
            trace!("Upper bound proof trie: <empty>");
        }

        let proof = KeyProofTrieRoot::merge_opt(path.fork(), lower_bound_proof, upper_bound_proof)?;
        if let Some(ref trie) = proof {
            trace!("Merged bound proof trie\n{}", trie.display());
        } else {
            trace!("Merged bound proof trie: <empty>");
        }

        let proof_bounds = proof
            .as_ref()
            .map(|proof| (proof.lower_bound(), proof.upper_bound()));
        trace!("Proof bounds: {proof_bounds:?}");

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
        trace!("Merged trie: {}", root.as_ref().display());

        let root = HashedRangeProof::new(path, root)?;
        trace!("Hashed trie: {}", root.as_ref().display());

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

enum Child<T> {
    Unhashed(T),
    Hashed(HashType, T),
    Remote(HashType),
}

impl<T> Child<T> {
    fn map<U>(self, f: impl FnOnce(T) -> U) -> Child<U> {
        match self {
            Child::Unhashed(node) => Child::Unhashed(f(node)),
            Child::Hashed(hash, node) => Child::Hashed(hash, f(node)),
            Child::Remote(hash) => Child::Remote(hash),
        }
    }
}

trait TrieNode<'a>: Sized + Copy {
    type Nibbles: Nibbles;
    fn partial_path(self) -> Self::Nibbles;
    fn value_digest(self) -> Option<ValueDigest<&'a [u8]>>;
    fn computed_hash(self) -> Option<HashType>;
    fn children(self) -> Children<Child<Self>>;

    fn display(self) -> DumpTrie<Self> {
        DumpTrie(self)
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

struct DumpTrie<T>(T);

impl<'a, T: TrieNode<'a>> std::fmt::Debug for DumpTrie<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        dump_trie(f, self.0)
    }
}

impl<'a, T: TrieNode<'a>> std::fmt::Display for DumpTrie<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        dump_trie(f, self.0)
    }
}

/// Dumps a trie to the given writer, for debugging purposes.
///
/// The output format is not stable and is only for human readable consumption.
///
/// This will walk the trie in pre-order and print each node's partial path, value
/// digest (if any), previously computed hash (if any), and the children recursively.
///
/// Children in the trie may either be by reference (hash only) or by value (full
/// node) and full nodes will be dumped recursively.
fn dump_trie<'a, W, T>(w: &mut W, node: T) -> std::fmt::Result
where
    W: std::fmt::Write + ?Sized,
    T: TrieNode<'a>,
{
    fn inner_dump<'a, W, T>(
        w: &mut W,
        node: T,
        depth: usize,
        mut path: PathGuard<'_>,
    ) -> std::fmt::Result
    where
        W: std::fmt::Write + ?Sized,
        T: TrieNode<'a>,
    {
        const INDENT: usize = 2;

        #[cfg(not(feature = "branch_factor_256"))]
        const NIBBLE_WIDTH: usize = 1;
        #[cfg(feature = "branch_factor_256")]
        const NIBBLE_WIDTH: usize = 2;

        // empty string, the width is used to pad with `depth` spaces
        const WS: &str = "";

        path.extend(node.partial_path().nibbles_iter());
        writeln!(w, "{WS:>depth$}Path: {}", path.display())?;

        if let Some(vd) = node.value_digest() {
            match vd {
                ValueDigest::Value(value) => {
                    let value = str::from_utf8(value)
                        .map_or_else(|_| hex::encode(value).into(), Cow::Borrowed);
                    writeln!(w, "{WS:>depth$}- Value: {value}")?;
                }
                #[cfg(not(feature = "ethhash"))]
                ValueDigest::Hash(digest) => {
                    writeln!(w, "{WS:>depth$}- Value Digest: {digest}")?;
                }
            }
        }

        if let Some(ch) = node.computed_hash() {
            writeln!(w, "{WS:>depth$}- Computed Hash: {ch:.64}")?;
        }

        for (idx, slot) in node.children().into_iter().enumerate() {
            let path = path.fork_push(PathNibble(idx as u8));
            write!(w, "{WS:>depth$}- Child {idx:0NIBBLE_WIDTH$x} ")?;
            match slot {
                Some(Child::Unhashed(node)) => {
                    writeln!(w, "(unhashed):")?;
                    inner_dump(w, node, depth.wrapping_add(INDENT), path)?;
                }
                Some(Child::Hashed(hash, node)) => {
                    writeln!(w, "(hashed: {hash}):")?;
                    inner_dump(w, node, depth.wrapping_add(INDENT), path)?;
                }
                Some(Child::Remote(hash)) => {
                    writeln!(w, "(remote: {hash})")?;
                }
                None => {
                    writeln!(w, "(empty)")?;
                }
            }
        }

        if depth > 0 {
            writeln!(w)?;
        }

        Ok(())
    }

    inner_dump(w, node, 0, PathGuard::new(&mut Vec::new()))
}
