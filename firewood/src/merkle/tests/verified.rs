// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Tests for [`VerifiedRangeProof`] and [`VerifiedChangeProof`], which pair a
//! proof with the verification context produced from it, and for applying a
//! verified change proof to a database.

use std::num::NonZeroUsize;
use std::sync::Arc;

use firewood_macros::hash_mode;
use firewood_storage::{EthHash, HashMode, HashedNodeReader, MerkleDbHash, NodeHashAlgorithm};

use super::init_merkle_in;
use crate::api::{self, BatchOp, Db as _, DbView as _, Proposal as _};
use crate::db::{Db, DbConfig};
use crate::{ProofError, VerifiedChangeProof, VerifiedRangeProof};

fn kvs(n: u8) -> Vec<(Vec<u8>, Vec<u8>)> {
    (0..n).map(|i| (vec![i, i ^ 0x5A], vec![i; 4])).collect()
}

fn other_mode(algorithm: NodeHashAlgorithm) -> NodeHashAlgorithm {
    if algorithm.is_ethereum() {
        NodeHashAlgorithm::MerkleDB
    } else {
        NodeHashAlgorithm::Ethereum
    }
}

fn assert_hash_mode_mismatch(result: Result<impl std::fmt::Debug, api::Error>) {
    match result {
        Err(api::Error::ProofError(ProofError::HashModeMismatch { .. })) => {}
        other => panic!("expected HashModeMismatch, got {other:?}"),
    }
}

fn assert_unexpected_hash(result: Result<impl std::fmt::Debug, api::Error>) {
    match result {
        Err(api::Error::ProofError(ProofError::UnexpectedHash { .. })) => {}
        other => panic!("expected UnexpectedHash, got {other:?}"),
    }
}

#[hash_mode]
#[test]
fn range_verify_pairs_proof_and_context<H: HashMode>() {
    let merkle = init_merkle_in::<H, _, _, _>(kvs(16));
    let root = HashedNodeReader::root_hash(merkle.nodestore()).unwrap();
    let start = kvs(16)[2].0.clone();
    let limit = NonZeroUsize::new(5);
    let proof = Arc::new(merkle.range_proof(Some(&start), None, limit).unwrap());

    let verified = VerifiedRangeProof::verify(
        Arc::clone(&proof),
        root.clone(),
        Some(&start),
        None,
        H::ALGORITHM,
        limit,
    )
    .unwrap();

    // The type stores the very `Arc` it was given, not a copy of the body.
    assert!(Arc::ptr_eq(verified.proof(), &proof));
    let verification = verified.verification();
    assert_eq!(verification.root(), &root);
    assert_eq!(verification.start_key(), Some(start.as_slice()));
    assert_eq!(verification.end_key(), None);
    assert_eq!(verification.max_length(), limit);
    // Truncated by `limit` with no requested upper bound, so the proven right
    // edge is the last reported key.
    let last = proof.key_values().last().map(|(k, _)| &**k);
    assert_eq!(verification.right_edge_key(), last);
}

#[hash_mode]
#[test]
fn range_verify_rejects_wrong_root<H: HashMode>() {
    let merkle = init_merkle_in::<H, _, _, _>(kvs(8));
    let other = init_merkle_in::<H, _, _, _>(kvs(9));
    let wrong_root = HashedNodeReader::root_hash(other.nodestore()).unwrap();
    let proof = Arc::new(merkle.range_proof(None, None, None).unwrap());

    assert_unexpected_hash(VerifiedRangeProof::verify(
        proof,
        wrong_root,
        None,
        None,
        H::ALGORITHM,
        None,
    ));
}

#[hash_mode]
#[test]
fn range_verify_rejects_other_hash_mode<H: HashMode>() {
    let merkle = init_merkle_in::<H, _, _, _>(kvs(8));
    let root = HashedNodeReader::root_hash(merkle.nodestore()).unwrap();
    let proof = Arc::new(merkle.range_proof(None, None, None).unwrap());

    assert_hash_mode_mismatch(VerifiedRangeProof::verify(
        proof,
        root,
        None,
        None,
        other_mode(H::ALGORITHM),
        None,
    ));
}

#[hash_mode]
#[test]
fn change_verify_pairs_proof_and_context<H: HashMode>() {
    let source = init_merkle_in::<H, _, _, _>(kvs(8));
    let target = init_merkle_in::<H, _, _, _>(kvs(12));
    let end_root = HashedNodeReader::root_hash(target.nodestore()).unwrap();
    let limit = NonZeroUsize::new(3);
    let proof = Arc::new(
        target
            .change_proof(None, None, source.nodestore(), limit)
            .unwrap(),
    );

    let verified = VerifiedChangeProof::verify(
        Arc::clone(&proof),
        end_root.clone(),
        None,
        None,
        H::ALGORITHM,
        limit,
    )
    .unwrap();

    assert!(Arc::ptr_eq(verified.proof(), &proof));
    let verification = verified.verification();
    assert_eq!(verification.end_root(), &end_root);
    assert_eq!(verification.start_key(), None);
    assert_eq!(verification.end_key(), None);
    assert_eq!(verification.max_length(), limit);
    // Truncated by `limit` with no requested upper bound, so the proof is
    // anchored at its last operation's key.
    let last = proof.batch_ops().last().map(|op| op.key().as_ref());
    assert_eq!(verification.right_edge_key(), last);
}

