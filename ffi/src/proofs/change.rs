// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::num::NonZeroUsize;

use firewood_metrics::firewood_increment;
use firewood_storage::PathIterItem;
#[cfg(feature = "ethhash")]
use firewood_storage::TrieHash;
#[cfg(feature = "ethhash")]
use rlp::Rlp;

use firewood::{
    ChangeProofVerificationContext, ProofError,
    api::{self, DbView as _, FrozenChangeProof, HashKey as ApiHashKey},
    change_proof_boundary_key, verify_change_proof_root_hash, verify_change_proof_structure,
};

use crate::{
    BorrowedBytes, ChangeProofResult, DatabaseHandle, HashKey, HashResult, KeyRange, Maybe,
    NextKeyRangeResult, OwnedBytes, ProposedChangeProofResult, ValueResult, VoidResult,
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

/// Arguments for verifying a change proof (used by both propose and commit).
#[derive(Debug)]
#[repr(C)]
pub struct VerifyChangeProofArgs<'a> {
    /// The change proof to verify.
    pub proof: Option<Box<ChangeProofContext>>,
    /// The root hash of the starting revision.
    pub start_root: HashKey,
    /// The root hash of the ending revision.
    pub end_root: HashKey,
    /// The lower bound of the key range that the proof is expected to cover. If
    /// `None`, the proof is expected to cover from the start of the keyspace.
    pub start_key: Maybe<BorrowedBytes<'a>>,
    /// The upper bound of the key range that the proof is expected to cover. If
    /// `None`, the proof is expected to cover to the end of the keyspace.
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

/// Type alias — the struct now lives in the firewood crate.
type VerificationContext = ChangeProofVerificationContext;

#[derive(Debug)]
enum ProposalState<'db> {
    Proposed(crate::ProposalHandle<'db>),
    Committed(Option<ApiHashKey>),
    Failed,
}

impl From<FrozenChangeProof> for ChangeProofContext {
    fn from(proof: FrozenChangeProof) -> Self {
        Self { proof }
    }
}

/// FFI wrapper for root hash verification. Delegates to the firewood crate's
/// `verify_change_proof_root_hash` after computing paths from the proposal.
fn verify_root_hash(
    proof: &FrozenChangeProof,
    verification: &VerificationContext,
    proposal: &crate::ProposalHandle<'_>,
) -> Result<(), api::Error> {
    let proposal_root_hash = proposal.root_hash();
    let start_path = get_proposal_path_for_proof(proof.start_proof().as_ref(), proposal)?;
    let end_path = get_proposal_path_for_proof(proof.end_proof().as_ref(), proposal)?;
    verify_change_proof_root_hash(
        proof,
        verification,
        proposal_root_hash.as_ref(),
        &start_path,
        &end_path,
    )
}

/// Retrieve the proposal's path aligned with the given proof nodes.
fn get_proposal_path_for_proof(
    proof_nodes: &[firewood::ProofNode],
    proposal: &crate::ProposalHandle<'_>,
) -> Result<Vec<PathIterItem>, api::Error> {
    let Some(key) = change_proof_boundary_key(proof_nodes) else {
        return Ok(Vec::new());
    };
    proposal.path_to_key(&key)
}

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
        db: &'db DatabaseHandle,
        start_root: ApiHashKey,
        end_root: ApiHashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<ProposedChangeProofContext<'db>, Box<(Self, api::Error)>> {
        // Destructure self so we can move `proof` into either the Ok or Err
        // result without cloning.
        let proof = self.proof;

        let verification = match verify_change_proof_structure(
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

        // Root hash verification: walk boundary proof paths bottom-up,
        // substituting in-range children from the proposal, and compare
        // the computed root against end_root.
        if let Err(e) = verify_root_hash(&proof, &verification, &proposal.handle) {
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
        db: &DatabaseHandle,
        start_root: ApiHashKey,
        end_root: ApiHashKey,
        start_key: Option<&[u8]>,
        end_key: Option<&[u8]>,
        max_length: Option<NonZeroUsize>,
    ) -> Result<Option<ApiHashKey>, api::Error> {
        let mut proposed = self
            .verify_and_propose(db, start_root, end_root, start_key, end_key, max_length)
            .map_err(|boxed| {
                let (_proof, err) = *boxed;
                err
            })?;
        proposed.commit()
    }
}

impl ProposedChangeProofContext<'_> {
    /// Commit a previously proposed change proof. Consumes the proposal handle.
    fn commit(&mut self) -> Result<Option<ApiHashKey>, api::Error> {
        // Pessimistically set state to Failed; success paths below restore it.
        let state = std::mem::replace(&mut self.proposal_state, ProposalState::Failed);
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
            ProposalState::Failed => Err(api::Error::CommitAlreadyFailed),
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
        let computed: HashKey = match &self.proposal_state {
            ProposalState::Proposed(handle) => {
                handle.root_hash().map(HashKey::from).unwrap_or_default()
            }
            ProposalState::Committed(hash) => hash.clone().map(HashKey::from).unwrap_or_default(),
            ProposalState::Failed => {
                return Err(api::Error::CommitAlreadyFailed);
            }
        };
        if computed != HashKey::from(self.verification.end_root.clone()) {
            return Err(api::Error::ProofError(ProofError::EndRootMismatch));
        }
        Ok(())
    }

    /// Returns the next key range that should be fetched after processing this
    /// change proof, or `None` if there are no more keys to fetch.
    ///
    /// # Termination analysis
    ///
    /// | `batch_ops` | `end_proof` | `end_key`  | Path                   | Root check? |
    /// |-------------|-------------|------------|------------------------|-------------|
    /// | empty       | empty       | any        | no `batch_ops` → nil   | yes         |
    /// | empty       | non-empty   | any        | no `batch_ops` → nil   | yes         |
    /// | non-empty   | empty       | any        | empty `end_proof`      | yes         |
    /// | non-empty   | non-empty   | Some, sat  | `last_op` >= `end_key` | no (safe*)  |
    /// | non-empty   | non-empty   | Some, !sat | continuation           | deferred    |
    /// | non-empty   | non-empty   | None       | continuation           | deferred    |
    ///
    /// *The `>=` comparison is byte-lexicographic on `Box<[u8]>`, which is
    /// standard byte ordering — no encoding confusion possible. The "no
    /// root check" row is safe because `end_key` is receiver-controlled,
    /// not attacker-controlled.
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
    let start_key = args.start_key.into_option();
    let end_key = args.end_key.into_option();
    let handle = db.and_then(|db| args.proof.map(|p| (db, p)));
    crate::invoke_with_handle(handle, |(db, ctx)| {
        ctx.verify_and_propose(
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
    let start_key = args.start_key.into_option();
    let end_key = args.end_key.into_option();
    let handle = db.and_then(|db| args.proof.map(|p| (db, p)));
    crate::invoke_with_handle(handle, |(db, ctx)| {
        ctx.verify_and_commit(
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
/// - [`ValueResult::Err`] containing an error message if the `ChangeProof`
///   cannot be serialized.
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
/// - [`ValueResult::Err`] containing an error message if the proposed `ChangeProof`
///   cannot be serialized.
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

impl crate::MetricsContextExt for (&DatabaseHandle, Box<ChangeProofContext>) {
    fn metrics_context(&self) -> Option<firewood_metrics::MetricsContext> {
        self.0.metrics_context()
    }
}

// Verification logic tests have been moved to firewood/src/merkle/tests/change.rs.
// Only FFI lifecycle tests (find_next_key, commit state machine) remain here.
#[cfg(test)]
mod tests {
    use firewood::{
        Key, Value,
        api::{BatchOp, Proposal as _},
    };

    use crate::{BorrowedBytes, CView, DatabaseHandle, DatabaseHandleArgs, NodeHashAlgorithm};

    use super::ChangeProofContext;

    fn put(key: &[u8], val: &[u8]) -> BatchOp<Key, Value> {
        BatchOp::Put {
            key: key.to_vec().into_boxed_slice(),
            value: val.to_vec().into_boxed_slice(),
        }
    }

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

    #[test]
    fn test_unbounded_proof_with_end_proof_checks_root_hash() {
        use std::num::NonZeroUsize;

        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

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

        let changes = vec![
            put(b"\x10", b"changed0"),
            put(b"\x20", b"changed1"),
            put(b"\x30", b"changed2"),
        ];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        let proof = db_a
            .change_proof(root1_a, root2.clone(), None, None, NonZeroUsize::new(1))
            .expect("truncated change proof");

        let change_ctx = ChangeProofContext::from(proof);
        let mut proposed = change_ctx
            .verify_and_propose(&db_b, root1_b, root2, None, None, None)
            .expect("verify_and_propose should succeed");

        let next = proposed
            .find_next_key()
            .expect("find_next_key should not error");
        assert!(
            next.is_some(),
            "truncated proof presented as unbounded must indicate more data needed"
        );
    }

    #[test]
    fn test_find_next_key_root_hash_positive() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\x20", b"v1")];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");

        let changes = vec![put(b"\x10", b"changed0"), put(b"\x20", b"changed1")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        let proof = db_a
            .change_proof(root1_a, root2.clone(), None, Some(b"\x20".as_ref()), None)
            .expect("change proof");

        let change_ctx = ChangeProofContext::from(proof);
        let mut proposed = change_ctx
            .verify_and_propose(&db_b, root1_b, root2, None, Some(b"\x20".as_ref()), None)
            .expect("verify_and_propose should succeed");

        let next = proposed
            .find_next_key()
            .expect("find_next_key should not error");
        assert_eq!(next, None, "single-round proof should be complete");
    }

    #[test]
    fn test_double_commit_returns_cached_hash() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\x20", b"v1")];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");

        let changes = vec![put(b"\x10", b"changed")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        let proof = db_a
            .change_proof(root1_a, root2.clone(), None, None, None)
            .expect("change proof");

        let change_ctx = ChangeProofContext::from(proof);
        let mut proposed = change_ctx
            .verify_and_propose(&db_b, root1_b, root2, None, None, None)
            .expect("verify_and_propose");

        let hash1 = proposed.commit().expect("first commit");
        let hash2 = proposed.commit().expect("second commit");
        assert_eq!(hash1, hash2, "double commit should return cached hash");
    }

    #[test]
    fn test_commit_after_failed_commit_returns_error() {
        use firewood::api::Proposal as _;

        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let initial = vec![put(b"\x10", b"v0"), put(b"\x20", b"v1")];
        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let root1_b = db_b.current_root_hash().expect("root");

        let changes = vec![put(b"\x10", b"changed")];
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        let proof = db_a
            .change_proof(root1_a, root2.clone(), None, None, None)
            .expect("change proof");

        let change_ctx = ChangeProofContext::from(proof);
        let mut proposed = change_ctx
            .verify_and_propose(&db_b, root1_b, root2, None, None, None)
            .expect("verify_and_propose");

        // Advance db_b with a sibling proposal, making ours stale
        let sibling = (&db_b)
            .create_proposal(vec![put(b"\x20", b"sibling")])
            .expect("sibling proposal");
        sibling.commit().expect("sibling commit");

        let err1 = proposed.commit().expect_err("first commit should fail");
        assert!(
            matches!(
                err1,
                firewood::api::Error::SiblingCommitted
                    | firewood::api::Error::ParentNotLatest { .. }
            ),
            "expected SiblingCommitted or ParentNotLatest, got: {err1}"
        );

        let err2 = proposed.commit().expect_err("second commit should fail");
        assert!(
            matches!(err2, firewood::api::Error::CommitAlreadyFailed),
            "expected CommitAlreadyFailed, got: {err2}"
        );
    }

    #[test]
    #[ignore = "requires find_next_key termination changes for end_key=None"]
    fn test_iterative_sync_converges() {
        let dir_a = tempfile::tempdir().expect("tempdir");
        let dir_b = tempfile::tempdir().expect("tempdir");
        let db_a = test_db(dir_a.path());
        let db_b = test_db(dir_b.path());

        let mut initial = Vec::new();
        for i in 0..100u32 {
            let key = format!("key{i:03}");
            let val = format!("val{i:03}");
            initial.push(put(key.as_bytes(), val.as_bytes()));
        }

        let p = (&db_a).create_proposal(initial.clone()).expect("proposal");
        p.commit().expect("commit");
        let root1_a = db_a.current_root_hash().expect("root");

        let p = (&db_b).create_proposal(initial).expect("proposal");
        p.commit().expect("commit");
        let mut root_b = db_b.current_root_hash().expect("root");
        assert_eq!(root1_a, root_b);

        let mut changes = Vec::new();
        for i in 0..50u32 {
            let key = format!("key{i:03}");
            let val = format!("changed{i:03}");
            changes.push(put(key.as_bytes(), val.as_bytes()));
        }
        let p = (&db_a).create_proposal(changes).expect("proposal");
        p.commit().expect("commit");
        let root2 = db_a.current_root_hash().expect("root");

        let mut start_key: Option<Vec<u8>> = None;
        let max_rounds = 20;

        for round in 0..max_rounds {
            let proof = db_a
                .change_proof(
                    root1_a.clone(),
                    root2.clone(),
                    start_key.as_deref(),
                    None,
                    std::num::NonZeroUsize::new(10),
                )
                .expect("proof");

            let ctx = ChangeProofContext::from(proof);
            let mut proposed = ctx
                .verify_and_propose(
                    &db_b,
                    root_b.clone(),
                    root2.clone(),
                    start_key.as_deref(),
                    None,
                    std::num::NonZeroUsize::new(10),
                )
                .unwrap_or_else(|e| panic!("round {round}: verify failed: {:?}", e.1));

            root_b = proposed
                .commit()
                .unwrap_or_else(|e| panic!("round {round}: commit failed: {e:?}"))
                .expect("commit should return a root hash");

            let next = proposed
                .find_next_key()
                .unwrap_or_else(|e| panic!("round {round}: find_next_key failed: {e:?}"));

            match next {
                None => break,
                Some((next_start, _)) => {
                    start_key = Some(next_start.to_vec());
                }
            }

            assert!(
                round < max_rounds - 1,
                "sync did not converge after {max_rounds} rounds"
            );
        }

        assert_eq!(
            root_b, root2,
            "after iterative sync, root hashes should match"
        );
    }
}
