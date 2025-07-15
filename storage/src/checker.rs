// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::logger::warn;
use crate::range_set::LinearAddressRangeSet;
use crate::{
    CheckerError, Committed, HashType, HashedNodeReader, LinearAddress, Node, NodeReader,
    NodeStore, Path, WritableStorage, hash_node,
};

/// [`NodeStore`] checker
// TODO: S needs to be writeable if we ask checker to fix the issues
#[expect(clippy::result_large_err)]
impl<S: WritableStorage> NodeStore<Committed, S> {
    /// Go through the filebacked storage and check for any inconsistencies. It proceeds in the following steps:
    /// 1. Check the header
    /// 2. traverse the trie and check the nodes
    /// 3. check the free list
    /// 4. check missed areas - what are the spaces between trie nodes and free lists we have traversed?
    /// # Errors
    /// Returns a [`CheckerError`] if the database is inconsistent.
    /// # Panics
    /// Panics if the header has too many free lists, which can never happen since freelists have a fixed size.
    // TODO: report all errors, not just the first one
    // TODO: add merkle hash checks as well
    pub fn check(&self, hash_check: bool) -> Result<(), CheckerError> {
        // 1. Check the header
        let db_size = self.size();
        let file_size = self.physical_size()?;
        if db_size < file_size {
            return Err(CheckerError::InvalidDBSize {
                db_size,
                description: format!(
                    "db size should not be smaller than the file size ({file_size})"
                ),
            });
        }

        let mut visited = LinearAddressRangeSet::new(db_size)?;

        // 2. traverse the trie and check the nodes
        if let (Some(root_address), Some(root_hash)) = (self.root_address(), self.root_hash()) {
            // the database is not empty, traverse the trie
            self.visit_trie(
                root_address,
                HashType::from(root_hash),
                Path::new(),
                &mut visited,
                hash_check,
            )?;
        }

        // 3. check the free list - this can happen in parallel with the trie traversal
        self.visit_freelist(&mut visited)?;

        // 4. check missed areas - what are the spaces between trie nodes and free lists we have traversed?
        let leaked_ranges = visited.complement();
        if !leaked_ranges.is_empty() {
            warn!("Found leaked ranges: {leaked_ranges}");
        }

        Ok(())
    }

    /// Recursively traverse the trie from the given root address.
    fn visit_trie(
        &self,
        subtree_root_address: LinearAddress,
        subtree_root_hash: HashType,
        path_prefix: Path,
        visited: &mut LinearAddressRangeSet,
        hash_check: bool,
    ) -> Result<(), CheckerError> {
        let (_, area_size) = self.area_index_and_size(subtree_root_address)?;
        let node = self.read_node(subtree_root_address)?;
        visited.insert_area(subtree_root_address, area_size)?;

        // iterate over the children
        if let Node::Branch(branch) = node.as_ref() {
            // this is an internal node, traverse the children
            for (nibble, (address, hash)) in branch.children_iter() {
                let mut child_path_prefix = path_prefix.clone();
                child_path_prefix.0.push(nibble as u8);
                self.visit_trie(
                    address,
                    hash.clone(),
                    child_path_prefix,
                    visited,
                    hash_check,
                )?;
            }
        }

        // hash check - at this point all children hashes have been verified
        if hash_check {
            let hash = hash_node(&node, &path_prefix);
            if hash != subtree_root_hash {
                return Err(CheckerError::HashMismatch {
                    partial_path: path_prefix,
                    address: subtree_root_address,
                    parent_stored_hash: subtree_root_hash,
                    computed_hash: hash,
                });
            }
        }

        Ok(())
    }