#[hash_mode]
#[test]
fn change_verify_rejects_wrong_root<H: HashMode>() {
    let source = init_merkle_in::<H, _, _, _>(kvs(8));
    let target = init_merkle_in::<H, _, _, _>(kvs(12));
    let other = init_merkle_in::<H, _, _, _>(kvs(13));
    let wrong_root = HashedNodeReader::root_hash(other.nodestore()).unwrap();
    let proof = target
        .change_proof(None, None, source.nodestore(), None)
        .unwrap();

    // The boundary proofs anchor to the real end root, so a foreign root
    // fails the structural pass before any operation is applied.
    match VerifiedChangeProof::verify(proof, wrong_root, None, None, H::ALGORITHM, None) {
        Err(api::Error::ProofError(ProofError::EdgeProofHashMismatch { .. })) => {}
        other => panic!("expected EdgeProofHashMismatch, got {other:?}"),
    }
}

#[hash_mode]
#[test]
fn change_verify_rejects_other_hash_mode<H: HashMode>() {
    let source = init_merkle_in::<H, _, _, _>(kvs(8));
    let target = init_merkle_in::<H, _, _, _>(kvs(12));
    let end_root = HashedNodeReader::root_hash(target.nodestore()).unwrap();
    let proof = Arc::new(
        target
            .change_proof(None, None, source.nodestore(), None)
            .unwrap(),
    );

    assert_hash_mode_mismatch(VerifiedChangeProof::verify(
        proof,
        end_root,
        None,
        None,
        other_mode(H::ALGORITHM),
        None,
    ));
}

fn new_db<H: HashMode>() -> (Db<H>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::<H>::new_with_hash_mode(
        dir.path(),
        DbConfig::builder()
            .node_hash_algorithm(H::ALGORITHM)
            .build(),
    )
    .unwrap();
    (db, dir)
}

fn commit_puts<H: HashMode>(db: &Db<H>, pairs: &[(Vec<u8>, Vec<u8>)]) -> api::HashKey {
    let batch: Vec<BatchOp<&[u8], &[u8]>> = pairs
        .iter()
        .map(|(k, v)| BatchOp::Put {
            key: k.as_slice(),
            value: v.as_slice(),
        })
        .collect();
    db.propose(batch).unwrap().commit().unwrap();
    db.root_hash().unwrap()
}

#[hash_mode]
#[test]
fn db_applies_verified_change_proof<H: HashMode>() {
    let (source, _source_dir) = new_db::<H>();
    let (target, _target_dir) = new_db::<H>();
    let base = kvs(8);
    let root1 = commit_puts(&source, &base);
    assert_eq!(commit_puts(&target, &base), root1);
    let root2 = commit_puts(&source, &kvs(12)[8..]);

    let proof = source
        .change_proof(root1, root2.clone(), None, None, None)
        .unwrap();
    let verified = VerifiedChangeProof::verify(
        Arc::new(proof),
        root2.clone(),
        None,
        None,
        H::ALGORITHM,
        None,
    )
    .unwrap();

    let proposal = target.apply_verified_change_proof(&verified).unwrap();
    assert_eq!(proposal.root_hash(), Some(root2.clone()));
    proposal.commit().unwrap();
    assert_eq!(target.root_hash().unwrap(), root2);

    // Applying the same verified proof again, now on top of the revision it
    // produced, is the path a commit retry takes after the database advanced.
    // The operations are idempotent puts, so the result is the same root.
    let again = target.apply_verified_change_proof(&verified).unwrap();
    assert_eq!(again.root_hash(), Some(root2));
}

#[test]
fn db_rejects_verified_change_proof_from_other_mode() {
    let (source, _source_dir) = new_db::<MerkleDbHash>();
    let (target, _target_dir) = new_db::<EthHash>();
    let root1 = commit_puts(&source, &kvs(4));
    let root2 = commit_puts(&source, &kvs(6)[4..]);

    let proof = source
        .change_proof(root1, root2.clone(), None, None, None)
        .unwrap();
    let verified = VerifiedChangeProof::verify(
        Arc::new(proof),
        root2,
        None,
        None,
        NodeHashAlgorithm::MerkleDB,
        None,
    )
    .unwrap();

    assert_hash_mode_mismatch(target.apply_verified_change_proof(&verified));
}
