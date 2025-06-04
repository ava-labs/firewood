mod error;
mod range_set;

use crate::{
    Child, Committed, FileBacked, HashedNodeReader, LinearAddress, Node, NodeReader, NodeStore,
};
use range_set::LinearAddressRangeSet;

pub use error::CheckerError;

/// Go through the filebacked storage and check for any inconsistencies. It proceeds in the following steps:
/// 1. traverse the trie and check the nodes
/// 2. check the free list
/// 3. check any bubbles - what are the spaces between trie nodes and free lists?
// TODO: add merkle hash checks as well
pub async fn check_node_store(
    node_store: &NodeStore<Committed, FileBacked>,
) -> Result<(), CheckerError> {
    let mut visited = LinearAddressRangeSet::new();

    // 1. traverse the trie and check the nodes
    let root_address = node_store.root_address_and_hash()?.map(|(addr, _)| addr);
    traverse_trie(node_store, root_address, &mut visited).await?;

    // 2. check the free list - this can happen in parallel with the trie traversal

    // 3. check any bubbles - what are the spaces between trie nodes and free lists?

    Ok(())
}

/// Recursively traverse the trie from the given root address.
async fn traverse_trie(
    node_store: &NodeStore<Committed, FileBacked>,
    subtree_root_address: Option<LinearAddress>,
    visited: &mut LinearAddressRangeSet,
) -> Result<(), CheckerError> {
    let Some(root_address) = subtree_root_address else {
        // empty subtree, do nothing
        return Ok(());
    };

    let (_, area_size) = node_store.area_index_and_size(root_address)?;
    visited.insert(root_address, area_size);

    let node = node_store.read_node(root_address)?;

    match &*node {
        Node::Branch(branch) => {
            for child in &branch.children {
                let child_address = child.as_ref().map(|c| {
                    let Child::AddressWithHash(address, _) = c else {
                        panic!("the children is not persisted yet, this should not happen");
                    };
                    address.clone()
                });
                Box::pin(traverse_trie(node_store, child_address, visited)).await?;
            }
        }
        Node::Leaf(_) => {
            // Don't need to traverse further since we are already at the leaf
        }
    }

    Ok(())
}
