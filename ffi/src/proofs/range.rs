// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::num::NonZeroUsize;

use firewood::{
    ProofError,
    logger::warn,
    v2::api::{self, DbView, FrozenRangeProof, HashKey},
};
use firewood_metrics::{MetricsContext, firewood_increment};

use crate::{
    BorrowedBytes, CodeIteratorHandle, CodeIteratorResult, DatabaseHandle, HashResult, Maybe,
    MultiDatabaseHandle, NextKeyRangeResult, RangeProofResult, ValueResult, VoidResult,
};

/// A key range represented by a start key and an optional end key.
pub type KeyRange = (Box<[u8]>, Option<Box<[u8]>>);

/// Arguments for creating a range proof.
#[derive(Debug)]
#[repr(C)]
pub struct CreateRangeProofArgs<'a> {
    /// The root hash of the revision to prove.
    pub root: crate::HashKey,
    /// The start key of the range to prove. If `None`, the range starts from the
    /// beginning of the keyspace.
    ///
    /// The start key must be less than the end key if both are provided.
    pub start_key: Maybe<BorrowedBytes<'a>>,
    /// The end key of the range to prove. If `None`, the range ends at the end
    /// of the keyspace or until `max_length` items have been been included in
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
pub struct VerifyRangeProofArgs<'a, 'db> {
    /// The range proof to verify. If null, the function will return
    /// [`VoidResult::NullHandlePointer`]. We need a mutable reference to
    /// update the validation context.
    pub proof: Option<&'a mut RangeProofContext<'db>>,
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
    /// This is ignored if the proof is truncated and does not cover the full,
    /// in which case the upper bound key is the final key in the key-value pairs.
    pub end_key: Maybe<BorrowedBytes<'a>>,
    /// The maximum number of key/value pairs that the proof is expected to cover.
    /// If the proof contains more items than this, it is considered invalid. If
    /// `0`, there is no limit.
    pub max_length: u32,
}

/// FFI context for for a parsed or generated range proof.
#[derive(Debug)]
pub struct RangeProofContext<'db> {
    proof: FrozenRangeProof,
    verification: Option<VerificationContext>,
    proposal_state: Option<ProposalState<'db>>,
}

#[derive(Debug)]
struct VerificationContext {
    root: HashKey,
    start_key: Option<Box<[u8]>>,
    end_key: Option<Box<[u8]>>,
    max_length: Option<NonZeroUsize>,
}

#[derive(Debug)]
enum ProposalState<'db> {
    Proposed(crate::ProposalHandle<'db>),
    Committed(Option<HashKey>),
}

impl From<FrozenRangeProof> for RangeProofContext<'_> {
    fn from(proof: FrozenRangeProof) -> Self {
        Self {
            proof,
            verification: None,
            proposal_state: None,
        }
    }
}

/// Shared range proof verification logic.
///
/// If the proof has already been verified with the same constraints, this is a no-op.
/// If verified with different constraints, an error is returned.
///
/// ## ⚠️ Unimplemented ⚠️
///
/// Currently a stub that does not perform cryptographic verification.
fn verify_range_proof(
    verification: &mut Option<VerificationContext>,
    root: HashKey,
    start_key: Option<&[u8]>,
    end_key: Option<&[u8]>,
    max_length: Option<NonZeroUsize>,
) -> Result<(), api::Error> {
    if let Some(ctx) = verification.as_ref() {
        if ctx.root == root
            && ctx.start_key.as_deref() == start_key
            && ctx.end_key.as_deref() == end_key
            && ctx.max_length == max_length
        {
            return Ok(());
        }

        return Err(api::Error::ProofError(ProofError::ValueMismatch));
    }

    warn!("range proof verification not yet implemented");
    *verification = Some(VerificationContext {
        root,
        start_key: start_key.map(Box::from),
        end_key: end_key.map(Box::from),
        max_length,
    });
    Ok(())
}

