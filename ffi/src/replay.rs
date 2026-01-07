// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Block replay recording for FFI operations.
//!
//! When the `block-replay` feature is enabled and the `FIREWOOD_BLOCK_REPLAY_PATH`
//! environment variable is set, this module records all database operations
//! passing through the FFI layer to a file for later replay.
//!
//! The recording is length-prefixed rkyv segments, compatible with the
//! `firewood-replay` crate's replay engine.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

use firewood_replay::{
    Batch, Commit, DbOperation, GetFromProposal, GetFromRoot, GetLatest, KeyValueOp, ProposeOnDB,
    ProposeOnProposal, ReplayLog,
};
use parking_lot::Mutex;

use crate::value::{BorrowedBytes, BorrowedKeyValuePairs, HashKey as FfiHashKey, KeyValuePair};

/// Environment variable that controls the output path for the replay log.
const REPLAY_PATH_ENV: &str = "FIREWOOD_BLOCK_REPLAY_PATH";

/// Number of operations to buffer before flushing to disk.
const FLUSH_THRESHOLD: usize = 10_000;

/// The global recorder instance.
static RECORDER: OnceLock<Mutex<Recorder>> = OnceLock::new();

/// Internal state for recording operations.
struct Recorder {
    /// Buffered operations awaiting flush.
    operations: Vec<DbOperation>,
    /// Counter for assigning proposal IDs.
    next_proposal_id: u64,
    /// Map from proposal handle pointer addresses to assigned IDs.
    proposal_ids: HashMap<usize, u64>,
    /// Output path, or `None` if recording is disabled.
    output_path: Option<PathBuf>,
}

impl Recorder {
    fn new() -> Self {
        let output_path = std::env::var(REPLAY_PATH_ENV).ok().map(PathBuf::from);
        Self {
            operations: Vec::new(),
            next_proposal_id: 1, // Start from 1 to make 0 stand out as invalid
            proposal_ids: HashMap::new(),
            output_path,
        }
    }

    /// Returns `true` if recording is enabled.
    const fn is_enabled(&self) -> bool {
        self.output_path.is_some()
    }

    /// Records a `GetLatest` operation.
    fn record_get_latest(&mut self, key: &[u8]) {
        self.operations
            .push(DbOperation::GetLatest(GetLatest { key: key.into() }));
        self.maybe_flush();
    }

    /// Records a `GetFromRoot` operation.
    fn record_get_from_root(&mut self, root: &[u8], key: &[u8]) {
        self.operations.push(DbOperation::GetFromRoot(GetFromRoot {
            root: root.into(),
            key: key.into(),
        }));
        self.maybe_flush();
    }

    /// Records a `GetFromProposal` operation.
    fn record_get_from_proposal(&mut self, handle_ptr: usize, key: &[u8]) {
        let Some(&proposal_id) = self.proposal_ids.get(&handle_ptr) else {
            return;
        };
        self.operations
            .push(DbOperation::GetFromProposal(GetFromProposal {
                proposal_id,
                key: key.into(),
            }));
        self.maybe_flush();
    }

