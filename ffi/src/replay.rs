// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Block replay recording for FFI operations.
//!
//! When the `block-replay` feature is enabled and the `FIREWOOD_BLOCK_REPLAY_PATH`
//! environment variable is set, this module records database operations passing
//! through the FFI layer to a file for later replay and analysis.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

use firewood_replay::{
    Batch, Commit, DbOperation, FreeProposal, FreeRevision, GetFromProposal, GetFromRevision,
    GetLatest, GetRevision, HashResultData, KeyValueOp, OperationMetadata, ProposalId,
    ProposalResultData, ProposeOnDB, ProposeOnProposal, ReplayLog, RevisionId, RevisionResultData,
    RootHash, ValueResultData, VoidResultData,
};
use parking_lot::Mutex;

use crate::value::{BatchOp, BorrowedBatchOps, BorrowedBytes};

/// Environment variable that controls the output path for the replay log.
const REPLAY_PATH_ENV: &str = "FIREWOOD_BLOCK_REPLAY_PATH";
/// Environment variable to include captured call results in replay log entries.
const INCLUDE_RESULTS_ENV: &str = "FIREWOOD_BLOCK_REPLAY_INCLUDE_RESULTS";
/// Environment variable to include timing metadata in replay log entries.
const INCLUDE_METADATA_ENV: &str = "FIREWOOD_BLOCK_REPLAY_INCLUDE_METADATA";

/// Number of operations to buffer before flushing to disk.
const FLUSH_THRESHOLD: usize = 10_000;

/// The global recorder instance. `None` if recording is disabled.
static RECORDER: OnceLock<Option<Mutex<Recorder>>> = OnceLock::new();

/// Internal state for recording operations.
struct Recorder {
    /// Buffered operations awaiting flush.
    operations: Vec<DbOperation>,
    /// Counter for assigning proposal IDs.
    next_proposal_id: u64,
    /// Map from proposal handle pointer addresses to assigned IDs.
    proposal_ids: HashMap<usize, ProposalId>,
    /// Counter for assigning revision IDs.
    next_revision_id: u64,
    /// Map from revision handle pointer addresses to assigned IDs.
    revision_ids: HashMap<usize, RevisionId>,
    /// Whether to include result payloads in operation logs.
    include_results: bool,
    /// Whether to include per-operation timing metadata.
    include_metadata: bool,
    /// Output path for the replay log.
    output_path: PathBuf,
}

impl Recorder {
    fn new(output_path: PathBuf, include_results: bool, include_metadata: bool) -> Self {
        Self {
            operations: Vec::new(),
            next_proposal_id: 1,
            proposal_ids: HashMap::new(),
            next_revision_id: 1,
            revision_ids: HashMap::new(),
            include_results,
            include_metadata,
            output_path,
        }
    }

    fn maybe_result<T>(&self, value: T) -> Option<T> {
        self.include_results.then_some(value)
    }

    fn metadata(&self, duration_ns: u64) -> Option<OperationMetadata> {
        self.include_metadata
            .then_some(OperationMetadata { duration_ns })
    }

    /// Records a `GetLatest` operation.
    fn record_get_latest(&mut self, key: &[u8], result: &crate::ValueResult, duration_ns: u64) {
        self.operations.push(DbOperation::GetLatest(GetLatest {
            key: key.into(),
            result: self.maybe_result(value_result_data(result)),
            metadata: self.metadata(duration_ns),
        }));
        self.maybe_flush();
    }

    /// Records a `GetFromProposal` operation.
    fn record_get_from_proposal(
        &mut self,
        handle_ptr: usize,
        key: &[u8],
        result: &crate::ValueResult,
        duration_ns: u64,
    ) {
        let Some(&proposal_id) = self.proposal_ids.get(&handle_ptr) else {
            return;
        };

        self.operations
            .push(DbOperation::GetFromProposal(GetFromProposal {
                proposal_id,
                key: key.into(),
                result: self.maybe_result(value_result_data(result)),
                metadata: self.metadata(duration_ns),
            }));
        self.maybe_flush();
    }

