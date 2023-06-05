// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Parser;
use criterion::Criterion;
use firewood::db::{Db, DbConfig, WalConfig};
use rand::{rngs::StdRng, Rng, SeedableRng};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(long)]
    nbatch: usize,
    #[arg(short, long)]
    batch_size: usize,
    #[arg(short, long, default_value_t = 0)]
    seed: u64,
    #[arg(short, long, default_value_t = false)]
    no_root_hash: bool,
}

fn main() {
    let args = Args::parse();

    let cfg = DbConfig::builder().wal(WalConfig::builder().max_revisions(10).build());
    let mut c = Criterion::default();
    let mut group = c.benchmark_group("insert");
    let mut rng = StdRng::seed_from_u64(args.seed);

    let workload: Vec<Vec<([u8; 32], [u8; 32])>> = (0..args.nbatch)
        .map(|_| {
            (0..args.batch_size)
                .map(|_| (rng.gen(), rng.gen()))
                .collect()
        })
        .collect();

    println!("workload prepared");

    group
        .sampling_mode(criterion::SamplingMode::Flat)
        .sample_size(10);

    let total = (args.nbatch * args.batch_size) as u64;
    group.throughput(criterion::Throughput::Elements(total));

    group.bench_with_input(
        format!("nbatch={} batch_size={}", args.nbatch, args.batch_size),
        &workload,
        |b, workload| {
            b.iter(|| {
                let db = Db::new("benchmark_db", &cfg.clone().truncate(true).build()).unwrap();

                for batch in workload.iter() {
                    let mut wb = db.new_writebatch();

                    for (k, v) in batch {
                        wb = wb.kv_insert(k, v.to_vec()).unwrap();
                    }

                    if args.no_root_hash {
                        wb = wb.no_root_hash();
                    }

                    wb.commit();
                }
            })
        },
    );
}
