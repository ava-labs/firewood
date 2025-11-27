use core::ffi::c_void;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use parking_lot::Mutex;
use rkyv::{Archive, Deserialize, Serialize};

use crate::value::{BorrowedBytes, BorrowedKeyValuePairs, HashKey as FfiHashKey, KeyValuePair};

const BLOCK_REPLAY_ENV_VAR: &str = "FIREWOOD_BLOCK_REPLAY_PATH";

#[derive(Debug, Clone, Archive, Serialize, Deserialize)]
pub struct GetLatest {
    key: Box<[u8]>,
}

#[derive(Debug, Clone, Archive, Serialize, Deserialize)]
pub struct GetFromProposal {
    proposal_id: u64,
    key: Box<[u8]>,
}

#[derive(Debug, Clone, Archive, Serialize, Deserialize)]
pub struct GetFromRoot {
    root: Box<[u8]>,
    key: Box<[u8]>,
}

/// A single key/value operation in a batch or proposal.
///
/// If `value` is `None`, this represents a delete-range operation for `key`.
#[derive(Debug, Clone, Archive, Serialize, Deserialize)]
pub struct KeyValueOp {
    key: Box<[u8]>,
    value: Option<Box<[u8]>>,
}

#[derive(Debug, Clone, Archive, Serialize, Deserialize)]
pub struct Batch {
    pairs: Vec<KeyValueOp>,
}

// propose on db / propose on proposal, different types?

#[derive(Debug, Clone, Archive, Serialize, Deserialize)]
pub struct ProposeOnDB {
    pairs: Vec<KeyValueOp>,
    returned_proposal_id: u64,
}

#[derive(Debug, Clone, Archive, Serialize, Deserialize)]
pub struct ProposeOnProposal {
    proposal_id: u64,
    pairs: Vec<KeyValueOp>,
    returned_proposal_id: u64,
}

#[derive(Debug, Clone, Archive, Serialize, Deserialize)]
pub struct Commit {
    proposal_id: u64,
    returned_hash: Option<Box<[u8]>>,
}

#[derive(Debug, Clone, Archive, Serialize, Deserialize)]
pub enum DbOperation {
    GetLatest(GetLatest),
    GetFromProposal(GetFromProposal),
    GetFromRoot(GetFromRoot),
    Batch(Batch),
    ProposeOnDB(ProposeOnDB),
    ProposeOnProposal(ProposeOnProposal),
    Commit(Commit),
}

/// The top-level structure that is serialized to the replay log.
#[derive(Debug, Archive, Serialize, Deserialize)]
pub struct ReplayLog {
    operations: Vec<DbOperation>,
}

impl ReplayLog {
    #[must_use]
    pub fn new(operations: Vec<DbOperation>) -> Self {
        Self { operations }
    }
}

#[derive(Debug)]
struct ReplayRecorder {
    operations: Vec<DbOperation>,
    next_proposal_id: u64,
    proposal_ids: HashMap<usize, u64>,
    output_path: Option<PathBuf>,
}

impl ReplayRecorder {
    fn new() -> Self {
        let output_path = std::env::var(BLOCK_REPLAY_ENV_VAR)
            .ok()
            .map(PathBuf::from);

        Self {
            operations: Vec::new(),
            // start from 1 to make "0" stand out if it ever appears
            next_proposal_id: 1,
            proposal_ids: HashMap::new(),
            output_path,
        }
    }

    fn is_enabled(&self) -> bool {
        self.output_path.is_some()
    }

    fn record_get_latest(&mut self, key: &[u8]) {
        self.operations
            .push(DbOperation::GetLatest(GetLatest { key: key.into() }));
    }

    fn record_get_from_root(&mut self, root: &[u8], key: &[u8]) {
        self.operations.push(DbOperation::GetFromRoot(GetFromRoot {
            root: root.into(),
            key: key.into(),
        }));
    }

