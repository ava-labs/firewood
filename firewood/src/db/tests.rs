// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod concurrent_rebase;
mod merge;

use super::test::TestDb;

mod key_length {
    use super::TestDb;
    use crate::api::{BatchOp, Db as _, Error, MAX_KEY_BYTES, Proposal as _};
    use crate::db::{DbConfig, UseParallel};
    use test_case::test_case;

    type Op = BatchOp<Vec<u8>, Vec<u8>>;

    fn put(len: usize) -> Op {
        BatchOp::Put {
            key: vec![0xAB; len],
            value: b"v".to_vec(),
        }
    }

    fn delete(len: usize) -> Op {
        BatchOp::Delete {
            key: vec![0xAB; len],
        }
    }

    fn delete_range(len: usize) -> Op {
        BatchOp::DeleteRange {
            prefix: vec![0xAB; len],
        }
    }

    /// Both batch paths, serial and parallel, accept a key at the limit and
    /// reject one a byte over, for every kind of operation.
    #[test_case(put, MAX_KEY_BYTES, true ; "put at the limit")]
    #[test_case(put, MAX_KEY_BYTES + 1, false ; "put one byte over")]
    #[test_case(delete, MAX_KEY_BYTES, true ; "delete at the limit")]
    #[test_case(delete, MAX_KEY_BYTES + 1, false ; "delete one byte over")]
    #[test_case(delete_range, MAX_KEY_BYTES, true ; "delete range at the limit")]
    #[test_case(delete_range, MAX_KEY_BYTES + 1, false ; "delete range one byte over")]
    fn propose_enforces_the_key_length_limit(op: fn(usize) -> Op, len: usize, accepted: bool) {
        for use_parallel in [UseParallel::Never, UseParallel::Always] {
            let path = format!("{use_parallel:?}");
            let db =
                TestDb::new_with_config(DbConfig::builder().use_parallel(use_parallel).build());
            match db.propose(vec![op(len)]).map(drop) {
                Ok(()) => assert!(accepted, "a {len}-byte key was accepted ({path})"),
                Err(Error::KeyTooLong { len: got, max }) => {
                    assert!(!accepted, "a {len}-byte key was rejected ({path})");
                    assert_eq!(got, len);
                    assert_eq!(max, MAX_KEY_BYTES);
                }
                Err(other) => {
                    panic!("unexpected error for a {len}-byte key ({path}): {other}")
                }
            }
        }
    }

    /// A proposal built on another proposal goes through the same check.
    #[test]
    fn proposing_on_a_proposal_enforces_the_key_length_limit() {
        let db = TestDb::new();
        let base = db.propose(vec![put(1)]).unwrap();
        let err = base
            .propose(vec![put(MAX_KEY_BYTES + 1)])
            .map(drop)
            .unwrap_err();
        assert!(
            matches!(err, Error::KeyTooLong { len, max } if len == MAX_KEY_BYTES + 1 && max == MAX_KEY_BYTES),
            "unexpected error: {err}"
        );
    }
}
