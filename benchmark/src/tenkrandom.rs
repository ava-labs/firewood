// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use anyhow::Context;
use firewood::db::Db;
use firewood::v2::api::{Db as _, Proposal as _};

use crate::{GlobalOpts, KeygenIterExt, RangeExt, TestRunner};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, clap::Args)]
pub struct TenKRandom;

impl TestRunner for TenKRandom {
    async fn run(&self, db: &Db, args: &GlobalOpts) -> anyhow::Result<()> {
        let key_range = args.key_range().context("key range overflows u64")?;

        // segments of 25% of a batch
        let mut segments = key_range
            .split(
                args.batch_size
                    .checked_mul(crate::FOUR)
                    .context("batch size overflows u64")?,
            )
            .cycle();

        let batch = segments.clone().take(3).flatten().iter_insert_ops();
        db.propose(batch)
            .await
            .context("failed to build proposal")?
            .commit()
            .await
            .context("failed to commit proposal")?;

        crate::repeat_for(args.duration(), async |batch_id| {
            let (_, value) = crate::keygen(batch_id);

            let deletes = segments.next().into_iter().flatten().iter_delete_ops();

            // clone the base iterator before we take any more segments so the
            // next `repeat_for` iteration starts from this point. This is cheap
            // since it's 48 bytes (size_of::<u64>() * 3 (SplitRange) * 2 (Cycle))
            let mut segments = segments.clone().take(3);

            let update1 = segments.next().into_iter().flatten().iter_update_ops(value);
            let update2 = segments.next().into_iter().flatten().iter_update_ops(value);
            let inserts = segments.next().into_iter().flatten().iter_insert_ops();

            let batch = inserts.chain(deletes).chain(update1).chain(update2);

            db.propose(batch)
                .await
                .context("failed to build proposal")?
                .commit()
                .await
                .context("failed to commit proposal")?;

            anyhow::Ok(())
        })
        .await
    }
}
