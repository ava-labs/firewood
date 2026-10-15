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
//! Under the Ethereum mode one stored hash is not a function of its subtree
//! alone: when an account branch has exactly one storage child, live hashing
//! stores that child's hash folded as a standalone storage-trie root, so the
//! same child content hashes differently under a one-child account than
//! under a many-child one. Where a boundary runs through an account whose
//! storage-child counts straddle one between the two sides, [`Walk::boundary`]
//! reads the local trie's shape at that position once — an account branch,
//! or, where no node sits at depth 64, the one compressed slot — and
//! re-hashes each shared depth-65 slot under the target's convention before
//! comparing; this is the one place the walk reads local branch structure
//! rather than a single probe result. The target has the same two shapes: an
//! account branch in the boundary proof, or a proof node whose partial path
//! runs through depth 64 and holds its one slot unfolded. Without the
//! re-hash identical storage would be labeled `Stale` until the sibling
//! difference was repaired.
//!
//! The local side must be canonical. A false match would be a hash
//! collision, so every mistake on the local side manufactures a hole rather
//! than hiding one; the symptom is a sync loop that fetches the same span,
//! verifies it, applies it, and finds the identical hole again. The two ways
//! to get it wrong are comparing a stored node hash when the probe lands
//! inside a compressed edge, which [`subtree_hash`] avoids by re-encoding,
//! and a non-canonical local trie, which this module assumes away and does
//! not check.

use firewood_storage::Child;
use firewood_storage::Children;
use firewood_storage::HashMode;
use firewood_storage::HashType;
use firewood_storage::HashableShunt;
use firewood_storage::HashedNodeReader;
use firewood_storage::Node;
use firewood_storage::NodeHashAlgorithm;
use firewood_storage::PathBuf;
use firewood_storage::PathComponent;
use firewood_storage::RlpError;
use firewood_storage::SharedNode;
use firewood_storage::TriePathAsPackedBytes;
use firewood_storage::TriePathFromPackedBytes;
use firewood_storage::ValueDigest;
use firewood_storage::hash_node_as_storage_trie_root_parts;
use firewood_storage::replace_list_field;

use crate::api;
use crate::merkle::Merkle;
use crate::merkle::RightBoundary;
use crate::merkle::Value;
use crate::merkle::descend::ProbeOutcome;
use crate::merkle::descend::descend_to_prefix;
use crate::merkle::descend::hashable_parts;
use crate::merkle::descend::subtree_hash;
use crate::merkle::proven_right_edge;
use crate::merkle::right_edge;
use crate::proofs::Hole;
use crate::proofs::KeySpan;
use crate::proofs::ProofError;
use crate::proofs::ProofNode;
use crate::proofs::VerifiedChangeProof;
use crate::proofs::VerifiedRangeProof;
use crate::proofs::eth::ACCOUNT_DEPTH_NIBBLES;
use crate::proofs::holes::merge_labels;
use crate::proofs::holes::open_interval;

/// Classify the key space outside the range a verified range proof was
/// applied over, against the local trie `view`.
///
/// The applied range is `[start_key, right_edge_key]` from the proof's
/// verification context; the obligation is that every key below `start_key`
/// or above `right_edge_key` is covered by exactly one emitted span, is a
/// probed point, or is a silent both-empty agreement. When the start proof
/// is empty, nothing below `start_key` is classified: the verifier does not
/// check end-proof node values below it against the key-value pairs, so the
/// proof says nothing about that space. See
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
    let mut walk = start::<H, T>(
        view,
        proof.hash_mode(),
        proof.start_proof().as_ref(),
        ctx.start_key(),
    )?;

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
/// nothing. Change-proof verification hashes the proposal's own content wherever the boundary proofs supply no sibling
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
    let mut walk = start::<H, T>(
        view,
        proof.hash_mode(),
        proof.start_proof().as_ref(),
        ctx.start_key(),
    )?;
    if let Some(edge) = ctx.right_edge_key() {
        walk.boundary(proof.end_proof().as_ref(), &nibbles(edge), Side::Above)?;
    }
    walk.finish()
}

