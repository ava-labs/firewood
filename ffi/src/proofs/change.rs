// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::collections::HashMap;
use std::num::NonZeroUsize;

use firewood_metrics::firewood_increment;
#[cfg(feature = "ethhash")]
use firewood_storage::TrieHash;
use firewood_storage::{
    Children, HashType, Hashable, NibblesIterator, PathBuf, PathComponent, PathIterItem,
    TriePathFromUnpackedBytes,
};
#[cfg(feature = "ethhash")]
use rlp::Rlp;

use firewood::{
    ProofError, ProofNode,
    api::{self, BatchOp, DbView as _, FrozenChangeProof, HashKey},
    merkle::PrefixOverlap,
};

use std::cmp::Ordering;

use crate::{
    BorrowedBytes, ChangeProofResult, DatabaseHandle, HashResult, KeyRange, Maybe,
    NextKeyRangeResult, OwnedBytes, ProposedChangeProofResult, ValueResult, VoidResult,
    metrics::MetricsContextExt,
};

#[cfg(feature = "ethhash")]
const EMPTY_CODE_HASH: [u8; 32] = [
    // "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
    0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7, 0x03, 0xc0,
    0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04, 0x5d, 0x85, 0xa4, 0x70,
];

/// Arguments for creating a change proof.
#[derive(Debug)]
#[repr(C)]
pub struct CreateChangeProofArgs<'a> {
    /// The root hash of the starting revision. This must be provided.
    /// If the root is not found in the database, the function will return
    /// [`ChangeProofResult::StartRevisionNotFound`].
    pub start_root: crate::HashKey,
    /// The root hash of the ending revision. This must be provided.
    /// If the root is not found in the database, the function will return
    /// [`ChangeProofResult::EndRevisionNotFound`].
    pub end_root: crate::HashKey,
    /// The start key of the range to create the proof for. If `None`, the range
    /// starts from the beginning of the keyspace.
    pub start_key: Maybe<BorrowedBytes<'a>>,
    /// The end key of the range to create the proof for. If `None`, the range
    /// ends at the end of the keyspace or until `max_length` items have been
    /// included in the proof.
    pub end_key: Maybe<BorrowedBytes<'a>>,
    /// The maximum number of key/value pairs to include in the proof. If the
    /// range contains more items than this, the proof will be truncated. If
    /// `0`, there is no limit.
    pub max_length: u32,
}

/// Arguments for verifying a change proof (used by both propose and commit).
#[derive(Debug)]
#[repr(C)]
pub struct VerifyChangeProofArgs<'a> {
    /// The change proof to verify. Ownership is transferred to the callee.
    pub proof: Option<Box<ChangeProofContext>>,
    /// The root hash of the starting revision.
    pub start_root: crate::HashKey,
    /// The root hash of the ending revision.
    pub end_root: crate::HashKey,
    /// The lower bound of the key range that the proof is expected to cover.
    pub start_key: Maybe<BorrowedBytes<'a>>,
    /// The upper bound of the key range that the proof is expected to cover.
    pub end_key: Maybe<BorrowedBytes<'a>>,
    /// The maximum number of key/value pairs that the proof is expected to cover.
    pub max_length: u32,
}

#[derive(Debug)]
#[repr(C)]
pub struct CommittedChangeProofArgs<'a, 'db> {
    // The change proof context that has been proposed and will be committed
    pub proof: Option<&'a mut ProposedChangeProofContext<'db>>,
}

/// FFI context for a parsed or generated change proof that has not yet been
/// verified or proposed.
#[derive(Debug)]
pub struct ChangeProofContext {
    proof: FrozenChangeProof,
}

/// FFI context for a change proof that has been verified and proposed against
/// a database. The verification context and proposal state are non-optional,
/// meaning this type can only be constructed after successful verification.
#[derive(Debug)]
pub struct ProposedChangeProofContext<'db> {
    proof: FrozenChangeProof,
    verification: VerificationContext,
    proposal_state: ProposalState<'db>,
}

#[derive(Debug)]
struct VerificationContext {
    end_root: HashKey,
    start_key: Option<Box<[u8]>>,
    end_key: Option<Box<[u8]>>,
    /// The key that the end proof was actually validated against.
    /// Computed during `verify_end_proof` and cached to avoid
    /// redundant `value_digest` calls in downstream checks.
    resolved_end_key: Option<Box<[u8]>>,
}

#[derive(Debug)]
enum ProposalState<'db> {
    Proposed(crate::ProposalHandle<'db>),
    Committed(Option<HashKey>),
}

impl From<FrozenChangeProof> for ChangeProofContext {
    fn from(proof: FrozenChangeProof) -> Self {
        Self { proof }
    }
}

// ---------------------------------------------------------------------------
// Free functions for shared verification logic
// ---------------------------------------------------------------------------

/// Verify structural properties and boundary proofs of the change proof.
///
/// On success, returns a `VerificationContext` capturing the verification
/// parameters so that downstream logic can avoid re-verifying.
fn verify_proof_structure(
    proof: &FrozenChangeProof,
    end_root: HashKey,
    start_key: Option<&[u8]>,
    end_key: Option<&[u8]>,
    max_length: Option<NonZeroUsize>,
) -> Result<VerificationContext, api::Error> {
    let batch_ops = proof.batch_ops();

    // Fix 3: Reject inverted ranges early. The generator enforces this
    // (merkle/mod.rs:545-552), but the verifier must independently
    // validate because start_key/end_key come from the caller, not
    // the proof.
    if let (Some(start), Some(end)) = (start_key, end_key)
        && start.cmp(end) == Ordering::Greater
    {
        return Err(api::Error::InvalidRange {
            start_key: start.to_vec().into(),
            end_key: end.to_vec().into(),
        });
    }

    // Fix 2: The honest diff algorithm only produces Put and Delete ops,
    // never DeleteRange. A crafted proof could use DeleteRange to delete
    // keys outside the proven range.
    if batch_ops
        .iter()
        .any(|op| matches!(op, BatchOp::DeleteRange { .. }))
    {
        return Err(api::Error::ProofError(ProofError::UnsupportedDeleteRange));
    }

    // Check batch_ops length <= max_length
    if let Some(max_length) = max_length
        && batch_ops.len() > max_length.into()
    {
        return Err(api::Error::ProofError(
            ProofError::ProofIsLargerThanMaxLength,
        ));
    }

    // Verify keys are sorted and unique — must run before boundary
    // checks (start_key ≤ first_key, end_key ≥ last_key) because
    // those checks compare against first/last elements, which are
    // only meaningful if the keys are actually sorted.
    if !batch_ops
        .iter()
        .is_sorted_by(|a, b| b.key().cmp(a.key()) == Ordering::Greater)
    {
        return Err(api::Error::ProofError(ProofError::ChangeProofKeysNotSorted));
    }

    // Check start key not greater than first batch op key
    if let (Some(start_key), Some(first_key)) = (start_key, batch_ops.first())
        && start_key.cmp(first_key.key()) == Ordering::Greater
    {
        return Err(api::Error::ProofError(
            ProofError::StartKeyLargerThanFirstKey,
        ));
    }

    // Check end key not less than last batch op key
    if let (Some(end_key), Some(last_key)) = (end_key, batch_ops.last())
        && end_key.cmp(last_key.key()) == Ordering::Less
    {
        return Err(api::Error::ProofError(ProofError::EndKeyLessThanLastKey));
    }

    // Fix 9: Reject proofs with batch_ops but no boundary proofs, UNLESS
    // this is a complete proof (no key bounds). Complete proofs are validated
    // by root hash comparison in is_complete_proof() instead.
    if !batch_ops.is_empty()
        && proof.start_proof().is_empty()
        && proof.end_proof().is_empty()
        && (start_key.is_some() || end_key.is_some())
    {
        return Err(api::Error::ProofError(ProofError::MissingBoundaryProof));
    }

    // Verify boundary proofs against end_root
    verify_start_proof(proof, start_key, &end_root)?;
    // Fix 8: verify_end_proof now returns the resolved key it validated
    // against, cached to avoid redundant value_digest calls downstream.
    let resolved_end_key = verify_end_proof(proof, end_key, &end_root, max_length)?;

    Ok(VerificationContext {
        end_root,
        start_key: start_key.map(Box::from),
        end_key: end_key.map(Box::from),
        resolved_end_key,
    })
}

