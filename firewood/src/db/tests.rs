// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

mod concurrent_rebase;
mod merge;

use super::test::TestDb;

mod empty_values {
    use super::TestDb;
    use crate::api::{BatchOp, Db as _, Error, Proposal as _};
    use crate::db::{DbConfig, UseParallel};
    use firewood_storage::{DefaultHashMode, HashMode};

    /// Under the Ethereum scheme a put with an empty value is refused on both
    /// batch paths, since the scheme hashes an empty value the same as no value.
    /// Under merkledb it is stored.
    #[test]
    fn empty_value_put_follows_the_hash_scheme() {
        for use_parallel in [UseParallel::Never, UseParallel::Always] {
            let path = format!("{use_parallel:?}");
            let db =
                TestDb::new_with_config(DbConfig::builder().use_parallel(use_parallel).build());
            let batch = vec![BatchOp::Put {
                key: b"k".to_vec(),
                value: Vec::<u8>::new(),
            }];
            let result = db.propose(batch);
            if DefaultHashMode::ALGORITHM.is_ethereum() {
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
