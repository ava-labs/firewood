// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.
pub mod build;
pub mod search;

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Write};
use std::time::Instant;

use firewood::db::{BatchOp, Db};
use firewood::v2::api::{self, Db as DbApi, DbView as DbViewApi, Proposal as ProposalApi};
use firewood_storage::{InvalidTrieHashLength, firewood_counter};
use rkyv::{Archive, Deserialize, Serialize};
use thiserror::Error;

/// Operation that reads the latest revision.
#[derive(Debug, Clone, Archive, Serialize, Deserialize, serde::Serialize, serde::Deserialize)]
pub struct GetLatest {
    pub key: Box<[u8]>,
}

/// Operation that reads from a proposal by ID.
#[derive(Debug, Clone, Archive, Serialize, Deserialize, serde::Serialize, serde::Deserialize)]
pub struct GetFromProposal {
    pub proposal_id: u64,
    pub key: Box<[u8]>,
}

/// Operation that reads from a specific root hash.
#[derive(Debug, Clone, Archive, Serialize, Deserialize, serde::Serialize, serde::Deserialize)]
pub struct GetFromRoot {
    pub root: Box<[u8]>,
    pub key: Box<[u8]>,
}

/// A single key/value operation in a batch or proposal.
///
/// If `value` is `None`, this represents a delete-range operation for `key`.
#[derive(Debug, Clone, Archive, Serialize, Deserialize, serde::Serialize, serde::Deserialize)]
pub struct KeyValueOp {
    pub key: Box<[u8]>,
    pub value: Option<Box<[u8]>>,
}

/// Batch operation directly on the database.
#[derive(Debug, Clone, Archive, Serialize, Deserialize, serde::Serialize, serde::Deserialize)]
pub struct Batch {
    pub pairs: Vec<KeyValueOp>,
}

/// Proposal created on the database.
#[derive(Debug, Clone, Archive, Serialize, Deserialize, serde::Serialize, serde::Deserialize)]
pub struct ProposeOnDB {
    pub pairs: Vec<KeyValueOp>,
    pub returned_proposal_id: u64,
}

/// Proposal created on top of another proposal.
#[derive(Debug, Clone, Archive, Serialize, Deserialize, serde::Serialize, serde::Deserialize)]
pub struct ProposeOnProposal {
    pub proposal_id: u64,
    pub pairs: Vec<KeyValueOp>,
    pub returned_proposal_id: u64,
}

/// Commit operation for a proposal.
#[derive(Debug, Clone, Archive, Serialize, Deserialize, serde::Serialize, serde::Deserialize)]
pub struct Commit {
    pub proposal_id: u64,
    pub returned_hash: Option<Box<[u8]>>,
}

/// All supported database operations recorded in the replay log.
#[derive(Debug, Clone, Archive, Serialize, Deserialize, serde::Serialize, serde::Deserialize)]
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
#[derive(Debug, Archive, Serialize, Deserialize, serde::Serialize, serde::Deserialize)]
pub struct ReplayLog {
    pub operations: Vec<DbOperation>,
}

