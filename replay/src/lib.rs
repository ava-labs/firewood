// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Replay log types and engine for Firewood database operations.
//!
//! This crate provides:
//! - Serializable types representing database operations ([`DbOperation`])
//! - A replay log container ([`ReplayLog`]) for batching operations
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
use firewood::v2::api::{self, Db as DbApi, DbView as DbViewApi, Proposal as ProposalApi};
use firewood_storage::{firewood_counter, InvalidTrieHashLength};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Serde helper for `Option<Box<[u8]>>` using efficient byte representation.
mod option_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    #[expect(clippy::ref_option, reason = "serde requires &T for serialize")]
    pub fn serialize<S: Serializer>(value: &Option<Box<[u8]>>, ser: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(bytes) => ser.serialize_some(&serde_bytes::Bytes::new(bytes)),
            None => ser.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Option<Box<[u8]>>, D::Error> {
        let opt: Option<serde_bytes::ByteBuf> = Deserialize::deserialize(de)?;
        Ok(opt.map(|b| b.into_vec().into_boxed_slice()))
    }
}

// ============================================================================
// Operation Types
// ============================================================================

/// Operation that reads a key from the latest committed revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetLatest {
    /// The key to read.
    #[serde(with = "serde_bytes")]
    pub key: Box<[u8]>,
}

/// Operation that reads a key from an uncommitted proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetFromProposal {
    /// The proposal ID assigned during recording.
    pub proposal_id: u64,
    /// The key to read.
    #[serde(with = "serde_bytes")]
    pub key: Box<[u8]>,
}

/// Operation that reads a key from a specific historical root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetFromRoot {
    /// The 32-byte root hash.
    #[serde(with = "serde_bytes")]
    pub root: Box<[u8]>,
    /// The key to read.
    #[serde(with = "serde_bytes")]
    pub key: Box<[u8]>,
}

/// A single key/value mutation within a batch or proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyValueOp {
    /// The key being mutated.
    #[serde(with = "serde_bytes")]
    pub key: Box<[u8]>,
    /// The value to set, or `None` for a delete-range operation.
    #[serde(with = "option_bytes")]
    pub value: Option<Box<[u8]>>,
}

/// Batch operation that commits immediately.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Batch {
    /// The key/value operations in this batch.
    pub pairs: Vec<KeyValueOp>,
}

/// Proposal created directly on the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposeOnDB {
    /// The key/value operations in this proposal.
    pub pairs: Vec<KeyValueOp>,
    /// The proposal ID assigned to the returned handle.
    pub returned_proposal_id: u64,
}

/// Proposal created on top of an existing uncommitted proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposeOnProposal {
    /// The parent proposal ID.
    pub proposal_id: u64,
    /// The key/value operations in this proposal.
    pub pairs: Vec<KeyValueOp>,
    /// The proposal ID assigned to the returned handle.
    pub returned_proposal_id: u64,
}

/// Commit operation for a proposal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    /// The proposal ID being committed.
    pub proposal_id: u64,
    /// The root hash returned by the commit, if any.
    #[serde(with = "option_bytes")]
    pub returned_hash: Option<Box<[u8]>>,
}

/// All supported database operations that can be recorded and replayed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DbOperation {
    /// Read from the latest revision.
    GetLatest(GetLatest),
    /// Read from an uncommitted proposal.
    GetFromProposal(GetFromProposal),
    /// Read from a historical root.
    GetFromRoot(GetFromRoot),
    /// Batch write (immediate commit).
    Batch(Batch),
    /// Create a proposal on the database.
    ProposeOnDB(ProposeOnDB),
    /// Create a proposal on another proposal.
    ProposeOnProposal(ProposeOnProposal),
    /// Commit a proposal.
    Commit(Commit),
}

// ============================================================================
// Replay Log
// ============================================================================

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

// ============================================================================
// Error Types
// ============================================================================

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
    #[error("unknown proposal ID {0}")]
    UnknownProposal(u64),
}

// ============================================================================
// Replay Engine
// ============================================================================