    /// Records a `GetRevision` operation.
    fn record_get_revision(
        &mut self,
        root: crate::HashKey,
        result: &crate::RevisionResult,
        duration_ns: u64,
    ) {
        let mut returned_revision_id = None;

        if let crate::RevisionResult::Ok { handle, .. } = result {
            let revision_id = RevisionId(self.next_revision_id);
            self.next_revision_id = self.next_revision_id.saturating_add(1);
            let ptr = std::ptr::from_ref(&**handle) as usize;
            self.revision_ids.insert(ptr, revision_id);
            returned_revision_id = Some(revision_id);
        }

        self.operations.push(DbOperation::GetRevision(GetRevision {
            root: hash_key_to_bytes(root).into(),
            returned_revision_id,
            result: self.maybe_result(revision_result_data(result)),
            metadata: self.metadata(duration_ns),
        }));
        self.maybe_flush();
    }

    /// Records a `GetFromRevision` operation.
    fn record_get_from_revision(
        &mut self,
        handle_ptr: usize,
        key: &[u8],
        result: &crate::ValueResult,
        duration_ns: u64,
    ) {
        let Some(&revision_id) = self.revision_ids.get(&handle_ptr) else {
            return;
        };

        self.operations
            .push(DbOperation::GetFromRevision(GetFromRevision {
                revision_id,
                key: key.into(),
                result: self.maybe_result(value_result_data(result)),
                metadata: self.metadata(duration_ns),
            }));
        self.maybe_flush();
    }

    /// Records a `RootHash` operation.
    fn record_root_hash(&mut self, result: &crate::HashResult, duration_ns: u64) {
        self.operations.push(DbOperation::RootHash(RootHash {
            result: self.maybe_result(hash_result_data(result)),
            metadata: self.metadata(duration_ns),
        }));
        self.maybe_flush();
    }

