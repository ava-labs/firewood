// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::num::NonZeroUsize;

#[cfg(feature = "ethhash")]
use firewood_storage::TrieHash;
#[cfg(feature = "ethhash")]
use rlp::Rlp;

use firewood::{
    ProofError,
    logger::warn,
    v2::api::{self, FrozenChangeProof},
};

use std::cmp::Ordering;

use crate::{
    BorrowedBytes, CResult, ChangeProofResult, DatabaseHandle, HashKey, HashResult, Maybe,
    NextKeyRangeResult, OwnedBytes, ValueResult, VoidResult,
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
    pub start_root: HashKey,
    /// The root hash of the ending revision. This must be provided.
    /// If the root is not found in the database, the function will return
    /// [`ChangeProofResult::EndRevisionNotFound`].
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
    pub proof: Option<&'a mut ChangeProofContext<'a>>,
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
pub struct ChangeProofContext<'db> {
    state: ChangeProofState<'db>,
    proof: FrozenChangeProof,
}

/// A `ChangeProofContext` can be in four different states. Calling `verify_and_propose`
/// and `verify_and_commit` will perform state transitions on the `ChangeProofContext`.
/// The state keeps a saved version of any previous verification context or database
/// handle. The saved version is used to eliminate repeated verifications or proposal
/// creation on a change proof.
///
/// If the new verification context doesn't match the the saved version exactly, then
/// we treat the change proof as unverified and verify it again.
///
/// The function `std::ptr::eq` is used to check that the saved database handle is
/// pointing to the same memory location as the handle passed to the propose and commit
/// functions. This ensures that we don't accidentally try to commit a proposal created
/// for one database when calling commit on a different database. If the database
/// handles don't match, we set the state back to `Unverified` and re-perform
/// `verify_and_propose` or `verify_and_commit` depending on which was called.
#[derive(Debug)]
#[expect(unused)]
enum ChangeProofState<'db> {
    Unverified,
    Verified(VerificationContext),
    Proposed(
        VerificationContext,
        &'db DatabaseHandle,
        crate::ProposalHandle<'db>,
    ),
    Committed(VerificationContext, &'db DatabaseHandle, Option<HashKey>),
}

impl From<FrozenChangeProof> for ChangeProofContext<'_> {
    fn from(proof: FrozenChangeProof) -> Self {
        Self {
            state: ChangeProofState::Unverified,
            proof,
        }
    }
}

impl<'db> ChangeProofContext<'db> {
    /// Only called if `ChangeProofContext` is in the unverified state. Causes a transition
    /// to the verified state.
    fn verify_helper(&mut self, context: VerificationContext) -> Result<(), api::Error> {
        let batch_ops = self.proof.batch_ops();

        // Check to make sure the BatchOp array size is less than or equal to `max_length`
        if let Some(max_length) = context.max_length
            && batch_ops.len() > max_length.into()
        {
            return Err(api::Error::ProofError(
                ProofError::ProofIsLargerThanMaxLength,
            ));
        }

        // Check the start key is not greater than the first key in the proof.
        if let (Some(start_key), Some(first_key)) = (&context.start_key, batch_ops.first())
            && start_key.cmp(first_key.key()) == Ordering::Greater
        {
            return Err(api::Error::ProofError(
                ProofError::StartKeyLargerThanFirstKey,
            ));
        }

        // Check the end key is not less than the last key in the proof.
        if let (Some(end_key), Some(last_key)) = (&context.end_key, batch_ops.last())
            && end_key.cmp(last_key.key()) == Ordering::Less
        {
            return Err(api::Error::ProofError(ProofError::EndKeyLessThanLastKey));
        }

        // Verify the keys are in sorted order.
        if batch_ops
            .iter()
            .is_sorted_by(|a, b| b.key().cmp(a.key()) == Ordering::Greater)
        {
            warn!("change proof verification not yet implemented");
            self.state = ChangeProofState::Verified(context);
            Ok(())
        } else {
            Err(api::Error::ProofError(ProofError::ChangeProofKeysNotSorted))
        }
    }

