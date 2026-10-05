// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::num::NonZeroUsize;
use std::sync::Arc;

use firewood::{
    KeyRange, ProofError, VerifiedRangeProof,
    api::{self, DbView, FrozenRangeProof, HashKey},
};
use firewood_metrics::{MetricsContext, firewood_counter};

use super::proposal_state::ProposalState;
use crate::{
    BorrowedBytes, CodeIteratorHandle, CodeIteratorResult, DatabaseHandle, HashResult, Maybe,
    NextKeyRangesResult, RangeProofResult, ValueResult, VerifiedRangeProofResult, VoidResult,
};

/// Arguments for creating a range proof.
#[derive(Debug)]
#[repr(C)]
pub struct CreateRangeProofArgs<'a> {
    /// The root hash of the revision to prove.
    pub root: crate::HashKey,
    /// The start key of the range to prove. If `None`, the range starts from the
    /// beginning of the keyspace.
    ///
    /// The start key must not be greater than the end key if both are provided.
    pub start_key: Maybe<BorrowedBytes<'a>>,
    /// The end key of the range to prove. If `None`, the range ends at the end
    /// of the keyspace or until `max_length` items have been included in
    /// the proof.
    ///
    /// If provided, end key is inclusive if not truncated. Otherwise, the end
    /// key will be the final key in the returned key-value pairs.
    pub end_key: Maybe<BorrowedBytes<'a>>,
    /// The maximum number of key/value pairs to include in the proof. If the
    /// range contains more items than this, the proof will be truncated. If
    /// `0`, there is no limit.
    pub max_length: u32,
}

/// Arguments for verifying a range proof.
#[derive(Debug)]
#[repr(C)]
pub struct VerifyRangeProofArgs<'a> {
    /// The root hash to verify the proof against. This must match the calculated
    /// hash of the root of the proof.
    pub root: crate::HashKey,
    /// The lower bound of the key range that the proof is expected to cover. If
    /// `None`, the proof is expected to cover from the start of the keyspace.
    ///
    /// Must be present if the range proof contains a lower bound proof and must
    /// be absent if the range proof does not contain a lower bound proof.
    pub start_key: Maybe<BorrowedBytes<'a>>,
    /// The upper bound of the key range that the proof is expected to cover. If
    /// `None`, the proof is expected to cover to the end of the keyspace.
    ///
    /// This is a ceiling, not an assertion: a proof carrying a key above this
    /// bound is invalid, but a truncated proof may prove less and anchor at
    /// its own right edge instead.
    pub end_key: Maybe<BorrowedBytes<'a>>,
    /// The maximum number of key/value pairs that the proof is expected to cover.
    /// If the proof contains more items than this, it is considered invalid. If
    /// `0`, there is no limit.
    pub max_length: u32,
}

/// FFI context for a parsed or generated range proof.
///
/// Holds no database reference and borrows nothing, so it is portable:
/// serialize it with [`fwd_range_proof_to_bytes`] or check it against a
/// database with [`fwd_db_verify_range_proof`], which produces a
/// [`VerifiedRangeProofContext`] and leaves this context usable.
#[derive(Debug)]
pub struct RangeProofContext {
    proof: Arc<FrozenRangeProof>,
}

impl From<FrozenRangeProof> for RangeProofContext {
    fn from(proof: FrozenRangeProof) -> Self {
        Self {
            proof: Arc::new(proof),
        }
    }
}

impl RangeProofContext {
    /// Verify the proof against `db`'s hash mode and the given constraints,
    /// then prepare the proposal that applies it to `db`'s latest revision.
    ///
    /// Verification runs under `db`'s hash mode; a proof whose header
    /// advertises a different mode is rejected with
    /// [`ProofError::HashModeMismatch`] before any hashing happens. A failed
    /// verification builds no proposal.
    fn verify<'db>(
        &self,
        db: &'db DatabaseHandle,
        root: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<VerifiedRangeProofContext<'db>, api::Error> {
        let verified = VerifiedRangeProof::verify(
            Arc::clone(&self.proof),
            root,
            start_key,
            end_key,
            db.node_hash_algorithm(),
            max_length,
        )?;
        let mut context = VerifiedRangeProofContext {
            db,
            verified,
            proposal_state: ProposalState::Pending,
        };
        context.proposal_state = ProposalState::Proposed(context.propose()?);
        Ok(context)
    }
}