/// Shared logic for `find_next_key` across single-head and multi-head range proof contexts.
///
/// Returns the next key range to fetch, or `None` if sync is complete.
// TODO(#352): proper implementation, this naively returns the last key in
// the range, which is correct, but not ideal.
fn range_proof_find_next_key(
    proof: &FrozenRangeProof,
    verification: Option<&VerificationContext>,
    root_hash: Option<HashKey>,
) -> Result<Option<KeyRange>, api::Error> {
    let verification = verification.ok_or(api::Error::ProofError(ProofError::Unverified))?;

    let Some((last_key, _)) = proof.key_values().last() else {
        return Ok(None);
    };

    if root_hash.as_ref() == Some(&verification.root) {
        return Ok(None);
    }

    if proof.end_proof().is_empty() {
        return Ok(None);
    }

    if let Some(ref end_key) = verification.end_key
        && **last_key >= **end_key
    {
        return Ok(None);
    }

    Ok(Some((last_key.clone(), verification.end_key.clone())))
}

impl<'db> RangeProofContext<'db> {
    fn verify(
        &mut self,
        root: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<(), api::Error> {
        verify_range_proof(&mut self.verification, root, start_key, end_key, max_length)
    }

    /// Verify the range proof and prepare a proposal against the given database
    /// without committing it.
    ///
    /// If the proof has already been verified, the cached validation context
    /// allows us to skip verifying again.
    ///
    /// If a proposal has already been prepared or the previously prepared
    /// proposal has been committed, this is a no-op.
    ///
    /// Returns an error if verification fails or if a database error occurs
    /// while preparing the proposal.
    fn verify_and_propose(
        &mut self,
        db: &'db crate::DatabaseHandle,
        root: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<(), api::Error> {
        self.verify(root, start_key, end_key, max_length)?;

        if self.proposal_state.is_some() {
            return Ok(());
        }

        let proposal = db.merge_key_value_range(start_key, end_key, self.proof.key_values())?;
        self.proposal_state = Some(ProposalState::Proposed(proposal.handle));

        Ok(())
    }

    /// Verify and commit the range proof to the given database.
    ///
    /// If the proof has already been verified, the cached validation context is
    /// used to skip re-verifying it. Similarly, if a proposal has already been
    /// prepared, it is committed instead of preparing a new one.
    ///
    /// However, if the prepared proposal is no longer valid (e.g., the
    /// database has changed since it was prepared), the proposal is discared
    /// and a just-in-time proposal is created and committed.
    ///
    /// After committing or if the proof has already been committed, the
    /// resulting root hash is returned. This hash may not be equal to the
    /// target hash if the proof was not of the full range.
    fn verify_and_commit(
        &mut self,
        db: &'db crate::DatabaseHandle,
        root: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<Option<HashKey>, api::Error> {
        self.verify(root, start_key, end_key, max_length)?;

        let mut allow_rebase = true;
        let proposal_handle = match self.proposal_state.take() {
            Some(ProposalState::Committed(hash)) => {
                self.proposal_state = Some(ProposalState::Committed(hash.clone()));
                return Ok(hash);
            }
            Some(ProposalState::Proposed(proposal)) => proposal,
            None => {
                allow_rebase = false;
                db.merge_key_value_range(start_key, end_key, self.proof.key_values())?
                    .handle
            }
        };

        let result = proposal_handle.commit_proposal();
        let result = if let Err(api::Error::ParentNotLatest { .. }) = result
            && allow_rebase
        {
            // proposal is stale, try rebasing and committing again
            let proposal_handle = db
                .merge_key_value_range(start_key, end_key, self.proof.key_values())?
                .handle;
            proposal_handle.commit_proposal()
        } else {
            result
        };

        let hash = result?;
        firewood_increment!(crate::registry::MERGE_COUNT, 1);
        self.proposal_state = Some(ProposalState::Committed(hash.clone()));

        Ok(hash)
    }

    fn find_next_key(&mut self) -> Result<Option<KeyRange>, api::Error> {
        let root_hash = match self.proposal_state {
            Some(ProposalState::Committed(ref hash)) => Ok(hash.clone()),
            Some(ProposalState::Proposed(ref proposal)) => Ok(proposal.root_hash()),
            None => Err(api::Error::ProofError(ProofError::Unverified)),
        }?;
        range_proof_find_next_key(&self.proof, self.verification.as_ref(), root_hash)
    }

    fn code_hash_iter(&self) -> Result<CodeIteratorHandle<'_>, api::Error> {
        CodeIteratorHandle::new(self.proof.key_values())
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
/// - [`RangeProofResult::Ok`] containing a pointer to the `RangeProofContext` if the proof
///   was successfully created.
/// - [`RangeProofResult::Err`] containing an error message if the proof could not be created.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_range_proof(
    db: Option<&DatabaseHandle>,
    args: CreateRangeProofArgs,
) -> RangeProofResult<'static> {
    // static lifetime is safe because the returned `RangeProofResult` does not
    // retain a reference to the provided database handle.

    crate::invoke_with_handle(db, |db| {
        let view = db.get_root(args.root.into())?;
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

/// Verify a range proof against the given start and end keys and root hash. The
/// proof will be updated with the validation context if the proof is valid to
/// avoid re-verifying it during commit.
///
/// # Arguments
///
/// - `args` - The arguments for verifying the range proof.
///
/// # Returns
///
/// - [`VoidResult::NullHandlePointer`] if the caller provided a null pointer to the proof.
/// - [`VoidResult::Ok`] if the proof was successfully verified.
/// - [`VoidResult::Err`] containing an error message if the proof could not be verified.
///
/// # Thread Safety
///
/// It is not safe to call this function concurrently with the same proof context
/// nor is it safe to call any other function that accesses the same proof context
/// concurrently. The caller must ensure exclusive access to the proof context
/// for the duration of the call.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_range_proof_verify(args: VerifyRangeProofArgs) -> VoidResult {
    let VerifyRangeProofArgs {
        proof,
        root,
        start_key,
        end_key,
        max_length,
    } = args;

    crate::invoke_with_handle(proof, |ctx| {
        let start_key = start_key.into_option();
        let end_key = end_key.into_option();
        ctx.verify(
            root.into(),
            start_key.as_deref(),
            end_key.as_deref(),
            NonZeroUsize::new(max_length as usize),
        )
    })
}

/// Verify a range proof and prepare a proposal to later commit or drop. If the
/// proof has already been verified, the cached validation context will be used
/// to avoid re-verifying the proof.
///
/// # Arguments
///
/// - `db` - The database to verify the proof against.
/// - `args` - The arguments for verifying the range proof.
///
/// # Returns
///
/// - [`VoidResult::NullHandlePointer`] if the caller provided a null pointer to either
///   the database or the proof.
/// - [`VoidResult::Ok`] if the proof was successfully verified.
/// - [`VoidResult::Err`] containing an error message if the proof could not be verified
///
/// # Thread Safety
///
/// It is not safe to call this function concurrently with the same proof context
/// nor is it safe to call any other function that accesses the same proof context
/// concurrently. The caller must ensure exclusive access to the proof context
/// for the duration of the call.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_verify_range_proof<'db>(
    db: Option<&'db DatabaseHandle>,
    args: VerifyRangeProofArgs<'_, 'db>,
) -> VoidResult {
    let VerifyRangeProofArgs {
        proof,
        root,
        start_key,
        end_key,
        max_length,
    } = args;

    let handle = db.and_then(|db| proof.map(|p| (db, p)));

    crate::invoke_with_handle(handle, |(db, ctx)| {
        let start_key = start_key.into_option();
        let end_key = end_key.into_option();
        ctx.verify_and_propose(
            db,
            root.into(),
            start_key.as_deref(),
            end_key.as_deref(),
            NonZeroUsize::new(max_length as usize),
        )
    })
}

/// Verify and commit a range proof to the database.
///
/// If a proposal was previously prepared by a call to [`fwd_db_verify_range_proof`],
/// it will be committed instead of re-verifying the proof. If the proof has not yet
/// been verified, it will be verified now. If the prepared proposal is no longer
/// valid (e.g., the database has changed since it was prepared), a new proposal
/// will be created and committed.
///
/// The proof context will be updated with additional information about the committed
/// proof to allow for optimized introspection of the committed changes.
///
/// # Arguments
///
/// - `db` - The database to commit the changes to.
/// - `args` - The arguments for verifying the range proof.
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
pub extern "C" fn fwd_db_verify_and_commit_range_proof<'db>(
    db: Option<&'db DatabaseHandle>,
    args: VerifyRangeProofArgs<'_, 'db>,
) -> HashResult {
    let VerifyRangeProofArgs {
        proof,
        root,
        start_key,
        end_key,
        max_length,
    } = args;

    let handle = db.and_then(|db| proof.map(|p| (db, p)));

    crate::invoke_with_handle(handle, |(db, ctx)| {
        let start_key = start_key.into_option();
        let end_key = end_key.into_option();
        ctx.verify_and_commit(
            db,
            root.into(),
            start_key.as_deref(),
            end_key.as_deref(),
            NonZeroUsize::new(max_length as usize),
        )
    })
}

/// Returns the next key range that should be fetched after processing the
/// current set of key-value pairs in a range proof that was truncated.
///
/// Can be called multiple times to get subsequent disjoint key ranges until
/// it returns [`NextKeyRangeResult::None`], indicating there are no more keys to
/// fetch and the proof is complete.
///
/// # Arguments
///
/// - `proof` - A [`RangeProofContext`] previously returned from the create
///   methods and has been prepared into a proposal or already committed.
///
/// # Returns
///
/// - [`NextKeyRangeResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`NextKeyRangeResult::NotPrepared`] if the proof has not been prepared into
///   a proposal nor committed to the database.
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
pub extern "C" fn fwd_range_proof_find_next_key(
    proof: Option<&mut RangeProofContext>,
) -> NextKeyRangeResult {
    crate::invoke_with_handle(proof, RangeProofContext::find_next_key)
}

/// Returns an iterator over the code hashes contained in the range proof.
/// The iterator must be freed after use.
///
/// Can be called at any time after the proof has been created.
///
/// # Arguments
///
/// - `proof` - A [`RangeProofContext`] previously returned from the create
///   method.
///
/// # Returns
///
/// - [`CodeIteratorResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`CodeIteratorResult::Ok`] containing a pointer to the `CodeIteratorHandle` if successful.
/// - [`CodeIteratorResult::Err`] containing an error message if the iterator could not be created.
///
/// # Thread Safety
///
/// It is not safe to call this function concurrently with the same proof context
/// nor is it safe to call any other function that accesses the same proof context
/// concurrently. The caller must ensure exclusive access to the proof context
/// for the duration of the call.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_range_proof_code_hash_iter<'a>(
    proof: Option<&'a RangeProofContext>,
) -> CodeIteratorResult<'a> {
    crate::invoke_with_handle(proof, RangeProofContext::code_hash_iter)
}

