// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::{error::Error, ops::Range, sync::Arc, time::Instant};

use firewood::{
    db::{Batch, BatchOp, Db, DbConfig},
    v2::api::{Db as DbApi, Proposal},
};
use rand::{distributions::Alphanumeric, Rng};

/// cargo run --release --example insert
#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    const KEYLEN: Range<usize> = 1..64;
    const DATALEN: usize = 32;
    const TOTAL_INSERTS: u32 = 100;
    const KEYS_PER_INSERT: usize = 2;

    let cfg = DbConfig::builder().truncate(true).build();

    let db = tokio::task::spawn_blocking(move || {
        Db::new("rev_db", &cfg).expect("db initiation should succeed")
    })
    .await
    .unwrap();
    let start = Instant::now();
    for _ in 0..TOTAL_INSERTS {
        let keylen = rand::thread_rng().gen_range(KEYLEN);
        let batch: Batch<Vec<u8>, Vec<u8>> = (0..KEYS_PER_INSERT)
            .map(|_| {
                (
                    rand::thread_rng()
                        .sample_iter(&Alphanumeric)
                        .take(keylen)
                        .collect::<Vec<u8>>(),
                    rand::thread_rng()
                        .sample_iter(&Alphanumeric)
                        .take(DATALEN)
                        .collect::<Vec<u8>>(),
                )
            })
            .map(|(key, value)| BatchOp::Put { key, value })
            .collect();
        let proposal: Arc<firewood::db::Proposal> = db.propose(batch).await.unwrap().into();
        proposal.commit().await?;
    }
    let duration = start.elapsed();
    println!("Inserted {TOTAL_INSERTS} in {duration:?}");
    Ok(())
}

// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use std::{error::Error, ops::Range, sync::Arc, time::Instant};

use firewood::{
    db::{Batch, BatchOp, Db, DbConfig},
    v2::api::{Db as DbApi, Proposal},
};
use rand::{distributions::Alphanumeric, Rng};

/// cargo run --release --example insert
#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    const KEYLEN: Range<usize> = 1..64;
    const DATALEN: usize = 32;
    const TOTAL_INSERTS: u32 = 100;
    const KEYS_PER_INSERT: usize = 2;

    let cfg = DbConfig::builder().truncate(true).build();

    let db = tokio::task::spawn_blocking(move || {
        Db::new("rev_db", &cfg).expect("db initiation should succeed")
    })
    .await
    .unwrap();
    let start = Instant::now();
    for _ in 0..TOTAL_INSERTS {
        let keylen = rand::thread_rng().gen_range(KEYLEN);
        let batch: Batch<Vec<u8>, Vec<u8>> = (0..KEYS_PER_INSERT)
            .map(|_| {
                (
                    rand::thread_rng()
                        .sample_iter(&Alphanumeric)
                        .take(keylen)
                        .collect::<Vec<u8>>(),
                    rand::thread_rng()
                        .sample_iter(&Alphanumeric)
                        .take(DATALEN)
                        .collect::<Vec<u8>>(),
                )
            })
            .map(|(key, value)| BatchOp::Put { key, value })
            .collect();
        let proposal: Arc<firewood::db::Proposal> = db.propose(batch).await.unwrap().into();
        proposal.commit().await?;
    }
    let duration = start.elapsed();
    println!("Inserted {TOTAL_INSERTS} in {duration:?}");
    Ok(())
}