    /// Records a `Batch` operation.
    fn record_batch(&mut self, ops: &[BatchOp<'_>], result: &crate::HashResult, duration_ns: u64) {
        let pairs = convert_ops(ops);
        self.operations.push(DbOperation::Batch(Batch {
            pairs,
            result: self.maybe_result(hash_result_data(result)),
            metadata: self.metadata(duration_ns),
        }));
        self.maybe_flush();
    }

    /// Records a `ProposeOnDB` operation.
    fn record_propose_on_db(
        &mut self,
        result: &crate::ProposalResult<'_>,
        ops: &[BatchOp<'_>],
        duration_ns: u64,
    ) {
        let mut returned_proposal_id = None;

        if let crate::ProposalResult::Ok { handle, .. } = result {
            let proposal_id = ProposalId(self.next_proposal_id);
            self.next_proposal_id = self.next_proposal_id.saturating_add(1);
            let ptr = std::ptr::from_ref(&**handle) as usize;
            self.proposal_ids.insert(ptr, proposal_id);
            returned_proposal_id = Some(proposal_id);
        }

        let pairs = convert_ops(ops);
        self.operations.push(DbOperation::ProposeOnDB(ProposeOnDB {
            pairs,
            returned_proposal_id,
            result: self.maybe_result(proposal_result_data(result)),
            metadata: self.metadata(duration_ns),
        }));
        self.maybe_flush();
    }

    /// Records a `ProposeOnProposal` operation.
    fn record_propose_on_proposal(
        &mut self,
        parent_ptr: usize,
        result: &crate::ProposalResult<'_>,
        ops: &[BatchOp<'_>],
        duration_ns: u64,
    ) {
        let Some(&parent_id) = self.proposal_ids.get(&parent_ptr) else {
            return;
        };

        let mut returned_proposal_id = None;
        if let crate::ProposalResult::Ok { handle, .. } = result {
            let proposal_id = ProposalId(self.next_proposal_id);
            self.next_proposal_id = self.next_proposal_id.saturating_add(1);
            let ptr = std::ptr::from_ref(&**handle) as usize;
            self.proposal_ids.insert(ptr, proposal_id);
            returned_proposal_id = Some(proposal_id);
        }

        let pairs = convert_ops(ops);
        self.operations
            .push(DbOperation::ProposeOnProposal(ProposeOnProposal {
                proposal_id: parent_id,
                pairs,
                returned_proposal_id,
                result: self.maybe_result(proposal_result_data(result)),
                metadata: self.metadata(duration_ns),
            }));
        self.maybe_flush();
    }

    /// Records a `Commit` operation.
    fn record_commit(&mut self, handle_ptr: usize, result: &crate::HashResult, duration_ns: u64) {
        let Some(proposal_id) = self.proposal_ids.remove(&handle_ptr) else {
            return;
        };

        let returned_hash = match result {
            crate::HashResult::Some(hash) => Some(hash_key_to_bytes(*hash).into()),
            crate::HashResult::None
            | crate::HashResult::Err(_)
            | crate::HashResult::NullHandlePointer => None,
        };

        self.operations.push(DbOperation::Commit(Commit {
            proposal_id,
            returned_hash,
            result: self.maybe_result(hash_result_data(result)),
            metadata: self.metadata(duration_ns),
        }));
        self.maybe_flush();
    }

    /// Records a `FreeProposal` operation.
    fn record_free_proposal(
        &mut self,
        handle_ptr: usize,
        result: &crate::VoidResult,
        duration_ns: u64,
    ) {
        let Some(proposal_id) = self.proposal_ids.remove(&handle_ptr) else {
            return;
        };

        self.operations
            .push(DbOperation::FreeProposal(FreeProposal {
                proposal_id,
                result: self.maybe_result(void_result_data(result)),
                metadata: self.metadata(duration_ns),
            }));
        self.maybe_flush();
    }

    /// Records a `FreeRevision` operation.
    fn record_free_revision(
        &mut self,
        handle_ptr: usize,
        result: &crate::VoidResult,
        duration_ns: u64,
    ) {
        let Some(revision_id) = self.revision_ids.remove(&handle_ptr) else {
            return;
        };

        self.operations
            .push(DbOperation::FreeRevision(FreeRevision {
                revision_id,
                result: self.maybe_result(void_result_data(result)),
                metadata: self.metadata(duration_ns),
            }));
        self.maybe_flush();
    }

    /// Flushes if the buffer exceeds the threshold.
    fn maybe_flush(&mut self) {
        if self.operations.len() >= FLUSH_THRESHOLD {
            if self.flush_to_disk().is_err() {
                // Keep buffering data if flush fails so callers can retry via
                // explicit flush/close instead of silently losing operations.
            }
        }
    }

    /// Flushes buffered operations to disk.
    fn flush_to_disk(&mut self) -> io::Result<()> {
        if self.operations.is_empty() {
            return Ok(());
        }

        let log = ReplayLog::new(std::mem::take(&mut self.operations));

        let bytes = match log.to_msgpack() {
            Ok(bytes) => bytes,
            Err(e) => {
                self.operations = log.operations;
                return Err(io::Error::other(format!(
                    "failed to serialize replay log: {e}"
                )));
            }
        };

        if let Some(parent) = self.output_path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                self.operations = log.operations;
                return Err(e);
            }
        }

        let mut file = match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.output_path)
        {
            Ok(file) => file,
            Err(e) => {
                self.operations = log.operations;
                return Err(e);
            }
        };

        let len: u64 = match bytes.len().try_into() {
            Ok(len) => len,
            Err(_) => {
                self.operations = log.operations;
                return Err(io::Error::other("replay segment too large"));
            }
        };

        if let Err(e) = file.write_all(&len.to_le_bytes()) {
            self.operations = log.operations;
            return Err(e);
        }
        if let Err(e) = file.write_all(&bytes) {
            self.operations = log.operations;
            return Err(e);
        }

        Ok(())
    }
}