impl ReplayLog {
    #[must_use]
    pub fn new(operations: Vec<DbOperation>) -> Self {
        Self { operations }
    }
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

/// Convert a length-prefixed rkyv-encoded replay log file into a length-prefixed
/// MessagePack (rmp-serde) file containing the same [`ReplayLog`] segments.
pub fn convert_rkyv_log_to_rmp_file(
    rkyv_path: impl AsRef<std::path::Path>,
    rmp_path: impl AsRef<std::path::Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut reader = fs::File::open(rkyv_path)?;
    let mut writer = fs::File::create(rmp_path)?;

    loop {
        let mut len_buf = [0u8; 8];
        match reader.read_exact(&mut len_buf) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(Box::new(err)),
        }

        let len = u64::from_le_bytes(len_buf);
        if len == 0 {
            continue;
        }

        let mut buf = vec![0u8; len as usize];
        reader.read_exact(&mut buf)?;

        let log: ReplayLog = rkyv::from_bytes::<ReplayLog, rkyv::rancor::Error>(&buf)?;

        let msgpack = rmp_serde::to_vec_named(&log)?;
        let out_len: u64 = msgpack
            .len()
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "segment too large"))?;

        writer.write_all(&out_len.to_le_bytes())?;
        writer.write_all(&msgpack)?;
    }

    Ok(())
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
    commits: &mut u64,
) -> Result<Option<Box<[u8]>>, ReplayError> {
    let mut new_hash = None;

    match operation {
        DbOperation::GetLatest(GetLatest { key }) => {
            // This is primarily a verification step; the result is discarded.
            //
            if let Some(root) = DbApi::root_hash(db)? {
                let view = DbApi::revision(db, root)?;
                let _ = DbViewApi::val(&*view, key)?;
            }
        }
        DbOperation::GetFromRoot(GetFromRoot { root, key }) => {
            // we disable this check for now
            // reason: replay log seems to be racy on get-from-root
            // if (hex::encode(&root)
            //     != "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")
            // {
            //     let hash = TrieHash::try_from(root.as_ref())?;
            //     let view = DbApi::revision(db, hash)?;
            //     let _ = DbViewApi::val(&*view, key)?;
            // }
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
            let start = Instant::now();
            let proposal = DbApi::propose(db, ops)?;
            firewood_counter!("firewood.replay.propose_ns", "")
                .increment(start.elapsed().as_nanos() as u64);
            firewood_counter!("firewood.replay.propose", "").increment(1);
            proposals.insert(returned_proposal_id, proposal);
        }
        DbOperation::ProposeOnProposal(ProposeOnProposal {
            proposal_id,
            pairs,
            returned_proposal_id,
        }) => {
            let ops = kv_ops_to_batch_ops(&pairs);
            let start = Instant::now();
            let new_proposal = {
                let parent = proposals
                    .get(&proposal_id)
                    .ok_or(ReplayError::UnknownProposal(proposal_id))?;
                ProposalApi::propose(parent, ops)?
            };
            firewood_counter!("firewood.replay.propose_ns", "")
                .increment(start.elapsed().as_nanos() as u64);
            firewood_counter!("firewood.replay.propose", "").increment(1);
            proposals.insert(returned_proposal_id, new_proposal);
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
            firewood_counter!("firewood.replay.commit_ns", "")
                .increment(start.elapsed().as_nanos() as u64);
            firewood_counter!("firewood.replay.commit", "").increment(1);
            *commits += 1;
            new_hash = returned_hash;
        }
    }

    Ok(new_hash)
}

/// Replays all operations from a block replay log into the provided database.
///
/// The log is expected to be in the length-prefixed format produced by the
/// recorder: a sequence of segments, each encoded as:
///
/// `[len: u64 little-endian][rkyv(ReplayLog)]`.
///
/// This function assumes that `db` is opened on an empty database and will
/// apply all operations in order.
pub fn replay_log_from_reader<R: Read>(
    mut reader: R,
    db: &Db,
    max_n_commits: Option<u64>,
) -> Result<Option<Box<[u8]>>, ReplayError> {
    let mut proposals: HashMap<u64, firewood::db::Proposal<'_>> = HashMap::new();
    let mut last_commit_hash = None;
    let mut total = 0;
    let mut logs = Vec::new();
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
        total += log.operations.len();
        logs.push(log);
    }
    let mut i = 0;
    let mut commits = 0;
    let max = max_n_commits.unwrap_or(u64::MAX);
    for log in logs {
        for op in log.operations {
            let res = apply_operation(db, &mut proposals, op, &mut commits)?;
            if i % 1000 == 0 {
                println!("Done: {}/{}", i, total);
            }
            i += 1;
            if let Some(last) = res {
                last_commit_hash = Some(last);
            }
            if commits > max {
                return Ok(last_commit_hash);
            }
        }
    }

    Ok(last_commit_hash)
}

/// Convenience helper to replay a block replay log from the file at `path`
/// into the provided database.
pub fn replay_log_from_file(
    path: impl AsRef<std::path::Path>,
    db: &Db,
    max_n_commits: Option<u64>,
) -> Result<Option<Box<[u8]>>, ReplayError> {
    let file = fs::File::open(path)?;
    replay_log_from_reader(file, db, max_n_commits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use firewood::db::DbConfig;
    use firewood::manager::RevisionManagerConfig;
    use std::io::Cursor;
    use tempfile::tempdir;

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
        ops.push(DbOperation::Batch(Batch { pairs: batch_pairs }));

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

        replay_log_from_reader(Cursor::new(buf), &db, None).expect("replay should succeed");

        // Verify that all keys 0..10 are present with the expected values.
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
