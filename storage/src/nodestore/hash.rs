// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! # Hash Module
//!
//! This module contains all node hashing functionality for the nodestore, including
//! specialized support for Ethereum-compatible hash processing.

#[cfg(feature = "ethhash")]
use crate::Children;
use crate::hashednode::hash_node;
use crate::linear::FileIoError;
use crate::logger::trace;
use crate::node::Node;
#[cfg(feature = "ethhash")]
use crate::node::branch::NodeRefWithHashMut;
use crate::{Child, HashType, MaybePersistedNode, NodeStore, Path, ReadableStorage, SharedNode};

use super::NodeReader;

use std::ops::{Deref, DerefMut};

#[derive(Debug)]
// Wrapper around a path that makes sure we truncate what gets extended to the path after it goes out of scope
// This allows the same memory space to be reused for different path prefixes
struct PathGuard<'a> {
    path: &'a mut Path,
    original_length: usize,
}

impl<'a> PathGuard<'a> {
    fn new(path: &'a mut PathGuard<'_>) -> Self {
        Self {
            original_length: path.0.len(),
            path: &mut path.path,
        }
    }

    fn from_path(path: &'a mut Path) -> Self {
        Self {
            original_length: path.0.len(),
            path,
        }
    }
}

impl Drop for PathGuard<'_> {
    fn drop(&mut self) {
        self.path.0.truncate(self.original_length);
    }
}

impl Deref for PathGuard<'_> {
    type Target = Path;
    fn deref(&self) -> &Self::Target {
        self.path
    }
}

impl DerefMut for PathGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.path
    }
}

/// Classified children for ethereum hash processing
#[cfg(feature = "ethhash")]
pub(super) struct ClassifiedChildren<'a> {
    pub num_unhashed: usize,
    pub hashed: Vec<(usize, NodeRefWithHashMut<'a>)>,
}