/// Starts a walk over `view` for a proof parsed under `hash_mode` and
/// classifies the space below `start_key`. An empty start proof classifies
/// nothing there; each entry point documents why.
fn start<'a, H: HashMode, T: HashedNodeReader>(
    view: &'a T,
    hash_mode: NodeHashAlgorithm,
    start_proof: &[ProofNode],
    start_key: Option<&[u8]>,
) -> Result<Walk<'a, H, T>, api::Error> {
    reject_mode_mismatch::<H>(hash_mode)?;
    // Forces a reconstructed view's lazy hashing, which swaps its in-memory
    // children for hashed ones; without it every probe below the root errors.
    view.root_hash();

    let mut walk = Walk::<H, T>::new(view);
    if let Some(start) = start_key
        && !start_proof.is_empty()
    {
        walk.boundary(start_proof, &nibbles(start), Side::Below)?;
    }
    Ok(walk)
}

fn reject_mode_mismatch<H: HashMode>(found: NodeHashAlgorithm) -> Result<(), api::Error> {
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

/// What the local trie holds at an account's position, when it stores its
/// depth-65 child hashes under a different fold convention than the target
/// does there. See the module documentation.
struct Straddle {
    /// What the local trie holds at the account's position.
    local: LocalAccount,
    /// Whether the target stores its depth-65 child hash folded: an account
    /// branch with exactly one storage child.
    target_folded: bool,
}

/// The local trie's shape at an account's 64-nibble position.
enum LocalAccount {
    /// A branch exactly at depth 64: its child slots hold the storage
    /// children, stored folded when there is exactly one.
    Branch(SharedNode),
    /// No node at depth 64: the account has no local value and exactly one
    /// occupied child slot at depth 65 (with any number of keys under it),
    /// so one node's partial path runs through depth 64 and
    /// `partial[consumed]` is that slot's nibble. Everything under the slot
    /// is stored unfolded, since there is no account branch to fold it.
    Compressed {
        /// The node whose partial path covers the position.
        node: SharedNode,
        /// Components of the partial path above the position.
        consumed: usize,
    },
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
        self.compare_hashes(prefix, target, local);
        Ok(())
    }

    /// Label the span under `prefix` from the two sides' commitments.
    fn compare_hashes(
        &mut self,
        prefix: PathBuf,
        target: Option<HashType>,
        local: Option<HashType>,
    ) {
        let label = match (target, local) {
            (None, None) => return,
            (None, Some(_)) => Hole::Surplus,
            (Some(_), None) => Hole::Missing,
            (Some(t), Some(l)) if t == l => Hole::Synced,
            (Some(_), Some(_)) => Hole::Stale,
        };
        self.out.push(label(KeySpan::new(prefix)));
    }

    /// Compare slot `n` at `depth` on the boundary path against the target's
    /// hash for it. When `straddle` is `Some`, the target
    /// reports a hash for this slot, and the local trie holds content there
    /// — an account branch's child, or the compressed one-slot subtree
    /// running through the position — that content is re-hashed under the
    /// target's fold convention before comparing; otherwise this is an
    /// ordinary probe.
    fn compare_slot(
        &mut self,
        boundary: &[PathComponent],
        depth: usize,
        n: PathComponent,
        target: Option<HashType>,
        straddle: Option<&Straddle>,
    ) -> Result<(), api::Error> {
        let prefix = path(boundary, depth, n);
        let Some(straddle) = straddle.filter(|_| target.is_some()) else {
            return self.compare(prefix, target);
        };
        let local = match &straddle.local {
            LocalAccount::Branch(account) => {
                let Node::Branch(branch) = &**account else {
                    return self.compare(prefix, target);
                };
                let Some(child) = &branch.children[n] else {
                    return self.compare(prefix, target);
                };
                self.child_hash_under(&prefix, n, child, straddle.target_folded)?
            }
            LocalAccount::Compressed { node, consumed } => {
                let partial = node.partial_path().as_components();
                if partial.get(*consumed) != Some(&n) {
                    return self.compare(prefix, target);
                }
                // The node continues past the slot nibble; the remainder is
                // the partial path a child at the slot would carry.
                debug_assert!(*consumed < partial.len());
                let rest = partial.get(consumed.wrapping_add(1)..).unwrap_or_default();
                let (value_digest, children) = hashable_parts(node)?;
                fold::<H>(&prefix, n, rest, value_digest, children)
            }
        };
        self.compare_hashes(prefix, target, Some(local));
        Ok(())
    }

    /// Under the Ethereum mode, when `depth` is an account's position, returns
    /// the local trie's shape there and the convention under which to compare
    /// its slots — whenever the two sides store the slot hashes under
    /// different fold conventions. `target_folded` says whether the target
    /// folds: true for an account branch with one storage child, false for
    /// one with several or for a proof node whose partial path runs through
    /// the position. The local side folds when it holds an account branch
    /// with one storage child, and stores unfolded when its branch has
    /// several or no node sits at depth 64 (one occupied slot, no account
    /// value). The local node at the position is found whether the probe
    /// lands on it or on the child edge of a branch at depth 63. Returns
    /// `None` when both sides already agree on the convention, or when the
    /// local trie holds nothing at the position, so the ordinary probe is
    /// right.
    fn straddle(
        &self,
        boundary: &[PathComponent],
        depth: usize,
        target_folded: bool,
    ) -> Result<Option<Straddle>, api::Error> {
        if !H::ALGORITHM.is_ethereum() || depth != ACCOUNT_DEPTH_NIBBLES {
            return Ok(None);
        }
        let Some(prefix) = boundary.get(..depth) else {
            return Ok(None);
        };
        let (node, consumed) = match descend_to_prefix(self.view, prefix)? {
            ProbeOutcome::AtNode { node, consumed } => (node, consumed),
            // The position is a child edge of a branch at depth 63: the child
            // is the local node at the account's position, with none of its
            // partial path above it.
            ProbeOutcome::EdgeExact(_) => {
                let Some((&slot, parent)) = prefix.split_last() else {
                    return Ok(None);
                };
                let ProbeOutcome::AtNode { node: parent, .. } =
                    descend_to_prefix(self.view, parent)?
                else {
                    return Ok(None);
                };
                let Node::Branch(branch) = &*parent else {
                    return Ok(None);
                };
                let Some(child) = &branch.children[slot] else {
                    return Ok(None);
                };
                (self.load_child(child)?, 0)
            }
            ProbeOutcome::Empty | ProbeOutcome::UnhashedChild => return Ok(None),
        };
        if consumed < node.partial_path().as_components().len() {
            return Ok(target_folded.then_some(Straddle {
                local: LocalAccount::Compressed { node, consumed },
                target_folded,
            }));
        }
        let Node::Branch(branch) = &*node else {
            return Ok(None);
        };
        if (branch.children.count() == 1) == target_folded {
            return Ok(None);
        }
        Ok(Some(Straddle {
            local: LocalAccount::Branch(node),
            target_folded,
        }))
    }

    /// Compare the slot `own` at `depth`, where a terminal proof node's
    /// partial path continues past the boundary key, against the node
    /// re-encoded below that slot. A proof node whose partial path runs
    /// through an account's position holds its one slot unfolded, so the
    /// local slot is re-hashed unfolded where the local account branch
    /// folds it.
    fn compare_continuation(
        &mut self,
        node: &ProofNode,
        boundary: &[PathComponent],
        depth: usize,
        own: PathComponent,
        partial: &[PathComponent],
        common: usize,
    ) -> Result<(), api::Error> {
        let hash = reencode::<H>(
            node,
            &path(boundary, depth, own),
            partial.get(common.wrapping_add(1)..).unwrap_or_default(),
        );
        let straddle = self.straddle(boundary, depth, false)?;
        self.compare_slot(boundary, depth, own, Some(hash), straddle.as_ref())
    }

    /// The node behind a hashed child slot. A child held in memory without a
    /// hash is an error here as everywhere else in the walk.
    fn load_child(&self, child: &Child) -> Result<SharedNode, api::Error> {
        match child {
            Child::Node(_) => Err(api::Error::UnhashedView {
                reason: "a child on the account's path is held in memory without a hash",
            }),
            Child::AddressWithHash(address, _) => Ok(self.view.read_node(*address)?),
            Child::MaybePersisted(maybe_persisted, _) => {
                Ok(maybe_persisted.as_shared_node(self.view)?)
            }
        }
    }

    /// The hash of the local storage child `child` in slot `n` of the account
    /// branch, under the target's convention: folded as a standalone
    /// storage-trie root when the target account has exactly one storage
    /// child, otherwise as an ordinary child at `prefix`.
    fn child_hash_under(
        &self,
        prefix: &[PathComponent],
        n: PathComponent,
        child: &Child,
        target_folded: bool,
    ) -> Result<HashType, api::Error> {
        let node = self.load_child(child)?;
        let (value_digest, children) = hashable_parts(&node)?;
        let partial = node.partial_path().as_components();
        if target_folded {
            // `prefix` is the account prefix plus the slot nibble; a shorter
            // one would fold under the wrong account and read as `Stale`.
            debug_assert_eq!(prefix.len(), ACCOUNT_DEPTH_NIBBLES.wrapping_add(1));
            return Ok(fold::<H>(prefix, n, partial, value_digest, children));
        }
        Ok(H::to_hash(&HashableShunt::new(
            prefix,
            partial,
            value_digest,
            children,
        )))
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
                        self.compare_continuation(node, boundary, depth, own, partial, common)?;
                    }
                    if side == Side::Below && depth.is_multiple_of(2) {
                        self.point(packed(boundary, depth), None)?;
                    }
                    self.compare(path(boundary, depth, at), None)?;
                } else if side == Side::Above {
                    // Terminal: the boundary key ends inside the partial path,
                    // so every key under this node extends it and lies above.
                    self.compare_continuation(node, boundary, depth, own, partial, common)?;
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
            let straddle = self.straddle(boundary, cur, children.count() == 1)?;

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
                        self.compare_slot(boundary, cur, n, hash, straddle.as_ref())?;
                    }
                }
                return Ok(());
            };

            // (4) Sibling slots at the branch.
            for n in side.nibbles(at) {
                self.compare_slot(boundary, cur, n, children[n].clone(), straddle.as_ref())?;
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

    /// Sorts the labels and checks the overlap invariant through
    /// [`merge_labels`]. Within one boundary walk the emitted spans are
    /// pairwise incomparable and the probed points lie under none of them.
    /// Across the pieces of one classification — the two boundary walks and
    /// the authenticated-empty decompositions — overlap can arise only
    /// through a terminal span that contains the boundary key and so
    /// straddles the applied range or the decomposed interval. Such a span is
    /// authenticated empty in the target over its whole extent, and so is
    /// anything it can meet from the other pieces; a label there that
    /// asserts target content would contradict it under the same root.
    fn finish(self) -> Result<Vec<Hole>, api::Error> {
        merge_labels(self.out)
    }
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

/// The hash of a storage child in slot `n` of the account at the first 64
/// components of `prefix`, folded as a standalone storage-trie root: what a
/// one-child account stores for it.
fn fold<H: HashMode>(
    prefix: &[PathComponent],
    n: PathComponent,
    partial: &[PathComponent],
    value_digest: Option<ValueDigest<&[u8]>>,
    children: Children<Option<HashType>>,
) -> HashType {
    let account_prefix = prefix.get(..ACCOUNT_DEPTH_NIBBLES).unwrap_or_default();
    hash_node_as_storage_trie_root_parts::<H, _, _>(
        account_prefix,
        n,
        partial,
        value_digest,
        children,
    )
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