    fn record_batch(&mut self, pairs: &[KeyValuePair<'_>]) {
        let pairs = pairs
            .iter()
            .map(|kv| KeyValueOp {
                key: kv.key.as_slice().into(),
                value: if kv.value.is_null() {
                    None
                } else {
                    Some(kv.value.as_slice().into())
                },
            })
            .collect();
        self.operations.push(DbOperation::Batch(Batch { pairs }));
    }

    fn record_propose_on_db(&mut self, handle_ptr: *const c_void, pairs: &[KeyValuePair<'_>]) {
        let proposal_id = self.next_proposal_id;
        self.next_proposal_id = self
            .next_proposal_id
            .checked_add(1)
            .unwrap_or(self.next_proposal_id);

        self.proposal_ids.insert(handle_ptr as usize, proposal_id);

        let pairs = pairs
            .iter()
            .map(|kv| KeyValueOp {
                key: kv.key.as_slice().into(),
                value: if kv.value.is_null() {
                    None
                } else {
                    Some(kv.value.as_slice().into())
                },
            })
            .collect();

        self.operations
            .push(DbOperation::ProposeOnDB(ProposeOnDB {
                pairs,
                returned_proposal_id: proposal_id,
            }));
    }

    fn record_propose_on_proposal(
        &mut self,
        parent_ptr: *const c_void,
        new_ptr: *const c_void,
        pairs: &[KeyValuePair<'_>],
    ) {
        let Some(&proposal_id) = self.proposal_ids.get(&(parent_ptr as usize)) else {
            return;
        };

        let new_id = self.next_proposal_id;
        self.next_proposal_id = self
            .next_proposal_id
            .checked_add(1)
            .unwrap_or(self.next_proposal_id);

        self.proposal_ids.insert(new_ptr as usize, new_id);

        let pairs = pairs
            .iter()
            .map(|kv| KeyValueOp {
                key: kv.key.as_slice().into(),
                value: if kv.value.is_null() {
                    None
                } else {
                    Some(kv.value.as_slice().into())
                },
            })
            .collect();

        self.operations
            .push(DbOperation::ProposeOnProposal(ProposeOnProposal {
                proposal_id,
                pairs,
                returned_proposal_id: new_id,
            }));
    }

    fn record_get_from_proposal(&mut self, handle_ptr: *const c_void, key: &[u8]) {
        let Some(&proposal_id) = self.proposal_ids.get(&(handle_ptr as usize)) else {
            return;
        };

        self.operations
            .push(DbOperation::GetFromProposal(GetFromProposal {
                proposal_id,
                key: key.into(),
            }));
    }

    fn record_commit(&mut self, handle_ptr: *const c_void, returned_hash: Option<&[u8]>) {
        let Some(&proposal_id) = self.proposal_ids.get(&(handle_ptr as usize)) else {
            return;
        };

        self.operations
            .push(DbOperation::Commit(Commit {
                proposal_id,
                returned_hash: returned_hash.map(Into::into),
            }));
    }

    fn to_log(&self) -> ReplayLog {
        ReplayLog::new(self.operations.clone())
    }

    fn write_to_disk(&self) -> std::io::Result<()> {
        let Some(path) = &self.output_path else {
            // Block replay is considered disabled if the path is not set.
            return Ok(());
        };

        let log = self.to_log();

        // Use rkyv to serialize the log. We ignore the specific error and just surface
        // a generic IO error if serialization fails.
        let bytes = match rkyv::to_bytes::<rkyv::rancor::Error>(&log) {
            Ok(bytes) => bytes,
            Err(_err) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "failed to serialize block replay log",
                ));
            }
        };

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::write(path, bytes.as_ref())
    }
}

static RECORDER: OnceLock<Mutex<ReplayRecorder>> = OnceLock::new();

fn recorder() -> Option<&'static Mutex<ReplayRecorder>> {
    // If the environment variable is not set, treat block replay as disabled.
    if std::env::var(BLOCK_REPLAY_ENV_VAR).is_err() {
        return None;
    }

    Some(RECORDER.get_or_init(|| Mutex::new(ReplayRecorder::new())))
}

pub(crate) fn record_get_latest(key: BorrowedBytes<'_>) {
    if let Some(rec) = recorder() {
        let mut recorder = rec.lock();
        if recorder.is_enabled() {
            recorder.record_get_latest(key.as_slice());
        }
    }
}

pub(crate) fn record_get_from_root(root: FfiHashKey, key: BorrowedBytes<'_>) {
    if let Some(rec) = recorder() {
        let mut recorder = rec.lock();
        if recorder.is_enabled() {
            // Convert the FFI hash key back into raw bytes.
            let api_hash: firewood::v2::api::HashKey = root.into();
            let bytes: [u8; 32] = api_hash.into();
            recorder.record_get_from_root(&bytes, key.as_slice());
        }
    }
}

