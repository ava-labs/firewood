use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;

use clap::Args;
use firewood::v2::api;
use storage::{CacheReadStrategy, FileBacked, NodeStore, check_node_store};

// TODO: (optionally) add a fix option
#[derive(Args)]
pub struct Options {
    /// The database path (if no path is provided, return an error). Defaults to firewood.
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
    let db_path = Path::new(&opts.db);
    let node_cache_size = NonZeroUsize::new(1).unwrap();
    let free_list_cache_size = NonZeroUsize::new(1).unwrap();

    let storage = Arc::new(FileBacked::new(
        db_path.to_path_buf(),
        node_cache_size,
        free_list_cache_size,
        false,
        CacheReadStrategy::WritesOnly, // cache none since this is a read-only workload - we don't want to cache any nodes since we won't read a node more than once
    )?);

    let node_store = NodeStore::open(storage)?;
    check_node_store(&node_store).await?;
    Ok(())
}
