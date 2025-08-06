// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::time::Instant;

use anyhow::Context;
use fastrace::prelude::SpanContext;
use fastrace::{Span, func_path};
use firewood::db::Db;
use firewood::v2::api::{Db as _, Proposal as _};
use log::info;

use pretty_duration::pretty_duration;

use crate::{GlobalOpts, KeygenIterExt, TestRunner};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, clap::Args)]
pub struct Create;

impl TestRunner for Create {
    async fn run(&self, db: &Db, args: &GlobalOpts) -> anyhow::Result<()> {
        let root = Span::root(func_path!(), SpanContext::random());

        let start = Instant::now();
        for (batch_id, range) in args
            .key_range_batches()
            .context("key range overflows u64")?
            .enumerate()
        {
            let _guard = root.set_local_parent();
            let start = Instant::now();
            let batch = range.clone().iter_insert_ops();

            db.propose(batch)
                .await
                .context("failed to build proposal")?
                .commit()
                .await
                .context("failed to commit proposal")?;

            if log::log_enabled!(log::Level::Debug) {
                let elapsed = start.elapsed();
                log::debug!(
                    "created batch ({batch_id}/{}) with {} keys in {}",
                    args.number_of_batches.get(),
                    range.size_hint().0,
                    pretty_duration(&elapsed, None)
                );
            }
        }

        info!(
            "Generated and inserted {} batches of size {} in {}",
            args.number_of_batches.get(),
            args.batch_size.get(),
            pretty_duration(&start.elapsed(), None)
        );

        Ok(())
    }
}
