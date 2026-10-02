// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::num::NonZeroUsize;
use std::sync::Arc;

use firewood::{
    KeyRange, ProofError,
    api::{self, FrozenChangeProof},
};

use super::proposal_state::ProposalState;
use crate::{
    BorrowedBytes, ChangeProofResult, CodeIteratorHandle, CodeIteratorResult, DatabaseHandle,
    HashKey, HashResult, Maybe, NextKeyRangesResult, OwnedBytes, OwnedSlice, ValueResult,
    VerifiedChangeProofResult, VoidResult,
};

/// Arguments for creating a change proof.
///
/// [`fwd_db_verify_change_proof`] takes the same struct; it ignores
/// `start_root`, because the proof is applied to the database's latest
/// revision.
#[derive(Debug)]
#[repr(C)]
pub struct CreateChangeProofArgs<'a> {
    /// The root hash of the starting revision. If [`fwd_db_change_proof`]
    /// does not find it in the database, it returns
    /// [`ChangeProofResult::StartRevisionNotFound`].
    /// [`fwd_db_verify_change_proof`] ignores this field.
    pub start_root: HashKey,
    /// The root hash of the ending revision. If [`fwd_db_change_proof`] does
    /// not find it in the database, it returns
    /// [`ChangeProofResult::EndRevisionNotFound`].
    /// [`fwd_db_verify_change_proof`] does not look it up: it checks the
    /// applied proof's result against this hash.
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

/// FFI context for a parsed or generated change proof.
///
/// Holds no database reference and borrows nothing, so it is portable:
/// serialize it with [`fwd_change_proof_to_bytes`] or check it against a
/// database with [`fwd_db_verify_change_proof`], which produces a
/// [`VerifiedChangeProofContext`] and leaves this context usable.
#[derive(Debug)]
pub struct ChangeProofContext {
    proof: Arc<FrozenChangeProof>,
}

impl From<FrozenChangeProof> for ChangeProofContext {
    fn from(proof: FrozenChangeProof) -> Self {
        Self {
            proof: Arc::new(proof),
        }
    }
}

impl ChangeProofContext {
    /// Verify the proof against `db` and prepare the proposal that applies it
    /// to `db`'s latest revision.
    ///
    /// Change-proof verification is structural validation, applying the batch
    /// operations to the latest revision, and checking the result against
    /// `end_root`; the root check needs the proposal, so verification always
    /// builds one. `db`'s hash mode is the one verified under.
    fn verify<'db>(
        &self,
        db: &'db DatabaseHandle,
        end_root: api::HashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<VerifiedChangeProofContext<'db>, api::Error> {
        let proposal = db.verify_change_proof(
            &self.proof,
            end_root.clone(),
            start_key,
            end_key,
            max_length,
        )?;
        Ok(VerifiedChangeProofContext {
            db,
            proof: Arc::clone(&self.proof),
            end_root,
            start_key: start_key.map(Box::from),
            end_key: end_key.map(Box::from),
            max_length,
            proposal_state: ProposalState::Proposed(proposal.handle),
        })
    }
}

/// FFI context for a change proof verified against one database.
///
/// Owns the proposal that applies the proof and records the constraints it
/// was verified with, so a commit can rebuild the proposal and
/// `next_key_ranges` can resume from the verified `end_key`.
#[derive(Debug)]
pub struct VerifiedChangeProofContext<'db> {
    db: &'db DatabaseHandle,
    proof: Arc<FrozenChangeProof>,
    end_root: api::HashKey,
    start_key: Option<Box<[u8]>>,
    end_key: Option<Box<[u8]>>,
    max_length: Option<NonZeroUsize>,
    proposal_state: ProposalState<'db>,
}