    /// Records a `Batch` operation.
    fn record_batch(&mut self, pairs: &[KeyValuePair<'_>]) {
        let pairs = convert_pairs(pairs);
        self.operations.push(DbOperation::Batch(Batch { pairs }));
        self.maybe_flush();
    }

    /// Records a `ProposeOnDB` operation.
    fn record_propose_on_db(&mut self, handle_ptr: usize, pairs: &[KeyValuePair<'_>]) {
        let proposal_id = self.next_proposal_id;
        self.next_proposal_id = self.next_proposal_id.saturating_add(1);
        self.proposal_ids.insert(handle_ptr, proposal_id);

        let pairs = convert_pairs(pairs);
        self.operations.push(DbOperation::ProposeOnDB(ProposeOnDB {
            pairs,
            returned_proposal_id: proposal_id,
        }));
        self.maybe_flush();
    }

    /// Records a `ProposeOnProposal` operation.
    fn record_propose_on_proposal(
        &mut self,
        parent_ptr: usize,
        new_ptr: usize,
        pairs: &[KeyValuePair<'_>],
    ) {
        let Some(&parent_id) = self.proposal_ids.get(&parent_ptr) else {
            return;
        };

        let new_id = self.next_proposal_id;
        self.next_proposal_id = self.next_proposal_id.saturating_add(1);
        self.proposal_ids.insert(new_ptr, new_id);

        let pairs = convert_pairs(pairs);
        self.operations
            .push(DbOperation::ProposeOnProposal(ProposeOnProposal {
                proposal_id: parent_id,
                pairs,
                returned_proposal_id: new_id,
            }));
        self.maybe_flush();
    }

    /// Records a `Commit` operation.
    fn record_commit(&mut self, handle_ptr: usize, returned_hash: Option<&[u8]>) {
        let Some(&proposal_id) = self.proposal_ids.get(&handle_ptr) else {
            return;
        };
        // Remove the proposal from tracking since it's now committed
        self.proposal_ids.remove(&handle_ptr);

        self.operations.push(DbOperation::Commit(Commit {
            proposal_id,
            returned_hash: returned_hash.map(Into::into),
        }));
        self.maybe_flush();
    }

    /// Flushes if the buffer exceeds the threshold.
    fn maybe_flush(&mut self) {
        if self.operations.len() >= FLUSH_THRESHOLD {
            let _ = self.flush_to_disk();
        }
    }

    /// Flushes buffered operations to disk.
    fn flush_to_disk(&mut self) -> io::Result<()> {
        let Some(path) = &self.output_path else {
            return Ok(());
        };

        if self.operations.is_empty() {
            return Ok(());
        }

        let operations = std::mem::take(&mut self.operations);
        let log = ReplayLog::new(operations);

        let bytes = log
            .to_msgpack()
            .map_err(|e| io::Error::other(format!("failed to serialize replay log: {e}")))?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new().create(true).append(true).open(path)?;

        let len: u64 = bytes
            .len()
            .try_into()
            .map_err(|_| io::Error::other("replay segment too large"))?;

        file.write_all(&len.to_le_bytes())?;
        file.write_all(&bytes)?;

        Ok(())
    }
}

/// Converts FFI key-value pairs to replay log format.
fn convert_pairs(pairs: &[KeyValuePair<'_>]) -> Vec<KeyValueOp> {
    pairs
        .iter()
        .map(|kv| KeyValueOp {
            key: kv.key.as_slice().into(),
            value: if kv.value.is_null() {
                None
            } else {
                Some(kv.value.as_slice().into())
            },
        })
        .collect()
}

/// Returns the recorder if recording is enabled.
fn recorder() -> Option<&'static Mutex<Recorder>> {
    if std::env::var(REPLAY_PATH_ENV).is_err() {
        return None;
    }
    Some(RECORDER.get_or_init(|| Mutex::new(Recorder::new())))
}

// ============================================================================
// Public Recording Functions
// ============================================================================

/// Records a `fwd_get_latest` call.
pub(crate) fn record_get_latest(key: BorrowedBytes<'_>) {
    if let Some(rec) = recorder() {
        let mut guard = rec.lock();
        if guard.is_enabled() {
            guard.record_get_latest(key.as_slice());
        }
    }
}

/// Records a `fwd_get_from_root` call.
pub(crate) fn record_get_from_root(root: FfiHashKey, key: BorrowedBytes<'_>) {
    if let Some(rec) = recorder() {
        let mut guard = rec.lock();
        if guard.is_enabled() {
            let api_hash: firewood::v2::api::HashKey = root.into();
            let bytes: [u8; 32] = api_hash.into();
            guard.record_get_from_root(&bytes, key.as_slice());
        }
    }
}

/// Records a `fwd_get_from_proposal` call.
pub(crate) fn record_get_from_proposal(
    handle: Option<&crate::ProposalHandle<'_>>,
    key: BorrowedBytes<'_>,
) {
    let Some(rec) = recorder() else { return };
    let Some(handle) = handle else { return };

    let mut guard = rec.lock();
    if guard.is_enabled() {
        let ptr = std::ptr::from_ref(handle) as usize;
        guard.record_get_from_proposal(ptr, key.as_slice());
    }
}

/// Records a `fwd_batch` call.
pub(crate) fn record_batch(values: BorrowedKeyValuePairs<'_>) {
    if let Some(rec) = recorder() {
        let mut guard = rec.lock();
        if guard.is_enabled() {
            guard.record_batch(values.as_slice());
        }
    }
}

/// Records a `fwd_propose_on_db` call after the proposal is created.
pub(crate) fn record_propose_on_db(
    result: &crate::ProposalResult<'_>,
    values: BorrowedKeyValuePairs<'_>,
) {
    let Some(rec) = recorder() else { return };

    let crate::ProposalResult::Ok { handle, .. } = result else {
        return;
    };

    let mut guard = rec.lock();
    if guard.is_enabled() {
        let ptr = std::ptr::from_ref(&**handle) as usize;
        guard.record_propose_on_db(ptr, values.as_slice());
    }
}

/// Records a `fwd_propose_on_proposal` call after the proposal is created.
pub(crate) fn record_propose_on_proposal<'db>(
    parent: Option<&crate::ProposalHandle<'db>>,
    result: &crate::ProposalResult<'db>,
    values: BorrowedKeyValuePairs<'_>,
) {
    let Some(rec) = recorder() else { return };
    let Some(parent) = parent else { return };
    let crate::ProposalResult::Ok { handle, .. } = result else {
        return;
    };

    let mut guard = rec.lock();
    if guard.is_enabled() {
        let parent_ptr = std::ptr::from_ref(parent) as usize;
        let new_ptr = std::ptr::from_ref(&**handle) as usize;
        guard.record_propose_on_proposal(parent_ptr, new_ptr, values.as_slice());
    }
}

/// Records a `fwd_commit_proposal` call after the commit completes.
pub(crate) fn record_commit(
    proposal_ptr: Option<*const crate::ProposalHandle<'_>>,
    result: &crate::HashResult,
) {
    let Some(rec) = recorder() else { return };
    let Some(ptr) = proposal_ptr else { return };

    let returned_hash_bytes = match result {
        crate::HashResult::Some(hash) => {
            let api_hash: firewood::v2::api::HashKey = (*hash).into();
            let bytes: [u8; 32] = api_hash.into();
            Some(bytes)
        }
        crate::HashResult::None => None,
        _ => return,
    };

    let mut guard = rec.lock();
    if guard.is_enabled() {
        guard.record_commit(ptr as usize, returned_hash_bytes.as_ref().map(AsRef::as_ref));
    }
}

/// Flushes any buffered operations to disk.
///
/// Called automatically when the database is closed, but can also be
/// invoked manually via `fwd_block_replay_flush`.
pub(crate) fn flush_to_disk() -> io::Result<()> {
    let Some(rec) = recorder() else {
        return Ok(());
    };

    let mut guard = rec.lock();
    if guard.is_enabled() {
        guard.flush_to_disk()
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorder_disabled_without_env_var() {
        // Ensure the env var is not set for this test.
        // SAFETY: This test is single-threaded and does not rely on other code
        // reading this env var concurrently.
        unsafe {
            std::env::remove_var(REPLAY_PATH_ENV);
        }

        // recorder() should return None when env var is not set
        assert!(recorder().is_none());
    }
}
