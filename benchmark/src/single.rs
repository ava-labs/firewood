// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{GlobalOpts, KeygenIterExt, RangeExt, TestRunner};
use anyhow::Context;
use firewood::db::Db;
use firewood::v2::api::{Db as _, Proposal as _};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, clap::Args)]
pub struct Single;

impl TestRunner for Single {
    async fn run(&self, db: &Db, args: &GlobalOpts) -> anyhow::Result<()> {
        let keys = args
            .key_range()
            .context("key range overflows u64")?
            .hashed_key_range()
            .collect::<Vec<_>>();

        crate::repeat_for(args.duration(), async |batch_id| {
            let (_, digest) = crate::keygen(batch_id);
            let batch = keys.iter().iter_update_ops(digest);

            db.propose(batch)
                .await
                .context("failed to build proposal")?
                .commit()
                .await
                .context("failed to commit proposal")?;

            Ok(())
        })
        .await
    }
}