    /// Traverse all the free areas in the freelist
    fn visit_freelist(&self, visited: &mut LinearAddressRangeSet) -> Result<(), CheckerError> {
        for free_area in self.free_list_iter_inner(0) {
            let (addr, stored_area_index, free_list_id) = free_area?;
            let area_size = Self::size_from_area_index(stored_area_index);
            if free_list_id != stored_area_index {
                return Err(CheckerError::FreelistAreaSizeMismatch {
                    address: addr,
                    size: area_size,
                    actual_free_list: free_list_id,
                    expected_free_list: stored_area_index,
                });
            }
            visited.insert_area(addr, area_size)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod test {
    #![expect(clippy::unwrap_used)]
    #![expect(clippy::indexing_slicing)]

    use super::*;
    use crate::linear::memory::MemStore;
    use crate::nodestore::NodeStoreHeader;
    use crate::nodestore::alloc::test_utils::{
        test_write_free_area, test_write_header, test_write_new_node,
    };
    use crate::nodestore::alloc::{AREA_SIZES, FreeLists};
    use crate::{BranchNode, Child, LeafNode, NodeStore, Path, hash_node};

    // set up a basic trie:
    // -------------------------
    // |     |  X  |  X  | ... |    Root node
    // -------------------------
    //    |
    //    V
    // -------------------------
    // |  X  |     |  X  | ... |    Branch node
    // -------------------------
    //          |
    //          V
    // -------------------------
    // |   [0,1] -> [3,4,5]    |    Leaf node
    // -------------------------
    #[expect(clippy::arithmetic_side_effects)]
    fn gen_test_trie(
        nodestore: &mut NodeStore<Committed, MemStore>,
    ) -> (Vec<(Node, LinearAddress)>, u64, (LinearAddress, HashType)) {
        let mut high_watermark = NodeStoreHeader::SIZE;
        let leaf = Node::Leaf(LeafNode {
            partial_path: Path::from([0, 1]),
            value: Box::new([3, 4, 5]),
        });
        let leaf_addr = LinearAddress::new(high_watermark).unwrap();
        let leaf_hash = hash_node(&leaf, &Path::from_nibbles_iterator([0u8, 1].into_iter()));
        let leaf_area = test_write_new_node(nodestore, &leaf, high_watermark);
        high_watermark += leaf_area;

        let mut branch_children: [Option<Child>; BranchNode::MAX_CHILDREN] = Default::default();
        branch_children[1] = Some(Child::AddressWithHash(leaf_addr, leaf_hash));
        let branch = Node::Branch(Box::new(BranchNode {
            partial_path: Path::from([0]),
            value: None,
            children: branch_children,
        }));
        let branch_addr = LinearAddress::new(high_watermark).unwrap();
        let branch_hash = hash_node(&branch, &Path::from_nibbles_iterator([0u8].into_iter()));
        let branch_area = test_write_new_node(nodestore, &branch, high_watermark);
        high_watermark += branch_area;

        let mut root_children: [Option<Child>; BranchNode::MAX_CHILDREN] = Default::default();
        root_children[0] = Some(Child::AddressWithHash(branch_addr, branch_hash));
        let root = Node::Branch(Box::new(BranchNode {
            partial_path: Path::from([]),
            value: None,
            children: root_children,
        }));
        let root_addr = LinearAddress::new(high_watermark).unwrap();
        let root_hash = hash_node(&root, &Path::new());
        let root_area = test_write_new_node(nodestore, &root, high_watermark);
        high_watermark += root_area;

        // write the header
        test_write_header(
            nodestore,
            high_watermark,
            Some(root_addr),
            FreeLists::default(),
        );

        (
            vec![(leaf, leaf_addr), (branch, branch_addr), (root, root_addr)],
            high_watermark,
            (root_addr, root_hash),
        )
    }

    #[test]
    // This test creates a simple trie and checks that the checker traverses it correctly.
    // We use primitive calls here to do a low-level check.
    // TODO: add a high-level test in the firewood crate
    fn checker_traverse_correct_trie() {
        let memstore = MemStore::new(vec![]);
        let mut nodestore = NodeStore::new_empty_committed(memstore.into()).unwrap();

        let (_, high_watermark, (root_addr, root_hash)) = gen_test_trie(&mut nodestore);

        // verify that all of the space is accounted for - since there is no free area
        let mut visited = LinearAddressRangeSet::new(high_watermark).unwrap();
        nodestore
            .visit_trie(root_addr, root_hash, Path::new(), &mut visited, true)
            .unwrap();
        let complement = visited.complement();
        assert_eq!(complement.into_iter().collect::<Vec<_>>(), vec![]);
    }

    #[test]
    // This test permutes the simple trie with a wrong hash and checks that the checker detects it.
    fn checker_traverse_trie_with_wrong_hash() {
        let memstore = MemStore::new(vec![]);
        let mut nodestore = NodeStore::new_empty_committed(memstore.into()).unwrap();

        let (mut nodes, high_watermark, (root_addr, root_hash)) = gen_test_trie(&mut nodestore);

        // replace the branch hash in the root node with a wrong hash
        let [_, (branch_node, branch_addr), (root_node, _)] = nodes.as_mut_slice() else {
            panic!("test trie content changed, the test should be updated");
        };
        let wrong_hash = HashType::default();
        let branch_path = &branch_node.as_branch().unwrap().partial_path;
        let Some(Child::AddressWithHash(_, hash)) = root_node.as_branch_mut().unwrap().children
            [branch_path[0] as usize]
            .replace(Child::AddressWithHash(*branch_addr, wrong_hash.clone()))
        else {
            panic!("test trie content changed, the test should be updated");
        };
        let branch_hash = hash;
        test_write_new_node(&nodestore, root_node, root_addr.get());

        // verify that all of the space is accounted for - since there is no free area
        let mut visited = LinearAddressRangeSet::new(high_watermark).unwrap();
        let err = nodestore
            .visit_trie(root_addr, root_hash, Path::new(), &mut visited, true)
            .unwrap_err();
        assert!(matches!(
        err,
        CheckerError::HashMismatch {
            address,
            partial_path,
            parent_stored_hash,
            computed_hash
        }
        if address == *branch_addr
            && partial_path == *branch_path
            && parent_stored_hash == wrong_hash
            && computed_hash == branch_hash
        ));
    }

    #[test]
    fn traverse_correct_freelist() {
        use rand::Rng;

        let mut rng = crate::test_utils::seeded_rng();

        let memstore = MemStore::new(vec![]);
        let mut nodestore = NodeStore::new_empty_committed(memstore.into()).unwrap();

        // write free areas
        let mut high_watermark = NodeStoreHeader::SIZE;
        let mut freelist = FreeLists::default();
        for (area_index, area_size) in AREA_SIZES.iter().enumerate() {
            let mut next_free_block = None;
            let num_free_areas = rng.random_range(0..4);
            for _ in 0..num_free_areas {
                test_write_free_area(
                    &nodestore,
                    next_free_block,
                    area_index as u8,
                    high_watermark,
                );
                next_free_block = Some(LinearAddress::new(high_watermark).unwrap());
                high_watermark += area_size;
            }

            freelist[area_index] = next_free_block;
        }

        // write header
        test_write_header(&mut nodestore, high_watermark, None, freelist);

        // test that the we traversed all the free areas
        let mut visited = LinearAddressRangeSet::new(high_watermark).unwrap();
        nodestore.visit_freelist(&mut visited).unwrap();
        let complement = visited.complement();
        assert_eq!(complement.into_iter().collect::<Vec<_>>(), vec![]);
    }
}