impl<'db> VerifiedChangeProofContext<'db> {
    /// Re-verify against the current latest revision and build a fresh proposal.
    fn propose(&self) -> Result<crate::ProposalHandle<'db>, api::Error> {
        Ok(self
            .db
            .verify_change_proof(
                &self.proof,
                self.end_root.clone(),
                self.start_key.as_deref(),
                self.end_key.as_deref(),
                self.max_length,
            )?
            .handle)
    }

    /// Commit the proof to the database and return the resulting root hash.
    ///
    /// A prepared proposal is committed as-is. If the database advanced since
    /// verification (`ParentNotLatest`), the proof is verified again against
    /// the then-latest revision and the fresh proposal is committed, so the
    /// proven range is checked against `end_root` on the state it actually
    /// lands on; the changes are never rebased onto a revision they were not
    /// verified against. A proposal consumed by a failed commit leaves the
    /// state `Pending`, and the next call re-verifies. After a successful
    /// commit the root is cached and returned by every later call.
    fn commit(&mut self) -> Result<Option<api::HashKey>, api::Error> {
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
        self.proposal_state = ProposalState::Committed(hash.clone());
        Ok(hash)
    }

    /// The key ranges still to fetch after applying this proof; empty when the
    /// proof covered the verified range. Reads only the proof structure and the
    /// verified `end_key`.
    fn next_key_ranges(&self) -> Result<Vec<KeyRange>, api::Error> {
        Ok(
            firewood::find_next_key_after_change_proof(&self.proof, self.end_key.as_deref())?
                .into_iter()
                .collect(),
        )
    }

    fn code_hash_iter(&self) -> Result<CodeIteratorHandle<'_>, api::Error> {
        CodeIteratorHandle::from_batch_ops(self.proof.hash_mode(), self.proof.batch_ops())
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
/// proof whose header advertises another mode is rejected here rather than
/// at verification.
///
/// # Returns
///
/// - [`ChangeProofResult::NullHandlePointer`] if the caller provided a null database pointer.
/// - [`ChangeProofResult::Ok`] containing a pointer to the `ChangeProofContext` if the proof
///   was successfully parsed. This does not imply that the proof is valid, only that it is
///   well-formed and uses `db`'s hash mode. Call [`fwd_db_verify_change_proof`] to check it.
/// - [`ChangeProofResult::Err`] containing an error message if the proof could not be parsed
///   or its hash mode does not match `db`'s.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_change_proof_from_bytes(
    db: Option<&DatabaseHandle>,
    bytes: BorrowedBytes<'_>,
) -> ChangeProofResult {
    crate::invoke_with_handle(db, move |db| {
        let proof = FrozenChangeProof::from_slice(&bytes)
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

/// Verify a change proof against `db` and prepare the proposal that applies it.
///
/// Performs structural validation, applies the batch operations to the latest
/// revision, and verifies the result against `args.end_root`. The input proof
/// is borrowed, not consumed. `args.start_root` is ignored.
///
/// # Returns
///
/// - [`VerifiedChangeProofResult::NullHandlePointer`] if the caller provided a null
///   pointer to either the database or the proof.
/// - [`VerifiedChangeProofResult::Ok`] containing a pointer to the
///   [`VerifiedChangeProofContext`] if verification succeeded.
/// - [`VerifiedChangeProofResult::Err`] containing an error message if verification failed.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_db_verify_change_proof<'db>(
    db: Option<&'db DatabaseHandle>,
    proof: Option<&ChangeProofContext>,
    args: CreateChangeProofArgs<'_>,
) -> VerifiedChangeProofResult<'db> {
    let handle = db.zip(proof);
    crate::invoke_with_handle(handle, |(db, ctx)| {
        ctx.verify(
            db,
            args.end_root.into(),
            args.start_key.into_option().as_deref(),
            args.end_key.into_option().as_deref(),
            NonZeroUsize::new(args.max_length as usize),
        )
    })
}

/// Commit a verified change proof to its database.
///
/// If the database advanced since verification, the proof is verified again
/// against the latest revision and committed from there, so the proven range
/// is checked against the verified end root on the state it lands on; a
/// proposal consumed by a failed commit is rebuilt on the next call; after
/// success the root is cached and a second call returns it without touching
/// the database. The context stays usable afterwards.
///
/// # Returns
///
/// - [`HashResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`HashResult::None`] if the trie has no root hash (merkledb mode only;
///   ethhash always returns a root hash, even for an empty trie).
/// - [`HashResult::Some`] containing the new root hash.
/// - [`HashResult::Err`] if the commit failed.
///
/// # Thread Safety
///
/// Takes the context mutably: the caller must ensure exclusive access for the
/// duration of the call.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_verified_change_proof_commit(
    proof: Option<&mut VerifiedChangeProofContext<'_>>,
) -> HashResult {
    crate::invoke_with_handle(proof, VerifiedChangeProofContext::commit)
}