    /// Only called if `ChangeProofContext` is in the verified state. Causes a transition to the
    /// proposed state.
    fn propose_helper(
        &mut self,
        db: &'db crate::DatabaseHandle,
        context: VerificationContext,
    ) -> Result<(), api::Error> {
        let proposal = db.apply_change_proof_to_parent(context.start_root.into(), &self.proof)?;
        self.state = ChangeProofState::Proposed(context, db, proposal.handle);
        Ok(())
    }

    fn verify_and_propose(
        &mut self,
        db: &'db crate::DatabaseHandle,
        context: VerificationContext,
    ) -> Result<(), api::Error> {
        match &mut self.state {
            ChangeProofState::Unverified => {
                self.verify_helper(context.clone())?;
                self.propose_helper(db, context)
            }
            // Check that the verification context matches the saved context exactly. If it
            // does, then verification has already been completed and we can create the proposal.
            ChangeProofState::Verified(ctx) if context == *ctx => self.propose_helper(db, context),
            // Check that the verification context and the database match the saved version.
            ChangeProofState::Proposed(ctx, db_handle, _)
                if context == *ctx && std::ptr::eq(*db_handle, db) =>
            {
                Ok(()) // Already been verified and proposed
            }
            // Check that the verification context and the database match the saved version.
            // Keeping this separate from the `ChangeProofState::Proposed` arm in case we want to
            // separate their behavior in the future.
            ChangeProofState::Committed(ctx, db_handle, _)
                if context == *ctx && std::ptr::eq(*db_handle, db) =>
            {
                Ok(()) // Already been verified, proposed and even committed.
            }
            _ => {
                // Verification context or database didn't match. Redo from unverified. We could
                // further distinguish between the two cases to not perform verification if only
                // the database didn't match. However, it may be better to always re-perform the
                // verification in case verification is changed in the future to be database
                // specific.
                self.state = ChangeProofState::Unverified;
                self.verify_and_propose(db, context)
            }
        }
    }
}

/// FFI context for verifying a change proof
#[derive(Debug, PartialEq, Clone)]
struct VerificationContext {
    start_root: HashKey,
    end_root: HashKey,
    start_key: Option<Box<[u8]>>,
    end_key: Option<Box<[u8]>>,
    max_length: Option<NonZeroUsize>,
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
    type Item = Result<HashKey, api::Error>;

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
            let code_hash: HashKey = TrieHash::try_from(code_hash_slice).ok()?.into();
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
pub extern "C" fn fwd_db_change_proof<'db>(
    db: Option<&DatabaseHandle>,
    args: CreateChangeProofArgs,
) -> ChangeProofResult<'db> {
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
pub extern "C" fn fwd_db_verify_change_proof<'db>(
    db: Option<&'db DatabaseHandle>,
    args: VerifyChangeProofArgs<'db>,
) -> VoidResult {
    let handle = db.and_then(|db| args.proof.map(|p| (db, p)));
    crate::invoke_with_handle(handle, |(db, ctx)| {
        let context = VerificationContext {
            start_root: args.start_root,
            end_root: args.end_root,
            start_key: args.start_key.into_option().as_deref().map(Box::from),
            end_key: args.end_key.into_option().as_deref().map(Box::from),
            max_length: NonZeroUsize::new(args.max_length as usize),
        };
        ctx.verify_and_propose(db, context)
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
    _db: Option<&DatabaseHandle>,
    _args: VerifyChangeProofArgs,
) -> HashResult {
    CResult::from_err("not yet implemented")
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
pub extern "C" fn fwd_change_proof_to_bytes(proof: Option<&ChangeProofContext>) -> ValueResult {
    crate::invoke_with_handle(proof, |ctx| {
        let mut vec = Vec::new();
        ctx.proof.write_to_vec(&mut vec);
        vec
    })
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

impl crate::MetricsContextExt for ChangeProofContext<'_> {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        None
    }
}

impl<'a> crate::MetricsContextExt for (&'a DatabaseHandle, &mut ChangeProofContext<'a>) {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        self.0.metrics_context()
    }
}

impl crate::MetricsContextExt for CodeIteratorHandle<'_> {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        None
    }
}
