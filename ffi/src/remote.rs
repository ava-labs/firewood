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
use firewood::remote::cache::{EvictionPolicy, ReadCache};
use firewood::remote::witness::{self, WitnessProof};
// Aliased to avoid collision with `crate::BatchOp` (the FFI C-compatible enum
// defined in `value/kvp.rs`). Both types are used in this module: `crate::BatchOp`
// arrives from C/Go callers and is converted into `CoreBatchOp` for the witness system.
use firewood::v2::api::BatchOp as CoreBatchOp;
use firewood::v2::api::Db as _;
use parking_lot::RwLock;

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
    #[cfg(panic = "unwind")]
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
    #[cfg(panic = "unwind")]
    fn from_err(err: impl ToString) -> Self {
        Self::Err(err.to_string().into_bytes().into())
    }
}

/// Converts FFI batch ops ([`BorrowedBatchOps`]) to core [`witness::OwnedBatchOp`]s.
fn ffi_ops_to_core(ops: &BorrowedBatchOps<'_>) -> Vec<witness::OwnedBatchOp> {
    ops.as_slice()
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
        .collect()
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

        let core_ops = ffi_ops_to_core(&batch_ops);

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

        let owned_ops = ffi_ops_to_core(&expected_ops);

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
    #[cfg(panic = "unwind")]
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

// ---------------------------------------------------------------------------
// RemoteClientHandle — owns committed trie + cache
// ---------------------------------------------------------------------------

/// An opaque handle that owns a committed [`TruncatedTrie`] and an optional
/// [`ReadCache`]. The Go remote client holds one of these instead of managing
/// the trie and cache separately.
///
/// The trie is protected by an [`RwLock`] for interior mutability — reads
/// take a shared lock, bootstrap/commit take an exclusive lock. The cache
/// has its own internal `Mutex`, so no external locking is needed for it.
///
/// Callers must free this handle with [`fwd_free_remote_client`].
pub struct RemoteClientHandle {
    trie: RwLock<TruncatedTrie>,
    cache: Option<ReadCache>,
}

impl std::fmt::Debug for RemoteClientHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemoteClientHandle")
            .field("cache", &self.cache)
            .finish_non_exhaustive()
    }
}

impl MetricsContextExt for &RemoteClientHandle {
    fn metrics_context(&self) -> Option<MetricsContext> {
        None
    }
}

/// C-compatible eviction policy selector for [`ReadCache`].
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub enum CEvictionPolicy {
    /// Least Recently Used — O(1) ops, best general-purpose choice.
    Lru = 0,
    /// Random eviction — O(1) ops, no access-order tracking overhead.
    Random = 1,
    /// Clock (second-chance) — approximates LRU with lower per-access cost.
    Clock = 2,
    /// Sample K random entries, evict the least recently accessed.
    SampleKLru = 3,
}

impl CEvictionPolicy {
    /// Converts to the Rust [`EvictionPolicy`], using `sample_k` for the
    /// `SampleKLru` variant.
    const fn into_policy(self, sample_k: usize) -> EvictionPolicy {
        match self {
            CEvictionPolicy::Lru => EvictionPolicy::Lru,
            CEvictionPolicy::Random => EvictionPolicy::Random,
            CEvictionPolicy::Clock => EvictionPolicy::Clock,
            CEvictionPolicy::SampleKLru => EvictionPolicy::SampleKLru {
                sample_size: sample_k,
            },
        }
    }
}

/// Result type for [`fwd_create_remote_client`].
#[derive(Debug)]
#[repr(C, usize)]
pub enum RemoteClientResult {
    /// The caller provided a null pointer to the input handle.
    NullHandlePointer,

    /// The remote client was successfully created.
    ///
    /// The caller must call [`fwd_free_remote_client`] to free the handle.
    Ok(Box<RemoteClientHandle>),

    /// An error occurred.
    ///
    /// The caller must call [`fwd_free_owned_bytes`](crate::fwd_free_owned_bytes)
    /// to free the memory associated with this error.
    Err(OwnedBytes),
}

