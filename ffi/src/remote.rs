// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! FFI layer for remote storage operations.
//!
//! Exposes [`TruncatedTrie`] and [`WitnessProof`] as opaque handles for
//! C/Go callers, enabling:
//!
//! - Creating a truncated trie from a database revision
//! - Querying and verifying the truncated trie's root hash
//! - Generating a witness proof for batch operations (server-side)
//! - Verifying a witness proof and updating the truncated trie (client-side)

use firewood::remote::TruncatedTrie;
use firewood::remote::witness::{self, WitnessProof};
// Aliased to avoid collision with `crate::BatchOp` (the FFI C-compatible enum
// defined in `value/kvp.rs`). Both types are used in this module: `crate::BatchOp`
// arrives from C/Go callers and is converted into `CoreBatchOp` for the witness system.
use firewood::v2::api::BatchOp as CoreBatchOp;
use firewood::v2::api::Db as _;

use crate::metrics::MetricsContextExt;
use crate::{
    BorrowedBatchOps, BorrowedBytes, DatabaseHandle, HashKey, HashResult, OwnedBytes, ValueResult,
    VoidResult,
};

use firewood_metrics::MetricsContext;

/// An opaque handle to a [`TruncatedTrie`].
///
/// Callers must free this handle with [`fwd_free_truncated_trie`].
#[derive(Debug)]
pub struct TruncatedTrieHandle {
    trie: TruncatedTrie,
}

impl MetricsContextExt for &TruncatedTrieHandle {
    fn metrics_context(&self) -> Option<MetricsContext> {
        None
    }
}

impl MetricsContextExt for &mut TruncatedTrieHandle {
    fn metrics_context(&self) -> Option<MetricsContext> {
        None
    }
}

/// An opaque handle to a [`WitnessProof`].
///
/// Callers must free this handle with [`fwd_free_witness_proof`].
#[derive(Debug)]
pub struct WitnessProofHandle {
    proof: WitnessProof,
}

impl MetricsContextExt for &WitnessProofHandle {
    fn metrics_context(&self) -> Option<MetricsContext> {
        None
    }
}

/// A result type returned from FFI functions that create a [`TruncatedTrieHandle`].
#[derive(Debug)]
#[repr(C, usize)]
pub enum TruncatedTrieResult {
    /// The caller provided a null pointer to the input handle.
    NullHandlePointer,

    /// The truncated trie was successfully created.
    ///
    /// The caller must call [`fwd_free_truncated_trie`] to free the handle.
    Ok {
        /// The truncated trie handle.
        handle: Box<TruncatedTrieHandle>,
        /// The root hash of the truncated trie.
        root_hash: HashKey,
    },

    /// An error occurred.
    ///
    /// The caller must call [`fwd_free_owned_bytes`](crate::fwd_free_owned_bytes)
    /// to free the memory associated with this error.
    Err(OwnedBytes),
}

/// A result type returned from FFI functions that create a [`WitnessProofHandle`].
#[derive(Debug)]
#[repr(C, usize)]
pub enum WitnessResult {
    /// The caller provided a null pointer to the input handle.
    NullHandlePointer,

    /// The witness proof was successfully generated.
    ///
    /// The caller must call [`fwd_free_witness_proof`] to free the handle.
    Ok(Box<WitnessProofHandle>),

    /// An error occurred.
    ///
    /// The caller must call [`fwd_free_owned_bytes`](crate::fwd_free_owned_bytes)
    /// to free the memory associated with this error.
    Err(OwnedBytes),
}

use crate::value::results::{CResult, NullHandleResult};

impl NullHandleResult for TruncatedTrieResult {
    fn null_handle_pointer_error() -> Self {
        Self::NullHandlePointer
    }
}

impl CResult for TruncatedTrieResult {
    fn from_err(err: impl ToString) -> Self {
        Self::Err(err.to_string().into_bytes().into())
    }
}

impl NullHandleResult for WitnessResult {
    fn null_handle_pointer_error() -> Self {
        Self::NullHandlePointer
    }
}

impl CResult for WitnessResult {
    fn from_err(err: impl ToString) -> Self {
        Self::Err(err.to_string().into_bytes().into())
    }
}

// -- FFI functions --

