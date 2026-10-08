// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Post-merge hole detection: classify the key space a verified proof did
//! not cover.
//!
//! A verified range or change proof proves a contiguous key range, and the
//! two boundary proofs that bracket it carry, at every level, the hash of
//! every sealed sibling subtree the range did not include. Each such hash
//! commits to the exact target content of a known span of key space. The
//! walk here compares every one of them against the canonical hash of the
//! local trie's content under the same span ([`subtree_hash`]) and labels the
//! span with a [`Hole`]: already correct, provably surplus, repairable from
//! a value in hand, or genuinely needing a fetch. Where the two are equal the
//! span is silently in agreement when both are empty and [`Hole::Synced`]
//! otherwise.
//!
//! The walk transcribes the boundary proof from its root node down. At each
//! node the sibling slots on the outside of the boundary key are compared
//! (source 4), as are the branches implied empty by a compressed partial
//! path (source 1); below the start key, the byte keys that are proper
//! prefixes of the boundary key are probed as points, with the value a
//! proof node carries where there is one (sources 2 and 3). The terminal
//! node ends the walk with one of four cases: a divergence inside its
//! partial path, the boundary key ending inside its partial path, the
//! boundary key ending at the node itself, or an absent child slot. Below a
//! start key whose start proof is empty nothing is emitted: the verifier
//! adopts end-proof node values below that key without checking them against
//! the key-value pairs, so a root match says nothing about the space there.
//!
//! Coverage: in a completed walk the emitted spans are pairwise incomparable,
//! the probed points lie on the boundary key's prefix chain and inside no
//! emitted span, and every key strictly on the walked side of the boundary
//! key lies in exactly one emitted span or equals exactly one probed point.
//! Soundness: unless a hash collision is extractable, every `Synced` span and
//! every silent agreement has equal content on both sides, and every
//! `Missing`, `Stale`, `Surplus`, or `PointSurplus` label marks a genuine
//! difference, with `Surplus` spans empty in the target. Nothing finer is
//! decidable from the proof: which keys inside a `Stale` span differ is
//! invisible behind the sibling hash.
//!
//! The local side must be canonical. A false match would be a hash
//! collision, so every mistake on the local side manufactures a hole rather
//! than hiding one; the symptom is a sync loop that fetches the same span,
//! verifies it, applies it, and finds the identical hole again. The two ways
//! to get it wrong are comparing a stored node hash when the probe lands
//! inside a compressed edge, which [`subtree_hash`] avoids by re-encoding,
//! and a non-canonical local trie, which this module assumes away and does
//! not check.

use firewood_storage::{
    Children, HashMode, HashType, HashableShunt, HashedNodeReader, PathBuf, PathComponent,
    RlpError, TriePathAsPackedBytes, TriePathFromPackedBytes, ValueDigest, replace_list_field,
};

use crate::api;
use crate::merkle::descend::subtree_hash;
use crate::merkle::{Merkle, RightBoundary, Value, proven_right_edge, right_edge};
use crate::proofs::eth::ACCOUNT_DEPTH_NIBBLES;
use crate::proofs::holes::open_interval;
use crate::proofs::{
    Hole, KeySpan, ProofError, ProofNode, VerifiedChangeProof, VerifiedRangeProof,
};