/// Returns the key ranges still to fetch after this change proof.
///
/// # Returns
///
/// - [`NextKeyRangesResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`NextKeyRangesResult::Ok`] containing the ranges; empty when nothing remains. The
///   caller frees it with [`fwd_free_next_key_ranges`].
/// - [`NextKeyRangesResult::Err`] containing an error message.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_verified_change_proof_next_key_ranges(
    proof: Option<&VerifiedChangeProofContext<'_>>,
) -> NextKeyRangesResult {
    crate::invoke_with_handle(proof, VerifiedChangeProofContext::next_key_ranges)
}

/// Returns an iterator over the code hashes contained in a verified change
/// proof. Only `BatchOp::Put` entries contribute code hashes; `Delete` and
/// `DeleteRange` entries are skipped. The iterator borrows the proof and must
/// be freed with [`fwd_code_hash_iter_free`] before the proof is.
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
pub extern "C" fn fwd_verified_change_proof_code_hash_iter<'a>(
    proof: Option<&'a VerifiedChangeProofContext<'_>>,
) -> CodeIteratorResult<'a> {
    crate::invoke_with_handle(proof, VerifiedChangeProofContext::code_hash_iter)
}

/// Serialize a change proof to bytes.
///
/// # Returns
///
/// - [`ValueResult::NullHandlePointer`] if the caller provided a null pointer.
/// - [`ValueResult::Some`] containing the serialized bytes if successful.
/// - [`ValueResult::Err`] if serialization failed.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_change_proof_to_bytes(proof: Option<&ChangeProofContext>) -> ValueResult {
    crate::invoke_with_handle(proof, |ctx| -> Result<Option<Box<[u8]>>, api::Error> {
        let mut vec = Vec::new();
        ctx.proof
            .write_to_vec(&mut vec)
            .map_err(api::Error::ProofError)?;
        Ok(Some(vec.into_boxed_slice()))
    })
}

/// Frees the memory associated with a `ChangeProofContext`.
///
/// # Returns
///
/// - [`VoidResult::Ok`] if the memory was successfully freed.
/// - [`VoidResult::Err`] if the process panics while freeing the memory.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_free_change_proof(proof: Option<Box<ChangeProofContext>>) -> VoidResult {
    crate::invoke_with_handle(proof, drop)
}

/// Frees the memory associated with a `VerifiedChangeProofContext`, dropping
/// its proposal if it was not committed.
///
/// # Returns
///
/// - [`VoidResult::Ok`] if the memory was successfully freed.
/// - [`VoidResult::Err`] if the process panics while freeing the memory.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_free_verified_change_proof(
    proof: Option<Box<VerifiedChangeProofContext<'_>>>,
) -> VoidResult {
    crate::invoke_with_handle(proof, drop)
}

/// Frees a list of key ranges returned by a `next_key_ranges` function,
/// including every key it holds.
///
/// # Returns
///
/// - [`VoidResult::Ok`] if the memory was successfully freed.
/// - [`VoidResult::Err`] if the process panics while freeing the memory.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_free_next_key_ranges(ranges: OwnedSlice<NextKeyRange>) -> VoidResult {
    crate::invoke(move || drop(ranges))
}

impl crate::MetricsContextExt for ChangeProofContext {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        None
    }
}

impl crate::MetricsContextExt for VerifiedChangeProofContext<'_> {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        self.db.metrics_context()
    }
}

impl crate::MetricsContextExt for (&DatabaseHandle, &ChangeProofContext) {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        self.0.metrics_context()
    }
}
