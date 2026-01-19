// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::{cmp::Ordering, num::NonZeroUsize};

use firewood::{
    ProofError,
    logger::warn,
    v2::api::{self, FrozenChangeProof},
};
use firewood_storage::{TrieHash, firewood_counter};

use crate::{
    BorrowedBytes, CResult, ChangeProofResult, DatabaseHandle, HashKey, HashResult, Maybe,
    NextKeyRangeResult, OwnedBytes, ValueResult, VoidResult,
};

/// Arguments for creating a change proof.
#[derive(Debug)]
#[repr(C)]
pub struct CreateChangeProofArgs<'a> {
    /// The root hash of the starting revision. This must be provided.
    /// If the root is not found in the database, the function will return
    /// [`ChangeProofResult::RevisionNotFound`].
    pub start_root: HashKey,
    /// The root hash of the ending revision. This must be provided.
    /// If the root is not found in the database, the function will return
    /// [`ChangeProofResult::RevisionNotFound`].
    pub end_root: HashKey,
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

/// Arguments for verifying a change proof.
#[derive(Debug)]
#[repr(C)]
pub struct VerifyChangeProofArgs<'a> {
    /// The change proof to verify. If null, the function will return
    /// [`VoidResult::NullHandlePointer`]. We need a mutable reference to
    /// update the validation context.
    pub proof: Option<&'a mut ChangeProofContext>,
    /// The root hash of the starting revision. This must match the starting
    /// root of the proof.
    pub start_root: HashKey,
    /// The root hash of the ending revision. This must match the ending root of
    /// the proof.
    pub end_root: HashKey,
    /// The lower bound of the key range that the proof is expected to cover. If
    /// `None`, the proof is expected to cover from the start of the keyspace.
    pub start_key: Maybe<BorrowedBytes<'a>>,
    /// The upper bound of the key range that the proof is expected to cover. If
    /// `None`, the proof is expected to cover to the end of the keyspace.
    pub end_key: Maybe<BorrowedBytes<'a>>,
    /// The maximum number of key/value pairs that the proof is expected to cover.
    /// If the proof contains more items than this, it is considered invalid. If
    /// `0`, there is no limit.
    pub max_length: u32,
}

/// FFI context for a parsed or generated change proof.
#[derive(Debug)]
pub struct ChangeProofContext {
    //_proof: (),              // currently not implemented
    //_validation_context: (), // placeholder for future use
    //_commit_context: (),     // placeholder for future use
    proof: FrozenChangeProof,
    //verification: Option<VerificationContext>,
    //proposal_state: Option<ProposalState<'db>>,
}

impl From<FrozenChangeProof> for ChangeProofContext {
    fn from(proof: FrozenChangeProof) -> Self {
        Self {
            proof,
            //verification: None,
            //proposal_state: None,
        }
    }
}

/*
#[derive(Debug)]
#[expect(dead_code)]
struct VerificationContext {
    start_root: HashKey,
    end_root: HashKey,
    start_key: Option<Box<[u8]>>,
    end_key: Option<Box<[u8]>>,
    max_length: Option<NonZeroUsize>,
}
*/