/// Creates a [`TruncatedTrieHandle`] from the database revision matching the
/// given root hash.
///
/// The truncated trie holds the top `depth` nibble levels of the trie with
/// children below that depth replaced by hash-only proxy nodes.
///
/// # Arguments
///
/// * `db` - The database handle returned by [`fwd_open_db`](crate::fwd_open_db)
/// * `root_hash` - The root hash of the revision to truncate
/// * `depth` - The truncation depth in nibble levels
///
/// # Returns
///
/// - [`TruncatedTrieResult::NullHandlePointer`] if `db` is null.
/// - [`TruncatedTrieResult::Ok`] with the handle and root hash on success.
/// - [`TruncatedTrieResult::Err`] if the revision was not found or truncation
///   failed.
///
/// # Safety
///
/// The caller must:
/// * ensure that `db` is a valid pointer to a [`DatabaseHandle`].
/// * call [`fwd_free_truncated_trie`] to free the returned handle.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_create_truncated_trie(
    db: Option<&DatabaseHandle>,
    root_hash: HashKey,
    depth: usize,
) -> TruncatedTrieResult {
    crate::invoke_with_handle(db, move |db| -> Result<_, String> {
        let api_hash: firewood::v2::api::HashKey = root_hash.into();
        let revision = db.db().revision(api_hash).map_err(|e| e.to_string())?;
        let trie = TruncatedTrie::from_trie(revision.as_ref(), depth).map_err(|e| e.to_string())?;
        let hash = trie
            .root_hash()
            .cloned()
            .map(HashKey::from)
            .unwrap_or_default();
        Ok((trie, hash))
    })
}

impl From<Result<(TruncatedTrie, HashKey), String>> for TruncatedTrieResult {
    fn from(value: Result<(TruncatedTrie, HashKey), String>) -> Self {
        match value {
            Ok((trie, hash)) => TruncatedTrieResult::Ok {
                handle: Box::new(TruncatedTrieHandle { trie }),
                root_hash: hash,
            },
            Err(err) => TruncatedTrieResult::Err(err.into_bytes().into()),
        }
    }
}

/// Returns the root hash of the truncated trie.
///
/// # Arguments
///
/// * `handle` - The truncated trie handle
///
/// # Returns
///
/// - [`HashResult::NullHandlePointer`] if `handle` is null.
/// - [`HashResult::Some`] with the root hash.
/// - [`HashResult::None`] if the trie is empty.
///
/// # Safety
///
/// The caller must ensure that `handle` is a valid pointer to a
/// [`TruncatedTrieHandle`].
#[unsafe(no_mangle)]
pub extern "C" fn fwd_truncated_trie_root_hash(handle: Option<&TruncatedTrieHandle>) -> HashResult {
    crate::invoke_with_handle(handle, |h| h.trie.root_hash().cloned().map(HashKey::from))
}

impl From<Option<HashKey>> for HashResult {
    fn from(value: Option<HashKey>) -> Self {
        match value {
            Some(hash) => HashResult::Some(hash),
            None => HashResult::None,
        }
    }
}

/// Verifies the truncated trie's root hash against an expected value.
///
/// # Arguments
///
/// * `handle` - The truncated trie handle
/// * `expected_hash` - The expected root hash
///
/// # Returns
///
/// - [`VoidResult::NullHandlePointer`] if `handle` is null.
/// - [`VoidResult::Ok`] if the hash matches.
/// - [`VoidResult::Err`] if the hash does not match.
///
/// # Safety
///
/// The caller must ensure that `handle` is a valid pointer to a
/// [`TruncatedTrieHandle`].
#[unsafe(no_mangle)]
pub extern "C" fn fwd_verify_truncated_trie_root_hash(
    handle: Option<&TruncatedTrieHandle>,
    expected_hash: HashKey,
) -> VoidResult {
    crate::invoke_with_handle(handle, move |h| -> Result<(), String> {
        let api_hash: firewood_storage::TrieHash = expected_hash.into();
        if h.trie.verify_root_hash(&api_hash) {
            Ok(())
        } else {
            Err("root hash mismatch".into())
        }
    })
}