impl NullHandleResult for RemoteClientResult {
    fn null_handle_pointer_error() -> Self {
        Self::NullHandlePointer
    }
}

impl CResult for RemoteClientResult {
    #[cfg(panic = "unwind")]
    fn from_err(err: impl ToString) -> Self {
        Self::Err(err.to_string().into_bytes().into())
    }
}

impl From<Result<RemoteClientHandle, String>> for RemoteClientResult {
    fn from(value: Result<RemoteClientHandle, String>) -> Self {
        match value {
            Ok(handle) => RemoteClientResult::Ok(Box::new(handle)),
            Err(err) => RemoteClientResult::Err(err.into_bytes().into()),
        }
    }
}

/// Result type for [`fwd_remote_client_cache_lookup`].
#[derive(Debug)]
#[repr(C, usize)]
pub enum CacheLookupResult {
    /// The caller provided a null pointer to the input handle.
    NullHandlePointer,

    /// The key was not in the cache.
    Miss,

    /// The key was found with a value.
    ///
    /// The caller must call [`fwd_free_owned_bytes`](crate::fwd_free_owned_bytes)
    /// to free the value.
    HitPresent(OwnedBytes),

    /// The key was found but verified absent (exclusion proof was cached).
    HitAbsent,

    /// An error occurred.
    ///
    /// The caller must call [`fwd_free_owned_bytes`](crate::fwd_free_owned_bytes)
    /// to free the error.
    Err(OwnedBytes),
}

impl NullHandleResult for CacheLookupResult {
    fn null_handle_pointer_error() -> Self {
        Self::NullHandlePointer
    }
}

impl CResult for CacheLookupResult {
    #[cfg(panic = "unwind")]
    fn from_err(err: impl ToString) -> Self {
        Self::Err(err.to_string().into_bytes().into())
    }
}

// -- RemoteClientHandle FFI functions --

/// Creates a new [`RemoteClientHandle`] with an optional read cache.
///
/// If `max_cache_bytes` is 0, no cache is created. The trie starts
/// uninitialized (empty); call [`fwd_remote_client_bootstrap`] to set it.
///
/// # Arguments
///
/// * `max_cache_bytes` - Memory budget for the cache in bytes (0 = no cache)
/// * `policy` - Eviction policy for the cache
/// * `sample_k` - Sample size for `SampleKLru` policy (ignored for others)
///
/// # Returns
///
/// - [`RemoteClientResult::Ok`] on success.
/// - [`RemoteClientResult::Err`] if cache creation fails.
///
/// # Safety
///
/// The caller must call [`fwd_free_remote_client`] to free the returned handle.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_create_remote_client(
    max_cache_bytes: usize,
    policy: CEvictionPolicy,
    sample_k: usize,
) -> RemoteClientResult {
    crate::invoke(move || -> Result<RemoteClientHandle, String> {
        let cache = if max_cache_bytes > 0 {
            let rust_policy = policy.into_policy(sample_k);
            Some(ReadCache::new(max_cache_bytes, rust_policy).map_err(|e| e.to_string())?)
        } else {
            None
        };
        Ok(RemoteClientHandle {
            trie: RwLock::new(TruncatedTrie::new(0)),
            cache,
        })
    })
}

/// Frees a [`RemoteClientHandle`].
///
/// # Safety
///
/// The caller must ensure that `client` is a valid pointer returned by a
/// prior call to [`fwd_create_remote_client`], and that it has not already
/// been freed.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_free_remote_client(
    client: Option<Box<RemoteClientHandle>>,
) -> VoidResult {
    match client {
        Some(_) => VoidResult::Ok,
        None => VoidResult::NullHandlePointer,
    }
}