/// FFI context for a range proof verified against one database.
///
/// Owns the proposal that applies the proof, so it borrows the database for
/// its whole life: the Go wrapper holds a keep-alive lease for it. The proof
/// and its verification context travel together as a [`VerifiedRangeProof`].
#[derive(Debug)]
pub struct VerifiedRangeProofContext<'db> {
    db: &'db DatabaseHandle,
    verified: VerifiedRangeProof,
    proposal_state: ProposalState<'db>,
}

impl<'db> VerifiedRangeProofContext<'db> {
    /// Build a fresh proposal applying the proof over the proven range.
    fn propose(&self) -> Result<crate::ProposalHandle<'db>, api::Error> {
        let verification = self.verified.verification();
        Ok(self
            .db
            .merge_key_value_range(
                verification.start_key(),
                verification.right_edge_key(),
                self.verified.proof().key_values(),
            )?
            .handle)
    }

    /// Commit the proof to the database and return the resulting root hash.
    ///
    /// A prepared proposal is committed as-is. If the database advanced since
    /// verification (`ParentNotLatest`), the proposal is rebuilt from the proof
    /// and committed once more. A proposal consumed by a failed commit leaves
    /// the state `Pending`, and the next call rebuilds it. After a successful
    /// commit the root is cached and returned by every later call.
    ///
    /// The returned hash may differ from the verification target when the
    /// proof covered less than the full keyspace.
    fn commit(&mut self) -> Result<Option<HashKey>, api::Error> {
        let (proposal, allow_rebuild) =
            match std::mem::replace(&mut self.proposal_state, ProposalState::Pending) {
                ProposalState::Committed(hash) => {
                    self.proposal_state = ProposalState::Committed(hash.clone());
                    return Ok(hash);
                }
                ProposalState::Proposed(proposal) => (proposal, true),
                ProposalState::Pending => (self.propose()?, false),
            };

        let result = match proposal.commit_proposal() {
            Err(api::Error::ParentNotLatest { .. }) if allow_rebuild => {
                self.propose()?.commit_proposal()
            }
            result => result,
        };

        let hash = result?;
        firewood_counter!(MERGE_COUNT).increment(1);
        self.proposal_state = ProposalState::Committed(hash.clone());
        Ok(hash)
    }

    /// The key ranges still to fetch after applying this proof, sorted
    /// ascending by start key; empty when nothing remains.
    ///
    /// Each range is `[start_key, end_key]`: `start_key` is the smallest key
    /// above the last one known to be synchronized, and `end_key` is the
    /// inclusive bound the proof was verified with, which need not match the
    /// bound it was created with.
    ///
    /// When the receiver's root already equals the verification target —
    /// read from the committed root, the prepared proposal, or the database's
    /// current root — nothing remains and the list is empty.
    ///
    /// The proof carries hashes for the state outside its key-value pairs, so
    /// the list could be tightened beyond the last key; it holds at most that
    /// one range (`find_next_key_after_range_proof`, tracked by #352).
    fn next_key_ranges(&self) -> Result<Vec<KeyRange>, api::Error> {
        let root_hash: Option<HashKey> = match &self.proposal_state {
            ProposalState::Committed(hash) => hash.clone(),
            ProposalState::Proposed(proposal) => proposal.root_hash(),
            ProposalState::Pending => self.db.current_root_hash(),
        };
        let verification = self.verified.verification();
        if root_hash.as_ref() == Some(verification.root()) {
            return Ok(Vec::new());
        }

        Ok(
            firewood::find_next_key_after_range_proof(self.verified.proof(), verification)?
                .into_iter()
                .collect(),
        )
    }

    fn code_hash_iter(&self) -> Result<CodeIteratorHandle<'_>, api::Error> {
        let proof = self.verified.proof();
        CodeIteratorHandle::from_key_values(proof.hash_mode(), proof.key_values())
    }
}