/// Generates a witness proof for a set of batch operations.
///
/// Walks the old trie (identified by `root_hash`) along each key's path and
/// collects all nodes below `depth` that the client would need to replay the
/// operations.
///
/// # Arguments
///
/// * `db` - The database handle
/// * `root_hash` - The root hash of the old committed revision
/// * `batch_ops` - The batch operations to generate a witness for
/// * `new_root_hash` - The root hash after applying the batch ops
/// * `depth` - The client's truncation depth in nibble levels
///
/// # Returns
///
/// - [`WitnessResult::NullHandlePointer`] if `db` is null.
/// - [`WitnessResult::Ok`] with the witness proof handle on success.
/// - [`WitnessResult::Err`] on failure.
///
/// # Safety
///
/// The caller must:
/// * ensure that `db` is a valid pointer to a [`DatabaseHandle`].
/// * ensure that `batch_ops` is valid for [`BorrowedBatchOps`].
/// * call [`fwd_free_witness_proof`] to free the returned handle.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_generate_witness(
    db: Option<&DatabaseHandle>,
    root_hash: HashKey,
    batch_ops: BorrowedBatchOps<'_>,
    new_root_hash: HashKey,
    depth: usize,
) -> WitnessResult {
    crate::invoke_with_handle(db, move |db| -> Result<WitnessProof, String> {
        let api_hash: firewood::v2::api::HashKey = root_hash.into();
        let revision = db.db().revision(api_hash).map_err(|e| e.to_string())?;

        // Convert FFI batch ops to core BatchOps (including DeleteRange)
        let core_ops: Vec<witness::OwnedBatchOp> = batch_ops
            .as_slice()
            .iter()
            .map(|op| match op {
                crate::BatchOp::Put { key, value } => CoreBatchOp::Put {
                    key: key.as_slice().into(),
                    value: value.as_slice().into(),
                },
                crate::BatchOp::Delete { key } => CoreBatchOp::Delete {
                    key: key.as_slice().into(),
                },
                crate::BatchOp::DeleteRange { prefix } => CoreBatchOp::DeleteRange {
                    prefix: prefix.as_slice().into(),
                },
            })
            .collect();

        // Expand DeleteRange ops into individual Delete ops using the revision
        let remote_ops = witness::expand_delete_ranges(revision.as_ref(), &core_ops)
            .map_err(|e| e.to_string())?;

        let new_hash: firewood_storage::TrieHash = new_root_hash.into();

        witness::generate_witness(revision.as_ref(), &remote_ops, new_hash, depth)
            .map_err(|e| e.to_string())
    })
}

impl From<Result<WitnessProof, String>> for WitnessResult {
    fn from(value: Result<WitnessProof, String>) -> Self {
        match value {
            Ok(proof) => WitnessResult::Ok(Box::new(WitnessProofHandle { proof })),
            Err(err) => WitnessResult::Err(err.into_bytes().into()),
        }
    }
}

/// Verifies a witness proof against a truncated trie and returns the updated
/// trie.
///
/// First validates that the witness proof's embedded `batch_ops` are consistent
/// with `expected_ops` (accounting for `DeleteRange` expansion), then
/// re-executes the batch operations on top of the client's truncated trie,
/// hashes the result, and verifies it matches the witness's `new_root_hash`.
///
/// # Arguments
///
/// * `trie_handle` - The client's truncated trie handle
/// * `witness_handle` - The witness proof handle
/// * `expected_ops` - The batch operations the client originally sent
///
/// # Returns
///
/// - [`TruncatedTrieResult::NullHandlePointer`] if either handle is null.
/// - [`TruncatedTrieResult::Ok`] with the updated trie handle on success.
/// - [`TruncatedTrieResult::Err`] if verification fails.
///
/// # Safety
///
/// The caller must:
/// * ensure that both handles are valid pointers.
/// * ensure that `expected_ops` is valid for [`BorrowedBatchOps`].
/// * call [`fwd_free_truncated_trie`] to free the returned handle.
/// * The old trie handle is NOT consumed; the caller must still free it.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_verify_witness(
    trie_handle: Option<&TruncatedTrieHandle>,
    witness_handle: Option<&WitnessProofHandle>,
    expected_ops: BorrowedBatchOps<'_>,
) -> TruncatedTrieResult {
    crate::invoke_with_handle(trie_handle, move |trie_h| -> Result<_, String> {
        let witness_h = witness_handle.ok_or("null witness handle")?;

        // Convert FFI batch ops to OwnedBatchOp (including DeleteRange)
        let owned_ops: Vec<witness::OwnedBatchOp> = expected_ops
            .as_slice()
            .iter()
            .map(|op| match op {
                crate::BatchOp::Put { key, value } => CoreBatchOp::Put {
                    key: key.as_slice().into(),
                    value: value.as_slice().into(),
                },
                crate::BatchOp::Delete { key } => CoreBatchOp::Delete {
                    key: key.as_slice().into(),
                },
                crate::BatchOp::DeleteRange { prefix } => CoreBatchOp::DeleteRange {
                    prefix: prefix.as_slice().into(),
                },
            })
            .collect();

        let new_trie = witness::verify_witness(&trie_h.trie, &witness_h.proof, &owned_ops)
            .map_err(|e| e.to_string())?;

        let hash = new_trie
            .root_hash()
            .cloned()
            .map(HashKey::from)
            .unwrap_or_default();

        Ok((new_trie, hash))
    })
}