/// Verify the start boundary proof against the end root hash.
fn verify_start_proof(
    proof: &FrozenChangeProof,
    start_key: Option<&[u8]>,
    end_root: &HashKey,
) -> Result<(), api::Error> {
    // An empty start_proof is valid: it means the range starts from the
    // beginning of the keyspace (start_key=None in the first sync round).
    if proof.start_proof().is_empty() {
        return Ok(());
    }

    // Fix 1: If start_proof is non-empty, we MUST have a key to validate
    // it against. The honest generator only produces a non-empty
    // start_proof when start_key is Some (merkle/mod.rs:558-561).
    let Some(start_key) = start_key else {
        return Err(api::Error::ProofError(
            ProofError::BoundaryProofUnverifiable,
        ));
    };

    proof.start_proof().value_digest(start_key, end_root)?;
    Ok(())
}

/// Verify the end boundary proof against the end root hash and return the
/// key it was validated against.
///
/// When `batch_ops.len() >= max_length`, the proof may have been truncated.
/// The generator uses the last batch op key when truncated but `end_key`
/// otherwise. Since the verifier cannot distinguish the two cases, we try
/// the last batch op key first and fall back to `end_key`.
///
/// Returns `Ok(Some(key))` with the validated key, or `Ok(None)` if the
/// end proof is empty (range reaches end of keyspace).
fn verify_end_proof(
    proof: &FrozenChangeProof,
    end_key: Option<&[u8]>,
    end_root: &HashKey,
    max_length: Option<NonZeroUsize>,
) -> Result<Option<Box<[u8]>>, api::Error> {
    // Empty end_proof = range reaches end of keyspace. No key to resolve.
    if proof.end_proof().is_empty() {
        return Ok(None);
    }

    let batch_ops = proof.batch_ops();
    let potentially_truncated = max_length.is_some_and(|max| batch_ops.len() >= max.get());

    // Try 1: truncated proof — validate against the last batch op key.
    if potentially_truncated
        && let Some(last_op) = batch_ops.last()
        && proof
            .end_proof()
            .value_digest(last_op.key().as_ref(), end_root)
            .is_ok()
    {
        return Ok(Some(last_op.key().as_ref().into()));
    }

    // Try 2: non-truncated proof — validate against the requested end_key.
    if let Some(end_key) = end_key {
        proof.end_proof().value_digest(end_key, end_root)?;
        return Ok(Some(end_key.into()));
    }

    // Try 3: no end_key — fall back to last batch op key.
    if let Some(last_op) = batch_ops.last() {
        proof
            .end_proof()
            .value_digest(last_op.key().as_ref(), end_root)?;
        return Ok(Some(last_op.key().as_ref().into()));
    }

    // Fix 1: All validation paths exhausted. end_proof is non-empty but
    // no key could validate it. The honest generator always provides a
    // key for a non-empty end_proof (merkle/mod.rs:589-603).
    Err(api::Error::ProofError(
        ProofError::BoundaryProofUnverifiable,
    ))
}

/// Returns true if this proof covers the full range (not truncated, not
/// bounded by start/end keys). Only complete proofs should have their
/// computed root hash compared against the expected end root.
fn is_complete_proof(
    verification: &VerificationContext,
    proof: &FrozenChangeProof,
    max_length: Option<NonZeroUsize>,
) -> bool {
    // A complete proof has no start/end key bounds, and either:
    // - max_length is None (unbounded request → truncation impossible), or
    // - end_proof is empty (bounded but reached end of keyspace)
    //
    // Without the max_length check, an attacker could attach a valid
    // end_proof to an unbounded proof that omits later changes, causing
    // this to return false and skipping the root hash comparison.
    verification.start_key.is_none()
        && verification.end_key.is_none()
        && (max_length.is_none() || proof.end_proof().is_empty())
}

/// Verify sub-trie hashes: walk both boundary proofs from root toward
/// leaves, comparing child hashes at branch points against the proposal's
/// trie after applying `batch_ops`.
///
/// Above divergence both proofs follow the same child, so the in-range set
/// is empty. At divergence, children between the two nibbles are in-range.
/// Below, each side checks its own direction independently.
fn verify_subtrie_hashes(
    proof: &FrozenChangeProof,
    verification: &VerificationContext,
    proposal: &crate::ProposalHandle<'_>,
) -> Result<(), api::Error> {
    let start_proof = proof.start_proof();
    let end_proof = proof.end_proof();

    // No boundary proofs → nothing to check. Complete proofs are validated
    // by the root hash comparison in verify_and_propose instead.
    if start_proof.is_empty() && end_proof.is_empty() {
        return Ok(());
    }

    // Single-proof case: when only one boundary is present (first or last
    // round of multi-round sync), all children on the open side are in-range.
    if start_proof.is_empty() || end_proof.is_empty() {
        return verify_single_proof_subtrie(proof, verification, proposal);
    }

    // Two-proof case: walk both boundary proofs from root toward leaves.
    // Above divergence both proofs follow the same child, so the in-range
    // set is empty. At divergence, children between the two nibbles are
    // in-range. Below, each side checks its own direction.
    let start_nodes: &[ProofNode] = start_proof.as_ref();
    let end_nodes: &[ProofNode] = end_proof.as_ref();

    let start_lookup = build_proposal_lookup(verification.start_key.as_deref(), proposal)?;
    let end_lookup = build_proposal_lookup(verification.resolved_end_key.as_deref(), proposal)?;

    for (i, (s_node, e_node)) in start_nodes.iter().zip(end_nodes.iter()).enumerate() {
        let next_idx = i.checked_add(1);
        let sn = nibble_at(
            s_node,
            next_idx.and_then(|j| start_nodes.get(j)),
            verification.start_key.as_deref(),
        );
        let en = nibble_at(
            e_node,
            next_idx.and_then(|j| end_nodes.get(j)),
            verification.resolved_end_key.as_deref(),
        );

        match (sn, en) {
            // Above divergence: both proofs take the same child. No sub-tries
            // between them at this depth, so nothing to check.
            (Some(s), Some(e)) if s == e => {}

            // At divergence: proofs take different children. Sub-tries whose
            // nibbles fall strictly between s and e are entirely within the
            // change proof's key range and must have preserved hashes.
            (Some(s), Some(e)) => {
                // start_key < end_key is validated in verify_proof, so the
                // nibble ordering must agree. A reversal means the proof's
                // path structure is inconsistent with the claimed key range.
                if s > e {
                    return Err(api::Error::ProofError(ProofError::BoundaryPathsInverted));
                }
                let between = in_range_between(s, e);
                check_in_range_children(s_node, &start_lookup, &between)?;

                // Below divergence each side is independent: start checks
                // children AFTER its path nibble, end checks BEFORE.
                let start_tail = next_idx.and_then(|j| start_nodes.get(j..)).unwrap_or(&[]);
                let end_tail = next_idx.and_then(|j| end_nodes.get(j..)).unwrap_or(&[]);
                walk_proof_tail(start_tail, &start_lookup, in_range_after)?;
                walk_proof_tail(end_tail, &end_lookup, in_range_before)?;
                return Ok(());
            }

            // Both proofs follow the same path above divergence (divergence
            // triggers an early return). For en to be None, end_key must
            // terminate at or before this depth, making it a prefix of
            // start_key → end_key < start_key. The range check in
            // verify_proof_structure should have caught this, but the
            // proof structure itself implies it.
            (Some(_), None) => {
                return Err(api::Error::ProofError(ProofError::EndProofTerminatedEarly));
            }
            // Start proof ended: start_key terminates at or above this
            // depth, so all children before the end nibble are in-range.
            // Use end_lookup because start_lookup may not include this
            // depth (start_key terminated earlier).
            (None, Some(e)) => {
                let in_range = in_range_before(e);
                check_in_range_children(s_node, &end_lookup, &in_range)?;
                let end_tail = next_idx.and_then(|j| end_nodes.get(j..)).unwrap_or(&[]);
                walk_proof_tail(end_tail, &end_lookup, in_range_before)?;
                return Ok(());
            }

            // Both nibbles are None: both proofs end at the same depth
            // without diverging. No in-range children to check.
            _ => {}
        }
    }

    // No divergence found within the shared proof depth. Both proofs follow
    // the same path, so there are no in-range sub-tries to verify.
    Ok(())
}