impl ChangeProofContext {
    fn verify(
        &mut self,
        _start_root: HashKey,
        _end_root: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<(), api::Error> {
        warn!("change proof verification not yet implemented");

        let key_values = self.proof.key_values();
        if key_values.is_empty() {
            return Err(api::Error::ProofError(ProofError::Empty));
        }

        // Check to make sure the BatchOp array size is less than or equal to `max_length`
        if let Some(max_length) = max_length
            && key_values.len() > max_length.into()
        {
            return Err(api::Error::ProofError(
                ProofError::ProofIsLargerThanMaxLength,
            ));
        }

        // Check the start key is not greater than the first key in the proof.
        if let (Some(start_key), Some(first_key)) = (start_key, key_values.first())
            && start_key.cmp(first_key.key()) == Ordering::Greater
        {
            return Err(api::Error::ProofError(
                ProofError::StartKeyLargerThanFirstKey,
            ));
        }

        // Check the end key is not less than the last key in the proof.
        if let (Some(end_key), Some(last_key)) = (end_key, key_values.last())
            && end_key.cmp(last_key.key()) == Ordering::Less
        {
            return Err(api::Error::ProofError(ProofError::EndKeyLessThanLastKey));
        }

        // Verify the keys are in sorted order.
        if key_values
            .iter()
            .is_sorted_by(|a, b| b.key().cmp(a.key()) == Ordering::Greater)
        {
            // TODO: Keeping a verification state may not be needed
            /*
            self.verification = Some(VerificationContext {
                start_root,
                end_root,
                start_key: start_key.map(Box::from),
                end_key: end_key.map(Box::from),
                max_length,
            });
            */
            Ok(())
        } else {
            Err(api::Error::ProofError(ProofError::ChangeProofKeysNotSorted))
        }
    }

    fn verify_and_commit(
        &mut self,
        db: &crate::DatabaseHandle,
        start_root: HashKey,
        end_root: HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<Option<TrieHash>, api::Error> {
        self.verify(start_root, end_root, start_key, end_key, max_length)?;

        // Rename to apply change proof.
        let proposal_handle = db
            .apply_change_proof(start_root.into(), &self.proof)?
            .handle;

        /*
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
        */

        let metrics_cb = |commit_time: coarsetime::Duration| {
            firewood_counter!(
                "firewood.ffi.commit_ms",
                "FFI commit timing in milliseconds"
            )
            .increment(commit_time.as_millis());
            firewood_counter!("firewood.ffi.merge", "Number of FFI merge operations").increment(1);
        };

        proposal_handle.commit_proposal(metrics_cb)
        /*
        let result = if let Err(api::Error::ParentNotLatest { .. }) = result
            && allow_rebase
        {
            // proposal is stale, try rebasing and committing again
            let proposal_handle = db
                .merge_key_value_range(start_key, end_key, self.proof.key_values())?
                .handle;
            proposal_handle.commit_proposal(metrics_cb)
        } else {
            result
        };

        let hash = result?;
        self.proposal_state = Some(ProposalState::Committed(hash.clone()));

        Ok(hash)
        */
    }
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
/// - [`ChangeProofResult::RevisionNotFound`] if the caller provided a start or end root
///   that was not found in the database. The missing root hash is included in the result.
///   The start root is checked first, and if both are missing, only the start root is
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
/// # Arguments
///
/// - `db` - The database to verify the proof against.
/// - `args` - The arguments for verifying the change proof.
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
pub extern "C" fn fwd_db_verify_change_proof(
    _db: Option<&DatabaseHandle>,
    args: VerifyChangeProofArgs,
) -> VoidResult {
    let VerifyChangeProofArgs {
        proof,
        start_root,
        end_root,
        start_key,
        end_key,
        max_length,
    } = args;

    crate::invoke_with_handle(proof, |ctx| {
        let start_key = start_key.into_option();
        let end_key = end_key.into_option();
        ctx.verify(
            start_root,
            end_root,
            start_key.as_deref(),
            end_key.as_deref(),
            NonZeroUsize::new(max_length as usize),
        )
    })
}

/// Verify and commit a change proof to the database.
///
/// If the proof has already been verified, the previously prepared proposal will be
/// committed instead of re-verifying. If the proof has not been verified, it will be
/// verified now. If the prepared proposal is no longer valid (e.g., the database has
/// changed since it was prepared), a new proposal will be created and committed.
///
/// The proof context will be updated with additional information about the committed
/// proof to allow for optimized introspection of the committed changes.
///
/// # Arguments
///
/// - `db` - The database to commit the changes to.
/// - `args` - The arguments for verifying the change proof.
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
    let VerifyChangeProofArgs {
        proof,
        start_root,
        end_root,
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
            start_root,
            end_root,
            start_key.as_deref(),
            end_key.as_deref(),
            NonZeroUsize::new(max_length as usize),
        )
    })
}

/// Returns the next key range that should be fetched after processing the
/// current set of operations in a change proof that was truncated.
///
/// Can be called multiple times to get subsequent disjoint key ranges until
/// it returns [`NextKeyRangeResult::None`], indicating there are no more keys to
/// fetch and the proof is complete.
///
/// # Arguments
///
/// - `proof` - A [`ChangeProofContext`] previously returned from the create
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
pub extern "C" fn fwd_change_proof_find_next_key(
    _proof: Option<&mut ChangeProofContext>,
) -> NextKeyRangeResult {
    CResult::from_err("not yet implemented")
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
pub extern "C" fn fwd_change_proof_to_bytes(_proof: Option<&ChangeProofContext>) -> ValueResult {
    CResult::from_err("not yet implemented")
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
pub extern "C" fn fwd_change_proof_from_bytes(_bytes: BorrowedBytes) -> ChangeProofResult {
    CResult::from_err("not yet implemented")
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