/// Advances the code hash iterator and returns the next code hash.
///
/// # Arguments
///
/// - `iter` - A [`CodeIteratorHandle`] previously returned from the
///   `fwd_range_proof_code_hash_iter` method.
///
/// # Returns
///
/// - [`HashResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`HashResult::Some`] containing the next code hash if successful.
/// - [`HashResult::None`] if there are no more code hashes to iterate over.
/// - [`HashResult::Err`] containing an error message if the next code hash could not be retrieved.
///
/// # Thread Safety
///
/// It is not safe to call this function concurrently with the same iterator
/// nor is it safe to call any other function that accesses the same iterator
/// concurrently. The caller must ensure exclusive access to the iterator
/// for the duration of the call.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_code_hash_iter_next<'a>(
    iter: Option<&'a mut CodeIteratorHandle<'a>>,
) -> HashResult {
    crate::invoke_with_handle(iter, CodeIteratorHandle::next)
}

/// Frees the memory associated with a `CodeIteratorHandle`.
///
/// # Arguments
///
/// - `iter` - The `CodeIteratorHandle` to free, previously returned from any Rust function.
///
/// # Returns
///
/// - [`VoidResult::Ok`] if the memory was successfully freed.
/// - [`VoidResult::Err`] if the process panics while freeing the memory.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_code_hash_iter_free(iter: Option<Box<CodeIteratorHandle>>) -> VoidResult {
    crate::invoke_with_handle(iter, drop)
}