/// Bootstraps the remote client with a serialized trie.
///
/// Deserializes `trie_bytes` into a [`TruncatedTrie`], verifies the root
/// hash matches `expected_hash`, write-locks the internal trie and swaps it
/// in, then clears the cache.
///
/// # Returns
///
/// - [`HashResult::NullHandlePointer`] if `client` is null.
/// - [`HashResult::Some`] with the root hash on success.
/// - [`HashResult::Err`] on deserialization or hash mismatch.
///
/// # Safety
///
/// The caller must ensure that `client` is a valid pointer and that
/// `trie_bytes` is valid for [`BorrowedBytes`].
#[unsafe(no_mangle)]
pub extern "C" fn fwd_remote_client_bootstrap(
    client: Option<&RemoteClientHandle>,
    trie_bytes: BorrowedBytes<'_>,
    expected_hash: HashKey,
) -> HashResult {
    crate::invoke_with_handle(client, move |c| -> Result<Option<firewood_storage::TrieHash>, String> {
        let new_trie =
            TruncatedTrie::from_slice(trie_bytes.as_slice()).map_err(|e| e.to_string())?;

        let api_hash: firewood_storage::TrieHash = expected_hash.into();
        if !new_trie.verify_root_hash(&api_hash) {
            return Err("root hash mismatch".into());
        }

        let root_hash = new_trie.root_hash().cloned();

        let mut trie_guard = c.trie.write();
        *trie_guard = new_trie;
        drop(trie_guard);

        if let Some(cache) = &c.cache {
            cache.clear();
        }

        Ok(root_hash)
    })
}

/// Looks up a key in the remote client's cache.
///
/// # Returns
///
/// - [`CacheLookupResult::NullHandlePointer`] if `client` is null.
/// - [`CacheLookupResult::Miss`] if the key is not cached or no cache is
///   configured.
/// - [`CacheLookupResult::HitPresent`] with the value if cached as present.
/// - [`CacheLookupResult::HitAbsent`] if cached as absent.
///
/// # Safety
///
/// The caller must ensure that `client` is a valid pointer and that `key`
/// is valid for [`BorrowedBytes`].
#[unsafe(no_mangle)]
pub extern "C" fn fwd_remote_client_cache_lookup(
    client: Option<&RemoteClientHandle>,
    key: BorrowedBytes<'_>,
) -> CacheLookupResult {
    let Some(c) = client else {
        return CacheLookupResult::NullHandlePointer;
    };
    let Some(cache) = &c.cache else {
        return CacheLookupResult::Miss;
    };
    match cache.lookup(key.as_slice()) {
        Some(entry) => match entry.into_value() {
            Some(value) => CacheLookupResult::HitPresent(value.into()),
            None => CacheLookupResult::HitAbsent,
        },
        None => CacheLookupResult::Miss,
    }
}


/// Verifies a single-key proof against the remote client's committed root
/// hash and, on success, stores the result in the cache.
///
/// # Arguments
///
/// * `client` - The remote client handle
/// * `key` - The key that was proven
/// * `value` - The value (may be empty for exclusion proofs)
/// * `value_is_present` - `true` for inclusion proof, `false` for exclusion
/// * `proof_bytes` - The serialized single-key proof
///
/// # Returns
///
/// - [`VoidResult::NullHandlePointer`] if `client` is null.
/// - [`VoidResult::Ok`] if verification succeeds.
/// - [`VoidResult::Err`] on verification failure or if not bootstrapped.
///
/// # Safety
///
/// The caller must ensure all pointers and borrowed byte slices are valid.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_remote_client_verify_get(
    client: Option<&RemoteClientHandle>,
    key: BorrowedBytes<'_>,
    value: BorrowedBytes<'_>,
    value_is_present: bool,
    proof_bytes: BorrowedBytes<'_>,
) -> VoidResult {
    crate::invoke_with_handle(client, move |c| -> Result<(), String> {
        let trie_guard = c.trie.read();

        let root_hash = trie_guard
            .root_hash()
            .ok_or_else(|| "client not bootstrapped".to_string())?;

        let proof = firewood::v2::api::FrozenProof::from_slice(proof_bytes.as_slice())
            .map_err(|e| e.to_string())?;

        let expected_value: Option<&[u8]> = if value_is_present {
            Some(value.as_slice())
        } else {
            None
        };

        proof
            .verify(key.as_slice(), expected_value, root_hash)
            .map_err(|e| e.to_string())?;

        drop(trie_guard);

        // Cache the verified result.
        if let Some(cache) = &c.cache {
            let owned_value: Option<Box<[u8]>> = if value_is_present {
                Some(Box::from(value.as_slice()))
            } else {
                None
            };
            cache.store(key.as_slice(), owned_value);
        }

        Ok(())
    })
}