/// Alias for the batch operation type used in replay.
type BoxedBatchOp = BatchOp<Box<[u8]>, Box<[u8]>>;

/// Converts a slice of [`KeyValueOp`] to firewood batch operations.
fn to_batch_ops(pairs: &[KeyValueOp]) -> Vec<BoxedBatchOp> {
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

/// Applies a single operation to the database.
///
/// Returns the root hash if the operation was a commit that produced one.
fn apply_operation<'db>(
    db: &'db Db,
    proposals: &mut HashMap<u64, Proposal<'db>>,
    operation: DbOperation,
) -> Result<Option<Box<[u8]>>, ReplayError> {
    match operation {
        DbOperation::GetLatest(GetLatest { key }) => {
            if let Some(root) = DbApi::root_hash(db)? {
                let view = DbApi::revision(db, root)?;
                let _ = DbViewApi::val(&*view, key)?;
            }
            Ok(None)
        }

        DbOperation::GetFromRoot(GetFromRoot { root: _, key: _ }) => {
            // Historical root reads are not replayed because the root may
            // not exist in the replayed database (different proposal IDs).
            Ok(None)
        }

        DbOperation::GetFromProposal(GetFromProposal { proposal_id, key }) => {
            let proposal = proposals
                .get(&proposal_id)
                .ok_or(ReplayError::UnknownProposal(proposal_id))?;
            let _ = DbViewApi::val(proposal, key)?;
            Ok(None)
        }

        DbOperation::Batch(Batch { pairs }) => {
            let ops = to_batch_ops(&pairs);
            let proposal = DbApi::propose(db, ops)?;
            proposal.commit()?;
            Ok(None)
        }

        DbOperation::ProposeOnDB(ProposeOnDB {
            pairs,
            returned_proposal_id,
        }) => {
            let ops = to_batch_ops(&pairs);
            let start = Instant::now();
            let proposal = DbApi::propose(db, ops)?;
            firewood_counter!("firewood.replay.propose_ns", "Time spent in propose (ns)")
                .increment(start.elapsed().as_nanos() as u64);
            firewood_counter!("firewood.replay.propose", "Number of propose calls").increment(1);
            proposals.insert(returned_proposal_id, proposal);
            Ok(None)
        }

        DbOperation::ProposeOnProposal(ProposeOnProposal {
            proposal_id,
            pairs,
            returned_proposal_id,
        }) => {
            let ops = to_batch_ops(&pairs);
            let start = Instant::now();
            let new_proposal = {
                let parent = proposals
                    .get(&proposal_id)
                    .ok_or(ReplayError::UnknownProposal(proposal_id))?;
                ProposalApi::propose(parent, ops)?
            };
            firewood_counter!("firewood.replay.propose_ns", "Time spent in propose (ns)")
                .increment(start.elapsed().as_nanos() as u64);
            firewood_counter!("firewood.replay.propose", "Number of propose calls").increment(1);
            proposals.insert(returned_proposal_id, new_proposal);
            Ok(None)
        }

        DbOperation::Commit(Commit {
            proposal_id,
            returned_hash,
        }) => {
            let proposal = proposals
                .remove(&proposal_id)
                .ok_or(ReplayError::UnknownProposal(proposal_id))?;
            let start = Instant::now();
            proposal.commit()?;
            firewood_counter!("firewood.replay.commit_ns", "Time spent in commit (ns)")
                .increment(start.elapsed().as_nanos() as u64);
            firewood_counter!("firewood.replay.commit", "Number of commit calls").increment(1);
            Ok(returned_hash)
        }
    }
}