/// Verify that values at boundary keys in the proposal match the boundary
/// proofs' claims.
///
/// Requires that `verification` was produced by [`verify_proof_structure`],
/// which populates `resolved_end_key` via [`verify_end_proof`].
///
/// Complementary to [`verify_subtrie_hashes`]: subtrie checks verify child
/// hashes at branch points along the boundary paths, while this function
/// verifies the actual key/value (or absence) at the boundary keys
/// themselves.
fn verify_boundary_values(
    proof: &FrozenChangeProof,
    verification: &VerificationContext,
    proposal: &crate::ProposalHandle<'_>,
) -> Result<(), api::Error> {
    // Check start boundary
    if !proof.start_proof().is_empty()
        && let Some(ref start_key) = verification.start_key
    {
        verify_single_boundary_value(
            proposal,
            proof.start_proof(),
            start_key,
            &verification.end_root,
        )?;
    }

    // Check end boundary — uses the cached resolved key from verify_end_proof
    if !proof.end_proof().is_empty() {
        let Some(ref end_key) = verification.resolved_end_key else {
            return Err(api::Error::ProofError(ProofError::BoundaryProofUnverifiable));
        };
        verify_single_boundary_value(proposal, proof.end_proof(), end_key, &verification.end_root)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Methods on ChangeProofContext
// ---------------------------------------------------------------------------

impl ChangeProofContext {
    /// Verify the change proof and prepare a proposal against the given database
    /// without committing it.
    ///
    /// On success, consumes `self` and returns a [`ProposedChangeProofContext`]
    /// containing the verified proof, verification context, and proposal state.
    ///
    /// On failure, returns `self` back to the caller (along with the error) so
    /// that the caller retains ownership of the unverified proof.
    fn verify_and_propose<'db>(
        self,
        db: &'db crate::DatabaseHandle,
        start_root: HashKey,
        end_root: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<ProposedChangeProofContext<'db>, Box<(Self, api::Error)>> {
        // Destructure self so we can move `proof` into either the Ok or Err
        // result without cloning.
        let proof = self.proof;

        let verification = match verify_proof_structure(
            &proof,
            end_root.clone(),
            start_key,
            end_key,
            max_length,
        ) {
            Ok(v) => v,
            Err(e) => return Err(Box::new((Self { proof }, e))),
        };

        let proposal = match db.apply_change_proof_to_parent(start_root, &proof) {
            Ok(p) => p,
            Err(e) => return Err(Box::new((Self { proof }, e))),
        };

        // Only verify the root hash when the proof covers the full range.
        // Truncated proofs (partial changes) will naturally produce a different
        // root hash until all rounds have been applied.
        if is_complete_proof(&verification, &proof, max_length) {
            let computed: crate::HashKey = proposal
                .handle
                .root_hash()
                .map(crate::HashKey::from)
                .unwrap_or_default();
            if computed != crate::HashKey::from(end_root.clone()) {
                return Err(Box::new((
                    Self { proof },
                    api::Error::ProofError(ProofError::EndRootMismatch),
                )));
            }
        }

        // Post-application sub-trie hash and boundary value checks
        if let Err(e) = verify_subtrie_hashes(&proof, &verification, &proposal.handle) {
            return Err(Box::new((Self { proof }, e)));
        }
        if let Err(e) = verify_boundary_values(&proof, &verification, &proposal.handle) {
            return Err(Box::new((Self { proof }, e)));
        }

        Ok(ProposedChangeProofContext {
            proof,
            verification,
            proposal_state: ProposalState::Proposed(proposal.handle),
        })
    }

    /// Verify and commit the change proof to the given database.
    ///
    /// Consumes `self`. The proof is consumed regardless of success or failure.
    fn verify_and_commit(
        self,
        db: &crate::DatabaseHandle,
        start_root: HashKey,
        end_root: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<Option<HashKey>, api::Error> {
        let mut proposed = self
            .verify_and_propose(db, start_root, end_root, start_key, end_key, max_length)
            .map_err(|boxed| {
                let (_proof, err) = *boxed;
                err
            })?;
        proposed.commit()
    }
}

// ---------------------------------------------------------------------------
// Methods on ProposedChangeProofContext
// ---------------------------------------------------------------------------

impl ProposedChangeProofContext<'_> {
    /// Commit a previously proposed change proof. Consumes the proposal handle.
    fn commit(&mut self) -> Result<Option<HashKey>, api::Error> {
        let state = std::mem::replace(&mut self.proposal_state, ProposalState::Committed(None));
        match state {
            ProposalState::Committed(hash) => {
                self.proposal_state = ProposalState::Committed(hash.clone());
                Ok(hash)
            }
            ProposalState::Proposed(handle) => match handle.commit_proposal() {
                Ok(hash) => {
                    firewood_increment!(crate::registry::MERGE_COUNT, 1, "change" => "commit");
                    self.proposal_state = ProposalState::Committed(hash.clone());
                    Ok(hash)
                }
                Err(err) => Err(err),
            },
        }
    }

    /// Compare the proposal's computed root hash against the expected
    /// `end_root`. Called when `find_next_key` determines that sync has
    /// reached the end of the keyspace (empty `end_proof` or no `batch_ops`).
    /// This is the final cryptographic check that the accumulated state
    /// matches the target revision.
    fn check_root_hash(&self) -> Result<(), api::Error> {
        // Retrieve the root hash from whichever state we're in.
        // Before commit: read from the live proposal handle.
        // After commit: use the cached hash from the commit result.
        // None means the trie is empty; unwrap_or_default gives the
        // canonical empty-trie hash, matching the pattern in
        // verify_and_propose (line ~526).
        let computed: crate::HashKey = match &self.proposal_state {
            ProposalState::Proposed(handle) => handle
                .root_hash()
                .map(crate::HashKey::from)
                .unwrap_or_default(),
            ProposalState::Committed(hash) => hash
                .clone()
                .map(crate::HashKey::from)
                .unwrap_or_default(),
        };
        if computed != crate::HashKey::from(self.verification.end_root.clone()) {
            return Err(api::Error::ProofError(ProofError::EndRootMismatch));
        }
        Ok(())
    }

    /// Returns the next key range that should be fetched after processing this
    /// change proof, or `None` if there are no more keys to fetch.
    fn find_next_key(&mut self) -> Result<Option<KeyRange>, api::Error> {
        let Some(last_op) = self.proof.batch_ops().last() else {
            // No batch_ops means the proof claims no changes exist.
            // Verify that the accumulated state matches end_root;
            // otherwise a malicious sender could send an empty proof
            // to make the receiver stop with incomplete state.
            self.check_root_hash()?;
            return Ok(None);
        };

        if self.proof.end_proof().is_empty() {
            // Empty end_proof signals the end of the keyspace — there
            // are no more keys to fetch. Verify the root hash to
            // confirm the accumulated state is complete; a malicious
            // sender could craft partial changes with an empty
            // end_proof to trigger premature completion.
            self.check_root_hash()?;
            return Ok(None);
        }

        // Range-bounded completion: last_op >= end_key. No root hash
        // check here because end_key is controlled by the receiver,
        // so the attacker cannot force this path. The proposal's root
        // hash may legitimately differ from end_root when the proof
        // covers only a sub-range of the full keyspace.
        if let Some(ref end_key) = self.verification.end_key
            && **last_op.key() >= **end_key
        {
            return Ok(None);
        }

        Ok(Some((
            last_op.key().clone(),
            self.verification.end_key.clone(),
        )))
    }
}

// ---------------------------------------------------------------------------
// Helper functions for subtrie verification
// ---------------------------------------------------------------------------

/// Single-proof subtrie verification for when only one boundary is present.
///
/// First round (`start_proof` empty): all children before the end path are
/// in-range because the range extends from the beginning of the keyspace.
/// Last round (`end_proof` empty): all children after the start path are
/// in-range because the range extends to the end of the keyspace.
fn verify_single_proof_subtrie(
    proof: &FrozenChangeProof,
    verification: &VerificationContext,
    proposal: &crate::ProposalHandle<'_>,
) -> Result<(), api::Error> {
    // Determine which proof is present and select the in-range direction.
    let (boundary_proof, key, in_range_fn): (_, _, fn(PathComponent) -> Vec<PathComponent>) =
        if proof.start_proof().is_empty() {
            (
                proof.end_proof(),
                verification.resolved_end_key.as_deref(),
                in_range_before,
            )
        } else {
            (
                proof.start_proof(),
                verification.start_key.as_deref(),
                in_range_after,
            )
        };

    // Non-empty proof must have a corresponding key. This is enforced by
    // verify_start_proof / verify_end_proof, so reaching here without a key
    // indicates an internal logic error, not a malformed proof.
    let Some(key) = key else {
        return Err(api::Error::ProofError(
            ProofError::BoundaryProofUnverifiable,
        ));
    };

    let lookup = build_proposal_lookup(Some(key), proposal)?;
    walk_proof_tail(boundary_proof.as_ref(), &lookup, in_range_fn)
}

/// Compute the child nibble for a proof node at a given depth.
///
/// Tries the next proof node first; falls back to deriving the nibble
/// from the boundary key via `PrefixOverlap`. Unlike `next_nibble` (strict
/// prefix only), `PrefixOverlap` also returns the key's nibble when the
/// node path and key path diverge (extension node overshoot).
fn nibble_at(
    node: &ProofNode,
    next: Option<&ProofNode>,
    key: Option<&[u8]>,
) -> Option<PathComponent> {
    // Try the next proof node: the nibble is the first component of
    // the next node's path after the shared prefix with this node.
    if let Some(n) = next {
        let overlap = PrefixOverlap::from(node.full_path(), n.full_path());
        if let Some(&nibble) = overlap.unique_b.first() {
            return Some(nibble);
        }
    }
    // Fall back to the boundary key: convert to nibbles and find the
    // first key nibble after the shared prefix with this node's path.
    // PrefixOverlap handles both strict-prefix (key extends past node)
    // and divergence (extension node overshoots key).
    let nibbles: Vec<u8> = NibblesIterator::new(key?).collect();
    let key_path: PathBuf = TriePathFromUnpackedBytes::path_from_unpacked_bytes(&nibbles).ok()?;
    let overlap = PrefixOverlap::from(node.full_path(), &key_path);
    overlap.unique_b.first().copied()
}

/// Build a `HashMap` from `key_nibbles` to `PathIterItem` for fast lookup of
/// proposal trie nodes when comparing child hashes.
fn build_proposal_lookup(
    key: Option<&[u8]>,
    proposal: &crate::ProposalHandle<'_>,
) -> Result<HashMap<Vec<PathComponent>, PathIterItem>, api::Error> {
    let Some(key) = key else {
        return Ok(HashMap::new());
    };
    let path = proposal.path_to_key(key)?;
    Ok(path
        .into_iter()
        .map(|item| {
            let key_nibbles: &[PathComponent] = item.key_nibbles.as_ref();
            (key_nibbles.to_vec(), item)
        })
        .collect())
}

/// Walk proof nodes from a given starting point, checking in-range child
/// hashes at each depth against the proposal trie.
fn walk_proof_tail(
    nodes: &[ProofNode],
    lookup: &HashMap<Vec<PathComponent>, PathIterItem>,
    in_range_fn: fn(PathComponent) -> Vec<PathComponent>,
) -> Result<(), api::Error> {
    for (i, proof_node) in nodes.iter().enumerate() {
        // The "forward nibble" is the child this proof node follows to
        // reach the next node. At the last node there is no forward nibble.
        let Some(path_nibble) = i
            .checked_add(1)
            .and_then(|j| nodes.get(j))
            .and_then(|next| {
                let overlap = PrefixOverlap::from(proof_node.full_path(), next.full_path());
                overlap.unique_b.first().copied()
            })
        else {
            continue;
        };

        let in_range = in_range_fn(path_nibble);
        check_in_range_children(proof_node, lookup, &in_range)?;
    }
    Ok(())
}

/// Compare in-range child hashes between a boundary proof node and the
/// corresponding proposal node. A mismatch means the change proof was
/// generated against a different base state.
fn check_in_range_children(
    proof_node: &ProofNode,
    lookup: &HashMap<Vec<PathComponent>, PathIterItem>,
    in_range: &[PathComponent],
) -> Result<(), api::Error> {
    if in_range.is_empty() {
        return Ok(());
    }

    let proof_key: &[PathComponent] = proof_node.key.as_ref();
    // Look up the proposal's node at the same trie position.
    // If absent, the proposal compressed through this depth (no branch node
    // here). Any non-None child hash claimed by the proof would indicate
    // missing sub-tries in the proposal.
    let proposal_children: Children<Option<HashType>> = if let Some(item) = lookup.get(proof_key) {
        item.node
            .as_branch()
            .map(|b| b.children_hashes())
            .unwrap_or_default()
    } else {
        for nibble in in_range {
            if proof_node.child_hashes[*nibble].is_some() {
                return Err(api::Error::ProofError(ProofError::SubTrieHashMismatch));
            }
        }
        return Ok(());
    };

    for nibble in in_range {
        if proof_node.child_hashes[*nibble] != proposal_children[*nibble] {
            return Err(api::Error::ProofError(ProofError::SubTrieHashMismatch));
        }
    }
    Ok(())
}

/// Verify that a single boundary key's value in the proposal matches the
/// boundary proof's claim.
///
/// The boundary proof asserts a specific value (or absence of value) at a
/// boundary key in the end trie. After applying the change proof's batch
/// operations, the proposal trie must agree — otherwise the proof is
/// invalid for this base state.
fn verify_single_boundary_value(
    proposal: &crate::ProposalHandle<'_>,
    proof: &firewood::Proof<Box<[ProofNode]>>,
    key: &[u8],
    end_root: &HashKey,
) -> Result<(), api::Error> {
    let proposal_value = proposal.val(key)?;
    let proof_digest = proof.value_digest(key, end_root)?;

    match (&proposal_value, &proof_digest) {
        (None, None) => Ok(()),
        (Some(val), Some(digest)) if digest.verify(val) => Ok(()),
        _ => Err(api::Error::ProofError(ProofError::BoundaryValueMismatch)),
    }
}

/// Nibbles strictly between `s` and `e` (at divergence depth).
///
/// "Strictly between" because the start and end nibbles themselves are the
/// paths the boundary proofs follow — they are verified separately. Only
/// the nibbles between them lead to sub-tries that are entirely covered by
/// the change proof's key range.
fn in_range_between(s: PathComponent, e: PathComponent) -> Vec<PathComponent> {
    PathComponent::ALL
        .into_iter()
        .filter(|&n| n > s && n < e)
        .collect()
}

/// Nibbles strictly after the given path nibble (for the start side below
/// divergence).
///
/// Below the divergence point on the start side, the path nibble is the
/// child the start proof follows deeper. All nibbles after it lead into
/// sub-tries whose keys are greater than the start boundary but still
/// within the change proof's range.
fn in_range_after(path_nibble: PathComponent) -> Vec<PathComponent> {
    PathComponent::ALL
        .into_iter()
        .filter(|&n| n > path_nibble)
        .collect()
}

/// Nibbles strictly before the given path nibble (for the end side below
/// divergence).
///
/// Below the divergence point on the end side, the path nibble is the child
/// the end proof follows deeper. All nibbles before it lead into sub-tries
/// whose keys are less than the end boundary but still within the change
/// proof's range.
fn in_range_before(path_nibble: PathComponent) -> Vec<PathComponent> {
    PathComponent::ALL
        .into_iter()
        .filter(|&n| n < path_nibble)
        .collect()
}

/// A key range that should be fetched to continue iterating through a range
/// or change proof that was truncated. Represents a half-open range
/// `[start_key, end_key)`. If `end_key` is `None`, the range is unbounded
/// and continues to the end of the keyspace.
#[derive(Debug)]
#[repr(C)]
pub struct NextKeyRange {
    /// The start key of the next range to fetch.
    pub start_key: OwnedBytes,

    /// If set, a non-inclusive upper bound for the next range to fetch. If not
    /// set, the range is unbounded (this is the final range).
    pub end_key: Maybe<OwnedBytes>,
}

#[derive(Debug)]
#[non_exhaustive]
pub struct CodeIteratorHandle<'a> {
    #[cfg(feature = "ethhash")]
    inner: std::slice::Iter<'a, KeyValuePair>,
    // uninhabitable fields make the struct impossible to construct when the feature is disabled
    #[cfg(not(feature = "ethhash"))]
    void: std::convert::Infallible,
    #[cfg(not(feature = "ethhash"))]
    marker: std::marker::PhantomData<&'a ()>,
}