/// Verifies a witness proof against the remote client's committed trie and
/// returns a new [`TruncatedTrieHandle`] for the proposal.
///
/// The witness handle is NOT consumed — Go keeps it alive for
/// [`fwd_remote_client_commit_trie`].
///
/// # Returns
///
/// - [`TruncatedTrieResult::NullHandlePointer`] if either handle is null.
/// - [`TruncatedTrieResult::Ok`] with the updated trie handle on success.
/// - [`TruncatedTrieResult::Err`] if verification fails or not bootstrapped.
///
/// # Safety
///
/// The caller must ensure all handles and borrowed data are valid.
/// The returned trie handle must be freed with [`fwd_free_truncated_trie`].
#[unsafe(no_mangle)]
pub extern "C" fn fwd_remote_client_verify_witness(
    client: Option<&RemoteClientHandle>,
    witness_handle: Option<&WitnessProofHandle>,
    expected_ops: BorrowedBatchOps<'_>,
) -> TruncatedTrieResult {
    crate::invoke_with_handle(client, move |c| -> Result<(TruncatedTrie, HashKey), String> {
        let witness_h = witness_handle.ok_or("null witness handle")?;

        let trie_guard = c.trie.read();

        if trie_guard.root_hash().is_none() {
            return Err("client not bootstrapped".into());
        }

        let owned_ops = ffi_ops_to_core(&expected_ops);

        let new_trie = witness::verify_witness(&trie_guard, &witness_h.proof, &owned_ops)
            .map_err(|e| e.to_string())?;

        let hash = new_trie
            .root_hash()
            .cloned()
            .map(HashKey::from)
            .unwrap_or_default();

        Ok((new_trie, hash))
    })
}

/// Commits a new trie into the remote client handle and invalidates cache
/// entries affected by the witness's batch operations.
///
/// Takes ownership of `new_trie` (it is consumed). The witness handle is
/// borrowed to extract its `batch_ops` for cache invalidation, then left
/// alone (Go is responsible for freeing it).
///
/// # Returns
///
/// - [`HashResult::NullHandlePointer`] if `client` or `new_trie` is null.
/// - [`HashResult::Some`] with the new root hash on success.
/// - [`HashResult::Err`] on failure.
///
/// # Safety
///
/// The caller must ensure all handles are valid. `new_trie` is consumed
/// and must not be used after this call.
#[unsafe(no_mangle)]
pub extern "C" fn fwd_remote_client_commit_trie(
    client: Option<&RemoteClientHandle>,
    new_trie: Option<Box<TruncatedTrieHandle>>,
    witness: Option<&WitnessProofHandle>,
) -> HashResult {
    crate::invoke_with_handle(client, move |c| -> Result<Option<firewood_storage::TrieHash>, String> {
        let trie_handle = new_trie.ok_or("null new_trie handle")?;
        let witness_h = witness.ok_or("null witness handle")?;

        let root_hash = trie_handle.trie.root_hash().cloned();

        let mut trie_guard = c.trie.write();
        *trie_guard = trie_handle.trie;
        drop(trie_guard);

        // Invalidate cache entries affected by the witness's batch ops.
        if let Some(cache) = &c.cache {
            // Convert WitnessProof batch_ops (ClientOp) to the slice expected
            // by ReadCache::invalidate_batch.
            cache.invalidate_batch(&witness_h.proof.batch_ops);
        }

        Ok(root_hash)
    })
}
