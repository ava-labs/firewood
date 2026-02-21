// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Replay log types and engine for applying them to a Database.
//!
//! This crate provides:
//! - Serializable types representing database operations ([`DbOperation`])
//! - An engine to replay operations against a [`Db`] instance
//!
//! The log format uses length-prefixed `MessagePack` segments, allowing streaming
//! writes and reads without loading the entire log into memory.
//!
//! # Wire Format
//!
//! Each segment is written as:
//! ```text
//! [len: u64 little-endian][msgpack-serialized ReplayLog bytes]
//! ```
//!
//! A file may contain multiple segments appended sequentially.

use std::collections::HashMap;
use std::io::{self, Read};
use std::time::Instant;

use firewood::db::{BatchOp, Db, Proposal};
use firewood::v2::api::{
    self, ArcDynDbView, Db as DbApi, DbView as DbViewApi, Proposal as ProposalApi,
};
use firewood_metrics::firewood_increment;
use firewood_storage::InvalidTrieHashLength;

pub mod registry;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use thiserror::Error;

/// Strongly-typed proposal identifier.
///
/// Wraps a `u64` to prevent accidental misuse of raw integers as proposal IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProposalId(pub u64);

/// Strongly-typed revision identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RevisionId(pub u64);

/// Strongly-typed iterator identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IteratorId(pub u64);

/// Optional timing metadata for a recorded operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationMetadata {
    /// Duration of the FFI call in nanoseconds.
    pub duration_ns: u64,
}

/// Summary of FFI value-returning calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValueResultData {
    /// Null handle pointer was provided.
    NullHandlePointer,
    /// Revision was not found.
    #[serde(with = "serde_bytes")]
    RevisionNotFound(Box<[u8]>),
    /// Value was not found.
    None,
    /// Value was found.
    #[serde(with = "serde_bytes")]
    Some(Box<[u8]>),
    /// Error occurred.
    Err,
}

/// Summary of FFI hash-returning calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HashResultData {
    /// Null handle pointer was provided.
    NullHandlePointer,
    /// No hash is available.
    None,
    /// Hash was returned.
    #[serde(with = "serde_bytes")]
    Some(Box<[u8]>),
    /// Error occurred.
    Err,
}

/// Summary of FFI proposal-create calls.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProposalResultData {
    /// Null handle pointer was provided.
    NullHandlePointer,
    /// Proposal was created successfully.
    Ok {
        /// Returned proposal root hash.
        #[serde_as(as = "Option<serde_with::Bytes>")]
        root_hash: Option<Box<[u8]>>,
    },
    /// Error occurred.
    Err,
}

/// Summary of FFI revision-get calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RevisionResultData {
    /// Null handle pointer was provided.
    NullHandlePointer,
    /// Revision was not found.
    #[serde(with = "serde_bytes")]
    RevisionNotFound(Box<[u8]>),
    /// Revision was returned successfully.
    #[serde(with = "serde_bytes")]
    Ok(Box<[u8]>),
    /// Error occurred.
    Err,
}

/// Summary of FFI iterator-create calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IteratorResultData {
    /// Null handle pointer was provided.
    NullHandlePointer,
    /// Iterator was created successfully.
    Ok,
    /// Error occurred.
    Err,
}

/// A key/value pair returned by an iterator operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyValueRead {
    /// The returned key.
    #[serde(with = "serde_bytes")]
    pub key: Box<[u8]>,
    /// The returned value.
    #[serde(with = "serde_bytes")]
    pub value: Box<[u8]>,
}

/// Summary of `iter_next` calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyValueResultData {
    /// Null handle pointer was provided.
    NullHandlePointer,
    /// Iterator is exhausted.
    None,
    /// Iterator returned a key/value pair.
    Some(KeyValueRead),
    /// Error occurred.
    Err,
}

/// Summary of `iter_next_n` calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum KeyValueBatchResultData {
    /// Null handle pointer was provided.
    NullHandlePointer,
    /// Iterator returned a batch of key/value pairs.
    Some(Vec<KeyValueRead>),
    /// Error occurred.
    Err,
}