/// Converts FFI batch operations to replay log format.
fn convert_ops(ops: &[BatchOp<'_>]) -> Vec<KeyValueOp> {
    ops.iter()
        .map(|op| match op {
            BatchOp::Put { key, value } => KeyValueOp {
                key: key.as_slice().into(),
                value: Some(value.as_slice().into()),
                delete_exact: false,
            },
            BatchOp::Delete { key } => KeyValueOp {
                key: key.as_slice().into(),
                value: None,
                delete_exact: true,
            },
            BatchOp::DeleteRange { prefix } => KeyValueOp {
                key: prefix.as_slice().into(),
                value: None,
                delete_exact: false,
            },
        })
        .collect()
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .map(|value| {
            let value = value.trim();
            value == "1"
                || value.eq_ignore_ascii_case("true")
                || value.eq_ignore_ascii_case("yes")
                || value.eq_ignore_ascii_case("on")
        })
        .unwrap_or(false)
}

fn hash_key_to_bytes(hash: crate::HashKey) -> [u8; 32] {
    let api_hash: firewood::v2::api::HashKey = hash.into();
    api_hash.into()
}

fn value_result_data(result: &crate::ValueResult) -> ValueResultData {
    match result {
        crate::ValueResult::NullHandlePointer => ValueResultData::NullHandlePointer,
        crate::ValueResult::RevisionNotFound(root) => {
            ValueResultData::RevisionNotFound(hash_key_to_bytes(*root).into())
        }
        crate::ValueResult::None => ValueResultData::None,
        crate::ValueResult::Some(value) => ValueResultData::Some(value.as_slice().into()),
        crate::ValueResult::Err(_) => ValueResultData::Err,
    }
}

fn hash_result_data(result: &crate::HashResult) -> HashResultData {
    match result {
        crate::HashResult::NullHandlePointer => HashResultData::NullHandlePointer,
        crate::HashResult::None => HashResultData::None,
        crate::HashResult::Some(hash) => HashResultData::Some(hash_key_to_bytes(*hash).into()),
        crate::HashResult::Err(_) => HashResultData::Err,
    }
}

fn proposal_result_data(result: &crate::ProposalResult<'_>) -> ProposalResultData {
    match result {
        crate::ProposalResult::NullHandlePointer => ProposalResultData::NullHandlePointer,
        crate::ProposalResult::Ok { root_hash, .. } => ProposalResultData::Ok {
            root_hash: Some(hash_key_to_bytes(*root_hash).into()),
        },
        crate::ProposalResult::Err(_) => ProposalResultData::Err,
    }
}

fn revision_result_data(result: &crate::RevisionResult) -> RevisionResultData {
    match result {
        crate::RevisionResult::NullHandlePointer => RevisionResultData::NullHandlePointer,
        crate::RevisionResult::RevisionNotFound(root) => {
            RevisionResultData::RevisionNotFound(hash_key_to_bytes(*root).into())
        }
        crate::RevisionResult::Ok { root_hash, .. } => {
            RevisionResultData::Ok(hash_key_to_bytes(*root_hash).into())
        }
        crate::RevisionResult::Err(_) => RevisionResultData::Err,
    }
}

fn void_result_data(result: &crate::VoidResult) -> VoidResultData {
    match result {
        crate::VoidResult::NullHandlePointer => VoidResultData::NullHandlePointer,
        crate::VoidResult::Ok => VoidResultData::Ok,
        crate::VoidResult::Err(_) => VoidResultData::Err,
    }
}

/// Returns the recorder if recording is enabled.
fn recorder() -> Option<&'static Mutex<Recorder>> {
    RECORDER
        .get_or_init(|| {
            std::env::var(REPLAY_PATH_ENV).ok().map(|p| {
                Mutex::new(Recorder::new(
                    PathBuf::from(p),
                    env_flag(INCLUDE_RESULTS_ENV),
                    env_flag(INCLUDE_METADATA_ENV),
                ))
            })
        })
        .as_ref()
}

/// Records a `fwd_get_latest` call.
pub(crate) fn record_get_latest(
    key: BorrowedBytes<'_>,
    result: &crate::ValueResult,
    duration_ns: u64,
) {
    if let Some(rec) = recorder() {
        rec.lock()
            .record_get_latest(key.as_slice(), result, duration_ns);
    }
}

/// Records a `fwd_get_from_proposal` call.
pub(crate) fn record_get_from_proposal(
    handle: Option<&crate::ProposalHandle<'_>>,
    key: BorrowedBytes<'_>,
    result: &crate::ValueResult,
    duration_ns: u64,
) {
    let Some(rec) = recorder() else { return };
    let Some(handle) = handle else { return };

    let ptr = std::ptr::from_ref(handle) as usize;
    rec.lock()
        .record_get_from_proposal(ptr, key.as_slice(), result, duration_ns);
}