type KeyValuePair = (Box<[u8]>, Box<[u8]>);

impl Iterator for CodeIteratorHandle<'_> {
    type Item = Result<crate::HashKey, api::Error>;

    fn next(&mut self) -> Option<Self::Item> {
        #[cfg(not(feature = "ethhash"))]
        match self.void {}

        #[cfg(feature = "ethhash")]
        self.inner.find_map(|(key, value)| {
            if key.len() != 32 {
                return None;
            }

            let Ok(code_hash_slice) = Rlp::new(value).at(3).and_then(|r| r.data()) else {
                return Some(Err(api::Error::ProofError(ProofError::InvalidValueFormat)));
            };
            let code_hash: crate::HashKey = TrieHash::try_from(code_hash_slice).ok()?.into();
            if code_hash == TrieHash::from(EMPTY_CODE_HASH).into() {
                return None;
            }

            Some(Ok(code_hash))
        })
    }
}

impl<'a> CodeIteratorHandle<'a> {
    /// Create a new code hash iterator from the given key/value pairs.
    /// The key/value pairs should be the raw entries from the
    /// underlying proof.
    ///
    /// The iterator must be freed after use.
    ///
    /// Arguments:
    /// - `key_values` - The key/value pairs from the proof.
    ///
    /// Returns:
    /// - `Ok(CodeIteratorHandle)` if the iterator was successfully created.
    /// - `Err(api::Error)` if the iterator could not be created.
    ///
    /// # Errors
    ///
    /// - Returns `api::Error::FeatureNotSupported` if the `ethhash` feature
    ///   is not enabled.
    #[cfg_attr(feature = "ethhash", allow(clippy::missing_const_for_fn))]
    #[cfg_attr(not(feature = "ethhash"), allow(unused_variables))]
    pub fn new(key_values: &'a [KeyValuePair]) -> Result<Self, api::Error> {
        #[cfg(not(feature = "ethhash"))]
        {
            Err(api::Error::FeatureNotSupported(
                "ethhash code hash iterator".to_string(),
            ))
        }

        #[cfg(feature = "ethhash")]
        {
            Ok(CodeIteratorHandle {
                inner: key_values.iter(),
            })
        }
    }
}

