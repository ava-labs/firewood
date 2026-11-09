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
    #[hash_mode]
    #[test_matrix(
        [put, delete, delete_range],
        [MAX_KEY_BYTES, MAX_KEY_BYTES + 1],
        [UseParallel::Never, UseParallel::Always]
    )]
    fn propose_enforces_the_key_length_limit<H: HashMode>(
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

mod account_depth_inline_storage {
    use firewood_storage::EthHash;
    use firewood_storage::HashMode;
    use test_case::test_matrix;

    use super::TestDb;
    use crate::api::BatchOp;
    use crate::api::Db as _;
    use crate::api::FrozenChangeProof;
    use crate::api::Proposal as _;
    use crate::db::DbConfig;
    use crate::db::UseParallel;
    use crate::merkle::Key;
    use crate::merkle::Value;

    /// A change proof is applied and hashed before its root is compared. Each
    /// case adds puts for a 32-byte key and 33-byte keys extending it, one per
    /// suffix byte, which give that account inline storage children. One child
    /// reaches the single-child storage root and two reach the storage branch.
    /// Each case runs on the serial and the parallel batch path. The forged
    /// proof must fail verification rather than abort hashing.
    #[test_matrix(
        [&[0xAB], &[0x0B, 0xAB]],
        [UseParallel::Never, UseParallel::Always]
    )]
    fn change_proof_with_short_storage_keys_is_rejected(
        suffixes: &[u8],
        use_parallel: UseParallel,
    ) {
        let k_lo = vec![0x10; 32];
        let k_hi = vec![0xf0; 32];
        let source = TestDb::<EthHash>::new();
        let target = TestDb::<EthHash>::new_with_config(
            DbConfig::builder().use_parallel(use_parallel).build(),
        );
        for db in [&source, &target] {
            db.propose(vec![
                BatchOp::Put {
                    key: k_lo.clone(),
                    value: b"lo".to_vec(),
                },
                BatchOp::Put {
                    key: k_hi.clone(),
                    value: b"hi".to_vec(),
                },
            ])
            .unwrap()
            .commit()
            .unwrap();
        }
        let start_root = source.root_hash().unwrap();
        source
            .propose(vec![BatchOp::Put {
                key: k_hi.clone(),
                value: b"hi2".to_vec(),
            }])
            .unwrap()
            .commit()
            .unwrap();
        let end_root = source.root_hash().unwrap();
        let honest = source
            .change_proof(start_root, end_root.clone(), None, None, None)
            .unwrap();

        let account = vec![0x50; 32];
        let mut forged_ops: Vec<BatchOp<Key, Value>> = vec![BatchOp::Put {
            key: account.clone().into(),
            value: vec![0x01].into(),
        }];
        for &suffix in suffixes {
            forged_ops.push(BatchOp::Put {
                key: [account.as_slice(), &[suffix]].concat().into(),
                value: vec![0x02].into(),
            });
        }
        let mut ops: Vec<BatchOp<Key, Value>> = honest.batch_ops().to_vec();
        let last_op = ops.pop().expect("the honest proof changes k_hi");
        ops.extend(forged_ops);
        ops.push(last_op);
        let forged = FrozenChangeProof::new(
            crate::Proof::new(honest.start_proof().as_ref().into()),
            crate::Proof::new(honest.end_proof().as_ref().into()),
            ops.into_boxed_slice(),
            EthHash::ALGORITHM,
        );

        assert!(
            target
                .verify_change_proof(&honest, end_root.clone(), None, None, None)
                .is_ok()
        );
        let result = target
            .verify_change_proof(&forged, end_root, None, None, None)
            .map(drop);
        assert!(result.is_err(), "the forged change proof must be rejected");
    }
}