/// Records a `fwd_get_revision` call.
pub(crate) fn record_get_revision(
    root: crate::HashKey,
    result: &crate::RevisionResult,
    duration_ns: u64,
) {
    if let Some(rec) = recorder() {
        rec.lock().record_get_revision(root, result, duration_ns);
    }
}

/// Records a `fwd_get_from_revision` call.
pub(crate) fn record_get_from_revision(
    revision: Option<&crate::RevisionHandle>,
    key: BorrowedBytes<'_>,
    result: &crate::ValueResult,
    duration_ns: u64,
) {
    let Some(rec) = recorder() else { return };
    let Some(revision) = revision else { return };

    let ptr = std::ptr::from_ref(revision) as usize;
    rec.lock()
        .record_get_from_revision(ptr, key.as_slice(), result, duration_ns);
}

/// Records a `fwd_root_hash` call.
pub(crate) fn record_root_hash(result: &crate::HashResult, duration_ns: u64) {
    if let Some(rec) = recorder() {
        rec.lock().record_root_hash(result, duration_ns);
    }
}

/// Records a `fwd_batch` call.
pub(crate) fn record_batch(
    values: BorrowedBatchOps<'_>,
    result: &crate::HashResult,
    duration_ns: u64,
) {
    if let Some(rec) = recorder() {
        rec.lock()
            .record_batch(values.as_slice(), result, duration_ns);
    }
}

/// Records a `fwd_propose_on_db` call.
pub(crate) fn record_propose_on_db(
    result: &crate::ProposalResult<'_>,
    values: BorrowedBatchOps<'_>,
    duration_ns: u64,
) {
    let Some(rec) = recorder() else { return };
    rec.lock()
        .record_propose_on_db(result, values.as_slice(), duration_ns);
}

/// Records a `fwd_propose_on_proposal` call.
pub(crate) fn record_propose_on_proposal<'db>(
    parent: Option<&crate::ProposalHandle<'db>>,
    result: &crate::ProposalResult<'db>,
    values: BorrowedBatchOps<'_>,
    duration_ns: u64,
) {
    let Some(rec) = recorder() else { return };
    let Some(parent) = parent else { return };

    let parent_ptr = std::ptr::from_ref(parent) as usize;
    rec.lock()
        .record_propose_on_proposal(parent_ptr, result, values.as_slice(), duration_ns);
}

/// Records a `fwd_commit_proposal` call after the commit completes.
pub(crate) fn record_commit(
    proposal_ptr: Option<*const crate::ProposalHandle<'_>>,
    result: &crate::HashResult,
    duration_ns: u64,
) {
    let Some(rec) = recorder() else { return };
    let Some(ptr) = proposal_ptr else { return };

    rec.lock().record_commit(ptr as usize, result, duration_ns);
}

/// Records a `fwd_free_proposal` call.
pub(crate) fn record_free_proposal(
    proposal_ptr: Option<*const crate::ProposalHandle<'_>>,
    result: &crate::VoidResult,
    duration_ns: u64,
) {
    let Some(rec) = recorder() else { return };
    let Some(ptr) = proposal_ptr else { return };

    rec.lock()
        .record_free_proposal(ptr as usize, result, duration_ns);
}

/// Records a `fwd_free_revision` call.
pub(crate) fn record_free_revision(
    revision_ptr: Option<*const crate::RevisionHandle>,
    result: &crate::VoidResult,
    duration_ns: u64,
) {
    let Some(rec) = recorder() else { return };
    let Some(ptr) = revision_ptr else { return };

    rec.lock()
        .record_free_revision(ptr as usize, result, duration_ns);
}

/// Flushes any buffered operations to disk.
///
/// Called automatically when the database is closed, but can also be
/// invoked manually via `fwd_block_replay_flush`.
pub(crate) fn flush_to_disk() -> io::Result<()> {
    // we don't use recorder() here because flushing should not init
    // the recorder. TestMain opens and closes a db, that causes
    // some tests to fail.
    // TODO[AMIN]: this should change when we make record db-specific.
    if let Some(Some(rec)) = RECORDER.get() {
        rec.lock().flush_to_disk()
    } else {
        Ok(())
    }
}
