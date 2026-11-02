// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod concurrent_rebase;
mod merge;

use super::test::TestDb;

mod key_length {
    use firewood_macros::hash_mode;
    use firewood_storage::EthHash;
    use firewood_storage::HashMode;
    use firewood_storage::MerkleDbHash;
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
        // TODO(RodrigoVillar): extend #[hash_mode] to support test_matrix.
        check_proposal_key_length::<MerkleDbHash>(op, len, use_parallel.clone());
        check_proposal_key_length::<EthHash>(op, len, use_parallel);
    }

    fn check_proposal_key_length<H: HashMode>(
        op: fn(usize) -> Op,
        len: usize,
        use_parallel: UseParallel,
    ) {
        let db =
            TestDb::<H>::new_with_config(DbConfig::builder().use_parallel(use_parallel).build());
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
    #[hash_mode]
    #[test]
    fn proposing_on_a_proposal_enforces_the_key_length_limit<H: HashMode>() {
        let db = TestDb::<H>::new();
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