/// Summary of void-returning calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VoidResultData {
    /// Null handle pointer was provided.
    NullHandlePointer,
    /// Operation completed successfully.
    Ok,
    /// Error occurred.
    Err,
}

/// Operation that reads a key from the latest committed revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetLatest {
    /// The key to read.
    #[serde(with = "serde_bytes")]
    pub key: Box<[u8]>,
    /// Optional captured FFI result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ValueResultData>,
    /// Optional timing metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

/// Operation that reads a key from an uncommitted proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetFromProposal {
    /// The proposal ID assigned during recording.
    pub proposal_id: ProposalId,
    /// The key to read.
    #[serde(with = "serde_bytes")]
    pub key: Box<[u8]>,
    /// Optional captured FFI result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ValueResultData>,
    /// Optional timing metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

/// Operation that gets a revision from a root hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetRevision {
    /// Requested revision root hash.
    #[serde(with = "serde_bytes")]
    pub root: Box<[u8]>,
    /// Revision ID assigned to the returned handle, if successful.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returned_revision_id: Option<RevisionId>,
    /// Optional captured FFI result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<RevisionResultData>,
    /// Optional timing metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

/// Operation that reads a key from a revision handle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetFromRevision {
    /// The revision ID assigned during recording.
    pub revision_id: RevisionId,
    /// The key to read.
    #[serde(with = "serde_bytes")]
    pub key: Box<[u8]>,
    /// Optional captured FFI result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ValueResultData>,
    /// Optional timing metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

/// A single key/value mutation within a batch or proposal.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyValueOp {
    /// The key being mutated.
    #[serde(with = "serde_bytes")]
    pub key: Box<[u8]>,
    /// The value to set, or `None` for delete/delete-range operations.
    #[serde_as(as = "Option<serde_with::Bytes>")]
    pub value: Option<Box<[u8]>>,
    /// If true and `value` is `None`, this represents a single-key delete.
    ///
    /// If false and `value` is `None`, this represents a delete-range by prefix.
    /// When this field is missing in old logs, replay defaults to delete-range.
    #[serde(default)]
    pub delete_exact: bool,
}

/// Batch operation that commits immediately.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Batch {
    /// The key/value operations in this batch.
    pub pairs: Vec<KeyValueOp>,
    /// Optional captured FFI result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<HashResultData>,
    /// Optional timing metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

/// Proposal created directly on the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposeOnDB {
    /// The key/value operations in this proposal.
    pub pairs: Vec<KeyValueOp>,
    /// The proposal ID assigned to the returned handle, if successful.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returned_proposal_id: Option<ProposalId>,
    /// Optional captured FFI result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ProposalResultData>,
    /// Optional timing metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

/// Proposal created on top of an existing uncommitted proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposeOnProposal {
    /// The parent proposal ID.
    pub proposal_id: ProposalId,
    /// The key/value operations in this proposal.
    pub pairs: Vec<KeyValueOp>,
    /// The proposal ID assigned to the returned handle, if successful.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returned_proposal_id: Option<ProposalId>,
    /// Optional captured FFI result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ProposalResultData>,
    /// Optional timing metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

/// Commit operation for a proposal.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    /// The proposal ID being committed.
    pub proposal_id: ProposalId,
    /// The root hash returned by the commit, if any.
    #[serde_as(as = "Option<serde_with::Bytes>")]
    pub returned_hash: Option<Box<[u8]>>,
    /// Optional captured FFI result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<HashResultData>,
    /// Optional timing metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

/// Operation that reads the latest root hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootHash {
    /// Optional captured FFI result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<HashResultData>,
    /// Optional timing metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

/// Operation that opens an iterator on a revision.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterOnRevision {
    /// The revision ID used as iterator source.
    pub revision_id: RevisionId,
    /// Optional iterator start key.
    #[serde_as(as = "Option<serde_with::Bytes>")]
    pub start_key: Option<Box<[u8]>>,
    /// Iterator ID assigned to the returned handle, if successful.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returned_iterator_id: Option<IteratorId>,
    /// Optional captured FFI result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<IteratorResultData>,
    /// Optional timing metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