/// Classify the key space outside the range a verified range proof was
/// applied over, against the local trie `view`.
///
/// The applied range is `[start_key, right_edge_key]` from the proof's
/// verification context; the obligation is that every key below `start_key`
/// or above `right_edge_key` is covered by exactly one emitted span, is a
/// probed point, or is a silent both-empty agreement. See
/// [`CommittedView::find_holes_after_range_proof`] for the caller-facing
/// contract.
///
/// # Errors
///
/// [`ProofError::HashModeMismatch`] when the proof was parsed under a hash
/// mode other than `H`; [`api::Error::UnhashedView`] and I/O errors from the
/// local probes; [`api::Error::InternalError`] when the walk's own
/// invariants fail — the recomputed right edge disagrees with the context,
/// a verified proof ends without a terminal, or an authenticated-empty span
/// overlaps a label that is not a deletion.
///
/// [`CommittedView::find_holes_after_range_proof`]: crate::db::CommittedView::find_holes_after_range_proof
pub(crate) fn find_holes_after_range_proof<H: HashMode, T: HashedNodeReader>(
    verified: &VerifiedRangeProof,
    view: &T,
) -> Result<Vec<Hole>, api::Error> {
    let proof = verified.proof();
    let ctx = verified.verification();
    reject_mode_mismatch::<H>(proof.hash_mode())?;
    // Forces a reconstructed view's lazy hashing, which swaps its in-memory
    // children for hashed ones; without it every probe below the root errors.
    view.root_hash();

    let mut walk = Walk::<H, T>::new(view);

    // An empty start proof with a requested start key verifies, but it does
    // not authenticate the space below that key: the verifier adopts
    // end-proof node values there without checking them against the
    // key-value pairs, and an empty start proof anchors nothing. Nothing is
    // emitted below the start key, as on the change-proof path.
    if let Some(start) = ctx.start_key()
        && !proof.start_proof().is_empty()
    {
        walk.boundary(proof.start_proof().as_ref(), &nibbles(start), Side::Below)?;
    }

    let end_proof: &[ProofNode] = proof.end_proof().as_ref();
    let last_kv = proof.key_values().last().map(|(k, _)| k.as_ref());
    let boundary = right_edge(end_proof, last_kv, ctx.end_key());
    // The context's right edge is `proven_right_edge` of this same boundary.
    // The owning type makes a swapped context unconstructible, so a
    // disagreement here means the recomputation and the verifier drifted.
    if proven_right_edge(&boundary, last_kv, ctx.end_key()).as_deref() != ctx.right_edge_key() {
        return Err(internal(
            "recomputed right edge disagrees with the verification context",
        ));
    }
    match boundary {
        // Unbounded above: the applied range runs to the end of the key
        // space and there is nothing outside it on this side.
        RightBoundary::InRange(None) => {}
        RightBoundary::InRange(Some(end)) => {
            walk.boundary(end_proof, &nibbles(end), Side::Above)?;
        }
        RightBoundary::OutOfRange(terminal_key) => {
            // The end proof's terminal is a real target key above the proven
            // edge. The merge stops at the edge, the walk below classifies
            // keys above the terminal, and the terminal itself is a point
            // whose target value the proof carries. The open interval between
            // edge and terminal is proven empty in the target.
            let Some(edge) = ctx.right_edge_key() else {
                return Err(internal(
                    "an out-of-range terminal with no proven right edge",
                ));
            };
            let Some(terminal) = end_proof.last() else {
                return Err(internal("an out-of-range terminal from an empty end proof"));
            };
            let terminal_nibbles = nibbles(&terminal_key);
            walk.empty_interval(Some(&nibbles(edge)), &terminal_nibbles)?;
            walk.point(
                Box::from(terminal_key.as_ref()),
                terminal.value_digest.as_ref(),
            )?;
            walk.boundary(end_proof, &terminal_nibbles, Side::Above)?;
        }
    }

    walk.finish()
}

/// Classify the key space outside the range a verified change proof was
/// applied over, against the local trie `view`.
///
/// Both boundaries come from the verification context: `start_key` below
/// and `right_edge_key` above, the latter being the key the verifier
/// anchored the end proof at. An empty boundary proof on either side emits
/// nothing. Unlike a range proof, change-proof verification hashes the
/// proposal's own content wherever the boundary proofs supply no sibling
/// hash, so a root match there says nothing about whether that content
/// equals the target's or is absent from it; neither `Surplus` nor `Synced`
/// is justified, so that boundary emits nothing.
///
/// # Errors
///
/// As [`find_holes_after_range_proof`].
pub(crate) fn find_holes_after_change_proof<H: HashMode, T: HashedNodeReader>(
    verified: &VerifiedChangeProof,
    view: &T,
) -> Result<Vec<Hole>, api::Error> {
    let proof = verified.proof();
    let ctx = verified.verification();
    reject_mode_mismatch::<H>(proof.hash_mode())?;
    view.root_hash();

    let mut walk = Walk::<H, T>::new(view);
    if let Some(start) = ctx.start_key()
        && !proof.start_proof().is_empty()
    {
        walk.boundary(proof.start_proof().as_ref(), &nibbles(start), Side::Below)?;
    }
    if let Some(edge) = ctx.right_edge_key() {
        walk.boundary(proof.end_proof().as_ref(), &nibbles(edge), Side::Above)?;
    }
    walk.finish()
}

fn reject_mode_mismatch<H: HashMode>(
    found: firewood_storage::NodeHashAlgorithm,
) -> Result<(), api::Error> {
    if found == H::ALGORITHM {
        Ok(())
    } else {
        Err(api::Error::ProofError(ProofError::HashModeMismatch {
            expected: H::ALGORITHM,
            found,
        }))
    }
}

fn internal(message: &'static str) -> api::Error {
    api::Error::InternalError(format!("hole detection: {message}").into())
}

fn nibbles(key: &[u8]) -> Vec<PathComponent> {
    Vec::<PathComponent>::path_from_packed_bytes(key)
}

