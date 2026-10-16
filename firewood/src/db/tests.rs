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
    use crate::api::Error;
    use crate::api::Proposal as _;
    use crate::db::DbConfig;
    use crate::db::UseParallel;

    /// Under the Ethereum scheme a put with an empty value is refused on both
    /// batch paths, since the scheme hashes an empty value the same as no value.
    /// Under merkledb it is stored.
    #[hash_mode]
    #[test]
    fn empty_value_put_follows_the_hash_scheme<H: HashMode>() {
        for use_parallel in [UseParallel::Never, UseParallel::Always] {
            let path = format!("{use_parallel:?}");
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
                    "({path}) expected EmptyValue, got {:?}",
                    result.map(drop)
                );
            } else {
                result
                    .unwrap_or_else(|err| panic!("({path}) {err}"))
                    .commit()
                    .unwrap_or_else(|err| panic!("({path}) {err}"));
            }
        }
    }
}