/// A result type for `fwd_get_with_proof` containing both value and proof bytes.
#[derive(Debug)]
#[repr(C, usize)]
pub enum GetWithProofResult {
    /// The caller provided a null pointer to the input handle.
    NullHandlePointer,

    /// The key was found and value + proof are returned.
    ///
    /// The caller must call [`fwd_free_owned_bytes`](crate::fwd_free_owned_bytes)
    /// to free both the value and proof.
    Ok {
        /// The value associated with the key.
        value: OwnedBytes,
        /// The serialized single-key proof.
        proof: OwnedBytes,
    },

    /// The key was not found, but a proof of absence is returned.
    ///
    /// The caller must call [`fwd_free_owned_bytes`](crate::fwd_free_owned_bytes)
    /// to free the proof.
    NotFound {
        /// The serialized exclusion proof.
        proof: OwnedBytes,
    },

    /// An error occurred.
    ///
    /// The caller must call [`fwd_free_owned_bytes`](crate::fwd_free_owned_bytes)
    /// to free the error.
    Err(OwnedBytes),
}

impl NullHandleResult for GetWithProofResult {
    fn null_handle_pointer_error() -> Self {
        Self::NullHandlePointer
    }
}

impl CResult for GetWithProofResult {
    fn from_err(err: impl ToString) -> Self {
        Self::Err(err.to_string().into_bytes().into())
    }
}

// -- Serialization FFI functions --

/// Serializes a [`WitnessProofHandle`] to bytes.
///
/// # Safety
///
/// The caller must:
/// * ensure that `handle` is a valid pointer to a [`WitnessProofHandle`].
/// * call [`fwd_free_owned_bytes`](crate::fwd_free_owned_bytes) to free the
///   returned bytes.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_witness_proof_to_bytes(handle: Option<&WitnessProofHandle>) -> ValueResult {
    crate::invoke_with_handle(handle, |h| -> Result<Option<Box<[u8]>>, String> {
        let mut buf = Vec::new();
        h.proof.write_to_vec(&mut buf);
        Ok(Some(buf.into_boxed_slice()))
    })
}

impl From<Result<Option<Box<[u8]>>, String>> for ValueResult {
    fn from(value: Result<Option<Box<[u8]>>, String>) -> Self {
        match value {
            Ok(None) => ValueResult::None,
            Ok(Some(data)) => ValueResult::Some(data.into()),
            Err(err) => ValueResult::Err(err.into_bytes().into()),
        }
    }
}

/// Deserializes bytes into a [`WitnessProofHandle`].
///
/// # Safety
///
/// The caller must:
/// * ensure that `bytes` is valid for [`BorrowedBytes`].
/// * call [`fwd_free_witness_proof`] to free the returned handle.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_witness_proof_from_bytes(bytes: BorrowedBytes<'_>) -> WitnessResult {
    crate::invoke(move || -> Result<WitnessProof, String> {
        WitnessProof::from_slice(bytes.as_slice()).map_err(|e| e.to_string())
    })
}

/// Serializes a [`TruncatedTrieHandle`] to bytes.
///
/// # Safety
///
/// The caller must:
/// * ensure that `handle` is a valid pointer to a [`TruncatedTrieHandle`].
/// * call [`fwd_free_owned_bytes`](crate::fwd_free_owned_bytes) to free the
///   returned bytes.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_truncated_trie_to_bytes(handle: Option<&TruncatedTrieHandle>) -> ValueResult {
    crate::invoke_with_handle(handle, |h| -> Result<Option<Box<[u8]>>, String> {
        let mut buf = Vec::new();
        h.trie.write_to_vec(&mut buf);
        Ok(Some(buf.into_boxed_slice()))
    })
}