/// Operation that opens an iterator on a proposal.
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterOnProposal {
    /// The proposal ID used as iterator source.
    pub proposal_id: ProposalId,
    /// Optional iterator start key.
    #[serde_as(as = "Option<serde_with::Bytes>")]
    pub start_key: Option<Box<[u8]>>,
    /// Iterator ID assigned to the returned handle, if successful.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returned_iterator_id: Option<IteratorId>,
    /// Optional captured FFI result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<IteratorResultData>,
    /// Optional timing metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

/// Operation that advances an iterator once.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterNext {
    /// The iterator ID to advance.
    pub iterator_id: IteratorId,
    /// Optional captured FFI result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<KeyValueResultData>,
    /// Optional timing metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

/// Operation that advances an iterator by up to `n` entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IterNextN {
    /// The iterator ID to advance.
    pub iterator_id: IteratorId,
    /// Maximum number of entries requested.
    pub n: usize,
    /// Optional captured FFI result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<KeyValueBatchResultData>,
    /// Optional timing metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

/// Operation that frees a proposal handle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreeProposal {
    /// The proposal ID being freed.
    pub proposal_id: ProposalId,
    /// Optional captured FFI result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<VoidResultData>,
    /// Optional timing metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

/// Operation that frees a revision handle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreeRevision {
    /// The revision ID being freed.
    pub revision_id: RevisionId,
    /// Optional captured FFI result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<VoidResultData>,
    /// Optional timing metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

/// Operation that frees an iterator handle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreeIterator {
    /// The iterator ID being freed.
    pub iterator_id: IteratorId,
    /// Optional captured FFI result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<VoidResultData>,
    /// Optional timing metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<OperationMetadata>,
}

/// All supported database operations that can be recorded and replayed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DbOperation {
    /// Read from the latest revision.
    GetLatest(GetLatest),
    /// Read from an uncommitted proposal.
    GetFromProposal(GetFromProposal),
    /// Open a revision handle.
    GetRevision(GetRevision),
    /// Read from a revision handle.
    GetFromRevision(GetFromRevision),
    /// Read the latest root hash.
    RootHash(RootHash),
    /// Batch write (immediate commit).
    Batch(Batch),
    /// Create a proposal on the database.
    ProposeOnDB(ProposeOnDB),
    /// Create a proposal on another proposal.
    ProposeOnProposal(ProposeOnProposal),
    /// Open an iterator from a revision.
    IterOnRevision(IterOnRevision),
    /// Open an iterator from a proposal.
    IterOnProposal(IterOnProposal),
    /// Advance an iterator by one entry.
    IterNext(IterNext),
    /// Advance an iterator by up to `n` entries.
    IterNextN(IterNextN),
    /// Commit a proposal.
    Commit(Commit),
    /// Free a proposal handle.
    FreeProposal(FreeProposal),
    /// Free a revision handle.
    FreeRevision(FreeRevision),
    /// Free an iterator handle.
    FreeIterator(FreeIterator),
}

/// A container for a sequence of database operations.
///
/// Multiple `ReplayLog` instances can be serialized sequentially into a file,
/// each prefixed with its byte length.
#[derive(Debug, Serialize, Deserialize)]
pub struct ReplayLog {
    /// The operations in this log segment.
    pub operations: Vec<DbOperation>,
}

impl ReplayLog {
    /// Creates a new replay log with the given operations.
    #[must_use]
    pub const fn new(operations: Vec<DbOperation>) -> Self {
        Self { operations }
    }

    /// Serializes the log to `MessagePack` format with named fields.
    ///
    /// Uses named field serialization for Go compatibility.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization fails.
    pub fn to_msgpack(&self) -> Result<Vec<u8>, rmp_serde::encode::Error> {
        rmp_serde::to_vec_named(self)
    }
}

