// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;
use log::warn;
use std::collections::BTreeMap;
use std::io::Error;
use std::str;
use std::sync::Arc;

use firewood::db::{Db, DbConfig};
use firewood::v2::api::{self, Db as _};
use storage::{Committed, HashedNodeReader as _, LinearAddress, Node, NodeStore, ReadableStorage};

#[derive(Debug, Args)]
pub struct Options {
    /// The database path. Defaults to firewood.
    #[arg(
        long,
        required = false,
        value_name = "DB_NAME",
        default_value_t = String::from("firewood"),
        help = "Name of the database"
    )]
    pub db: String,
}

pub(super) async fn run(opts: &Options) -> Result<(), api::Error> {
    let cfg = DbConfig::builder().truncate(false);

    let db = Db::new(opts.db.clone(), cfg.build()).await?;

    let hash = db.root_hash().await?;

    let Some(hash) = hash else {
        println!("Database is empty");
        return Ok(());
    };

    let rev = db.revision(hash).await?;

    // walk the nodes

    let addr = rev.root_address_and_hash()?.expect("was not empty").0;
    let mut allocated = BTreeMap::new();

    visitor(rev.clone(), addr, &mut allocated)?;

    for (addr, size) in allocated.iter() {
        println!("{:?} {}", addr, size);
    }

    Ok(())
}

fn visitor<T: ReadableStorage>(
    rev: Arc<NodeStore<Committed, T>>,
    addr: LinearAddress,
    allocated: &mut BTreeMap<LinearAddress, u8>,
) -> Result<(), Error> {
    let (node, size) = rev.uncached_read_node_and_size(addr)?;
    if let Some(duplicate) = allocated.insert(addr, size) {
        warn!("Duplicate allocation at {:?} (size: {})", addr, duplicate);
    }

    println!("{:?} {:?}", addr, node);

    match node.as_ref() {
        Node::Branch(branch) => {
            for child in branch.children.iter() {
                match child {
                    None => {}
                    Some(child) => match child {
                        storage::Child::Node(_) => unreachable!(),
                        storage::Child::AddressWithHash(addr, _hash) => {
                            visitor(rev.clone(), *addr, allocated)?;
                        }
                    },
                }
            }
        }
        Node::Leaf(_leaf) => {}
    }
    Ok(())
}
