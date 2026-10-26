// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::convert::Into;
use std::num::NonZeroUsize;

use firewood::ProofError;
use firewood::api;
use firewood::api::FrozenChangeProof;

use crate::BorrowedBytes;
use crate::ChangeProofResult;
use crate::CodeIteratorHandle;
use crate::CodeIteratorResult;
use crate::DatabaseHandle;
use crate::HashKey;
use crate::HashResult;
use crate::KeyRange;
use crate::Maybe;
use crate::NextKeyRangeResult;
use crate::OwnedBytes;
use crate::SizedChangeProofResult;
use crate::SizingHint;
use crate::ValueResult;
use crate::VoidResult;

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

/// FFI context for a parsed or generated change proof. The proof is borrowed
/// (not consumed) during verification via `fwd_db_verify_change_proof`, so it
/// remains available for serialization and `find_next_key` afterward.
#[derive(Debug)]
pub struct ChangeProofContext {
    proof: Option<FrozenChangeProof>,
}

impl From<FrozenChangeProof> for ChangeProofContext {
    fn from(proof: FrozenChangeProof) -> Self {
        Self { proof: Some(proof) }
    }
}

impl ChangeProofContext {
    /// Returns the underlying proof, if present.
    #[must_use]
    pub const fn proof(&self) -> Option<&FrozenChangeProof> {
        self.proof.as_ref()
    }

    /// Returns the next key range to fetch for a change proof,
    /// or `None` if there are no more keys to fetch.
    ///
    /// Only inspects the proof structure — does not require a proposal.
    /// `end_key` is the original requested end key passed to the proof
    /// generator.
    fn find_next_key(&self, end_key: Option<&[u8]>) -> Result<Option<KeyRange>, api::Error> {
        let proof = self
            .proof
            .as_ref()
            .ok_or(api::Error::ProofError(ProofError::ProofIsNone))?;
        firewood::find_next_key_after_change_proof(proof, end_key)
    }

    fn code_hash_iter(&self) -> Result<CodeIteratorHandle<'_>, api::Error> {
        let proof = self
            .proof
            .as_ref()
            .ok_or(api::Error::ProofError(ProofError::ProofIsNone))?;
        CodeIteratorHandle::from_batch_ops(proof.hash_mode(), proof.batch_ops())
    }
}

/// A key range still to fetch after a truncated range or change proof,
/// `[start_key, end_key]`, both inclusive: `start_key` is the smallest key
/// above the last one already synchronized, so passing it as the next
/// request's start bound resumes without covering that key again. An absent
/// `end_key` means the range is unbounded above.
#[derive(Debug)]
#[repr(C)]
pub struct NextKeyRange {
    /// The inclusive start key of the next range to fetch.
    pub start_key: OwnedBytes,

    /// If set, the inclusive upper bound of the next range to fetch. If not
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

/// Deserialize a change proof from bytes for use with `db`.
///
/// The database supplies the hash mode the proof must be encoded with; a
/// proof whose header advertises another mode is rejected here, before its
/// body is decompressed, rather than at verification.
///
/// # Returns
///
/// - [`ChangeProofResult::NullHandlePointer`] if the caller provided a null database pointer.
/// - [`ChangeProofResult::Ok`] containing a pointer to the `ChangeProofContext` if the proof
///   was successfully parsed. This does not imply that the proof is valid, only that it is
///   well-formed and uses `db`'s hash mode. Use [`fwd_db_verify_change_proof`] or
///   [`fwd_db_verify_and_commit_change_proof`] to verify the proof.
/// - [`ChangeProofResult::Err`] containing an error message if the proof could not be parsed
///   or its hash mode does not match `db`'s.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_change_proof_from_bytes(
    db: Option<&DatabaseHandle>,
    bytes: BorrowedBytes<'_>,
) -> ChangeProofResult {
    crate::invoke_with_handle(db, move |db| {
        FrozenChangeProof::from_slice_in_mode(&bytes, db.node_hash_algorithm())
            .map_err(api::Error::ProofError)
    })
}

