// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod concurrent_rebase;
mod merge;

use super::test::TestDb;

mod empty_values {
    use firewood_macros::hash_mode;
    use firewood_storage::EthHash;
    use firewood_storage::HashMode;
    use firewood_storage::MerkleDbHash;

    use super::TestDb;
    use crate::api::BatchOp;
    use crate::api::Db as _;
    use crate::api::DbView as _;
    use crate::api::Error;
    use crate::api::Proposal as _;
    use crate::db::DbConfig;
    use crate::db::UseParallel;

    /// Under the Ethereum hash scheme a put with an empty value is refused on both
    /// batch paths, since the scheme hashes an empty value the same as no value.
    /// Under merkledb it is stored.
    #[hash_mode]
    #[test]
    fn empty_value_put_follows_the_hash_scheme<H: HashMode>() {
        for use_parallel in [UseParallel::Never, UseParallel::Always] {
            let label = format!("{use_parallel:?}");
            let db = TestDb::<H>::new_with_config(
                DbConfig::builder().use_parallel(use_parallel).build(),
            );
            let batch = vec![BatchOp::Put {
                key: b"k".to_vec(),
                value: Vec::<u8>::new(),
            }];
            let result = db.propose(batch);
            if H::ALGORITHM.is_ethereum() {
                assert!(
                    matches!(result, Err(Error::EmptyValue)),
                    "({label}) expected EmptyValue, got {:?}",
                    result.map(drop)
                );
            } else {
                let proposal = result.unwrap_or_else(|err| panic!("({label}) {err}"));
                let stored = proposal
                    .val(b"k")
                    .unwrap_or_else(|err| panic!("({label}) {err}"));
                assert_eq!(
                    stored.as_deref(),
                    Some(b"".as_slice()),
                    "({label}) the empty value was not stored"
                );
                proposal
                    .commit()
                    .unwrap_or_else(|err| panic!("({label}) {err}"));
            }
        }
    }
}

mod key_length {
    use firewood_storage::DefaultHashMode;
    use test_case::test_matrix;

    use super::TestDb;
    use crate::api::BatchOp;
    use crate::api::Db as _;
    use crate::api::Error;
    use crate::api::MAX_KEY_BYTES;
    use crate::api::Proposal as _;
    use crate::db::DbConfig;
    use crate::db::UseParallel;

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
    #[test_matrix(
        [put, delete, delete_range],
        [MAX_KEY_BYTES, MAX_KEY_BYTES + 1],
        [UseParallel::Never, UseParallel::Always]
    )]
    fn propose_enforces_the_key_length_limit(
        op: fn(usize) -> Op,
        len: usize,
        use_parallel: UseParallel,
    ) {
        let db = TestDb::<DefaultHashMode>::new_with_config(
            DbConfig::builder().use_parallel(use_parallel).build(),
        );
        let result = db.propose(vec![op(len)]).map(drop);
        if len <= MAX_KEY_BYTES {
            assert!(result.is_ok(), "{result:?}");
        } else {
            assert!(
                matches!(result, Err(Error::KeyTooLong { len: got, max }) if got == len && max == MAX_KEY_BYTES),
                "{result:?}"
            );
        }
    }

    /// A proposal built on another proposal goes through the same check.
    #[test]
    fn proposing_on_a_proposal_enforces_the_key_length_limit() {
        let db = TestDb::<DefaultHashMode>::new();
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