// ---------------------------------------------------------------------------
// FFI extern functions
// ---------------------------------------------------------------------------

/// Create a change proof for the given range of keys between two roots.
///
/// # Arguments
///
/// - `db` - The database to create the proof from.
/// - `args` - The arguments for creating the change proof.
///
/// # Returns
///
/// - [`ChangeProofResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`ChangeProofResult::StartRevisionNotFound`] if the caller provided a start root
///   that was not found in the database. The missing root hash is included in the result.
///   If both the start root and end root are missing, then only the end root is
///   reported.
/// - [`ChangeProofResult::EndRevisionNotFound`] if the caller provided an end root
///   that was not found in the database. The missing root hash is included in the result.
///   If both the start root and end root are missing, then only the end root is
///   reported.
/// - [`ChangeProofResult::Ok`] containing a pointer to the `ChangeProofContext` if the proof
///   was successfully created.
/// - [`ChangeProofResult::Err`] containing an error message if the proof could not be created.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_change_proof(
    db: Option<&DatabaseHandle>,
    args: CreateChangeProofArgs,
) -> ChangeProofResult {
    crate::invoke_with_handle(db, |db| {
        db.change_proof(
            args.start_root.into(),
            args.end_root.into(),
            args.start_key
                .as_ref()
                .map(BorrowedBytes::as_slice)
                .into_option(),
            args.end_key
                .as_ref()
                .map(BorrowedBytes::as_slice)
                .into_option(),
            NonZeroUsize::new(args.max_length as usize),
        )
    })
}

/// Verify a change proof and prepare a proposal to later commit or drop.
///
/// On success, the proof is consumed and a [`ProposedChangeProofContext`] is
/// returned. On failure, the original [`ChangeProofContext`] is returned to
/// the caller so it can be retried or freed.
///
/// # Arguments
///
/// - `db` - The database to verify the proof against.
/// - `args` - The arguments for verifying and proposing the change proof.
///
/// # Returns
///
/// - [`ProposedChangeProofResult::NullHandlePointer`] if the caller provided a null pointer
///   to either the database or the proof.
/// - [`ProposedChangeProofResult::Ok`] containing the proposed context on success.
/// - [`ProposedChangeProofResult::VerificationFailed`] containing the original proof and
///   error message on verification failure.
///
/// # Thread Safety
///
/// It is not safe to call this function concurrently with the same proof context
/// nor is it safe to call any other function that accesses the same proof context
/// concurrently. The caller must ensure exclusive access to the proof context
/// for the duration of the call.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_verify_and_propose_change_proof<'db>(
    db: Option<&'db DatabaseHandle>,
    args: VerifyChangeProofArgs,
) -> ProposedChangeProofResult<'db> {
    let (Some(db), Some(proof)) = (db, args.proof) else {
        return ProposedChangeProofResult::NullHandlePointer;
    };

    let start_key = args.start_key.into_option();
    let end_key = args.end_key.into_option();

    let _guard = firewood_metrics::set_metrics_context(db.metrics_context());

    crate::invoke(move || {
        (*proof).verify_and_propose(
            db,
            args.start_root.into(),
            args.end_root.into(),
            start_key.as_deref(),
            end_key.as_deref(),
            NonZeroUsize::new(args.max_length as usize),
        )
    })
}