/// Serialize a `RangeProof` to bytes.
///
/// # Arguments
///
/// - `proof` - A [`RangeProofContext`] previously returned from the create
///   method. If from a parsed proof, the proof will not be verified before
///   serialization.
///
/// # Returns
///
/// - [`ValueResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`ValueResult::Some`] containing the serialized bytes if successful.
/// - [`ValueResult::Err`] if the caller provided a null pointer.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_range_proof_to_bytes(proof: Option<&RangeProofContext>) -> ValueResult {
    crate::invoke_with_handle(proof, |ctx| {
        let mut vec = Vec::new();
        ctx.proof.write_to_vec(&mut vec);
        vec
    })
}

/// Deserialize a `RangeProof` from bytes.
///
/// # Arguments
///
/// - `bytes` - The bytes to deserialize the proof from.
///
/// # Returns
///
/// - [`RangeProofResult::NullHandlePointer`] if the caller provided a null or zero-length slice.
/// - [`RangeProofResult::Ok`] containing a pointer to the `RangeProofContext` if the proof
///   was successfully parsed. This does not imply that the proof is valid, only that it is
///   well-formed. The verify method must be called to ensure the proof is cryptographically valid.
/// - [`RangeProofResult::Err`] containing an error message if the proof could not be parsed.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_range_proof_from_bytes(
    bytes: BorrowedBytes<'_>,
) -> RangeProofResult<'static> {
    crate::invoke(move || {
        FrozenRangeProof::from_slice(&bytes)
            .map_err(|err| api::Error::ProofError(ProofError::Deserialization(err)))
    })
}