pub(crate) fn record_batch(values: BorrowedKeyValuePairs<'_>) {
    if let Some(rec) = recorder() {
        let mut recorder = rec.lock();
        if recorder.is_enabled() {
            recorder.record_batch(values.as_slice());
        }
    }
}

pub(crate) fn record_propose_on_db<'db>(
    result: &crate::ProposalResult<'db>,
    values: BorrowedKeyValuePairs<'_>,
) {
    let Some(rec) = recorder() else {
        return;
    };

    let mut recorder = rec.lock();
    if !recorder.is_enabled() {
        return;
    }

    if let crate::ProposalResult::Ok { handle, .. } = result {
        let handle_ptr = (&**handle) as *const crate::ProposalHandle<'db> as *const c_void;
        recorder.record_propose_on_db(handle_ptr, values.as_slice());
    }
}

pub(crate) fn record_propose_on_proposal<'db>(
    parent: Option<&crate::ProposalHandle<'db>>,
    result: &crate::ProposalResult<'db>,
    values: BorrowedKeyValuePairs<'_>,
) {
    let Some(rec) = recorder() else {
        return;
    };

    let mut recorder = rec.lock();
    if !recorder.is_enabled() {
        return;
    }

    let Some(parent_handle) = parent else {
        return;
    };

    if let crate::ProposalResult::Ok { handle: new_handle, .. } = result {
        let parent_ptr = (parent_handle as *const crate::ProposalHandle<'db>) as *const c_void;
        let new_ptr = (&**new_handle) as *const crate::ProposalHandle<'db> as *const c_void;
        recorder.record_propose_on_proposal(parent_ptr, new_ptr, values.as_slice());
    }
}

pub(crate) fn record_get_from_proposal<'db>(
    handle: Option<&crate::ProposalHandle<'db>>,
    key: BorrowedBytes<'_>,
) {
    let Some(rec) = recorder() else {
        return;
    };

    let mut recorder = rec.lock();
    if !recorder.is_enabled() {
        return;
    }

    let Some(handle) = handle else {
        return;
    };

    let handle_ptr = (handle as *const crate::ProposalHandle<'db>) as *const c_void;
    recorder.record_get_from_proposal(handle_ptr, key.as_slice());
}

pub(crate) fn record_commit<'db>(
    proposal_ptr: Option<*const crate::ProposalHandle<'db>>,
    result: &crate::HashResult,
) {
    let Some(rec) = recorder() else {
        return;
    };

    let mut recorder = rec.lock();
    if !recorder.is_enabled() {
        return;
    }

    let Some(handle_ptr) = proposal_ptr else {
        return;
    };

    let handle_ptr = handle_ptr as *const c_void;

    let returned_hash_bytes = match result {
        crate::HashResult::Some(hash) => {
            let api_hash: firewood::v2::api::HashKey = (*hash).into();
            let bytes: [u8; 32] = api_hash.into();
            Some(bytes)
        }
        crate::HashResult::None => None,
        _ => return,
    };

    recorder.record_commit(
        handle_ptr,
        returned_hash_bytes.as_ref().map(std::convert::AsRef::as_ref),
    );
}

/// Flushes the current replay log to the path specified by `FIREWOOD_BLOCK_REPLAY_PATH`.
///
/// If the environment variable is not set, this is a no-op.
pub(crate) fn flush_to_disk() -> std::io::Result<()> {
    let Some(rec) = recorder() else {
        return Ok(());
    };

    let recorder = rec.lock();
    if !recorder.is_enabled() {
        return Ok(());
    }

    recorder.write_to_disk()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_get_latest_adds_operation() {
        unsafe {
            std::env::set_var(BLOCK_REPLAY_ENV_VAR, "/tmp/firewood-block-replay-test.rkyv");
        }

        let key = BorrowedBytes::from_slice(b"hello");
        record_get_latest(key);

        let rec = recorder().expect("recorder should be initialized");
        let rec = rec.lock();

        assert!(
            matches!(rec.operations.last(), Some(DbOperation::GetLatest(_))),
            "expected last operation to be GetLatest, got {:?}",
            rec.operations.last()
        );
    }
}