/// Which side of the boundary key a walk classifies.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    /// Keys sorting before the boundary key: the left boundary proof.
    Below,
    /// Keys sorting after the boundary key: the right boundary proof.
    Above,
}

impl Side {
    /// The child slots on this side of `at`.
    fn nibbles(self, at: PathComponent) -> impl Iterator<Item = PathComponent> {
        PathComponent::ALL.into_iter().filter(move |n| match self {
            Side::Below => *n < at,
            Side::Above => *n > at,
        })
    }

    fn contains(self, at: PathComponent, n: PathComponent) -> bool {
        match self {
            Side::Below => n < at,
            Side::Above => n > at,
        }
    }
}

/// Accumulates labels for one or two boundary walks over the same view.
struct Walk<'a, H, T> {
    view: &'a T,
    out: Vec<Hole>,
    hash_mode: std::marker::PhantomData<H>,
}

impl<'a, H: HashMode, T: HashedNodeReader> Walk<'a, H, T> {
    const fn new(view: &'a T) -> Self {
        Self {
            view,
            out: Vec::new(),
            hash_mode: std::marker::PhantomData,
        }
    }

    /// Compare the target's commitment to the span under `prefix` against
    /// the local trie's and label the difference, if any.
    fn compare(&mut self, prefix: PathBuf, target: Option<HashType>) -> Result<(), api::Error> {
        let local = subtree_hash::<H, T>(self.view, &prefix)?;
        let label = match (target, local) {
            (None, None) => return Ok(()),
            (None, Some(_)) => Hole::Surplus,
            (Some(_), None) => Hole::Missing,
            (Some(t), Some(l)) if t == l => Hole::Synced,
            (Some(_), Some(_)) => Hole::Stale,
        };
        self.out.push(label(KeySpan::new(prefix)));
        Ok(())
    }

    /// Reconcile the exact byte key `key` between the target's value digest,
    /// if the target has one, and the local value.
    ///
    /// Under the Ethereum mode an account value (a 32-byte key) is compared
    /// with its `storageRoot` field masked; see [`values_agree`].
    fn point(
        &mut self,
        key: Box<[u8]>,
        target: Option<&ValueDigest<Value>>,
    ) -> Result<(), api::Error> {
        let local = Merkle::from(self.view).get_value(&key)?;
        match (target, local) {
            (None, None) => {}
            (None, Some(_)) => self.out.push(Hole::PointSurplus { key }),
            (Some(digest), local) => {
                if local.is_some_and(|value| values_agree::<H>(&key, digest, &value)) {
                    return Ok(());
                }
                // A digest that is only a hash cannot be written locally; the
                // key must be fetched.
                self.out.push(match digest.value() {
                    Some(value) => Hole::PointFix {
                        key,
                        value: Box::from(value),
                    },
                    None => Hole::PointStale { key },
                });
            }
        }
        Ok(())
    }

    /// Label the open interval between two nibble paths as authenticated
    /// empty in the target: every span and point there compares against no
    /// target content.
    fn empty_interval(
        &mut self,
        lower_exclusive: Option<&[PathComponent]>,
        upper_exclusive: &[PathComponent],
    ) -> Result<(), api::Error> {
        let interval = open_interval(lower_exclusive, upper_exclusive);
        for span in interval.spans {
            self.compare(span.into_prefix(), None)?;
        }
        for point in interval.points {
            self.point(point, None)?;
        }
        Ok(())
    }

