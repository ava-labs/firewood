mod error;

use crate::{Committed, FileBacked, NodeStore};

pub use error::CheckerError;

/// Go through the filebacked storage and check for any inconsistencies. It proceeds in the following steps:
/// 1. check the header to see if we know how to check it
/// 2. traverse the trie and check the nodes
/// 3. check the free list
/// 4. check any bubbles - what are the spaces between trie nodes and free lists?
// TODO: add merkle hash checks as well
pub async fn check_node_store(
    node_store: &NodeStore<Committed, FileBacked>,
) -> Result<(), CheckerError> {
    // 1. check the header to see if we know how to check it

    // 2. traverse the trie and check the nodes

    // 3. check the free list

    // 4. check any bubbles - what are the spaces between trie nodes and free lists?

    Ok(())
}
