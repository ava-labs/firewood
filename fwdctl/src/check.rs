// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use clap::Args;
use log::warn;
use std::collections::BTreeMap;
use std::io::{Error, ErrorKind};
use std::ops::Bound;
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
        default_value_t = String::from("firewood.db"),
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

    let mut expected = 2048;
    for (addr, size) in allocated.iter() {
        match addr.get().cmp(&expected) {
            std::cmp::Ordering::Less => {
                warn!(
                    "Node at {:?} is before the expected address {}",
                    addr, expected
                );
            }
            std::cmp::Ordering::Greater => {
                warn!("{} bytes missing at {}", addr.get() - expected, expected);
            }
            std::cmp::Ordering::Equal => {}
        }
        expected = addr.get() + rev.size_from_area_index(*size);
    }

    Ok(())
}

fn visitor<T: ReadableStorage>(
    rev: Arc<NodeStore<Committed, T>>,
    addr: LinearAddress,
    allocated: &mut BTreeMap<LinearAddress, u8>,
) -> Result<(), Error> {
    // find the node before this one, check if it overlaps
    if let Some((found_addr, found_size)) = allocated
        .range((Bound::Unbounded, Bound::Included(addr)))
        .next_back()
    {
        match found_addr
            .get()
            .checked_add(rev.size_from_area_index(*found_size))
        {
            None => warn!("Node at {:?} overflows a u64", found_addr),
            Some(end) => {
                if end > addr.get() {
                    warn!(
                        "Node at {:?} overlaps with another node at {:?} (size: {})",
                        addr, found_addr, found_size
                    );
                    return Err(Error::new(ErrorKind::Other, "Overlapping nodes"));
                }
            }
        }
    }
    if addr.get() > rev.header().size() {
        warn!(
            "Node at {:?} starts past the database high water mark",
            addr
        );
        return Err(Error::new(ErrorKind::Other, "Node overflows database"));
    }

    let (node, size) = rev.uncached_read_node_and_size(addr)?;
    if addr.get() + rev.size_from_area_index(size) > rev.header().size() {
        warn!(
            "Node at {:?} extends past the database high water mark",
            addr
        );
        return Err(Error::new(ErrorKind::Other, "Node overflows database"));
    }

    allocated.insert(addr, size);

    if let Node::Branch(branch) = node.as_ref() {
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
    Ok(())
}