/// Deserializes bytes into a [`TruncatedTrieHandle`].
///
/// # Safety
///
/// The caller must:
/// * ensure that `bytes` is valid for [`BorrowedBytes`].
/// * call [`fwd_free_truncated_trie`] to free the returned handle.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_truncated_trie_from_bytes(bytes: BorrowedBytes<'_>) -> TruncatedTrieResult {
    crate::invoke(move || -> Result<(TruncatedTrie, HashKey), String> {
        let trie = TruncatedTrie::from_slice(bytes.as_slice()).map_err(|e| e.to_string())?;
        let hash = trie
            .root_hash()
            .cloned()
            .map(HashKey::from)
            .unwrap_or_default();
        Ok((trie, hash))
    })
}

/// Gets a value and its single-key Merkle proof from the revision identified
/// by `root_hash`.
///
/// # Safety
///
/// The caller must:
/// * ensure that `db` is a valid pointer to a [`DatabaseHandle`].
/// * ensure that `key` is valid for [`BorrowedBytes`].
/// * call [`fwd_free_owned_bytes`](crate::fwd_free_owned_bytes) to free
///   the returned value and proof bytes.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_get_with_proof(
    db: Option<&DatabaseHandle>,
    root_hash: HashKey,
    key: BorrowedBytes<'_>,
) -> GetWithProofResult {
    crate::invoke_with_handle(db, move |db| -> Result<_, String> {
        let view = db.get_root(root_hash.into()).map_err(|e| e.to_string())?;

        // Generate proof
        let proof = view
            .single_key_proof(key.as_slice())
            .map_err(|e| e.to_string())?;

        // Get value
        let value = view.val(key.as_slice()).map_err(|e| e.to_string())?;

        // Serialize proof
        let mut proof_bytes = Vec::new();
        proof.write_to_vec(&mut proof_bytes);

        Ok((value, proof_bytes))
    })
}

impl From<Result<(Option<Box<[u8]>>, Vec<u8>), String>> for GetWithProofResult {
    fn from(value: Result<(Option<Box<[u8]>>, Vec<u8>), String>) -> Self {
        match value {
            Ok((Some(val), proof)) => GetWithProofResult::Ok {
                value: val.into(),
                proof: proof.into_boxed_slice().into(),
            },
            Ok((None, proof)) => GetWithProofResult::NotFound {
                proof: proof.into_boxed_slice().into(),
            },
            Err(err) => GetWithProofResult::Err(err.into_bytes().into()),
        }
    }
}

/// Verifies a single-key proof against a known root hash.
///
/// No database handle is needed; verification is purely cryptographic.
///
/// # Arguments
///
/// * `root_hash` - The expected root hash
/// * `key` - The key that was proven
/// * `value` - The value (may be empty for exclusion proofs)
/// * `value_is_present` - `true` if the value is an inclusion proof, `false`
///   for exclusion
/// * `proof` - The serialized single-key proof
///
/// # Safety
///
/// The caller must ensure all borrowed byte slices are valid.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_verify_single_key_proof(
    root_hash: HashKey,
    key: BorrowedBytes<'_>,
    value: BorrowedBytes<'_>,
    value_is_present: bool,
    proof_bytes: BorrowedBytes<'_>,
) -> VoidResult {
    crate::invoke(move || -> Result<(), String> {
        let api_hash: firewood_storage::TrieHash = root_hash.into();
        let proof = firewood::v2::api::FrozenProof::from_slice(proof_bytes.as_slice())
            .map_err(|e| e.to_string())?;

        let expected_value: Option<&[u8]> = if value_is_present {
            Some(value.as_slice())
        } else {
            None
        };

        proof
            .verify(key.as_slice(), expected_value, &api_hash)
            .map_err(|e| e.to_string())
    })
}

/// Frees a [`TruncatedTrieHandle`].
///
/// # Safety
///
/// The caller must ensure that `handle` is a valid pointer returned by a
/// prior call to [`fwd_create_truncated_trie`] or [`fwd_verify_witness`],
/// and that it has not already been freed.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_free_truncated_trie(handle: Option<Box<TruncatedTrieHandle>>) -> VoidResult {
    match handle {
        Some(_) => VoidResult::Ok,
        None => VoidResult::NullHandlePointer,
    }
}

/// Frees a [`WitnessProofHandle`].
///
/// # Safety
///
/// The caller must ensure that `handle` is a valid pointer returned by a
/// prior call to [`fwd_generate_witness`], and that it has not already been
/// freed.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_free_witness_proof(handle: Option<Box<WitnessProofHandle>>) -> VoidResult {
    match handle {
        Some(_) => VoidResult::Ok,
        None => VoidResult::NullHandlePointer,
    }
}