/// Arguments for creating a size-targeted change proof.
#[derive(Debug)]
#[repr(C)]
pub struct CreateSizedChangeProofArgs<'a> {
    /// The root hash of the starting committed revision.
    pub start_root: HashKey,
    /// The root hash of the ending committed revision.
    pub end_root: HashKey,
    /// The inclusive start key. If `None`, the proof starts from the
    /// beginning of the keyspace.
    pub start_key: Maybe<BorrowedBytes<'a>>,
    /// The wire byte budget the serialized proof must fit; zero always
    /// yields [`SizedChangeProofResult::OverBudget`].
    pub budget: usize,
    /// The previous chunk's hint, if any.
    pub hint: Maybe<SizingHint>,
}

/// Generate a change proof from `start_root` to `end_root` from `start_key`
/// whose serialized size approaches `budget` bytes and never exceeds it. The
/// wire bytes are returned with the proof, so it need not be serialized again.
///
/// # Arguments
///
/// - `db` - The database to create the proof from.
/// - `args` - The arguments for creating the sized change proof.
///
/// # Returns
///
/// - [`SizedChangeProofResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`SizedChangeProofResult::StartRevisionNotFound`] if the start root was not found.
///   The missing root hash is included in the result.
/// - [`SizedChangeProofResult::EndRevisionNotFound`] if the end root was not found. If
///   both roots are missing, only the end root is reported. The missing root hash is
///   included in the result.
/// - [`SizedChangeProofResult::OverBudget`] if one operation, or the edge proofs alone,
///   do not fit.
/// - [`SizedChangeProofResult::Ok`] containing the proof, its wire bytes, and the next
///   hint.
/// - [`SizedChangeProofResult::Err`] containing an error message if the proof could not
///   be created.
#[expect(
    improper_ctypes_definitions,
    reason = "`SizingHint` wraps a Rust-layout value that callers only echo back"
)]
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_change_proof_sized(
    db: Option<&DatabaseHandle>,
    args: CreateSizedChangeProofArgs,
) -> SizedChangeProofResult {
    crate::invoke_with_handle(db, |db| {
        db.change_proof_sized(
            args.start_root.into(),
            args.end_root.into(),
            args.start_key
                .as_ref()
                .map(BorrowedBytes::as_slice)
                .into_option(),
            args.budget,
            args.hint.into_option().map(Into::into),
        )
    })
}

/// Verify a change proof and create a standard proposal.
///
/// Performs structural validation, applies batch ops to the latest
/// revision, and verifies the root hash against `end_root`. The proof is
/// borrowed, not consumed — the caller retains it for `find_next_key` or
/// serialization.
///
/// # Returns
///
/// - `ProposalResult::NullHandlePointer` if the caller provided a null
///   pointer to either the database or the proof.
/// - `ProposalResult::Ok` if verification succeeded and a proposal was
///   created.
/// - `ProposalResult::Err` containing an error message if verification
///   failed.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_verify_change_proof<'db>(
    db: Option<&'db DatabaseHandle>,
    proof: Option<&ChangeProofContext>,
    args: CreateChangeProofArgs<'_>,
) -> crate::ProposalResult<'db> {
    let handle = db.zip(proof);
    crate::invoke_with_handle(handle, |(db, ctx)| {
        let proof = ctx
            .proof()
            .ok_or(api::Error::ProofError(ProofError::ProofIsNone))?;
        db.verify_change_proof(
            proof,
            args.end_root.into(),
            args.start_key.into_option().as_deref(),
            args.end_key.into_option().as_deref(),
            NonZeroUsize::new(args.max_length as usize),
        )
    })
}