    /// Walk a verified boundary proof for `boundary` from the root down and
    /// label every span and point on `side` of it.
    fn boundary(
        &mut self,
        proof_nodes: &[ProofNode],
        boundary: &[PathComponent],
        side: Side,
    ) -> Result<(), api::Error> {
        let mut cur = 0usize;
        for node in proof_nodes {
            // The node's hash commits to its full key in both modes, but to
            // `partial_len` only under the Ethereum mode, so the partial path
            // is taken from the key at the walk's own position rather than
            // from `partial_path()`.
            let Some(partial) = boundary
                .get(..cur)
                .and_then(|consumed| node.key.as_slice().strip_prefix(consumed))
            else {
                return Err(internal(
                    "a boundary proof node's key leaves the boundary path",
                ));
            };
            let rest = boundary.get(cur..).unwrap_or_default();
            let common = partial.iter().zip(rest).take_while(|(a, b)| a == b).count();
            // Every position formed below is bounded by `boundary.len()` or
            // `partial.len()`, so the additions cannot overflow.
            debug_assert!(cur <= boundary.len() && common <= rest.len());

            // (1) Branches off the compressed segment: the target has no node
            // there, so each is authenticated empty.
            for (offset, &at) in rest.iter().enumerate().take(common) {
                for n in side.nibbles(at) {
                    self.compare(path(boundary, cur.wrapping_add(offset), n), None)?;
                }
            }
            // (2) Byte keys that are proper prefixes of the boundary key and
            // end inside the segment: no node, so no target value.
            if side == Side::Below {
                for offset in 0..common {
                    let depth = cur.wrapping_add(offset);
                    if depth.is_multiple_of(2) {
                        self.point(packed(boundary, depth), None)?;
                    }
                }
            }

            if common < partial.len() {
                let Some(&own) = partial.get(common) else {
                    return Err(internal("partial path shorter than its common prefix"));
                };
                let depth = cur.wrapping_add(common);
                if let Some(&at) = rest.get(common) {
                    // Terminal: the partial path diverges from the boundary
                    // key at `depth`. Every sibling slot there is empty except
                    // the one this node continues down, which the proof holds
                    // in full; the boundary key's own continuation is empty.
                    for n in side.nibbles(at).filter(|n| *n != own) {
                        self.compare(path(boundary, depth, n), None)?;
                    }
                    if side.contains(at, own) {
                        let prefix = path(boundary, depth, own);
                        let hash = reencode::<H>(
                            node,
                            &prefix,
                            partial.get(common.wrapping_add(1)..).unwrap_or_default(),
                        );
                        self.compare(prefix, Some(hash))?;
                    }
                    if side == Side::Below && depth.is_multiple_of(2) {
                        self.point(packed(boundary, depth), None)?;
                    }
                    self.compare(path(boundary, depth, at), None)?;
                } else if side == Side::Above {
                    // Terminal: the boundary key ends inside the partial path,
                    // so every key under this node extends it and lies above.
                    let prefix = path(boundary, depth, own);
                    let hash = reencode::<H>(
                        node,
                        &prefix,
                        partial.get(common.wrapping_add(1)..).unwrap_or_default(),
                    );
                    self.compare(prefix, Some(hash))?;
                    for n in PathComponent::ALL.into_iter().filter(|n| *n != own) {
                        self.compare(path(boundary, depth, n), None)?;
                    }
                }
                // Below, a boundary key ending inside the partial path has
                // nothing on its side: every key under this node lies above.
                return Ok(());
            }

            cur = cur.wrapping_add(common);
            let children: Children<Option<HashType>> = (&node.child_hashes).into();

            // (3) The node's own key, a proper prefix of the boundary key: the
            // proof carries its value in full.
            if side == Side::Below && cur < boundary.len() && cur.is_multiple_of(2) {
                self.point(packed(boundary, cur), node.value_digest.as_ref())?;
            }

            // Terminal: the boundary key ends at this node. Every child
            // extends it and lies above.
            let Some(&at) = boundary.get(cur) else {
                if side == Side::Above {
                    for (n, hash) in children {
                        self.compare(path(boundary, cur, n), hash)?;
                    }
                }
                return Ok(());
            };

            // (4) Sibling slots at the branch.
            for n in side.nibbles(at) {
                self.compare(path(boundary, cur, n), children[n].clone())?;
            }

            // Terminal: the boundary key's slot is absent, so the whole prefix
            // is empty in the target.
            if children[at].is_none() {
                self.compare(path(boundary, cur, at), None)?;
                return Ok(());
            }
            cur = cur.wrapping_add(1);
        }
        if proof_nodes.is_empty() {
            return Ok(());
        }
        Err(internal(
            "a verified boundary proof ended without a terminal node",
        ))
    }

    fn finish(self) -> Result<Vec<Hole>, api::Error> {
        merge_labels(self.out)
    }
}

/// Sort the labels by their lowest key and check the overlap invariant: two
/// labels whose extents overlap must both be deletions. Within one boundary
/// walk the emitted spans are pairwise incomparable and the probed points lie
/// under none of them. Across the pieces of one classification — the two
/// boundary walks and the authenticated-empty decompositions — overlap can
/// arise only through a terminal span that contains the boundary key and so
/// straddles the applied range or the decomposed interval. Such a span is
/// authenticated empty in the target over its whole extent, and so is
/// anything it can meet from the other pieces; a label there that asserts
/// target content would contradict it under the same root.
///
/// # Errors
///
/// [`api::Error::InternalError`] on a disallowed overlap.
pub(crate) fn merge_labels(mut out: Vec<Hole>) -> Result<Vec<Hole>, api::Error> {
    out.sort_by_cached_key(lower_bound);
    let mut rest = out.iter();
    while let Some(a) = rest.next() {
        for b in rest.clone() {
            if overlaps(a, b) && !(is_deletion(a) && is_deletion(b)) {
                return Err(internal("a span overlaps a label that is not a deletion"));
            }
        }
    }
    Ok(out)
}