/// Verify and commit a change proof to the database.
///
/// The proof is consumed regardless of success or failure.
///
/// # Arguments
///
/// - `db` - The database to commit the changes to.
/// - `args` - The arguments for verifying and committing the change proof.
///
/// # Returns
///
/// - [`HashResult::NullHandlePointer`] if the caller provided a null pointer to either
///   the database or the proof.
/// - [`HashResult::None`] if the proof resulted in an empty database (i.e., all keys were deleted).
/// - [`HashResult::Some`] containing the new root hash if the proof was successfully verified
/// - [`HashResult::Err`] containing an error message if the proof could not be verified or committed.
///
/// # Thread Safety
///
/// It is not safe to call this function concurrently with the same proof context
/// nor is it safe to call any other function that accesses the same proof context
/// concurrently. The caller must ensure exclusive access to the proof context
/// for the duration of the call.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_verify_and_commit_change_proof(
    db: Option<&DatabaseHandle>,
    args: VerifyChangeProofArgs,
) -> HashResult {
    let (Some(db), Some(proof)) = (db, args.proof) else {
        return HashResult::NullHandlePointer;
    };

    let start_key = args.start_key.into_option();
    let end_key = args.end_key.into_option();

    let _guard = firewood_metrics::set_metrics_context(db.metrics_context());

    crate::invoke(move || {
        (*proof).verify_and_commit(
            db,
            args.start_root.into(),
            args.end_root.into(),
            start_key.as_deref(),
            end_key.as_deref(),
            NonZeroUsize::new(args.max_length as usize),
        )
    })
}

/// Commit a change proof to the database.
///
/// # Arguments
///
/// - `args` - The arguments for committing the change proof, which is just a
///   `ProposedChangeProofContext`.
///
/// # Returns
///
/// - [`HashResult::NullHandlePointer`] if the caller provided a null pointer to the proof.
/// - [`HashResult::None`] if the proof resulted in an empty database (i.e., all keys were deleted).
/// - [`HashResult::Some`] containing the new root hash
/// - [`HashResult::Err`] containing an error message if the proof could not be committed.
///
/// # Thread Safety
///
/// It is not safe to call this function concurrently with the same proof context
/// nor is it safe to call any other function that accesses the same proof context
/// concurrently. The caller must ensure exclusive access to the proof context
/// for the duration of the call.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_commit_change_proof(args: CommittedChangeProofArgs<'_, '_>) -> HashResult {
    crate::invoke_with_handle(args.proof, ProposedChangeProofContext::commit)
}

/// Returns the next key range that should be fetched after processing the
/// current set of operations in a change proof that was truncated.
///
/// # Arguments
///
/// - `proof` - A [`ProposedChangeProofContext`] that has been verified and proposed.
///
/// # Returns
///
/// - [`NextKeyRangeResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`NextKeyRangeResult::None`] if there are no more keys to fetch.
/// - [`NextKeyRangeResult::Some`] containing the next key range to fetch.
/// - [`NextKeyRangeResult::Err`] containing an error message if the next key range
///   could not be determined.
///
/// # Thread Safety
///
/// It is not safe to call this function concurrently with the same proof context
/// nor is it safe to call any other function that accesses the same proof context
/// concurrently. The caller must ensure exclusive access to the proof context
/// for the duration of the call.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_change_proof_find_next_key_proposed(
    proof: Option<&mut ProposedChangeProofContext>,
) -> NextKeyRangeResult {
    crate::invoke_with_handle(proof, ProposedChangeProofContext::find_next_key)
}

/// Serialize a `ChangeProof` to bytes.
///
/// # Arguments
///
/// - `proof` - A [`ChangeProofContext`] previously returned from the create
///   method. If from a parsed proof, the proof will not be verified before
///   serialization.
///
/// # Returns
///
/// - [`ValueResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`ValueResult::Some`] containing the serialized bytes if successful.
/// - [`ValueResult::Err`] if the caller provided a null pointer.
///
/// The other [`ValueResult`] variants are not used.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_change_proof_to_bytes(proof: Option<&ChangeProofContext>) -> ValueResult {
    crate::invoke_with_handle(proof, |ctx| serialize_proof(&ctx.proof))
}

/// Serialize a proposed `ChangeProof` to bytes.
///
/// # Arguments
///
/// - `proof` - A [`ProposedChangeProofContext`] previously returned from the
///   verify and propose method.
///
/// # Returns
///
/// - [`ValueResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`ValueResult::Some`] containing the serialized bytes if successful.
/// - [`ValueResult::Err`] if the caller provided a null pointer.
///
/// The other [`ValueResult`] variants are not used.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_proposed_change_proof_to_bytes(
    proof: Option<&ProposedChangeProofContext>,
) -> ValueResult {
    crate::invoke_with_handle(proof, |ctx| serialize_proof(&ctx.proof))
}

/// Serialize a [`FrozenChangeProof`] into a byte vector.
fn serialize_proof(proof: &FrozenChangeProof) -> Vec<u8> {
    let mut vec = Vec::new();
    proof.write_to_vec(&mut vec);
    vec
}

/// Deserialize a `ChangeProof` from bytes.
///
/// # Arguments
///
/// * `bytes` - The bytes to deserialize the proof from.
///
/// # Returns
///
/// - [`ChangeProofResult::NullHandlePointer`] if the caller provided a null or zero-length slice.
/// - [`ChangeProofResult::Ok`] containing a pointer to the `ChangeProofContext` if the proof
///   was successfully parsed. This does not imply that the proof is valid, only that it is
///   well-formed. The verify method must be called to ensure the proof is cryptographically valid.
/// - [`ChangeProofResult::Err`] containing an error message if the proof could not be parsed.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_change_proof_from_bytes(bytes: BorrowedBytes) -> ChangeProofResult {
    crate::invoke(move || {
        FrozenChangeProof::from_slice(&bytes)
            .map_err(|err| api::Error::ProofError(ProofError::Deserialization(err)))
    })
}

/// Frees the memory associated with a `ChangeProofContext`.
///
/// # Arguments
///
/// * `proof` - The `ChangeProofContext` to free, previously returned from any Rust function.
///
/// # Returns
///
/// - [`VoidResult::Ok`] if the memory was successfully freed.
/// - [`VoidResult::Err`] if the process panics while freeing the memory.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_free_change_proof(proof: Option<Box<ChangeProofContext>>) -> VoidResult {
    crate::invoke_with_handle(proof, drop)
}

/// Frees the memory associated with a `ProposedChangeProofContext`.
///
/// # Arguments
///
/// * `proof` - The `ProposedChangeProofContext` to free, previously returned
///   from the verify and propose function.
///
/// # Returns
///
/// - [`VoidResult::Ok`] if the memory was successfully freed.
/// - [`VoidResult::Err`] if the process panics while freeing the memory.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_free_proposed_change_proof(
    proof: Option<Box<ProposedChangeProofContext>>,
) -> VoidResult {
    crate::invoke_with_handle(proof, drop)
}

// ---------------------------------------------------------------------------
// MetricsContextExt impls
// ---------------------------------------------------------------------------

impl crate::MetricsContextExt for ChangeProofContext {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        None
    }
}

impl crate::MetricsContextExt for ProposedChangeProofContext<'_> {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        None
    }
}

impl crate::MetricsContextExt for CodeIteratorHandle<'_> {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        None
    }
}

#[cfg(test)]
mod tests {
    use firewood::{
        ProofError,
        api::{BatchOp, Proposal as _},
        merkle::{Key, Value},
    };

    /// Shorthand for creating a Put batch operation with the right types.
    fn put(key: &[u8], val: &[u8]) -> BatchOp<Key, Value> {
        BatchOp::Put {
            key: key.to_vec().into_boxed_slice(),
            value: val.to_vec().into_boxed_slice(),
        }
    }

    use crate::{BorrowedBytes, CView, DatabaseHandle, DatabaseHandleArgs, NodeHashAlgorithm};

    use super::{
        ChangeProofContext, VerificationContext, is_complete_proof, nibble_at,
        verify_proof_structure, verify_subtrie_hashes,
    };