/// Verify and commit a change proof in a single call.
///
/// Verifies structural validity and root hash, creates a proposal, and
/// commits it with automatic rebase if needed. The proof is borrowed,
/// not consumed — it remains available for `fwd_change_proof_find_next_key`
/// or serialization afterward.
///
/// # Returns
///
/// - [`HashResult::NullHandlePointer`] if the caller provided a null pointer
///   to either the database or the proof.
/// - [`HashResult::None`] if the trie has no root hash (MerkleDB mode only;
///   Ethereum mode always returns a root hash, even for an empty trie).
/// - [`HashResult::Some`] containing the new root hash.
/// - [`HashResult::Err`] if verification or commit failed.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_verify_and_commit_change_proof(
    db: Option<&DatabaseHandle>,
    proof: Option<&ChangeProofContext>,
    args: CreateChangeProofArgs<'_>,
) -> HashResult {
    let handle = db.zip(proof);
    crate::invoke_with_handle(handle, |(db, ctx)| {
        let proof = ctx
            .proof()
            .ok_or(api::Error::ProofError(ProofError::ProofIsNone))?;
        let proposal = db.verify_change_proof(
            proof,
            args.end_root.into(),
            args.start_key.into_option().as_deref(),
            args.end_key.into_option().as_deref(),
            NonZeroUsize::new(args.max_length as usize),
        )?;
        proposal.handle.commit_proposal_with_rebase()
    })
}

/// Determine the next key range to fetch for a change proof.
///
/// The proof is not consumed by this call. `end_key` is the original
/// requested end key passed to the proof generator.
///
/// Returns:
/// - [`NextKeyRangeResult::None`] if there are no more keys to fetch.
/// - [`NextKeyRangeResult::Some`] containing the next key range to fetch.
/// - [`NextKeyRangeResult::Err`] if an error occurred.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_change_proof_find_next_key(
    proof: Option<&ChangeProofContext>,
    end_key: Maybe<BorrowedBytes>,
) -> NextKeyRangeResult {
    crate::invoke_with_handle(proof, |ctx| {
        ctx.find_next_key(end_key.into_option().as_deref())
    })
}

/// Returns an iterator over the code hashes contained in the change proof.
/// The iterator must be freed after use.
///
/// Only `BatchOp::Put` entries contribute code hashes; `Delete` and
/// `DeleteRange` entries are skipped. Can be called at any time after
/// the proof has been created.
///
/// # Arguments
///
/// - `proof` - A [`ChangeProofContext`] previously returned from the create
///   method or deserialized from bytes.
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
pub extern "C" fn fwd_change_proof_code_hash_iter(
    proof: Option<&ChangeProofContext>,
) -> CodeIteratorResult<'_> {
    crate::invoke_with_handle(proof, ChangeProofContext::code_hash_iter)
}

fn serialize_change_proof(
    proof: Option<&FrozenChangeProof>,
) -> Result<Option<Box<[u8]>>, api::Error> {
    let proof = proof.ok_or(api::Error::ProofError(ProofError::ProofIsNone))?;
    let mut vec = Vec::new();
    proof
        .write_to_vec(&mut vec)
        .map_err(api::Error::ProofError)?;
    Ok(Some(vec.into_boxed_slice()))
}

/// Serialize a `ChangeProof` to bytes.
///
/// # Arguments
///
/// - `proof` - A [`ChangeProofContext`] previously returned from the create
///   method.
///
/// # Returns
///
/// - [`ValueResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`ValueResult::Some`] containing the serialized bytes if successful.
/// - [`ValueResult::Err`] if serialization failed.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_change_proof_to_bytes(proof: Option<&ChangeProofContext>) -> ValueResult {
    crate::invoke_with_handle(proof, |ctx| serialize_change_proof(ctx.proof.as_ref()))
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

impl crate::MetricsContextExt for ChangeProofContext {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        None
    }
}

impl crate::MetricsContextExt for (&DatabaseHandle, &ChangeProofContext) {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        self.0.metrics_context()
    }
}
