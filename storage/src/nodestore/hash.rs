// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! # Hash Module
//!
//! This module contains all node hashing functionality for the nodestore, including
//! specialized support for Ethereum-compatible hash processing.

use std::ops::{Deref, DerefMut};

use triomphe::Arc;

use super::NodeReader;
use crate::hashednode::hash_node;
use crate::linear::FileIoError;
use crate::logger::trace;
use crate::node::Node;
use crate::{
    Child, Children, HashType, MaybePersistedNode, NodeStore, Path, PathComponent, ReadableStorage,
    SharedNode,
};

/// Wrapper around a path that makes sure we truncate what gets extended to the path after it goes out of scope
/// This allows the same memory space to be reused for different path prefixes
#[derive(Debug)]
struct PathGuard<'a> {
    path: &'a mut Path,
    original_length: usize,
}

impl<'a> PathGuard<'a> {
    fn fork(&mut self) -> PathGuard<'_> {
        PathGuard::from_path(self.path)
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
pub(super) struct ClassifiedChildren<'a> {
    pub num_unhashed: usize,
    pub hashed: Vec<(PathComponent, Arc<Node>, &'a mut HashType)>,
}

impl<T, S: ReadableStorage> NodeStore<T, S>
where
    NodeStore<T, S>: NodeReader,
{
    /// Helper function to classify children for ethereum hash processing
    /// We have some special cases based on the number of children
    /// and whether they are hashed or unhashed, so we need to classify them.
    pub(super) fn ethhash_classify_children<'a>(
        &self,
        children: &'a mut Children<Option<Child>>,
    ) -> Result<ClassifiedChildren<'a>, FileIoError> {
        let mut num_unhashed = 0_usize;
        let mut hashed = Vec::with_capacity(16);
        for (idx, child) in children {
            match child {
                None => {}
                Some(Child::Node(_)) => {
                    num_unhashed = num_unhashed.wrapping_add(1);
                }
                Some(Child::AddressWithHash(addr, hash)) => {
                    let node = self.read_node(*addr)?;
                    hashed.push((idx, node, hash));
                }
                Some(Child::MaybePersisted(node, hash)) => {
                    let node = node.as_shared_node(self)?;
                    hashed.push((idx, node, hash));
                }
            }
        }

        Ok(ClassifiedChildren {
            num_unhashed,
            hashed,
        })
    }

    /// Hashes the given `node` and the subtree rooted at it. The `root_path` should be empty
    /// if this is called from the root, or it should include the partial path if this is called
    /// on a subtrie. Returns the hashed node and its hash.
    ///
    /// # Errors
    ///
    /// Can return a `FileIoError` if it is unable to read a node that it is hashing.
    pub fn hash_helper(
        &self,
        node: Node,
        mut root_path: Path,
    ) -> Result<(MaybePersistedNode, HashType), FileIoError> {
        // num_siblings is only relevant for nodes at depth 65 which is never
        // the root or its immediate children
        debug_assert!(!cfg!(feature = "ethhash") || root_path.0.len() != 65);
        self.hash_helper_inner(node, PathGuard::from_path(&mut root_path), 0)
    }

    /// Inner recursive hashing function.
    ///
    /// # Arguments
    ///
    /// - `node`: The node to hash. The hashing will be done recursively on its children.
    /// - `path_prefix`: The path prefix leading to this node.
    /// - `num_siblings`: (only with `ethhash` feature) The number of siblings of this
    ///   node in its parent. This is only relevant for the immediate children of
    ///   account branch nodes (nodes at depth 65 -- the first nibble past the account
    ///   prefix).
    fn hash_helper_inner(
        &self,
        mut node: Node,
        mut path_prefix: PathGuard<'_>,
        num_siblings: usize,
    ) -> Result<(MaybePersistedNode, HashType), FileIoError> {
        // If this is a branch, find all unhashed children and recursively hash them.
        trace!("hashing {node:?} at {path_prefix:?}");
        if let Node::Branch(ref mut b) = node {
            // branch children cases:
            // 1. 1 child, already hashed
            // 2. >1 child, already hashed,
            // 3. 1 hashed child, 1 unhashed child
            // 4. 0 hashed, 1 unhashed <-- handle child special
            // 5. 1 hashed, >0 unhashed <-- rehash case
            // 6. everything already hashed

            // special case code for ethereum hashes at the account level
            let num_children = if cfg!(feature = "ethhash")
                && path_prefix.0.len().saturating_add(b.partial_path.0.len()) == 64
            {
                let ClassifiedChildren {
                    num_unhashed,
                    mut hashed,
                } = self.ethhash_classify_children(&mut b.children)?;
                trace!("hashed {hashed:?} unhashed {num_unhashed:?}");
                let num_children = hashed.len().wrapping_add(num_unhashed);
                // If there was only one child in the current account branch when previously hashed, we need to rehash it
                if let [(child_idx, child_node, child_hash)] = &mut hashed[..] {
                    let mut path_prefix = path_prefix.fork();
                    path_prefix.extend(b.partial_path.0.iter().copied());
                    path_prefix.push(*child_idx);
                    **child_hash =
                        Self::compute_node_hash(child_node, &mut path_prefix, num_children);
                }

                num_children
            } else {
                // not an account branch - does not matter what we return here
                0
            };

            // general case: recusively hash unhashed children
            for (nibble, child) in &mut b.children {
                // If this is empty or already hashed, we're done
                let Some(child_node) = child.as_mut().and_then(|child| child.as_mut_node()) else {
                    continue;
                };

                // remove the child from the children array, we will replace it with a hashed variant
                let child_node = std::mem::take(child_node);

                // Hash this child and update
                let mut path_prefix = path_prefix.fork();
                path_prefix.extend(b.partial_path.0.iter().copied());
                path_prefix.push(nibble);
                let (child_node, child_hash) =
                    self.hash_helper_inner(child_node, path_prefix, num_children)?;

                *child = Some(Child::MaybePersisted(child_node, child_hash));
                trace!("child now {child:?}");
            }
        }

        // At this point, we either have a leaf or a branch with all children hashed.
        // if the encoded child hash <32 bytes then we use that RLP
        let hash = Self::compute_node_hash(&node, &mut path_prefix, num_siblings);

        Ok((SharedNode::new(node).into(), hash))
    }

    /// This function computes the ethhash of a single node assuming all its children are hashed.
    /// Note that `num_siblings` is the number of children of the parent node, which includes this node.
    /// The function appends to `path_prefix` and then truncate it back to the original length - we only reuse the memory space to avoid allocations
    pub(crate) fn compute_node_hash(
        node: &Node,
        path_prefix: &mut Path,
        num_siblings: usize,
    ) -> HashType {
        if cfg!(feature = "ethhash") && path_prefix.0.len() == 65 && num_siblings == 1 {
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