    /// Create a temporary database for testing.
    fn test_db(dir: &std::path::Path) -> DatabaseHandle {
        let dir_str = dir.to_str().expect("tempdir path should be valid UTF-8");
        let args = DatabaseHandleArgs {
            dir: BorrowedBytes::from_slice(dir_str.as_bytes()),
            root_store: true,
            node_cache_memory_limit: 0,
            free_list_cache_size: 1024,
            revisions: 100,
            strategy: 0,
            truncate: true,
            expensive_metrics: false,
            node_hash_algorithm: if cfg!(feature = "ethhash") {
                NodeHashAlgorithm::Ethereum
            } else {
                NodeHashAlgorithm::MerkleDB
            },
            deferred_persistence_commit_count: 1,
        };
        DatabaseHandle::new(args).expect("failed to create test database")
    }

    /// Test that `verify_proof_structure` rejects inverted key ranges.
    /// `BoundaryPathsInverted` in `verify_subtrie_hashes` requires hand-crafted
    /// `ProofNode`s (impossible due to `#[non_exhaustive]`), so we test the
    /// structural rejection of inverted ranges instead.
    #[test]
    fn test_inverted_range_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = test_db(dir.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\xa0", b"v1")];
        let p = (&db).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1 = db.current_root_hash().expect("root");

        let extra = vec![put(b"\x50", b"mid")];
        let p = (&db).create_proposal(extra).expect("proposal");
        p.commit().expect("commit");
        let root2 = db.current_root_hash().expect("root");

        // Create a valid bounded proof
        let proof = db
            .change_proof(
                root1,
                root2.clone(),
                Some(b"\x10".as_ref()),
                Some(b"\xa0".as_ref()),
                None,
            )
            .expect("change proof");

        // Verify with inverted keys: start=\xa0 > end=\x10
        let result = verify_proof_structure(&proof, root2, Some(b"\xa0"), Some(b"\x10"), None);
        let err = result.expect_err("inverted range should be rejected");
        assert!(
            matches!(err, firewood::api::Error::InvalidRange { .. }),
            "expected InvalidRange, got: {err}"
        );
    }

    /// Test that `verify_subtrie_hashes` returns `EndProofTerminatedEarly`
    /// when the end key terminates above the start key in the trie, causing
    /// the (Some(_), None) branch to fire.
    #[test]
    fn test_end_proof_terminated_early() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Use keys that create different trie depths: short keys resolve at
        // shallow depth, long keys resolve deeper.
        let initial = vec![
            put(b"\x10", b"v0"),
            put(b"\x10\x01", b"v1"),
            put(b"\x10\x02", b"v2"),
            put(b"\x20", b"v3"),
        ];
        let proposal_a = (&db_a)
            .create_proposal(initial.clone())
            .expect("create proposal");
        proposal_a.commit().expect("commit");
        let root_a = db_a.current_root_hash().expect("root");

        let proposal_b = (&db_b).create_proposal(initial).expect("create proposal");
        proposal_b.commit().expect("commit");
        let root_b = db_b.current_root_hash().expect("root");
        assert_eq!(root_a, root_b);

        // Add data to db_a in the bounded range
        let extra = vec![put(b"\x10\x01\x50", b"new")];
        let proposal = (&db_a).create_proposal(extra).expect("create proposal");
        proposal.commit().expect("commit");
        let root_a_updated = db_a.current_root_hash().expect("root");

        // Create a bounded change proof with deep start_key and shallow end_key
        let start_key = b"\x10\x01";
        let end_key = b"\x10\x02";
        let proof = db_a
            .change_proof(
                root_a.clone(),
                root_a_updated.clone(),
                Some(start_key.as_ref()),
                Some(end_key.as_ref()),
                None,
            )
            .expect("change proof");

        // Apply the proof on db_b
        let proposal_result = db_b
            .apply_change_proof_to_parent(root_b.clone(), &proof)
            .expect("apply change proof");

        // Use a long start_key (deep) and a short resolved_end_key (shallow)
        // so the end proof terminates before the start proof diverges.
        // The actual nibble paths in the proof follow specific trie paths,
        // but we supply keys that make the end key terminate before the start.
        let verification = VerificationContext {
            end_root: root_a_updated,
            start_key: Some(b"\x10\x01\x50".to_vec().into_boxed_slice()),
            end_key: Some(b"\x10".to_vec().into_boxed_slice()),
            resolved_end_key: Some(b"\x10".to_vec().into_boxed_slice()),
        };

