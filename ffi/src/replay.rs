use core::ffi::c_void;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

use firewood::db::{BatchOp, Db};
use firewood::v2::api::{
    self, Db as DbApi, DbView as DbViewApi, HashKeyExt, Proposal as ProposalApi,
};
use firewood_storage::{InvalidTrieHashLength, TrieHash};
use parking_lot::Mutex;
use rkyv::{Archive, Deserialize, Serialize};
use thiserror::Error;

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
    flush_threshold: usize,
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
            flush_threshold: 10_000,
        }
    }

    fn is_enabled(&self) -> bool {
        self.output_path.is_some()
    }

    fn record_get_latest(&mut self, key: &[u8]) {
        self.operations
            .push(DbOperation::GetLatest(GetLatest { key: key.into() }));
        self.maybe_flush();
    }

    fn record_get_from_root(&mut self, root: &[u8], key: &[u8]) {
        self.operations.push(DbOperation::GetFromRoot(GetFromRoot {
            root: root.into(),
            key: key.into(),
        }));
        self.maybe_flush();
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
        self.maybe_flush();
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
        self.maybe_flush();
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
        self.maybe_flush();
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
        self.maybe_flush();
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
        self.maybe_flush();
    }

    fn to_log(&self) -> ReplayLog {
        ReplayLog::new(self.operations.clone())
    }

    fn flush_to_disk_inner(&mut self) -> io::Result<()> {
        let Some(path) = &self.output_path else {
            // Block replay is considered disabled if the path is not set.
            return Ok(());
        };

        if self.operations.is_empty() {
            return Ok(());
        }

        // Move the current operations out so we can clear the in-memory buffer even if
        // the write fails. This is best-effort logging and should not affect the main
        // database behavior.
        let operations = std::mem::take(&mut self.operations);
        let log = ReplayLog::new(operations);

        // Use rkyv to serialize the log. We ignore the specific error and just surface
        // a generic IO error if serialization fails.
        let bytes = match rkyv::to_bytes::<rkyv::rancor::Error>(&log) {
            Ok(bytes) => bytes,
            Err(_err) => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "failed to serialize block replay log",
                ));
            }
        };

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Append as a length-prefixed segment so the log file can contain multiple
        // replay segments.
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        let len: u64 = bytes
            .len()
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "segment too large to log"))?;

        file.write_all(&len.to_le_bytes())?;
        file.write_all(bytes.as_ref())?;

        Ok(())
    }

    fn maybe_flush(&mut self) {
        if self.operations.len() >= self.flush_threshold {
            // Ignore errors here; logging must not interfere with the main path.
            let _ = self.flush_to_disk_inner();
        }
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

    let mut recorder = rec.lock();
    if !recorder.is_enabled() {
        return Ok(());
    }

    recorder.flush_to_disk_inner()
}

