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

/// A proposal that only writes a storage slot is rebased over a commit that
/// changed the slot's account. The rebased batch is rebuilt by diffing the
/// proposal against its old parent, and the account value differs there only
/// because the hasher rewrote its storage root. The rebuilt `Put` carries the
/// old parent's nonce and overwrites the newer commit.
///
/// Applying the proposal's own batch after that commit keeps the newer nonce,
/// which is what a rebase must be equivalent to.
#[test]
fn test_rebase_keeps_account_fields_changed_by_the_winner() {
    let db = TestDb::<EthHash>::new();
    let key = vec![0x42u8; 32];
    let mut slot = key.clone();
    slot.extend_from_slice(&[0x07u8; 32]);

    let seed: Vec<Op> = vec![BatchOp::Put {
        key: key.clone(),
        value: account(1),
    }];
    db.propose(seed).unwrap().commit().unwrap();

    let storage_only: Vec<Op> = vec![BatchOp::Put {
        key: slot.clone(),
        value: vec![0x83, 0xaa, 0xbb, 0xcc],
    }];
    let stale = db.propose(storage_only).unwrap();

    let bump: Vec<Op> = vec![BatchOp::Put {
        key: key.clone(),
        value: account(2),
    }];
    db.propose(bump).unwrap().commit().unwrap();

    stale.commit_with_rebase().unwrap();

    let head = db.revision(db.root_hash().unwrap()).unwrap();
    assert!(
        head.val(&slot).unwrap().is_some(),
        "the rebased slot is present"
    );
    let nonce = nonce_of(&head.val(&key).unwrap().expect("account present"));
    assert_eq!(
        nonce, 2,
        "the rebase restored the nonce of the proposal's old parent"
    );
}