/// Frees the memory associated with a `RangeProofContext`.
///
/// # Arguments
///
/// * `proof` - The `RangeProofContext` to free, previously returned from any Rust function.
///
/// # Returns
///
/// - [`VoidResult::Ok`] if the memory was successfully freed.
/// - [`VoidResult::Err`] if the process panics while freeing the memory.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_free_range_proof(proof: Option<Box<RangeProofContext>>) -> VoidResult {
    crate::invoke_with_handle(proof, drop)
}

impl crate::MetricsContextExt for RangeProofContext<'_> {
    fn metrics_context(&self) -> Option<MetricsContext> {
        None
    }
}

impl<'a> crate::MetricsContextExt for (&'a DatabaseHandle, &mut RangeProofContext<'a>) {
    fn metrics_context(&self) -> Option<MetricsContext> {
        self.0.metrics_context()
    }
}

// ========================== Multi-Head Range Proof ==========================

/// FFI context for a range proof in multi-validator mode.
///
/// Mirrors [`RangeProofContext`] but uses [`MultiDatabaseHandle`] and
/// [`crate::MultiProposalHandle`] for validator-scoped operations.
#[derive(Debug)]
pub struct MultiRangeProofContext<'db> {
    proof: FrozenRangeProof,
    verification: Option<VerificationContext>,
    proposal_state: Option<MultiProposalState<'db>>,
}