/// Whether the target's value digest for `key` agrees with the local value.
///
/// Under the Ethereum mode an account value — the value at a 32-byte key —
/// embeds the root of the account's storage trie as its third RLP field, and
/// hashing derives that field from the storage children rather than trusting
/// the stored bytes (see [`subtree_hash`]). The field is
/// therefore not independently repairable: writing the target's value
/// locally stores the local storage root again, so a [`Hole::PointFix`] on a
/// `storageRoot`-only difference would be re-emitted by every later walk.
/// The storage difference it stands for is already reported by the span
/// labels at and below the account. Account values are compared with that
/// field masked, so a point label at an account key means some other field
/// differs: nonce, balance, code hash, or a trailing field a client appends.
/// The mask also makes the comparison indifferent to databases written
/// before `firewood-v1-hfix`, whose stored account values hold a stale
/// `storageRoot`.
///
/// A value that is not well-formed account RLP is compared as-is.
fn values_agree<H: HashMode>(key: &[u8], target: &ValueDigest<Value>, local: &[u8]) -> bool {
    if H::ALGORITHM.is_ethereum()
        && key.len() == ACCOUNT_DEPTH_NIBBLES / 2
        && let Some(target) = target.value()
        && let (Ok(target), Ok(local)) = (mask_storage_root(target), mask_storage_root(local))
    {
        return target == local;
    }
    target.verify(local)
}

/// `value` with its `storageRoot` field (the third item of an account's RLP
/// list) replaced by zeros.
fn mask_storage_root(value: &[u8]) -> Result<Box<[u8]>, RlpError> {
    replace_list_field(value, 2, &[0; 32])
}

/// The hash a proof node would have with `prefix` as its parent prefix and
/// `partial` as its partial path: the target's commitment to a subtree the
/// walk holds in full rather than as a sibling hash.
fn reencode<H: HashMode>(
    node: &ProofNode,
    prefix: &[PathComponent],
    partial: &[PathComponent],
) -> HashType {
    H::to_hash(&HashableShunt::new(
        prefix,
        partial,
        node.value_digest.as_ref().map(ValueDigest::as_ref),
        (&node.child_hashes).into(),
    ))
}

/// The first `depth` components of `boundary` followed by `last`.
fn path(boundary: &[PathComponent], depth: usize, last: PathComponent) -> PathBuf {
    let mut prefix: PathBuf = boundary.iter().take(depth).copied().collect();
    prefix.push(last);
    prefix
}

/// The first `depth` components of `boundary` as a byte key; `depth` is even.
fn packed(boundary: &[PathComponent], depth: usize) -> Box<[u8]> {
    let prefix: PathBuf = boundary.iter().take(depth).copied().collect();
    prefix.as_packed_bytes().collect()
}

/// A label's extent as a nibble path: a span's prefix, or a point's key.
fn extent(hole: &Hole) -> Vec<PathComponent> {
    match hole {
        Hole::Missing(span) | Hole::Stale(span) | Hole::Surplus(span) | Hole::Synced(span) => {
            span.prefix().to_vec()
        }
        Hole::PointFix { key, .. } | Hole::PointStale { key } | Hole::PointSurplus { key } => {
            nibbles(key)
        }
    }
}

fn lower_bound(hole: &Hole) -> Box<[u8]> {
    match hole {
        Hole::Missing(span) | Hole::Stale(span) | Hole::Surplus(span) | Hole::Synced(span) => {
            span.as_key_range().0
        }
        Hole::PointFix { key, .. } | Hole::PointStale { key } | Hole::PointSurplus { key } => {
            key.clone()
        }
    }
}

const fn is_point(hole: &Hole) -> bool {
    matches!(
        hole,
        Hole::PointFix { .. } | Hole::PointStale { .. } | Hole::PointSurplus { .. }
    )
}

const fn is_deletion(hole: &Hole) -> bool {
    matches!(hole, Hole::Surplus(_) | Hole::PointSurplus { .. })
}

/// Whether two labels' extents overlap: one span's prefix is a prefix of the
/// other's, a point lies under a span, or two points name the same key.
fn overlaps(a: &Hole, b: &Hole) -> bool {
    let (ea, eb) = (extent(a), extent(b));
    match (is_point(a), is_point(b)) {
        (true, true) => ea == eb,
        (true, false) => ea.starts_with(&eb),
        (false, true) => eb.starts_with(&ea),
        (false, false) => ea.starts_with(&eb) || eb.starts_with(&ea),
    }
}
