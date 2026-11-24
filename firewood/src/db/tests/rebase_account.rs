// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! `commit_with_rebase` under the Ethereum hash scheme, where an account's
//! stored value carries a storage root the hasher derives from its slots.

use firewood_storage::EthHash;

use super::TestDb;
use crate::api::BatchOp;
use crate::api::Db as _;
use crate::api::DbView as _;
use crate::api::Proposal as _;

type Op = BatchOp<Vec<u8>, Vec<u8>>;

fn account(nonce: u64) -> Vec<u8> {
    let mut rlp = rlp::RlpStream::new_list(4);
    rlp.append(&nonce);
    rlp.append(&1_000u64);
    rlp.append(&[0u8; 32].as_slice());
    rlp.append(&[0x11u8; 32].as_slice());
    rlp.out().to_vec()
}

fn nonce_of(value: &[u8]) -> u64 {
    rlp::Rlp::new(value).val_at(0).expect("account RLP")
}

fn account_key() -> Vec<u8> {
    vec![0x42u8; 32]
}

fn slot_key() -> Vec<u8> {
    let mut key = account_key();
    key.extend_from_slice(&[0x07u8; 32]);
    key
}

fn put_account(nonce: u64) -> Op {
    BatchOp::Put {
        key: account_key(),
        value: account(nonce),
    }
}

fn put_slot() -> Op {
    BatchOp::Put {
        key: slot_key(),
        value: vec![0x83, 0xaa, 0xbb, 0xcc],
    }
}

/// Seeds the account at nonce 1, proposes `stale` on it, commits `winner`, and
/// rebases `stale`.
///
/// Asserts that the rebased root is the one `stale` gets when it is proposed on
/// the winner directly.
fn rebase_over(stale: Vec<Op>, winner: Vec<Op>) -> TestDb<EthHash> {
    let db = TestDb::<EthHash>::new();
    db.propose(vec![put_account(1)]).unwrap().commit().unwrap();

    let proposal = db.propose(stale.clone()).unwrap();
    db.propose(winner).unwrap().commit().unwrap();
    let replayed = db.propose(stale).unwrap().root_hash();

    let rebased = proposal.commit_with_rebase().unwrap();
    assert_eq!(rebased, replayed);
    db
}

fn head_value(db: &TestDb<EthHash>, key: &[u8]) -> Option<Vec<u8>> {
    let head = db.revision(db.root_hash().unwrap()).unwrap();
    head.val(key).unwrap().map(|value| value.to_vec())
}

fn head_nonce(db: &TestDb<EthHash>) -> Option<u64> {
    head_value(db, &account_key()).map(|value| nonce_of(&value))
}

#[test]
fn test_rebase_keeps_account_fields_changed_by_the_winner() {
    let db = rebase_over(vec![put_slot()], vec![put_account(2)]);
    assert_eq!(head_nonce(&db), Some(2));
    assert!(head_value(&db, &slot_key()).is_some());
}

#[test]
fn test_rebase_applies_account_fields_the_proposal_changed() {
    let db = rebase_over(vec![put_account(3), put_slot()], vec![put_account(2)]);
    assert_eq!(head_nonce(&db), Some(3));
    assert!(head_value(&db, &slot_key()).is_some());
}

#[test]
fn test_rebase_over_a_commit_to_another_account() {
    let other = BatchOp::Put {
        key: vec![0x99u8; 32],
        value: account(7),
    };
    let db = rebase_over(vec![put_slot()], vec![other]);
    assert_eq!(head_nonce(&db), Some(1));
    assert!(head_value(&db, &slot_key()).is_some());
}

#[test]
fn test_rebase_leaves_an_account_the_winner_deleted_deleted() {
    let delete = BatchOp::Delete { key: account_key() };
    let db = rebase_over(vec![put_slot()], vec![delete]);
    assert_eq!(head_nonce(&db), None);
    assert!(head_value(&db, &slot_key()).is_some());
}