/// Errors that can occur when replaying a log against a database.
#[derive(Debug, Error)]
pub enum ReplayError {
    /// An I/O error occurred while reading the replay log.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// The log segment could not be deserialized.
    #[error("failed to decode replay segment: {0}")]
    Decode(#[from] rmp_serde::decode::Error),

    /// A database error occurred while applying an operation.
    #[error("database error: {0}")]
    Db(#[from] api::Error),

    /// A root hash in the log had an invalid length.
    #[error("invalid root hash: {0}")]
    InvalidHash(#[from] InvalidTrieHashLength),

    /// The log referenced a proposal ID that was not created or already consumed.
    #[error("unknown proposal ID {}", .0.0)]
    UnknownProposal(ProposalId),

    /// The log referenced a revision ID that was not created or already consumed.
    #[error("unknown revision ID {}", .0.0)]
    UnknownRevision(RevisionId),

    /// The log referenced an iterator ID that was not created or already consumed.
    #[error("unknown iterator ID {}", .0.0)]
    UnknownIterator(IteratorId),
}

/// Alias for the batch operation type used in replay.
type BoxedBatchOp = BatchOp<Box<[u8]>, Box<[u8]>>;

/// Consumes a `Vec` of [`KeyValueOp`] and converts them to firewood batch operations.
///
/// Takes ownership to avoid cloning keys and values in the hot path.
fn into_batch_ops(pairs: Vec<KeyValueOp>) -> Vec<BoxedBatchOp> {
    pairs
        .into_iter()
        .map(|op| match op.value {
            Some(value) => BatchOp::Put { key: op.key, value },
            None if op.delete_exact => BatchOp::Delete { key: op.key },
            None => BatchOp::DeleteRange { prefix: op.key },
        })
        .collect()
}

/// Retrieves a proposal reference from the map, returning an error if not found.
fn get_proposal<'a, 'db>(
    proposals: &'a HashMap<ProposalId, Proposal<'db>>,
    id: ProposalId,
) -> Result<&'a Proposal<'db>, ReplayError> {
    proposals.get(&id).ok_or(ReplayError::UnknownProposal(id))
}

/// Removes and returns a proposal from the map, returning an error if not found.
fn take_proposal<'db>(
    proposals: &mut HashMap<ProposalId, Proposal<'db>>,
    id: ProposalId,
) -> Result<Proposal<'db>, ReplayError> {
    proposals
        .remove(&id)
        .ok_or(ReplayError::UnknownProposal(id))
}

/// Retrieves a revision view from the map, returning an error if not found.
fn get_revision<'a>(
    revisions: &'a HashMap<RevisionId, ArcDynDbView>,
    id: RevisionId,
) -> Result<&'a ArcDynDbView, ReplayError> {
    revisions.get(&id).ok_or(ReplayError::UnknownRevision(id))
}

/// Removes and returns a revision view from the map, returning an error if not found.
fn take_revision(
    revisions: &mut HashMap<RevisionId, ArcDynDbView>,
    id: RevisionId,
) -> Result<ArcDynDbView, ReplayError> {
    revisions
        .remove(&id)
        .ok_or(ReplayError::UnknownRevision(id))
}

/// Iterator replay state.
#[derive(Clone)]
struct ReplayIteratorState {
    view: ArcDynDbView,
    start_key: Option<Box<[u8]>>,
    consumed: usize,
}

/// Retrieves mutable iterator state from the map, returning an error if not found.
fn get_iterator_mut<'a>(
    iterators: &'a mut HashMap<IteratorId, ReplayIteratorState>,
    id: IteratorId,
) -> Result<&'a mut ReplayIteratorState, ReplayError> {
    iterators
        .get_mut(&id)
        .ok_or(ReplayError::UnknownIterator(id))
}

/// Removes and returns iterator state from the map, returning an error if not found.
fn take_iterator(
    iterators: &mut HashMap<IteratorId, ReplayIteratorState>,
    id: IteratorId,
) -> Result<ReplayIteratorState, ReplayError> {
    iterators
        .remove(&id)
        .ok_or(ReplayError::UnknownIterator(id))
}