/// Replays all operations from a length-prefixed replay log.
///
/// The log is expected to be a sequence of segments, each formatted as:
/// `[len: u64 LE][rkyv(ReplayLog) bytes]`
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
    let mut proposals: HashMap<u64, Proposal<'_>> = HashMap::new();
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
            if let Some(hash) = apply_operation(db, &mut proposals, op)? {
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use firewood::db::DbConfig;
    use firewood::manager::RevisionManagerConfig;
    use std::io::Cursor;
    use tempfile::tempdir;

    fn create_test_db() -> (tempfile::TempDir, Db) {
        let tmpdir = tempdir().expect("create tempdir");
        let db_path = tmpdir.path().join("test.db");
        let cfg = DbConfig::builder()
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
            })
            .collect();

        let log = ReplayLog::new(vec![DbOperation::Batch(Batch { pairs })]);
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
            })
            .collect();

        let ops = vec![
            DbOperation::ProposeOnDB(ProposeOnDB {
                pairs,
                returned_proposal_id: 1,
            }),
            DbOperation::Commit(Commit {
                proposal_id: 1,
                returned_hash: None,
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
                }],
                returned_proposal_id: 1,
            }),
            DbOperation::ProposeOnProposal(ProposeOnProposal {
                proposal_id: 1,
                pairs: vec![KeyValueOp {
                    key: vec![2].into_boxed_slice(),
                    value: Some(vec![20].into_boxed_slice()),
                }],
                returned_proposal_id: 2,
            }),
            DbOperation::Commit(Commit {
                proposal_id: 1,
                returned_hash: None,
            }),
            DbOperation::Commit(Commit {
                proposal_id: 2,
                returned_hash: None,
            }),
        ];

        let log = ReplayLog::new(ops);
        let buf = serialize_log(&log);

        replay_from_reader(Cursor::new(buf), &db, None).expect("replay");

        let root = DbApi::root_hash(&db)
            .expect("root_hash")
            .expect("non-empty");
        let view = DbApi::revision(&db, root).expect("revision");

        let v1 = DbViewApi::val(&*view, &[1]).expect("val").expect("exists");
        let v2 = DbViewApi::val(&*view, &[2]).expect("val").expect("exists");
        assert_eq!(*v1, [10]);
        assert_eq!(*v2, [20]);
    }

    #[test]
    fn replay_respects_max_commits() {
        let (_tmpdir, db) = create_test_db();

        let ops: Vec<DbOperation> = (0u8..10)
            .flat_map(|i| {
                vec![
                    DbOperation::ProposeOnDB(ProposeOnDB {
                        pairs: vec![KeyValueOp {
                            key: vec![i].into_boxed_slice(),
                            value: Some(vec![i].into_boxed_slice()),
                        }],
                        returned_proposal_id: u64::from(i),
                    }),
                    DbOperation::Commit(Commit {
                        proposal_id: u64::from(i),
                        returned_hash: None,
                    }),
                ]
            })
            .collect();

        let log = ReplayLog::new(ops);
        let buf = serialize_log(&log);

        // Only replay 3 commits
        replay_from_reader(Cursor::new(buf), &db, Some(3)).expect("replay");

        let root = DbApi::root_hash(&db)
            .expect("root_hash")
            .expect("non-empty");
        let view = DbApi::revision(&db, root).expect("revision");

        // Keys 0, 1, 2 should exist
        for i in 0u8..3 {
            assert!(
                DbViewApi::val(&*view, vec![i].as_slice())
                    .expect("val")
                    .is_some(),
                "key {i} should exist"
            );
        }

        // Key 3 should not exist (stopped after 3 commits)
        assert!(
            DbViewApi::val(&*view, &[3]).expect("val").is_none(),
            "key 3 should not exist"
        );
    }

    #[test]
    fn replay_unknown_proposal_errors() {
        let (_tmpdir, db) = create_test_db();

        let ops = vec![DbOperation::Commit(Commit {
            proposal_id: 999,
            returned_hash: None,
        })];

        let log = ReplayLog::new(ops);
        let buf = serialize_log(&log);

        let result = replay_from_reader(Cursor::new(buf), &db, None);
        assert!(matches!(result, Err(ReplayError::UnknownProposal(999))));
    }

    #[test]
    fn replay_empty_log_succeeds() {
        let (_tmpdir, db) = create_test_db();
        let result = replay_from_reader(Cursor::new(Vec::new()), &db, None);
        assert!(result.is_ok());
        assert!(result.expect("ok").is_none());
    }
}