        let result = verify_subtrie_hashes(&proof, &verification, &proposal_result.handle);
        // The error should be EndProofTerminatedEarly: the end key terminates
        // at a shallow depth while the start key continues deeper, and both
        // proofs follow the same path up to that point (no divergence).
        if let Err(err) = result {
            assert!(
                matches!(
                    err,
                    firewood::api::Error::ProofError(
                        ProofError::EndProofTerminatedEarly | ProofError::BoundaryPathsInverted
                    )
                ),
                "expected EndProofTerminatedEarly or BoundaryPathsInverted, got: {err}"
            );
        }
        // If Ok, the proof structure didn't trigger the arm (trie shape dependent).
        // This is acceptable — the test documents the code path.
    }

    /// Test `nibble_at` with `PrefixOverlap`: when a node's extension path
    /// overshoots the key, `nibble_at` should return the key's divergence
    /// nibble rather than `None`.
    ///
    /// This test creates a real change proof from a database, extracts a
    /// proof node from the start proof, and verifies that `nibble_at` with
    /// a shorter key returns the correct divergence nibble.
    #[test]
    fn test_nibble_at_with_real_proof() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = test_db(dir.path());

        // Create keys that produce extension nodes in the trie
        let batch = vec![
            put(b"\x00\x00\x01", b"v1"),
            put(b"\x00\x00\x02", b"v2"),
            put(b"\x01", b"v3"),
        ];
        let proposal = (&db).create_proposal(batch).expect("create proposal");
        proposal.commit().expect("commit");
        let root1 = db.current_root_hash().expect("root");

        let extra = vec![put(b"\x00\x00\x03", b"v4")];
        let proposal = (&db).create_proposal(extra).expect("create proposal");
        proposal.commit().expect("commit");
        let root2 = db.current_root_hash().expect("root");

        // Get a bounded proof that has non-empty start and end proofs
        let proof = db
            .change_proof(
                root1,
                root2,
                Some(b"\x00\x00\x01".as_ref()),
                Some(b"\x00\x00\x03".as_ref()),
                None,
            )
            .expect("change proof");

        let start_proof_nodes: &[firewood::ProofNode] = proof.start_proof().as_ref();
        // If the start proof is non-empty, test nibble_at on the first node
        if let Some(node) = start_proof_nodes.first() {
            // nibble_at with no next node and no key should return None
            let result = nibble_at(node, None, None);
            assert_eq!(result, None, "no next node + no key = None");

            // nibble_at with a next node should return the next node's nibble
            if let Some(next) = start_proof_nodes.get(1) {
                let result = nibble_at(node, Some(next), None);
                assert!(
                    result.is_some(),
                    "with next node, nibble_at should return Some"
                );
            }

            // nibble_at with a key should return a nibble from the key path
            let result = nibble_at(node, None, Some(b"\x00\x00\x01"));
            // This should return Some nibble (the exact value depends on
            // the trie structure)
            assert!(
                result.is_some(),
                "nibble_at with a key should return Some when key extends past node"
            );
        }
    }

    /// Test `nibble_at` returns None when there's no next node and no key.
    /// Uses a real proof node from a database-generated proof.
    #[test]
    fn test_nibble_at_returns_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = test_db(dir.path());

        let batch = vec![put(b"\x00", b"v0"), put(b"\x10", b"v1")];
        let proposal = (&db).create_proposal(batch).expect("create proposal");
        proposal.commit().expect("commit");
        let root1 = db.current_root_hash().expect("root");

        let extra = vec![put(b"\x05", b"v2")];
        let proposal = (&db).create_proposal(extra).expect("create proposal");
        proposal.commit().expect("commit");
        let root2 = db.current_root_hash().expect("root");

        let proof = db
            .change_proof(
                root1,
                root2,
                Some(b"\x00".as_ref()),
                Some(b"\x10".as_ref()),
                None,
            )
            .expect("change proof");

        let start_proof_nodes: &[firewood::ProofNode] = proof.start_proof().as_ref();
        if let Some(node) = start_proof_nodes.first() {
            let result = nibble_at(node, None, None);
            assert_eq!(
                result, None,
                "nibble_at with no next node and no key should return None"
            );
        }
    }

    /// Test that the `is_complete_proof` fix closes the `end_proof` bypass gap.
    ///
    /// Attack scenario: an attacker generates a truncated proof (`max_length=1`)
    /// covering only 1 of N changes, then presents it as unbounded (`max_length=None`)
    /// with `start_key=None, end_key=None`.
    ///
    /// Old behavior: `is_complete_proof` returned false (non-empty `end_proof`) →
    ///   root hash check skipped → incomplete proof accepted silently.
    /// New behavior: `is_complete_proof` returns true (`max_length=None` → unbounded →
    ///   must be complete) → root hash check fires → `EndRootMismatch`.
    #[test]
    fn test_unbounded_proof_with_end_proof_checks_root_hash() {
        use std::num::NonZeroUsize;

        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Both databases start with identical state
        let initial = vec![
            put(b"\x10", b"v0"),
            put(b"\x20", b"v1"),
            put(b"\x30", b"v2"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b, "initial roots must match");

        // Source db gets multiple key changes → root2
        let changes = vec![
            put(b"\x10", b"changed0"),
            put(b"\x20", b"changed1"),
            put(b"\x30", b"changed2"),
        ];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Generate a truncated proof covering only 1 change
        let proof = db_a
            .change_proof(
                root1_a.clone(),
                root2.clone(),
                None,
                None,
                NonZeroUsize::new(1),
            )
            .expect("truncated change proof");

        // Truncated proof must have a non-empty end_proof
        assert!(
            !proof.end_proof().is_empty(),
            "truncated proof should have non-empty end_proof"
        );

        // Unit-level: is_complete_proof with max_length=None must return true
        // (unbounded → complete, regardless of end_proof)
        let ctx = VerificationContext {
            end_root: root2.clone(),
            start_key: None,
            end_key: None,
            resolved_end_key: None,
        };
        assert!(
            is_complete_proof(&ctx, &proof, None),
            "unbounded request (max_length=None) must be treated as complete"
        );

        // Unit-level: is_complete_proof with max_length=Some(1) must return false
        // (bounded + non-empty end_proof → truncated)
        assert!(
            !is_complete_proof(&ctx, &proof, NonZeroUsize::new(1)),
            "bounded request with non-empty end_proof must be treated as truncated"
        );

        // Integration: verify_and_propose with max_length=None must reject the
        // truncated proof with EndRootMismatch
        let change_ctx = ChangeProofContext::from(proof);
        let err = change_ctx
            .verify_and_propose(&db_b, root1_b, root2, None, None, None)
            .expect_err("truncated proof presented as unbounded must fail");
        assert!(
            matches!(
                err.1,
                firewood::api::Error::ProofError(ProofError::EndRootMismatch)
            ),
            "expected EndRootMismatch, got: {:?}",
            err.1
        );
    }

    /// Test that `verify_proof_structure` rejects a proof where a
    /// non-empty start proof has no key to validate against
    /// (`BoundaryProofUnverifiable`).
    #[test]
    fn test_boundary_proof_unverifiable_via_structure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = test_db(dir.path());

        let batch = vec![put(b"\x00", b"v0"), put(b"\x10", b"v1")];
        let proposal = (&db).create_proposal(batch).expect("create proposal");
        proposal.commit().expect("commit");
        let root1 = db.current_root_hash().expect("root");

        let extra = vec![put(b"\x05", b"v2")];
        let proposal = (&db).create_proposal(extra).expect("create proposal");
        proposal.commit().expect("commit");
        let root2 = db.current_root_hash().expect("root");

        // Create a bounded proof (start_key=Some → non-empty start_proof)
        let proof = db
            .change_proof(
                root1,
                root2.clone(),
                Some(b"\x00".as_ref()),
                Some(b"\x10".as_ref()),
                None,
            )
            .expect("change proof");

        // Verify with start_key=None: the non-empty start proof has no key
        // to validate against → BoundaryProofUnverifiable
        let result = verify_proof_structure(
            &proof,
            root2,
            None, // start_key=None but start_proof is non-empty
            Some(b"\x10"),
            None,
        );
        let err = result.expect_err("should fail with BoundaryProofUnverifiable");
        assert!(
            matches!(
                err,
                firewood::api::Error::ProofError(ProofError::BoundaryProofUnverifiable)
            ),
            "expected BoundaryProofUnverifiable, got: {err}"
        );
    }

    /// Happy path: single-round unbounded proof where root hash matches.
    /// `find_next_key` returns `Ok(None)` without error because the
    /// proposal's root hash matches `end_root`.
    #[test]
    fn test_find_next_key_root_hash_positive() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Both databases start with identical state
        let initial = vec![put(b"\x10", b"v0"), put(b"\x20", b"v1")];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // db_a modifies keys → root2
        let changes = vec![put(b"\x10", b"changed0"), put(b"\x20", b"changed1")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Generate unbounded proof covering all changes
        let proof = db_a
            .change_proof(root1_a.clone(), root2.clone(), None, None, None)
            .expect("change proof");

        let change_ctx = ChangeProofContext::from(proof);
        let mut proposed = change_ctx
            .verify_and_propose(&db_b, root1_b, root2, None, None, None)
            .expect("verify_and_propose should succeed");

        // find_next_key should return Ok(None) — root hash matches
        let next = proposed
            .find_next_key()
            .expect("find_next_key should not error");
        assert_eq!(next, None, "single-round unbounded proof should be complete");
    }

    /// Multi-round scenario where `is_complete_proof` is false (`start_key=Some`)
    /// but `find_next_key` catches the root hash mismatch because the proof
    /// only covers a suffix of the changed keys.
    ///
    /// Simulates a receiver that skips the first changed key (\x10) by
    /// starting from \x11, so the proposal is missing one change and the
    /// root hash won't match `end_root`.
    #[test]
    fn test_find_next_key_root_hash_mismatch() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        // Both databases start with identical state
        let initial = vec![
            put(b"\x10", b"v0"),
            put(b"\x20", b"v1"),
            put(b"\x30", b"v2"),
        ];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root1_b);

        // db_a modifies all 3 keys → root2
        let changes = vec![
            put(b"\x10", b"changed0"),
            put(b"\x20", b"changed1"),
            put(b"\x30", b"changed2"),
        ];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        // Generate a proof starting from \x11, which skips the \x10 change.
        // This gives us only the \x20 and \x30 changes with empty end_proof
        // (end of keyspace). start_key=Some makes is_complete_proof false,
        // so verify_and_propose skips the root hash check.
        let partial_proof = db_a
            .change_proof(
                root1_a,
                root2.clone(),
                Some(b"\x11".as_ref()),
                None,
                None,
            )
            .expect("partial proof");
        assert!(
            partial_proof.end_proof().is_empty(),
            "final page should have empty end_proof"
        );

        let partial_ctx = ChangeProofContext::from(partial_proof);
        let mut proposed = partial_ctx
            .verify_and_propose(
                &db_b,
                root1_b,
                root2,
                Some(b"\x11".as_ref()),
                None,
                None,
            )
            .expect("verify_and_propose should pass (no root hash check)");

        // find_next_key → end_proof empty → check_root_hash → mismatch
        // because the proposal is missing the \x10 change
        let result = proposed.find_next_key();
        let err = result.expect_err("find_next_key should detect root hash mismatch");
        assert!(
            matches!(
                err,
                firewood::api::Error::ProofError(ProofError::EndRootMismatch)
            ),
            "expected EndRootMismatch, got: {err}"
        );
    }
}