/// Rebuilds and advances an iterator based on recorded cursor state.
///
/// This intentionally favors simplicity and determinism over performance.
fn iter_take(
    state: &mut ReplayIteratorState,
    n: usize,
) -> Result<Vec<(Box<[u8]>, Box<[u8]>)>, ReplayError> {
    let mut iter = state.view.iter_option(state.start_key.as_deref())?;

    // Skip already-consumed entries.
    for _ in 0..state.consumed {
        match iter.next() {
            Some(item) => {
                let _ = item.map_err(api::Error::from)?;
            }
            None => return Ok(Vec::new()),
        }
    }

    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        match iter.next() {
            Some(item) => {
                let (key, value) = item.map_err(api::Error::from)?;
                out.push((key, value));
            }
            None => break,
        }
    }

    state.consumed = state.consumed.saturating_add(out.len());
    Ok(out)
}

/// Applies a single operation to the database.
///
/// Returns the root hash if the operation was a commit that produced one.
fn apply_operation<'db>(
    db: &'db Db,
    proposals: &mut HashMap<ProposalId, Proposal<'db>>,
    revisions: &mut HashMap<RevisionId, ArcDynDbView>,
    iterators: &mut HashMap<IteratorId, ReplayIteratorState>,
    operation: DbOperation,
) -> Result<Option<Box<[u8]>>, ReplayError> {
    match operation {
        DbOperation::GetLatest(GetLatest { key, .. }) => {
            if let Some(root) = DbApi::root_hash(db)? {
                let view = DbApi::revision(db, root)?;
                let _ = DbViewApi::val(&*view, key)?;
            }
            Ok(None)
        }

        DbOperation::GetFromProposal(GetFromProposal {
            proposal_id, key, ..
        }) => {
            let proposal = get_proposal(proposals, proposal_id)?;
            let _ = DbViewApi::val(proposal, key)?;
            Ok(None)
        }

        DbOperation::GetRevision(GetRevision {
            root,
            returned_revision_id,
            ..
        }) => {
            let Some(revision_id) = returned_revision_id else {
                return Ok(None);
            };

            let root = firewood::v2::api::HashKey::try_from(root.as_ref())?;
            let view: ArcDynDbView = DbApi::revision(db, root)?;
            revisions.insert(revision_id, view);
            Ok(None)
        }

        DbOperation::GetFromRevision(GetFromRevision {
            revision_id, key, ..
        }) => {
            let revision = get_revision(revisions, revision_id)?;
            let _ = revision.val(key.as_ref())?;
            Ok(None)
        }

        DbOperation::RootHash(_) => {
            let _ = DbApi::root_hash(db)?;
            Ok(None)
        }

        DbOperation::Batch(Batch { pairs, result, .. }) => {
            if matches!(
                result,
                Some(HashResultData::Err | HashResultData::NullHandlePointer)
            ) {
                return Ok(None);
            }

            let ops = into_batch_ops(pairs);
            let proposal = DbApi::propose(db, ops)?;
            proposal.commit()?;
            Ok(None)
        }

        DbOperation::ProposeOnDB(ProposeOnDB {
            pairs,
            returned_proposal_id,
            result,
            ..
        }) => {
            if matches!(
                result,
                Some(ProposalResultData::Err | ProposalResultData::NullHandlePointer)
            ) {
                return Ok(None);
            }
            let Some(returned_proposal_id) = returned_proposal_id else {
                return Ok(None);
            };

            let ops = into_batch_ops(pairs);
            let start = Instant::now();
            let proposal = DbApi::propose(db, ops)?;
            firewood_increment!(registry::PROPOSE_NS, start.elapsed().as_nanos() as u64);
            firewood_increment!(registry::PROPOSE_COUNT, 1);
            proposals.insert(returned_proposal_id, proposal);
            Ok(None)
        }

        DbOperation::ProposeOnProposal(ProposeOnProposal {
            proposal_id,
            pairs,
            returned_proposal_id,
            result,
            ..
        }) => {
            if matches!(
                result,
                Some(ProposalResultData::Err | ProposalResultData::NullHandlePointer)
            ) {
                return Ok(None);
            }
            let Some(returned_proposal_id) = returned_proposal_id else {
                return Ok(None);
            };

            let ops = into_batch_ops(pairs);
            let start = Instant::now();
            let parent = get_proposal(proposals, proposal_id)?;
            let new_proposal = ProposalApi::propose(parent, ops)?;
            firewood_increment!(registry::PROPOSE_NS, start.elapsed().as_nanos() as u64);
            firewood_increment!(registry::PROPOSE_COUNT, 1);
            proposals.insert(returned_proposal_id, new_proposal);
            Ok(None)
        }

        DbOperation::IterOnRevision(IterOnRevision {
            revision_id,
            start_key,
            returned_iterator_id,
            result,
            ..
        }) => {
            if matches!(
                result,
                Some(IteratorResultData::Err | IteratorResultData::NullHandlePointer)
            ) {
                return Ok(None);
            }
            let Some(returned_iterator_id) = returned_iterator_id else {
                return Ok(None);
            };

            let revision = get_revision(revisions, revision_id)?;
            iterators.insert(
                returned_iterator_id,
                ReplayIteratorState {
                    view: revision.clone(),
                    start_key,
                    consumed: 0,
                },
            );
            Ok(None)
        }

        DbOperation::IterOnProposal(IterOnProposal {
            proposal_id,
            start_key,
            returned_iterator_id,
            result,
            ..
        }) => {
            if matches!(
                result,
                Some(IteratorResultData::Err | IteratorResultData::NullHandlePointer)
            ) {
                return Ok(None);
            }
            let Some(returned_iterator_id) = returned_iterator_id else {
                return Ok(None);
            };

            let proposal = get_proposal(proposals, proposal_id)?;
            iterators.insert(
                returned_iterator_id,
                ReplayIteratorState {
                    view: proposal.view(),
                    start_key,
                    consumed: 0,
                },
            );
            Ok(None)
        }

        DbOperation::IterNext(IterNext {
            iterator_id,
            result,
            ..
        }) => {
            if matches!(
                result,
                Some(KeyValueResultData::Err | KeyValueResultData::NullHandlePointer)
            ) {
                return Ok(None);
            }

            let state = get_iterator_mut(iterators, iterator_id)?;
            let _ = iter_take(state, 1)?;
            Ok(None)
        }

        DbOperation::IterNextN(IterNextN {
            iterator_id,
            n,
            result,
            ..
        }) => {
            if matches!(
                result,
                Some(KeyValueBatchResultData::Err | KeyValueBatchResultData::NullHandlePointer)
            ) {
                return Ok(None);
            }

            let state = get_iterator_mut(iterators, iterator_id)?;
            let _ = iter_take(state, n)?;
            Ok(None)
        }

        DbOperation::Commit(Commit {
            proposal_id,
            returned_hash,
            result,
            ..
        }) => {
            if matches!(
                result,
                Some(HashResultData::Err | HashResultData::NullHandlePointer)
            ) {
                // FFI consumes proposal handles regardless of success.
                let _ = proposals.remove(&proposal_id);
                return Ok(None);
            }

            let proposal = take_proposal(proposals, proposal_id)?;
            let start = Instant::now();
            proposal.commit()?;
            firewood_increment!(registry::COMMIT_NS, start.elapsed().as_nanos() as u64);
            firewood_increment!(registry::COMMIT_COUNT, 1);
            Ok(returned_hash)
        }

        DbOperation::FreeProposal(FreeProposal { proposal_id, .. }) => {
            let _ = take_proposal(proposals, proposal_id)?;
            Ok(None)
        }

        DbOperation::FreeRevision(FreeRevision { revision_id, .. }) => {
            let _ = take_revision(revisions, revision_id)?;
            Ok(None)
        }

        DbOperation::FreeIterator(FreeIterator { iterator_id, .. }) => {
            let _ = take_iterator(iterators, iterator_id)?;
            Ok(None)
        }
    }
}

