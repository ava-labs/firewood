// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::TestRunner;
use firewood::db::{BatchOp, Db};
use firewood::v2::api::{Db as _, Proposal as _};
use log::{debug, trace};
use pretty_duration::pretty_duration;
use rand::prelude::Distribution as _;
use rand::thread_rng;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::error::Error;
use std::time::Instant;

#[derive(clap::Args, Debug)]
pub struct Args {
    #[arg(short, long, help = "zipf exponent", default_value_t = 1.2)]
    exponent: f64,
}

#[derive(Clone)]
pub struct Zipf;

impl TestRunner for Zipf {
    async fn run(&self, db: &Db, args: &crate::Args) -> Result<(), Box<dyn Error>> {
        let exponent = if let crate::TestName::Zipf(args) = &args.test_name {
            args.exponent
        } else {
            unreachable!()
        };
        let rows = (args.number_of_batches * args.batch_size) as usize;
        let zipf = zipf::ZipfDistribution::new(rows, exponent).unwrap();
        let start = Instant::now();

        for batch_id in 0.. {
            let batch: Vec<BatchOp<_, _>> =
                generate_updates(batch_id, args.batch_size as usize, &zipf).collect();
            if log::log_enabled!(log::Level::Debug) {
                let mut distinct = HashSet::new();
                for op in &batch {
                    match op {
                        BatchOp::Put { key, value: _ } => {
                            distinct.insert(key);
                        }
                        _ => unreachable!(),
                    }
                }
                debug!(
                    "inserting batch {} with {} distinct data values",
                    batch_id,
                    distinct.len()
                );
            }
            let proposal = db.propose(batch).await.expect("proposal should succeed");
            proposal.commit().await?;

            if log::log_enabled!(log::Level::Debug) {
                debug!(
                    "completed batch {} in {}",
                    batch_id,
                    pretty_duration(&start.elapsed(), None)
                );
            }
        }
        unreachable!()
    }
}
fn generate_updates(
    batch_id: u32,
    batch_size: usize,
    zipf: &zipf::ZipfDistribution,
) -> impl Iterator<Item = BatchOp<Vec<u8>, Vec<u8>>> {
    let hash_of_batch_id = Sha256::digest(batch_id.to_ne_bytes()).to_vec();
    let rng = thread_rng();
    zipf.sample_iter(rng)
        .take(batch_size)
        .map(|inner_key| {
            let digest = Sha256::digest(inner_key.to_ne_bytes()).to_vec();
            trace!(
                "updating {:?} with digest {} to {}",
                inner_key,
                hex::encode(&digest),
                hex::encode(&hash_of_batch_id)
            );
            (digest, hash_of_batch_id.clone())
        })
        .map(|(key, value)| BatchOp::Put { key, value })
        .collect::<Vec<_>>()
        .into_iter()
}