/// Generate a range proof for the given range of keys for the latest revision.
///
/// # Arguments
///
/// - `db` - The database to create the proof from.
/// - `args` - The arguments for creating the range proof.
///
/// # Returns
///
/// - [`RangeProofResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`RangeProofResult::RevisionNotFound`] if the caller provided a root that was
///   not found in the database. The missing root hash is included in the result.
/// - [`RangeProofResult::EmptyTrie`] if the revision has no root.
/// - [`RangeProofResult::Ok`] containing a pointer to the `RangeProofContext` if the proof
///   was successfully created.
/// - [`RangeProofResult::Err`] containing an error message if the proof could not be created.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_range_proof(
    db: Option<&DatabaseHandle>,
    args: CreateRangeProofArgs,
) -> RangeProofResult {
    crate::invoke_with_handle(db, |db| {
        let view = db.view(args.root.into())?;
        view.range_proof(
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

/// Deserialize a range proof from bytes for use with `db`.
///
/// The database supplies the hash mode the proof must be encoded with; a
/// proof whose header advertises another mode is rejected here rather than
/// at verification.
///
/// # Arguments
///
/// - `db` - The database the proof will be verified against.
/// - `bytes` - The bytes to deserialize the proof from.
///
/// # Returns
///
/// - [`RangeProofResult::NullHandlePointer`] if the caller provided a null database pointer.
/// - [`RangeProofResult::Ok`] containing a pointer to the `RangeProofContext` if the proof
///   was successfully parsed. This does not imply that the proof is valid, only that it is
///   well-formed and uses `db`'s hash mode. Call [`fwd_db_verify_range_proof`] to check it.
/// - [`RangeProofResult::Err`] containing an error message if the proof could not be parsed
///   or its hash mode does not match `db`'s.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_range_proof_from_bytes(
    db: Option<&DatabaseHandle>,
    bytes: BorrowedBytes<'_>,
) -> RangeProofResult {
    crate::invoke_with_handle(db, move |db| {
        let proof = FrozenRangeProof::from_slice(&bytes)
            .map_err(|err| api::Error::ProofError(ProofError::Deserialization(err)))?;
        let expected = db.node_hash_algorithm();
        if proof.hash_mode() != expected {
            return Err(api::Error::ProofError(ProofError::HashModeMismatch {
                expected,
                found: proof.hash_mode(),
            }));
        }
        Ok(proof)
    })
}

/// Verify a range proof against `db` and prepare the proposal that applies it.
///
/// The input proof is borrowed, not consumed: it stays usable for
/// serialization or for verifying again with other constraints.
///
/// # Arguments
///
/// - `db` - The database to verify the proof against.
/// - `proof` - The parsed or generated proof.
/// - `args` - The constraints to verify the proof under.
///
/// # Returns
///
/// - [`VerifiedRangeProofResult::NullHandlePointer`] if the caller provided a null pointer
///   to either the database or the proof.
/// - [`VerifiedRangeProofResult::Ok`] containing a pointer to the
///   [`VerifiedRangeProofContext`] if the proof was successfully verified.
/// - [`VerifiedRangeProofResult::Err`] containing an error message if the proof could not be
///   verified or the proposal could not be prepared.
///
/// # Thread Safety
///
/// The proof context is read, not mutated; concurrent calls on the same proof
/// are safe provided none of them frees it.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_verify_range_proof<'db>(
    db: Option<&'db DatabaseHandle>,
    proof: Option<&RangeProofContext>,
    args: VerifyRangeProofArgs<'_>,
) -> VerifiedRangeProofResult<'db> {
    let VerifyRangeProofArgs {
        root,
        start_key,
        end_key,
        max_length,
    } = args;

    let handle = db.zip(proof);

    crate::invoke_with_handle(handle, |(db, ctx)| {
        let start_key = start_key.into_option();
        let end_key = end_key.into_option();
        ctx.verify(
            db,
            root.into(),
            start_key.as_deref(),
            end_key.as_deref(),
            NonZeroUsize::new(max_length as usize),
        )
    })
}

/// Commit a verified range proof to its database.
///
/// A prepared proposal is committed as-is; one made stale by a later commit
/// is rebuilt from the proof; after success the root is cached and a second
/// call returns it without touching the database. The context stays usable
/// afterwards for [`fwd_verified_range_proof_next_key_ranges`] and
/// [`fwd_verified_range_proof_code_hash_iter`].
///
/// # Returns
///
/// - [`HashResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`HashResult::None`] if the trie has no root hash (merkledb mode only;
///   ethhash always returns a root hash, even for an empty trie).
/// - [`HashResult::Some`] containing the new root hash.
/// - [`HashResult::Err`] containing an error message if the commit failed.
///
/// # Thread Safety
///
/// Takes the context mutably: the caller must ensure exclusive access for the
/// duration of the call.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_verified_range_proof_commit(
    proof: Option<&mut VerifiedRangeProofContext<'_>>,
) -> HashResult {
    crate::invoke_with_handle(proof, VerifiedRangeProofContext::commit)
}