/// Replays all operations from a length-prefixed replay log.
///
/// The log is expected to be a sequence of segments, each formatted as:
/// `[len: u64 LE][message_pack(ReplayLog) bytes]`
///
/// # Arguments
///
/// * `reader` - A reader positioned at the start of the replay log.
/// * `db` - The database to replay operations against.
/// * `max_commits` - Optional limit on the number of commits to replay.
///
/// # Returns
///
/// The root hash from the last commit operation, if any.
///
/// # Errors
///
/// Returns an error if:
/// - An I/O error occurs while reading the log
/// - A log segment cannot be deserialized
/// - A database operation fails
/// - The log references an unknown proposal ID
pub fn replay_from_reader<R: Read>(
    mut reader: R,
    db: &Db,
    max_commits: Option<u64>,
) -> Result<Option<Box<[u8]>>, ReplayError> {
    let mut proposals: HashMap<ProposalId, Proposal<'_>> = HashMap::new();
    let mut revisions: HashMap<RevisionId, ArcDynDbView> = HashMap::new();
    let mut iterators: HashMap<IteratorId, ReplayIteratorState> = HashMap::new();
    let mut last_commit_hash = None;
    let mut commit_count = 0u64;
    let max = max_commits.unwrap_or(u64::MAX);

    loop {
        // Read segment length
        let mut len_buf = [0u8; 8];
        match reader.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(ReplayError::Io(e)),
        }

        let len = u64::from_le_bytes(len_buf);
        if len == 0 {
            continue;
        }

        // Read segment payload
        let mut buf = vec![0u8; len as usize];
        reader.read_exact(&mut buf)?;

        let log: ReplayLog = rmp_serde::from_slice(&buf)?;

        for op in log.operations {
            let is_commit = matches!(op, DbOperation::Commit(_));
            if let Some(hash) =
                apply_operation(db, &mut proposals, &mut revisions, &mut iterators, op)?
            {
                last_commit_hash = Some(hash);
            }
            if is_commit {
                commit_count = commit_count.saturating_add(1);
                if commit_count >= max {
                    return Ok(last_commit_hash);
                }
            }
        }
    }

    Ok(last_commit_hash)
}