impl<T, S: ReadableStorage> NodeStore<T, S>
where
    NodeStore<T, S>: NodeReader,
{
    /// Helper function to classify children for ethereum hash processing
    /// We have some special cases based on the number of children
    /// and whether they are hashed or unhashed, so we need to classify them.
    #[cfg(feature = "ethhash")]
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "num_hashed will be less than number of children (small const)"
    )]
    pub(super) fn ethhash_classify_children<'a>(
        &self,
        children: &'a mut Children<Child>,
    ) -> Result<ClassifiedChildren<'a>, FileIoError> {
        let mut num_unhashed = 0;
        let mut hashed = Vec::new();
        for (idx, child) in children.iter_mut().enumerate() {
            match child {
                None => {}
                Some(child) => match child.node_ref_and_hash_mut(self)? {
                    Some(node_ref_with_hash) => hashed.push((idx, node_ref_with_hash)),
                    None => num_unhashed += 1,
                },
            }
        }

        Ok(ClassifiedChildren {
            num_unhashed,
            hashed,
        })
    }

    /// Hashes the given `node` and the subtree rooted at it.
    /// Returns the hashed node and its hash.
    pub(super) fn hash_helper(
        #[cfg(feature = "ethhash")] &self,
        node: Node,
    ) -> Result<(MaybePersistedNode, HashType, usize), FileIoError> {
        let mut root_path = Path::new();
        #[cfg(not(feature = "ethhash"))]
        let res = Self::hash_helper_inner(node, PathGuard::from_path(&mut root_path))?;
        #[cfg(feature = "ethhash")]
        let res = self.hash_helper_inner(node, PathGuard::from_path(&mut root_path), 1)?;
        Ok(res)
    }

    // Recursive helper that hashes the given `node` and the subtree rooted at it.
    // This function takes a mut `node` to update the hash in place.
    // The `path_prefix` is also mut because we will extend it to the path of the child we are hashing in recursive calls - it will be restored after the recursive call returns.
    // The `num_siblings` is the number of children of the parent node, which includes this node.
    fn hash_helper_inner(
        #[cfg(feature = "ethhash")] &self,
        mut node: Node,
        mut path_prefix: PathGuard<'_>,
        #[cfg(feature = "ethhash")] num_siblings: usize,
    ) -> Result<(MaybePersistedNode, HashType, usize), FileIoError> {
        // If this is a branch, find all unhashed children and recursively hash them.
        trace!("hashing {node:?} at {path_prefix:?}");
        let mut nodes_processed = 1usize; // Count this node
        if let Node::Branch(ref mut b) = node {
            #[cfg(feature = "ethhash")]
            // special case for ethhash at the account level
            let num_children = if path_prefix.0.len().saturating_add(b.partial_path.0.len()) == 64 {
                let ClassifiedChildren {
                    num_unhashed,
                    mut hashed,
                } = self.ethhash_classify_children(&mut b.children)?;
                trace!("hashed {hashed:?} unhashed {num_unhashed:?}");
                #[expect(
                    clippy::arithmetic_side_effects,
                    reason = "hashed and unhashed can have at most 16 elements"
                )]
                let num_children = hashed.len() + num_unhashed;
                // If there was only one child in the current account branch when previously hashed, we need to rehash it
                if let [
                    (
                        child_idx,
                        NodeRefWithHashMut {
                            node_ref: child_node,
                            hash: child_hash,
                        },
                    ),
                ] = &mut hashed[..]
                {
                    let hash = {
                        let mut path_guard = PathGuard::new(&mut path_prefix);
                        path_guard.0.extend(b.partial_path.0.iter().copied());
                        path_guard.0.push(*child_idx as u8);
                        Self::compute_node_ethhash(child_node, &mut path_guard, num_children)
                    };
                    **child_hash = hash;
                }

                num_children
            } else {
                // not an account branch - does not matter what we return here
                0
            };

            // branch children cases:
            // 1. 1 child, already hashed
            // 2. >1 child, already hashed,
            // 3. 1 hashed child, 1 unhashed child
            // 4. 0 hashed, 1 unhashed <-- handle child special
            // 5. 1 hashed, >0 unhashed <-- rehash case
            // 6. everything already hashed

            // general case: recusively hash unhashed children
            for (nibble, child) in b.children.iter_mut().enumerate() {
                // If this is empty or already hashed, we're done
                let Some(child_node) = child.as_mut().and_then(|child| child.as_mut_node()) else {
                    continue;
                };

                // remove the child from the children array, we will replace it with a hashed variant
                let child_node = std::mem::take(child_node);

                // Hash this child and update
                let (child_node, child_hash) = {
                    let mut child_path_prefix = PathGuard::new(&mut path_prefix);
                    child_path_prefix.0.extend(b.partial_path.0.iter().copied());
                    child_path_prefix.0.push(nibble as u8);
                    #[cfg(feature = "ethhash")]
                    let (child_node, child_hash, child_count) =
                        self.hash_helper_inner(child_node, child_path_prefix, num_children)?;
                    #[cfg(not(feature = "ethhash"))]
                    let (child_node, child_hash, child_count) =
                        Self::hash_helper_inner(child_node, child_path_prefix)?;
                    nodes_processed = nodes_processed.saturating_add(child_count);
                    (child_node, child_hash)
                };

                *child = Some(Child::MaybePersisted(child_node, child_hash));
                trace!("child now {child:?}");
            }
        }

        // At this point, we either have a leaf or a branch with all children hashed.
        // if the encoded child hash <32 bytes then we use that RLP
        #[cfg(feature = "ethhash")]
        let hash = Self::compute_node_ethhash(&node, &mut path_prefix, num_siblings);
        #[cfg(not(feature = "ethhash"))]
        let hash = hash_node(&node, &path_prefix);

        Ok((SharedNode::new(node).into(), hash, nodes_processed))
    }

    #[cfg(feature = "ethhash")]
    /// This function computes the ethhash of a single node assuming all its children are hashed.
    /// Note that `num_siblings` is the number of children of the parent node, which includes this node.
    /// The function appends to `path_prefix` and then truncate it back to the original length - we only reuse the memory space to avoid allocations
    pub(crate) fn compute_node_ethhash(
        node: &Node,
        path_prefix: &mut Path,
        num_siblings: usize,
    ) -> HashType {
        if path_prefix.0.len() == 65 && num_siblings == 1 {
            // This is the special case when this node is the only child of an account branch node
            //  - 64 nibbles for account + 1 nibble for its position in account branch node
            let mut fake_root = node.clone();
            let extra_nibble = path_prefix.0.pop().expect("path_prefix not empty");
            fake_root.update_partial_path(Path::from_nibbles_iterator(
                std::iter::once(extra_nibble).chain(fake_root.partial_path().0.iter().copied()),
            ));
            let hash = hash_node(&fake_root, path_prefix);
            path_prefix.0.push(extra_nibble);
            hash
        } else {
            hash_node(node, path_prefix)
        }
    }
}
