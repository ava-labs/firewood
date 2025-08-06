// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

// #![expect(
//     clippy::cast_precision_loss,
//     reason = "Found 1 occurrences after enabling the lint."
// )]

use crate::{GlobalOpts, KeygenIterExt, RangeExt, TestRunner};
use anyhow::Context;
use firewood::db::{BatchOp, Db};
use firewood::v2::api::{Db as _, Proposal as _};
use log::debug;
use rand::prelude::*;
use std::collections::HashSet;
use std::num::NonZeroU64;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, clap::Args)]
pub struct Zipf {
    #[arg(short, long, help = "zipf exponent", default_value_t = 1.2)]
    exponent: f64,
}

#[inline]
const fn f_to_u(f: f64) -> u64 {
    #![expect(clippy::cast_sign_loss)]

    f as u64
}

#[inline]
const fn u_to_f(u: u64) -> f64 {
    #![expect(clippy::cast_precision_loss)]

    u as f64
}

impl TestRunner for Zipf {
    async fn run(&self, db: &Db, args: &GlobalOpts) -> anyhow::Result<()> {
        let exponent = self.exponent;

        let range = &args.key_range().context("key range overflows u64")?;
        let rows = range
            .end
            .checked_sub(range.start)
            .and_then(NonZeroU64::new)
            .context("key range is negative or zero")?
            .get();

        let dist = rand_distr::Zipf::<f64>::new(u_to_f(rows), exponent)?.map(f_to_u);
        let mut rng = rand::rng();

        crate::repeat_for(args.duration(), async |batch_id| {
            let (_, value) = crate::keygen(batch_id);
            let batch = range
                .random_batch(args.batch_size, &dist, &mut rng)
                .iter_update_ops(value)
                .collect::<Vec<_>>();

            if log::log_enabled!(log::Level::Debug) {
                let distinct = batch
                    .iter()
                    .map(BatchOp::key)
                    .copied()
                    .collect::<HashSet<_>>()
                    .len();

                debug!("inserting batch {batch_id} with {distinct} distinct data values");
            }

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