/// Replays a log file against the provided database.
///
/// This is a convenience wrapper around [`replay_from_reader`].
///
/// # Errors
///
/// Returns an error if the file cannot be opened or if replay fails.
/// See [`replay_from_reader`] for detailed error conditions.
pub fn replay_from_file(
    path: impl AsRef<std::path::Path>,
    db: &Db,
    max_commits: Option<u64>,
) -> Result<Option<Box<[u8]>>, ReplayError> {
    let file = std::fs::File::open(path)?;
    replay_from_reader(file, db, max_commits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use firewood::db::DbConfig;
    use firewood::manager::RevisionManagerConfig;
    use firewood_storage::NodeHashAlgorithm;
    use std::io::Cursor;
    use tempfile::tempdir;

    fn create_test_db() -> (tempfile::TempDir, Db) {
        let tmpdir = tempdir().expect("create tempdir");
        let db_path = tmpdir.path().join("test.db");
        let node_hash_algorithm = if NodeHashAlgorithm::Ethereum.matches_compile_option() {
            NodeHashAlgorithm::Ethereum
        } else {
            NodeHashAlgorithm::MerkleDB
        };
        let cfg = DbConfig::builder()
            .node_hash_algorithm(node_hash_algorithm)
            .truncate(true)
            .manager(RevisionManagerConfig::builder().build())
            .build();
        let db = Db::new(&db_path, cfg).expect("db creation should succeed");
        (tmpdir, db)
    }

    fn serialize_log(log: &ReplayLog) -> Vec<u8> {
        let bytes = rmp_serde::to_vec(log).expect("serialize");
        let len: u64 = bytes.len().try_into().expect("fits");
        let mut buf = Vec::new();
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(&bytes);
        buf
    }

    #[test]
    fn replay_batch_inserts_keys() {
        let (_tmpdir, db) = create_test_db();

        let pairs: Vec<KeyValueOp> = (0u8..5)
            .map(|i| KeyValueOp {
                key: vec![i].into_boxed_slice(),
                value: Some(vec![i.wrapping_add(100)].into_boxed_slice()),
                delete_exact: false,
            })
            .collect();

        let log = ReplayLog::new(vec![DbOperation::Batch(Batch {
            pairs,
            result: None,
            metadata: None,
        })]);
        let buf = serialize_log(&log);

        replay_from_reader(Cursor::new(buf), &db, None).expect("replay");

        let root = DbApi::root_hash(&db)
            .expect("root_hash")
            .expect("non-empty");
        let view = DbApi::revision(&db, root).expect("revision");

        for i in 0u8..5 {
            let val = DbViewApi::val(&*view, vec![i].as_slice())
                .expect("val")
                .expect("exists");
            assert_eq!(*val, [i.wrapping_add(100)]);
        }
    }

    #[test]
    fn replay_propose_and_commit() {
        let (_tmpdir, db) = create_test_db();

        let pairs: Vec<KeyValueOp> = (0u8..3)
            .map(|i| KeyValueOp {
                key: vec![i].into_boxed_slice(),
                value: Some(vec![i.wrapping_mul(2)].into_boxed_slice()),
                delete_exact: false,
            })
            .collect();

        let ops = vec![
            DbOperation::ProposeOnDB(ProposeOnDB {
                pairs,
                returned_proposal_id: Some(ProposalId(1)),
                result: None,
                metadata: None,
            }),
            DbOperation::Commit(Commit {
                proposal_id: ProposalId(1),
                returned_hash: None,
                result: None,
                metadata: None,
            }),
        ];

        let log = ReplayLog::new(ops);
        let buf = serialize_log(&log);

        replay_from_reader(Cursor::new(buf), &db, None).expect("replay");

        let root = DbApi::root_hash(&db)
            .expect("root_hash")
            .expect("non-empty");
        let view = DbApi::revision(&db, root).expect("revision");

        for i in 0u8..3 {
            let val = DbViewApi::val(&*view, vec![i].as_slice())
                .expect("val")
                .expect("exists");
            assert_eq!(*val, [i.wrapping_mul(2)]);
        }
    }

    #[test]
    fn replay_chained_proposals() {
        let (_tmpdir, db) = create_test_db();

        let ops = vec![
            DbOperation::ProposeOnDB(ProposeOnDB {
                pairs: vec![KeyValueOp {
                    key: vec![1].into_boxed_slice(),
                    value: Some(vec![10].into_boxed_slice()),
                    delete_exact: false,
                }],
                returned_proposal_id: Some(ProposalId(1)),
                result: None,
                metadata: None,
            }),
            DbOperation::ProposeOnProposal(ProposeOnProposal {
                proposal_id: ProposalId(1),
                pairs: vec![KeyValueOp {
                    key: vec![2].into_boxed_slice(),
                    value: Some(vec![20].into_boxed_slice()),
                    delete_exact: false,
                }],
                returned_proposal_id: Some(ProposalId(2)),
                result: None,
                metadata: None,
            }),
            DbOperation::Commit(Commit {
                proposal_id: ProposalId(1),
                returned_hash: None,
                result: None,
                metadata: None,
            }),
            DbOperation::Commit(Commit {
                proposal_id: ProposalId(2),
                returned_hash: None,
                result: None,
                metadata: None,
            }),
        ];

        let log = ReplayLog::new(ops);
        let buf = serialize_log(&log);

        replay_from_reader(Cursor::new(buf), &db, None).expect("replay");

        let root = DbApi::root_hash(&db)
            .expect("root_hash")
            .expect("non-empty");
        let view = DbApi::revision(&db, root).expect("revision");

        let v1 = DbViewApi::val(&*view, [1]).expect("val").expect("exists");
        let v2 = DbViewApi::val(&*view, [2]).expect("val").expect("exists");
        assert_eq!(*v1, [10]);
        assert_eq!(*v2, [20]);
    }

    #[test]
    fn replay_empty_log_succeeds() {
        let (_tmpdir, db) = create_test_db();
        let result = replay_from_reader(Cursor::new(Vec::new()), &db, None);
        assert!(result.is_ok());
        assert!(result.expect("ok").is_none());
    }
}