#[derive(Debug)]
enum MultiProposalState<'db> {
    Proposed(crate::MultiProposalHandle<'db>),
    Committed(Option<HashKey>),
}

impl From<FrozenRangeProof> for MultiRangeProofContext<'_> {
    fn from(proof: FrozenRangeProof) -> Self {
        Self {
            proof,
            verification: None,
            proposal_state: None,
        }
    }
}

impl<'db> MultiRangeProofContext<'db> {
    fn verify(
        &mut self,
        root: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<(), api::Error> {
        verify_range_proof(&mut self.verification, root, start_key, end_key, max_length)
    }

    /// Verify the range proof and prepare a proposal for a validator.
    fn verify_and_propose(
        &mut self,
        db: &'db MultiDatabaseHandle,
        validator_id: u64,
        root: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<(), api::Error> {
        self.verify(root, start_key, end_key, max_length)?;

        if self.proposal_state.is_some() {
            return Ok(());
        }

        let proposal =
            db.merge_key_value_range(validator_id, start_key, end_key, self.proof.key_values())?;
        self.proposal_state = Some(MultiProposalState::Proposed(proposal.handle));

        Ok(())
    }

    /// Verify and commit the range proof for a validator.
    ///
    /// Includes rebase logic: if the prepared proposal is stale
    /// (`ParentNotLatest`), a fresh proposal is created and committed.
    fn verify_and_commit(
        &mut self,
        db: &'db MultiDatabaseHandle,
        validator_id: u64,
        root: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<Option<HashKey>, api::Error> {
        self.verify(root, start_key, end_key, max_length)?;

        let mut allow_rebase = true;
        let proposal_handle = match self.proposal_state.take() {
            Some(MultiProposalState::Committed(hash)) => {
                self.proposal_state = Some(MultiProposalState::Committed(hash.clone()));
                return Ok(hash);
            }
            Some(MultiProposalState::Proposed(proposal)) => proposal,
            None => {
                allow_rebase = false;
                db.merge_key_value_range(validator_id, start_key, end_key, self.proof.key_values())?
                    .handle
            }
        };

        let result = proposal_handle.commit_proposal_with_source("proof");
        let result = if let Err(api::Error::ParentNotLatest { .. }) = result
            && allow_rebase
        {
            // proposal is stale, try rebasing and committing again
            let proposal_handle = db
                .merge_key_value_range(validator_id, start_key, end_key, self.proof.key_values())?
                .handle;
            proposal_handle.commit_proposal_with_source("proof")
        } else {
            result
        };

        let hash = result?;
        firewood_increment!(crate::registry::MERGE_COUNT, 1);
        self.proposal_state = Some(MultiProposalState::Committed(hash.clone()));

        Ok(hash)
    }

    fn find_next_key(&mut self) -> Result<Option<KeyRange>, api::Error> {
        let root_hash = match self.proposal_state {
            Some(MultiProposalState::Committed(ref hash)) => Ok(hash.clone()),
            Some(MultiProposalState::Proposed(ref proposal)) => Ok(proposal.root_hash()),
            None => Err(api::Error::ProofError(ProofError::Unverified)),
        }?;
        range_proof_find_next_key(&self.proof, self.verification.as_ref(), root_hash)
    }

    fn code_hash_iter(&self) -> Result<CodeIteratorHandle<'_>, api::Error> {
        CodeIteratorHandle::new(self.proof.key_values())
    }
}

/// Arguments for verifying a multi-head range proof.
#[derive(Debug)]
#[repr(C)]
pub struct MultiVerifyRangeProofArgs<'a, 'db> {
    /// The range proof to verify.
    pub proof: Option<&'a mut MultiRangeProofContext<'db>>,
    /// The root hash to verify against.
    pub root: crate::HashKey,
    /// The lower bound of the key range.
    pub start_key: Maybe<BorrowedBytes<'a>>,
    /// The upper bound of the key range.
    pub end_key: Maybe<BorrowedBytes<'a>>,
    /// The maximum number of key/value pairs.
    pub max_length: u32,
    /// The validator ID for the commit.
    pub validator_id: u64,
}

/// Generate a range proof from a multi-head database at the given root.
///
/// This is a read-only operation — no validator ID is needed. The returned
/// [`MultiRangeProofContext`] can later be verified and committed for a
/// specific validator via [`fwd_multi_db_verify_and_commit_range_proof`].
#[unsafe(no_mangle)]
pub extern "C" fn fwd_multi_db_range_proof(
    db: Option<&MultiDatabaseHandle>,
    args: CreateRangeProofArgs,
) -> crate::MultiRangeProofResult<'static> {
    crate::invoke_with_handle(db, |db| {
        let view = db.get_root(args.root.into())?;
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

/// Verify a range proof and prepare a proposal for a validator without
/// committing it.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_multi_db_verify_range_proof<'db>(
    db: Option<&'db MultiDatabaseHandle>,
    args: MultiVerifyRangeProofArgs<'_, 'db>,
) -> VoidResult {
    let MultiVerifyRangeProofArgs {
        proof,
        root,
        start_key,
        end_key,
        max_length,
        validator_id,
    } = args;

    let handle = db.and_then(|db| proof.map(|p| (db, p)));

    crate::invoke_with_handle(handle, |(db, ctx)| {
        let start_key = start_key.into_option();
        let end_key = end_key.into_option();
        ctx.verify_and_propose(
            db,
            validator_id,
            root.into(),
            start_key.as_deref(),
            end_key.as_deref(),
            NonZeroUsize::new(max_length as usize),
        )
    })
}

/// Verify and commit a range proof for a validator.
///
/// If a proposal was previously prepared, it will be committed. If the
/// proposal is stale, a new one is created and committed.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_multi_db_verify_and_commit_range_proof<'db>(
    db: Option<&'db MultiDatabaseHandle>,
    args: MultiVerifyRangeProofArgs<'_, 'db>,
) -> HashResult {
    let MultiVerifyRangeProofArgs {
        proof,
        root,
        start_key,
        end_key,
        max_length,
        validator_id,
    } = args;

    let handle = db.and_then(|db| proof.map(|p| (db, p)));

    crate::invoke_with_handle(handle, |(db, ctx)| {
        let start_key = start_key.into_option();
        let end_key = end_key.into_option();
        ctx.verify_and_commit(
            db,
            validator_id,
            root.into(),
            start_key.as_deref(),
            end_key.as_deref(),
            NonZeroUsize::new(max_length as usize),
        )
    })
}

/// Returns the next key range to fetch for a multi-head range proof.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_multi_range_proof_find_next_key(
    proof: Option<&mut MultiRangeProofContext>,
) -> NextKeyRangeResult {
    crate::invoke_with_handle(proof, MultiRangeProofContext::find_next_key)
}

/// Returns an iterator over the code hashes in a multi-head range proof.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_multi_range_proof_code_hash_iter<'a>(
    proof: Option<&'a MultiRangeProofContext>,
) -> crate::CodeIteratorResult<'a> {
    crate::invoke_with_handle(proof, MultiRangeProofContext::code_hash_iter)
}

/// Frees a `MultiRangeProofContext`.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_free_multi_range_proof(
    proof: Option<Box<MultiRangeProofContext>>,
) -> VoidResult {
    crate::invoke_with_handle(proof, drop)
}

impl crate::MetricsContextExt for MultiRangeProofContext<'_> {
    fn metrics_context(&self) -> Option<MetricsContext> {
        None
    }
}

impl<'a> crate::MetricsContextExt for (&'a MultiDatabaseHandle, &mut MultiRangeProofContext<'a>) {
    fn metrics_context(&self) -> Option<MetricsContext> {
        self.0.metrics_context()
    }
}