/// Error type returned when replaying a block replay log against a database.
#[derive(Debug, Error)]
pub enum ReplayError {
    /// An I/O error occurred while reading the replay log.
    #[error("I/O error while reading replay log: {0}")]
    Io(#[from] io::Error),
    /// The log could not be deserialized from the rkyv format.
    #[error("failed to decode replay segment: {0}")]
    Decode(#[from] rkyv::rancor::Error),
    /// A database error occurred while applying an operation.
    #[error("database error while applying operation: {0}")]
    Db(#[from] api::Error),
    /// A root hash in the replay log had an invalid length.
    #[error("invalid root hash in replay log: {0}")]
    InvalidHash(#[from] InvalidTrieHashLength),
    /// The replay log referenced a proposal ID that has not been created.
    #[error("unknown proposal id {0} in replay log")]
    UnknownProposal(u64),
}

fn kv_ops_to_batch_ops(pairs: &[KeyValueOp]) -> Vec<BatchOp<Box<[u8]>, Box<[u8]>>> {
    pairs
        .iter()
        .map(|op| match &op.value {
            Some(value) => BatchOp::Put {
                key: op.key.clone(),
                value: value.clone(),
            },
            None => BatchOp::DeleteRange {
                prefix: op.key.clone(),
            },
        })
        .collect()
}

fn apply_operation<'db>(
    db: &'db Db,
    proposals: &mut HashMap<u64, firewood::db::Proposal<'db>>,
    operation: DbOperation,
) -> Result<(), ReplayError> {
    match operation {
        DbOperation::GetLatest(GetLatest { key }) => {
            // This is primarily a verification step; the result is discarded.
            let root = match DbApi::root_hash(db)? {
                Some(root) => root,
                None => {
                    return Err(api::Error::RevisionNotFound {
                        provided: api::HashKey::default_root_hash(),
                    }
                    .into());
                }
            };
            let view = DbApi::revision(db, root)?;
            let _ = DbViewApi::val(&*view, key)?;
        }
        DbOperation::GetFromRoot(GetFromRoot { root, key }) => {
            let hash = TrieHash::try_from(root.as_ref())?;
            let view = DbApi::revision(db, hash)?;
            let _ = DbViewApi::val(&*view, key)?;
        }
        DbOperation::GetFromProposal(GetFromProposal { proposal_id, key }) => {
            let proposal = proposals
                .get(&proposal_id)
                .ok_or(ReplayError::UnknownProposal(proposal_id))?;
            let _ = DbViewApi::val(proposal, key)?;
        }
        DbOperation::Batch(Batch { pairs }) => {
            let ops = kv_ops_to_batch_ops(&pairs);
            let proposal = DbApi::propose(db, ops)?;
            proposal.commit()?;
        }
        DbOperation::ProposeOnDB(ProposeOnDB {
            pairs,
            returned_proposal_id,
        }) => {
            let ops = kv_ops_to_batch_ops(&pairs);
            let proposal = DbApi::propose(db, ops)?;
            proposals.insert(returned_proposal_id, proposal);
        }
        DbOperation::ProposeOnProposal(ProposeOnProposal {
            proposal_id,
            pairs,
            returned_proposal_id,
        }) => {
            let ops = kv_ops_to_batch_ops(&pairs);
            let new_proposal = {
                let parent = proposals
                    .get(&proposal_id)
                    .ok_or(ReplayError::UnknownProposal(proposal_id))?;
                ProposalApi::propose(parent, ops)?
            };
            proposals.insert(returned_proposal_id, new_proposal);
        }
        DbOperation::Commit(Commit { proposal_id, .. }) => {
            let proposal = proposals
                .remove(&proposal_id)
                .ok_or(ReplayError::UnknownProposal(proposal_id))?;
            proposal.commit()?;
        }
    }

    Ok(())
}

/// Replays all operations from a block replay log into the provided database.
///
/// The log is expected to be in the length-prefixed format produced by the
/// recorder in this module: a sequence of segments, each encoded as:
///
/// `[len: u64 little-endian][rkyv(ReplayLog)]`.
///
/// This function assumes that `db` is opened on an empty database and will
/// apply all operations in order.
pub fn replay_log_from_reader<R: Read>(mut reader: R, db: &Db) -> Result<(), ReplayError> {
    let mut proposals: HashMap<u64, firewood::db::Proposal<'_>> = HashMap::new();

    loop {
        let mut len_buf = [0u8; 8];
        match reader.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(ReplayError::Io(err)),
        }

        let len = u64::from_le_bytes(len_buf);
        if len == 0 {
            continue;
        }

        let mut buf = vec![0u8; len as usize];
        reader.read_exact(&mut buf)?;

        let log: ReplayLog = rkyv::from_bytes::<ReplayLog, rkyv::rancor::Error>(&buf)?;
        for op in log.operations {
            apply_operation(db, &mut proposals, op)?;
        }
    }

    Ok(())
}

/// Convenience helper to replay a block replay log from the file at `path`
/// into the provided database.
pub fn replay_log_from_file(path: impl AsRef<std::path::Path>, db: &Db) -> Result<(), ReplayError> {
    let file = fs::File::open(path)?;
    replay_log_from_reader(file, db)
}

#[cfg(test)]
mod tests {
    use super::*;
    use firewood::db::DbConfig;
    use firewood::manager::RevisionManagerConfig;
    use std::io::Cursor;
    use tempfile::tempdir;

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

    #[test]
    fn replay_log_applies_batch_and_proposals() {
        let tmpdir = tempdir().expect("create tempdir");
        let db_path = tmpdir.path().join("replay.db");

        let cfg = DbConfig::builder()
            .truncate(true)
            .manager(RevisionManagerConfig::builder().build())
            .build();
        let db = Db::new(&db_path, cfg).expect("db initiation should succeed");

        // Create a simple log:
        // 1. Batch inserting keys 0..5
        // 2. Proposal on DB inserting keys 5..10
        // 3. Commit that proposal
        let mut ops = Vec::new();

        let batch_pairs = (0u8..5)
            .map(|i| KeyValueOp {
                key: vec![i].into_boxed_slice(),
                value: Some(vec![i + 1].into_boxed_slice()),
            })
            .collect();
        ops.push(DbOperation::Batch(Batch {
            pairs: batch_pairs,
        }));

        let proposal_pairs = (5u8..10)
            .map(|i| KeyValueOp {
                key: vec![i].into_boxed_slice(),
                value: Some(vec![i + 1].into_boxed_slice()),
            })
            .collect();
        ops.push(DbOperation::ProposeOnDB(ProposeOnDB {
            pairs: proposal_pairs,
            returned_proposal_id: 1,
        }));
        ops.push(DbOperation::Commit(Commit {
            proposal_id: 1,
            returned_hash: None,
        }));

        let log = ReplayLog::new(ops);
        let bytes =
            rkyv::to_bytes::<rkyv::rancor::Error>(&log).expect("serializing replay log must work");

        let mut buf = Vec::new();
        let len: u64 = bytes.len().try_into().expect("fits in u64");
        buf.extend_from_slice(&len.to_le_bytes());
        buf.extend_from_slice(bytes.as_ref());

        replay_log_from_reader(Cursor::new(buf), &db).expect("replay should succeed");

        // Verify that all keys 0..10 are present with the expected values.
        use firewood::v2::api::DbView as _;

        let root = DbApi::root_hash(&db)
            .expect("root_hash should succeed")
            .expect("root should not be empty");
        let view = DbApi::revision(&db, root).expect("revision should succeed");

        for i in 0u8..10 {
            let expected = vec![i + 1].into_boxed_slice();
            let value = DbViewApi::val(&*view, vec![i].as_slice())
                .expect("val should succeed")
                .expect("value should exist");
            assert_eq!(value, expected, "value mismatch for key {}", i);
        }
    }
}