/// Returns the key ranges still to fetch after this proof, sorted ascending.
///
/// # Returns
///
/// - [`NextKeyRangesResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`NextKeyRangesResult::Ok`] containing the ranges; empty when nothing remains. The
///   caller frees it with [`fwd_free_next_key_ranges`].
/// - [`NextKeyRangesResult::Err`] containing an error message.
///
/// [`fwd_free_next_key_ranges`]: crate::fwd_free_next_key_ranges
#[unsafe(no_mangle)]
pub extern "C" fn fwd_verified_range_proof_next_key_ranges(
    proof: Option<&VerifiedRangeProofContext<'_>>,
) -> NextKeyRangesResult {
    crate::invoke_with_handle(proof, VerifiedRangeProofContext::next_key_ranges)
}

/// Returns an iterator over the code hashes contained in a verified range
/// proof. The iterator borrows the proof and must be freed with
/// [`fwd_code_hash_iter_free`] before the proof is.
///
/// # Returns
///
/// - [`CodeIteratorResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`CodeIteratorResult::Ok`] containing a pointer to the `CodeIteratorHandle` if successful.
/// - [`CodeIteratorResult::Err`] containing an error message if the iterator could not be
///   created, including when the proof is not an Ethereum-mode proof.
///
/// [`fwd_code_hash_iter_free`]: crate::fwd_code_hash_iter_free
#[unsafe(no_mangle)]
pub extern "C" fn fwd_verified_range_proof_code_hash_iter<'a>(
    proof: Option<&'a VerifiedRangeProofContext<'_>>,
) -> CodeIteratorResult<'a> {
    crate::invoke_with_handle(proof, VerifiedRangeProofContext::code_hash_iter)
}

/// Serialize a range proof to bytes.
///
/// # Returns
///
/// - [`ValueResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`ValueResult::Some`] containing the serialized bytes if successful.
/// - [`ValueResult::Err`] containing an error message if serialization failed.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_range_proof_to_bytes(proof: Option<&RangeProofContext>) -> ValueResult {
    crate::invoke_with_handle(proof, |ctx| -> Result<Option<Box<[u8]>>, api::Error> {
        let mut vec = Vec::new();
        ctx.proof
            .write_to_vec(&mut vec)
            .map_err(api::Error::ProofError)?;
        Ok(Some(vec.into_boxed_slice()))
    })
}

/// Frees the memory associated with a `RangeProofContext`.
///
/// # Returns
///
/// - [`VoidResult::Ok`] if the memory was successfully freed.
/// - [`VoidResult::Err`] if the process panics while freeing the memory.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_free_range_proof(proof: Option<Box<RangeProofContext>>) -> VoidResult {
    crate::invoke_with_handle(proof, drop)
}

/// Frees the memory associated with a `VerifiedRangeProofContext`, dropping
/// its proposal if it was not committed.
///
/// # Returns
///
/// - [`VoidResult::Ok`] if the memory was successfully freed.
/// - [`VoidResult::Err`] if the process panics while freeing the memory.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_free_verified_range_proof(
    proof: Option<Box<VerifiedRangeProofContext<'_>>>,
) -> VoidResult {
    crate::invoke_with_handle(proof, drop)
}

impl crate::MetricsContextExt for RangeProofContext {
    fn metrics_context(&self) -> Option<MetricsContext> {
        None
    }
}

impl crate::MetricsContextExt for VerifiedRangeProofContext<'_> {
    fn metrics_context(&self) -> Option<MetricsContext> {
        self.db.metrics_context()
    }
}

impl crate::MetricsContextExt for (&DatabaseHandle, &RangeProofContext) {
    fn metrics_context(&self) -> Option<MetricsContext> {
        self.0.metrics_context()
    }
}
