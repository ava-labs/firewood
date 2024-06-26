// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::hashednode::HashedNodeStore;
use crate::proof::{Proof, ProofError};
use crate::stream::{MerkleKeyValueStream, PathIterator};
use crate::v2::api;
use futures::{StreamExt, TryStreamExt};
use std::collections::HashSet;
use std::future::ready;
use std::io::Write;
use std::iter::{empty, once};
use storage::{
    BranchNode, LeafNode, LinearAddress, NibblesIterator, Node, Path, PathIterItem,
    ProposedImmutable, ReadLinearStore, TrieHash, WriteLinearStore,
};

use std::ops::{Deref, DerefMut};
use thiserror::Error;

pub type Key = Box<[u8]>;
pub type Value = Vec<u8>;

#[derive(Debug, Error)]
pub enum MerkleError {
    #[error("uninitialized")]
    Uninitialized,
    #[error("read only")]
    ReadOnly,
    #[error("node not a branch node")]
    NotBranchNode,
    #[error("format error: {0:?}")]
    Format(#[from] std::io::Error),
    #[error("parent should not be a leaf branch")]
    ParentLeafBranch,
    #[error("removing internal node references failed")]
    UnsetInternal,
    #[error("merkle serde error: {0}")]
    BinarySerdeError(String),
    #[error("invalid utf8")]
    UTF8Error,
}

#[derive(Debug)]
pub struct Merkle<T: ReadLinearStore>(HashedNodeStore<T>);

impl<T: ReadLinearStore> Deref for Merkle<T> {
    type Target = HashedNodeStore<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: ReadLinearStore> DerefMut for Merkle<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

// convert a set of nibbles into a printable string
// panics if there is a non-nibble byte in the set
fn nibbles_formatter<X: IntoIterator<Item = u8>>(nib: X) -> String {
    nib.into_iter()
        .map(|c| {
            *b"0123456789abcdef"
                .get(c as usize)
                .expect("requires nibbles") as char
        })
        .collect::<String>()
}

macro_rules! write_attributes {
    ($writer:ident, $node:expr, $value:expr) => {
        if !$node.partial_path.0.is_empty() {
            write!(
                $writer,
                " pp={}",
                nibbles_formatter($node.partial_path.0.clone())
            )?;
        }
        #[allow(clippy::unnecessary_to_owned)]
        if !$value.is_empty() {
            match std::str::from_utf8($value) {
                Ok(string) if string.chars().all(char::is_alphanumeric) => {
                    write!($writer, " val={}", string)?
                }
                _ => {
                    write!($writer, " val={}", hex::encode($value))?;
                }
            }
        }
    };
}

impl<T: ReadLinearStore> Merkle<T> {
    pub const fn new(store: HashedNodeStore<T>) -> Merkle<T> {
        Merkle(store)
    }

    pub const fn root_address(&self) -> Option<LinearAddress> {
        self.0.root_address()
    }

    // TODO: can we make this &self instead of &mut self?
    pub fn root_hash(&mut self) -> Result<TrieHash, std::io::Error> {
        self.0.root_hash()
    }

    /// Constructs a merkle proof for key. The result contains all encoded nodes
    /// on the path to the value at key. The value itself is also included in the
    /// last node and can be retrieved by verifying the proof.
    ///
    /// If the trie does not contain a value for key, the returned proof contains
    /// all nodes of the longest existing prefix of the key, ending with the node
    /// that proves the absence of the key (at least the root node).
    pub fn prove(&self, _key: &[u8]) -> Result<Proof<Vec<u8>>, MerkleError> {
        todo!()
        // let mut proofs = HashMap::new();
        // if root_addr.is_null() {
        //     return Ok(Proof(proofs));
        // }

        // let sentinel_node = self.get_node(root_addr)?;

        // let path_iter = self.path_iter(sentinel_node, key.as_ref());

        // let nodes = path_iter
        //     .map(|result| result.map(|(_, node)| node))
        //     .collect::<Result<Vec<NodeObjRef>, MerkleError>>()?;

        // // Get the hashes of the nodes.
        // for node in nodes.into_iter() {
        //     let encoded = node.get_encoded(&self.store);
        //     let hash: [u8; TRIE_HASH_LEN] = sha3::Keccak256::digest(encoded).into();
        //     proofs.insert(hash, encoded.to_vec());
        // }
        // Ok(Proof(proofs))
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Box<[u8]>>, MerkleError> {
        let path_iter = self.path_iter(key)?;

        let Some(last_node) = path_iter.last() else {
            return Ok(None);
        };

        let last_node = last_node?;

        if last_node
            .key_nibbles
            .iter()
            .copied()
            .eq(NibblesIterator::new(key))
        {
            match &*last_node.node {
                Node::Branch(branch) => Ok(branch.value.clone()),
                Node::Leaf(leaf) => Ok(Some(leaf.value.clone())),
            }
        } else {
            Ok(None)
        }
    }

    pub fn verify_proof<N: AsRef<[u8]> + Send>(
        &self,
        _key: &[u8],
        _proof: &Proof<N>,
    ) -> Result<Option<Vec<u8>>, MerkleError> {
        todo!()
    }

    pub fn verify_range_proof<N: AsRef<[u8]> + Send, V: AsRef<[u8]>>(
        &self,
        _proof: &Proof<N>,
        _first_key: &[u8],
        _last_key: &[u8],
        _keys: Vec<&[u8]>,
        _vals: Vec<V>,
    ) -> Result<bool, ProofError> {
        todo!()
    }

    // TODO danlaine: can we use the LinearAddress of the `root` instead?
    pub fn path_iter<'a>(&self, key: &'a [u8]) -> Result<PathIterator<'_, 'a, T>, MerkleError> {
        PathIterator::new(self, key)
    }

    pub(crate) fn _key_value_iter(&self) -> MerkleKeyValueStream<'_, T> {
        MerkleKeyValueStream::_new(self)
    }

    pub(crate) fn _key_value_iter_from_key(&self, key: Key) -> MerkleKeyValueStream<'_, T> {
        // TODO danlaine: change key to &[u8]
        MerkleKeyValueStream::_from_key(self, key)
    }

    pub(super) async fn _range_proof(
        &self,
        first_key: Option<&[u8]>,
        last_key: Option<&[u8]>,
        limit: Option<usize>,
    ) -> Result<Option<api::RangeProof<Vec<u8>, Vec<u8>>>, api::Error> {
        if let (Some(k1), Some(k2)) = (&first_key, &last_key) {
            if k1 > k2 {
                return Err(api::Error::InvalidRange {
                    first_key: k1.to_vec(),
                    last_key: k2.to_vec(),
                });
            }
        }

        // limit of 0 is always an empty RangeProof
        if limit == Some(0) {
            return Ok(None);
        }

        let mut stream = match first_key {
            // TODO: fix the call-site to force the caller to do the allocation
            Some(key) => self._key_value_iter_from_key(key.to_vec().into_boxed_slice()),
            None => self._key_value_iter(),
        };

        // fetch the first key from the stream
        let first_result = stream.next().await;

        // transpose the Option<Result<T, E>> to Result<Option<T>, E>
        // If this is an error, the ? operator will return it
        let Some((first_key, first_value)) = first_result.transpose()? else {
            // nothing returned, either the trie is empty or the key wasn't found
            return Ok(None);
        };

        let first_key_proof = self
            .prove(&first_key)
            .map_err(|e| api::Error::InternalError(Box::new(e)))?;
        let limit = limit.map(|old_limit| old_limit - 1);

        let mut middle = vec![(first_key.into_vec(), first_value)];

        // we stop streaming if either we hit the limit or the key returned was larger
        // than the largest key requested
        #[allow(clippy::unwrap_used)]
        middle.extend(
            stream
                .take(limit.unwrap_or(usize::MAX))
                .take_while(|kv_result| {
                    // no last key asked for, so keep going
                    let Some(last_key) = last_key else {
                        return ready(true);
                    };

                    // return the error if there was one
                    let Ok(kv) = kv_result else {
                        return ready(true);
                    };

                    // keep going if the key returned is less than the last key requested
                    ready(&*kv.0 <= last_key)
                })
                .map(|kv_result| kv_result.map(|(k, v)| (k.into_vec(), v)))
                .try_collect::<Vec<(Vec<u8>, Vec<u8>)>>()
                .await?,
        );

        // remove the last key from middle and do a proof on it
        let last_key_proof = match middle.last() {
            None => {
                return Ok(Some(api::RangeProof {
                    first_key_proof: first_key_proof.clone(),
                    middle: vec![],
                    last_key_proof: first_key_proof,
                }))
            }
            Some((last_key, _)) => self
                .prove(last_key)
                .map_err(|e| api::Error::InternalError(Box::new(e)))?,
        };

        Ok(Some(api::RangeProof {
            first_key_proof,
            middle,
            last_key_proof,
        }))
    }

    pub fn dump_node(
        &self,
        addr: LinearAddress,
        hash: Option<&TrieHash>,
        seen: &mut HashSet<LinearAddress>,
        writer: &mut dyn Write,
    ) -> Result<(), std::io::Error> {
        let node = self.read_node(addr)?;
        match &*node {
            Node::Branch(b) => {
                write!(writer, "  {addr}[label=\"@{addr} hash={hash:?}")?;

                write_attributes!(writer, b, &b.value.clone().unwrap_or(Box::from([])));
                writeln!(writer, "\"]")?;
                for (childidx, child) in b.children.iter().enumerate() {
                    match child {
                        None => {}
                        Some((childaddr, child_hash)) => {
                            if !seen.insert(*childaddr) {
                                // we have already seen this child, so
                                writeln!(
                                    writer,
                                    "  {addr} -> {childaddr}[label=\"{childidx} (dup)\" color=red]"
                                )?;
                            } else {
                                writeln!(writer, "  {addr} -> {childaddr}[label=\"{childidx}\"]")?;
                                self.dump_node(*childaddr, child_hash.as_ref(), seen, writer)?;
                            }
                        }
                    }
                }
            }
            Node::Leaf(l) => {
                write!(writer, "  {addr}[label=\"@{addr} hash={hash:?}")?;
                write_attributes!(writer, l, &l.value);
                writeln!(writer, "\" shape=rect]")?;
            }
        };
        Ok(())
    }

    pub fn dump(&self) -> Result<String, std::io::Error> {
        let mut result = vec![];
        writeln!(result, "digraph Merkle {{")?;
        if let Some(addr) = self.root_address() {
            writeln!(result, " root -> {addr}")?;
            let mut seen = HashSet::new();
            // TODO danlaine: Figure out a way to print the root hash instead of
            // passing None. self.root_hash takes in &mut self but dump takes &self.
            self.dump_node(addr, None, &mut seen, &mut result)?;
        }
        write!(result, "}}")?;

        Ok(String::from_utf8_lossy(&result).to_string())
    }
}

impl<T: WriteLinearStore> Merkle<T> {
    pub fn insert(&mut self, key: &[u8], value: Box<[u8]>) -> Result<(), MerkleError> {
        let path = Path::from_nibbles_iterator(NibblesIterator::new(key));

        // The path from the root down to and including the node with the greatest prefix of `path`.
        let ancestors =
            PathIterator::new(self, key)?.collect::<Result<Vec<PathIterItem>, MerkleError>>()?;

        let mut ancestors = ancestors.iter();

        let Some(greatest_prefix_node) = ancestors.next_back() else {
            // There is no node (not even the root) which is a prefix of `path`.
            // See if there is a root.
            let Some(old_root_addr) = self.root_address() else {
                // The trie is empty. Create a new leaf node with `value` and set
                // it as the root.
                let root = Node::Leaf(LeafNode {
                    partial_path: path,
                    value,
                });
                let root_addr = self.create_node(root)?;
                self.set_root(Some(root_addr))?;
                return Ok(());
            };
            // There is a root but it's not a prefix of `path`.
            // Insert a new branch node above the existing root and make the
            // old root a child of the new branch node.
            //
            //  old_root  -->  new_root
            //                 /       \
            //              old_root   *new_leaf
            //
            // *Note that new_leaf is only created if `path` isn't a prefix of old root.
            let old_root = self.read_node(old_root_addr)?;

            let path_overlap = PrefixOverlap::from(old_root.partial_path().as_ref(), path.as_ref());

            let (&old_root_child_index, old_root_partial_path) = path_overlap
                .unique_a
                .split_first()
                .expect("old_root shouldn't be prefix of path");

            // Shorten the partial path of `old_root` since it has a parent now.
            let old_root = match &*old_root {
                Node::Leaf(old_root) => Node::Leaf(LeafNode {
                    value: old_root.value.clone(),
                    partial_path: Path::from(old_root_partial_path),
                }),
                Node::Branch(old_root) => Node::Branch(Box::new(BranchNode {
                    children: old_root.children.clone(),
                    partial_path: Path::from(old_root_partial_path),
                    value: old_root.value.clone(),
                })),
            };
            let old_root_addr = self.update_node(empty(), old_root_addr, old_root)?;

            let mut new_root = BranchNode {
                partial_path: Path::from(path_overlap.shared),
                value: None,
                children: Default::default(),
            };
            new_root.update_child(old_root_child_index, Some(old_root_addr));

            if let Some((&new_leaf_child_index, remaining_path)) =
                path_overlap.unique_b.split_first()
            {
                let new_leaf_addr = self.create_node(Node::Leaf(LeafNode {
                    value,
                    partial_path: Path::from(remaining_path),
                }))?;
                new_root.update_child(new_leaf_child_index, Some(new_leaf_addr));
            } else {
                new_root.value = Some(value);
            }

            let new_root_addr = self.create_node(Node::Branch(Box::new(new_root)))?;
            self.set_root(Some(new_root_addr))?;
            return Ok(());
        };
        // `greatest_prefix_node` is a prefix of `path`

        // `remaining_path` is `path` with prefix `greatest_prefix_node` removed.
        let remaining_path = {
            let overlap = PrefixOverlap::from(&greatest_prefix_node.key_nibbles, &path);
            assert!(overlap.unique_a.is_empty());
            overlap.unique_b
        };

        let Some((&child_index, remaining_path)) = remaining_path.split_first() else {
            // `greatest_prefix_node` is at `path`. Update its value.
            //     ...                        ...
            //      |                 -->      |
            //  greatest_prefix_node       greatest_prefix_node
            let node = match &*greatest_prefix_node.node {
                Node::Leaf(node) => Node::Leaf(LeafNode {
                    value,
                    partial_path: node.partial_path.clone(),
                }),
                Node::Branch(node) => Node::Branch(Box::new(BranchNode {
                    children: node.children.clone(),
                    partial_path: node.partial_path.clone(),
                    value: Some(value),
                })),
            };

            self.update_node(ancestors, greatest_prefix_node.addr, node)?;

            return Ok(());
        };

        match &*greatest_prefix_node.node {
            Node::Leaf(leaf) => {
                // `leaf` is at a strict prefix of `path`. Replace it with a branch.
                //
                //     ...                ...
                //      |      -->         |
                //     leaf            new_branch
                //                         |
                //                        leaf
                let new_leaf_addr = self.create_node(Node::Leaf(LeafNode {
                    value,
                    partial_path: Path::from(remaining_path),
                }))?;

                let mut new_branch: BranchNode = leaf.into();
                new_branch.update_child(child_index, Some(new_leaf_addr));
                self.update_node(
                    ancestors,
                    greatest_prefix_node.addr,
                    Node::Branch(Box::new(new_branch)),
                )?;

                Ok(())
            }
            Node::Branch(branch) => {
                // See if `branch` has a child at `child_index` already.
                let Some((&child_addr, _)) = branch.child(child_index) else {
                    // Create a new leaf at empty `child_index`.
                    //     ...                ...
                    //      |      -->         |
                    //    branch             branch
                    //    /   \           /    |        \
                    //  ...   ...       ...  new_leaf   ...
                    let new_leaf_addr = self.create_node(Node::Leaf(LeafNode {
                        value,
                        partial_path: Path::from(remaining_path),
                    }))?;

                    let mut branch = BranchNode {
                        children: branch.children.clone(),
                        partial_path: branch.partial_path.clone(),
                        value: branch.value.clone(),
                    };
                    // We don't need to call update_child because we know there is no
                    // child at `child_index` whose hash we need to invalidate.
                    branch.update_child(child_index, Some(new_leaf_addr));

                    self.update_node(
                        ancestors,
                        greatest_prefix_node.addr,
                        Node::Branch(Box::new(branch)),
                    )?;
                    return Ok(());
                };

                // There is a child at `child_index` already.
                // Note that `child` can't be a prefix of the key we're inserting
                // because it's deeper than `branch`, which is guaranteed to be the
                // node with the largest prefix of the key we're inserting.
                //    ...                ...
                //     |      -->         |
                //  branch             branch
                //  /    \            /      \
                // ...  child       ...     new_branch
                //                            |       \
                //                         new_leaf*   child
                //
                // *Note that new_leaf is only created if the remaining path is non-empty.
                // Note that child may be a leaf or a branch node.
                let child = self.read_node(child_addr)?;

                let path_overlap = PrefixOverlap::from(child.partial_path(), remaining_path);

                let (&new_branch_to_child_index, child_partial_path) = path_overlap
                    .unique_a
                    .split_first()
                    .expect("child can't be prefix of key");

                // Update `child` to shorten its partial path.
                let child = match &*child {
                    Node::Branch(child) => Node::Branch(Box::new(BranchNode {
                        children: child.children.clone(),
                        partial_path: Path::from(child_partial_path),
                        value: child.value.clone(),
                    })),
                    Node::Leaf(child) => Node::Leaf(LeafNode {
                        value: child.value.clone(),
                        partial_path: Path::from(child_partial_path),
                    }),
                };
                let child_addr = self.update_node(
                    ancestors.clone().chain(once(greatest_prefix_node)),
                    child_addr,
                    child,
                )?;

                let mut new_branch = BranchNode {
                    children: Default::default(),
                    partial_path: Path::from(path_overlap.shared),
                    value: None,
                };
                new_branch.update_child(new_branch_to_child_index, Some(child_addr));

                if let Some((&new_leaf_child_index, remaining_path)) =
                    path_overlap.unique_b.split_first()
                {
                    let new_leaf_addr = self.create_node(Node::Leaf(LeafNode {
                        value,
                        partial_path: Path::from(remaining_path),
                    }))?;
                    new_branch.update_child(new_leaf_child_index, Some(new_leaf_addr));
                } else {
                    new_branch.value = Some(value);
                }
                let new_branch_addr = self.create_node(Node::Branch(Box::new(new_branch)))?;

                let mut branch = BranchNode {
                    children: branch.children.clone(),
                    partial_path: branch.partial_path.clone(),
                    value: branch.value.clone(),
                };
                branch.update_child(child_index, Some(new_branch_addr));
                self.update_node(
                    ancestors,
                    greatest_prefix_node.addr,
                    Node::Branch(Box::new(branch)),
                )?;

                Ok(())
            }
        }
    }

    /// Removes the value associated with the given `key`.
    /// Returns the value that was removed, if any.
    /// Otherwise returns `None`.
    pub fn remove(&mut self, key: &[u8]) -> Result<Option<Box<[u8]>>, MerkleError> {
        let path = Path::from_nibbles_iterator(NibblesIterator::new(key));

        let ancestors =
            PathIterator::new(self, key)?.collect::<Result<Vec<PathIterItem>, MerkleError>>()?;

        // If the last element of ancestors doesn't contain the key/value pair we are looking for,
        // then we can return early without making any changes

        // The path from the root down to and including the node with the greatest prefix of `path`
        let mut ancestors = ancestors.iter();

        let Some(greatest_prefix_node) = ancestors.next_back() else {
            // There is no node which is a prefix of `path`.
            // Therefore `path` is not in the trie.
            return Ok(None);
        };

        if &*greatest_prefix_node.key_nibbles != path.0.as_ref() {
            // `greatest_prefix_node` is a prefix of `path` but not equal to `path`.
            // Therefore `path` is not in the trie.
            return Ok(None);
        }

        let removed = greatest_prefix_node;

        match &*removed.node {
            Node::Branch(branch) => {
                let Some(removed_value) = &branch.value else {
                    // The node at `path` has no value so there's nothing to remove.
                    return Ok(None);
                };

                // We know the key exists, so see if the branch has more than 1 child.
                let mut branch_children = branch
                    .children
                    .iter()
                    .enumerate()
                    .filter_map(|(index, addr)| addr.as_ref().map(|addr| (index as u8, addr)));

                let (child_index, (child_addr, _)) =
                    branch_children.next().expect("branch must have children");

                if branch_children.next().is_some() {
                    // The branch has more than 1 child. Remove the value but keep the branch.
                    //    ...                ...
                    //     |      -->         |
                    //  branch             branch (no value now)
                    //  /    \            /      \
                    // ...  child       ...     child
                    //
                    // Note in the after diagram `child` may be the only child of `branch`.
                    let branch = BranchNode {
                        children: branch.children.clone(),
                        partial_path: branch.partial_path.clone(),
                        value: None,
                    };
                    self.update_node(ancestors, removed.addr, Node::Branch(Box::new(branch)))?;
                    return Ok(Some(removed_value.clone()));
                }

                // The branch has only 1 child.
                // The branch must be combined with its child since it now has no value.
                //      ...                ...
                //       |      -->         |
                //     branch            combined
                //       |
                //     child
                //
                // where combined has `child`'s value.
                // Note that child/combined may be a leaf or a branch.

                let combined = {
                    let child = self.read_node(*child_addr)?;

                    // `combined`'s partial path is the concatenation of `branch`'s partial path,
                    // `child_index` and `child`'s partial path.
                    let partial_path = Path::from_nibbles_iterator(
                        branch
                            .partial_path
                            .iter()
                            .chain(once(&child_index))
                            .chain(child.partial_path().iter())
                            .copied(),
                    );

                    child.new_with_partial_path(partial_path)
                };

                self.delete_node(*child_addr)?;

                self.update_node(ancestors, removed.addr, combined)?;

                Ok(Some(removed_value.clone()))
            }
            Node::Leaf(leaf) => {
                self.delete_node(removed.addr)?;

                while let Some(ancestor) = ancestors.next_back() {
                    // Remove all ancestors until we find one that has a value
                    // or multiple children.
                    let ancestor_addr = ancestor.addr;
                    let child_index = ancestor.next_nibble.expect("parent has a child");
                    let ancestor = ancestor.node.as_branch().expect("parent must be a branch");

                    let num_children = ancestor
                        .children
                        .iter()
                        .filter(|addr| addr.is_some())
                        .count();

                    if num_children > 1 {
                        #[rustfmt::skip]
                        // Update `ancestor` to remove its child.
                        //    ...                 ...
                        //     |          -->      |
                        //  ancestor            ancestor
                        //  /      \               |
                        // ...    child           ...
                        let mut ancestor = BranchNode {
                            children: ancestor.children.clone(),
                            partial_path: ancestor.partial_path.clone(),
                            value: ancestor.value.clone(),
                        };
                        ancestor.update_child(child_index, None);
                        self.update_node(
                            ancestors,
                            ancestor_addr,
                            Node::Branch(Box::new(ancestor)),
                        )?;
                        return Ok(Some(leaf.value.clone()));
                    }

                    // The ancestor has only 1 child, which is now deleted.
                    if let Some(ancestor_value) = &ancestor.value {
                        // Turn the ancestor into a leaf.
                        let ancestor = Node::Leaf(LeafNode {
                            value: ancestor_value.clone(),
                            partial_path: ancestor.partial_path.clone(),
                        });
                        self.update_node(ancestors, ancestor_addr, ancestor)?;
                        return Ok(Some(leaf.value.clone()));
                    }

                    // The ancestor had 1 child and no value so it should be removed.
                    self.delete_node(ancestor_addr)?;
                }

                // The trie is now empty.
                self.set_root(None)?;
                Ok(Some(leaf.value.clone()))
            }
        }
    }

    pub fn freeze(self) -> Result<Merkle<ProposedImmutable>, MerkleError> {
        Ok(Merkle(self.0.freeze()?))
    }
}

impl<T: WriteLinearStore> Merkle<T> {
    pub fn put_node(&mut self, node: Node) -> Result<LinearAddress, MerkleError> {
        self.create_node(node).map_err(MerkleError::Format)
    }
}

/// Returns an iterator where each element is the result of combining
/// 2 nibbles of `nibbles`. If `nibbles` is odd length, panics in
/// debug mode and drops the final nibble in release mode.
pub fn nibbles_to_bytes_iter(nibbles: &[u8]) -> impl Iterator<Item = u8> + '_ {
    debug_assert_eq!(nibbles.len() & 1, 0);
    #[allow(clippy::indexing_slicing)]
    nibbles.chunks_exact(2).map(|p| (p[0] << 4) | p[1])
}

/// The [`PrefixOverlap`] type represents the _shared_ and _unique_ parts of two potentially overlapping slices.
/// As the type-name implies, the `shared` property only constitues a shared *prefix*.
/// The `unique_*` properties, [`unique_a`][`PrefixOverlap::unique_a`] and [`unique_b`][`PrefixOverlap::unique_b`]
/// are set based on the argument order passed into the [`from`][`PrefixOverlap::from`] constructor.
#[derive(Debug)]
struct PrefixOverlap<'a, T> {
    shared: &'a [T],
    unique_a: &'a [T],
    unique_b: &'a [T],
}

impl<'a, T: PartialEq> PrefixOverlap<'a, T> {
    fn from(a: &'a [T], b: &'a [T]) -> Self {
        let split_index = a
            .iter()
            .zip(b)
            .position(|(a, b)| *a != *b)
            .unwrap_or_else(|| std::cmp::min(a.len(), b.len()));

        let (shared, unique_a) = a.split_at(split_index);
        let unique_b = b.get(split_index..).expect("");

        Self {
            shared,
            unique_a,
            unique_b,
        }
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used)]
mod tests {
    use super::*;
    use storage::MemStore;
    use test_case::test_case;

    #[test]
    fn insert_one() {
        let mut merkle = create_in_memory_merkle();
        merkle.insert(b"abc", Box::new([])).unwrap()
    }

    fn create_in_memory_merkle() -> Merkle<MemStore> {
        Merkle::new(HashedNodeStore::initialize(MemStore::new(vec![])).unwrap())
    }

    // use super::*;
    // use test_case::test_case;

    //     fn branch(path: &[u8], value: &[u8], encoded_child: Option<Vec<u8>>) -> Node {
    //         let (path, value) = (path.to_vec(), value.to_vec());
    //         let path = Nibbles::<0>::new(&path);
    //         let path = Path(path.into_iter().collect());

    //         let children = Default::default();
    //         let value = if value.is_empty() { None } else { Some(value) };
    //         let mut children_encoded = <[Option<Vec<u8>>; BranchNode::MAX_CHILDREN]>::default();

    //         if let Some(child) = encoded_child {
    //             children_encoded[0] = Some(child);
    //         }

    //         Node::from_branch(BranchNode {
    //             partial_path: path,
    //             children,
    //             value,
    //             children_encoded,
    //         })
    //     }

    //     fn branch_without_value(path: &[u8], encoded_child: Option<Vec<u8>>) -> Node {
    //         let path = path.to_vec();
    //         let path = Nibbles::<0>::new(&path);
    //         let path = Path(path.into_iter().collect());

    //         let children = Default::default();
    //         // TODO: Properly test empty value
    //         let value = None;
    //         let mut children_encoded = <[Option<Vec<u8>>; BranchNode::MAX_CHILDREN]>::default();

    //         if let Some(child) = encoded_child {
    //             children_encoded[0] = Some(child);
    //         }

    //         Node::from_branch(BranchNode {
    //             partial_path: path,
    //             children,
    //             value,
    //             children_encoded,
    //         })
    //     }

    //     #[test_case(leaf(Vec::new(), Vec::new()) ; "empty leaf encoding")]
    //     #[test_case(leaf(vec![1, 2, 3], vec![4, 5]) ; "leaf encoding")]
    //     #[test_case(branch(b"", b"value", vec![1, 2, 3].into()) ; "branch with chd")]
    //     #[test_case(branch(b"", b"value", None); "branch without chd")]
    //     #[test_case(branch_without_value(b"", None); "branch without value and chd")]
    //     #[test_case(branch(b"", b"", None); "branch without path value or children")]
    //     #[test_case(branch(b"", b"value", None) ; "branch with value")]
    //     #[test_case(branch(&[2], b"", None); "branch with path")]
    //     #[test_case(branch(b"", b"", vec![1, 2, 3].into()); "branch with children")]
    //     #[test_case(branch(&[2], b"value", None); "branch with path and value")]
    //     #[test_case(branch(b"", b"value", vec![1, 2, 3].into()); "branch with value and children")]
    //     #[test_case(branch(&[2], b"", vec![1, 2, 3].into()); "branch with path and children")]
    //     #[test_case(branch(&[2], b"value", vec![1, 2, 3].into()); "branch with path value and children")]
    //     fn encode(node: Node) {
    //         let merkle = create_in_memory_merkle();

    //         let node_ref = merkle.put_node(node).unwrap();
    //         let encoded = node_ref.get_encoded(&merkle.store);
    //         let new_node = Node::from(NodeType::decode(encoded).unwrap());
    //         let new_node_encoded = new_node.get_encoded(&merkle.store);

    //         assert_eq!(encoded, new_node_encoded);
    //     }

    //     #[test_case(Bincode::new(), leaf(vec![], vec![4, 5]) ; "leaf without partial path encoding with Bincode")]
    //     #[test_case(Bincode::new(), leaf(vec![1, 2, 3], vec![4, 5]) ; "leaf with partial path encoding with Bincode")]
    //     #[test_case(Bincode::new(), branch(b"abcd", b"value", vec![1, 2, 3].into()) ; "branch with partial path and value with Bincode")]
    //     #[test_case(Bincode::new(), branch(b"abcd", &[], vec![1, 2, 3].into()) ; "branch with partial path and no value with Bincode")]
    //     #[test_case(Bincode::new(), branch(b"", &[1,3,3,7], vec![1, 2, 3].into()) ; "branch with no partial path and value with Bincode")]
    //     #[test_case(PlainCodec::new(), leaf(Vec::new(), vec![4, 5]) ; "leaf without partial path encoding with PlainCodec")]
    //     #[test_case(PlainCodec::new(), leaf(vec![1, 2, 3], vec![4, 5]) ; "leaf with partial path encoding with PlainCodec")]
    //     #[test_case(PlainCodec::new(), branch(b"abcd", b"value", vec![1, 2, 3].into()) ; "branch with partial path and value with PlainCodec")]
    //     #[test_case(PlainCodec::new(), branch(b"abcd", &[], vec![1, 2, 3].into()) ; "branch with partial path and no value with PlainCodec")]
    //     #[test_case(PlainCodec::new(), branch(b"", &[1,3,3,7], vec![1, 2, 3].into()) ; "branch with no partial path and value with PlainCodec")]
    //     fn node_encode_decode<T>(_codec: T, node: Node)
    //     where
    //         T: BinarySerde,
    //         for<'de> EncodedNode<T>: serde::Serialize + serde::Deserialize<'de>,
    //     {
    //         let merkle = create_generic_test_merkle::<T>();
    //         let node_ref = merkle.put_node(node.clone()).unwrap();

    //         let encoded = merkle.encode(node_ref.inner()).unwrap();
    //         let new_node = Node::from(merkle.decode(encoded.as_ref()).unwrap());

    //         assert_eq!(node, new_node);
    //     }

    #[test]
    fn test_insert_and_get() {
        let mut merkle = create_in_memory_merkle();

        // insert values
        for key_val in u8::MIN..=u8::MAX {
            let key = vec![key_val];
            let val = Box::new([key_val]);

            merkle.insert(&key, val.clone()).unwrap();

            let fetched_val = merkle.get(&key).unwrap();

            // make sure the value was inserted
            assert_eq!(fetched_val.as_deref(), val.as_slice().into());
        }

        // make sure none of the previous values were forgotten after initial insert
        for key_val in u8::MIN..=u8::MAX {
            let key = vec![key_val];
            let val = vec![key_val];

            let fetched_val = merkle.get(&key).unwrap();

            assert_eq!(fetched_val.as_deref(), val.as_slice().into());
        }
    }

    #[test]
    fn remove_root() {
        let key0 = vec![0];
        let val0 = [0];
        let key1 = vec![0, 1];
        let val1 = [0, 1];
        let key2 = vec![0, 1, 2];
        let val2 = [0, 1, 2];
        let key3 = vec![0, 1, 15];
        let val3 = [0, 1, 15];

        let mut merkle = create_in_memory_merkle();

        merkle.insert(&key0, Box::from(val0)).unwrap();
        merkle.insert(&key1, Box::from(val1)).unwrap();
        merkle.insert(&key2, Box::from(val2)).unwrap();
        merkle.insert(&key3, Box::from(val3)).unwrap();
        // Trie is:
        //   key0
        //    |
        //   key1
        //  /    \
        // key2  key3

        // Test removal of root when it's a branch with 1 branch child
        let removed_val = merkle.remove(&key0).unwrap();
        assert_eq!(removed_val, Some(Box::from(val0)));
        assert!(merkle.get(&key0).unwrap().is_none());
        // Removing an already removed key is a no-op
        assert!(merkle.remove(&key0).unwrap().is_none());

        // Trie is:
        //   key1
        //  /    \
        // key2  key3
        // Test removal of root when it's a branch with multiple children
        assert_eq!(merkle.remove(&key1).unwrap(), Some(Box::from(val1)));
        assert!(merkle.get(&key1).unwrap().is_none());
        assert!(merkle.remove(&key1).unwrap().is_none());

        // Trie is:
        //   key1 (now has no value)
        //  /    \
        // key2  key3
        let removed_val = merkle.remove(&key2).unwrap();
        assert_eq!(removed_val, Some(Box::from(val2)));
        assert!(merkle.get(&key2).unwrap().is_none());
        assert!(merkle.remove(&key2).unwrap().is_none());

        // Trie is:
        // key1 (now has no value)
        //  |
        // key3
        let removed_val = merkle.remove(&key3).unwrap();
        assert_eq!(removed_val, Some(Box::from(val3)));
        assert!(merkle.get(&key3).unwrap().is_none());
        assert!(merkle.remove(&key3).unwrap().is_none());

        assert!(merkle.root_address().is_none());
    }

    #[test]
    fn remove_many() {
        let mut merkle = create_in_memory_merkle();

        // insert key-value pairs
        for key_val in u8::MIN..=u8::MAX {
            let key = [key_val];
            let val = [key_val];

            merkle.insert(&key, Box::new(val)).unwrap();
            let got = merkle.get(&key).unwrap().unwrap();
            assert_eq!(&*got, val);
        }

        // remove key-value pairs
        for key_val in u8::MIN..=u8::MAX {
            let key = [key_val];
            let val = [key_val];

            let got = merkle.remove(&key).unwrap().unwrap();
            assert_eq!(&*got, val);

            // Removing an already removed key is a no-op
            assert!(merkle.remove(&key).unwrap().is_none());

            let got = merkle.get(&key).unwrap();
            assert!(got.is_none());
        }
        assert!(merkle.root_address().is_none());
    }

    //     #[test]
    //     fn get_empty_proof() {
    //         let merkle = create_in_memory_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         let proof = merkle.prove(b"any-key", root_addr).unwrap();

    //         assert!(proof.0.is_empty());
    //     }

    //     #[tokio::test]
    //     async fn empty_range_proof() {
    //         let merkle = create_in_memory_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         assert!(merkle
    //             .range_proof::<&[u8]>(root_addr, None, None, None)
    //             .await
    //             .unwrap()
    //             .is_none());
    //     }

    //     #[tokio::test]
    //     async fn range_proof_invalid_bounds() {
    //         let merkle = create_in_memory_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();
    //         let start_key = &[0x01];
    //         let end_key = &[0x00];

    //         match merkle
    //             .range_proof::<&[u8]>(root_addr, Some(start_key), Some(end_key), Some(1))
    //             .await
    //         {
    //             Err(api::Error::InvalidRange {
    //                 first_key,
    //                 last_key,
    //             }) if first_key == start_key && last_key == end_key => (),
    //             Err(api::Error::InvalidRange { .. }) => panic!("wrong bounds on InvalidRange error"),
    //             _ => panic!("expected InvalidRange error"),
    //         }
    //     }

    //     #[tokio::test]
    //     async fn full_range_proof() {
    //         let mut merkle = create_in_memory_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();
    //         // insert values
    //         for key_val in u8::MIN..=u8::MAX {
    //             let key = &[key_val];
    //             let val = &[key_val];

    //             merkle.insert(key, val.to_vec(), root_addr).unwrap();
    //         }
    //         merkle.flush_dirty();

    //         let rangeproof = merkle
    //             .range_proof::<&[u8]>(root_addr, None, None, None)
    //             .await
    //             .unwrap()
    //             .unwrap();
    //         assert_eq!(rangeproof.middle.len(), u8::MAX as usize + 1);
    //         assert_ne!(rangeproof.first_key_proof.0, rangeproof.last_key_proof.0);
    //         let left_proof = merkle.prove([u8::MIN], root_addr).unwrap();
    //         let right_proof = merkle.prove([u8::MAX], root_addr).unwrap();
    //         assert_eq!(rangeproof.first_key_proof.0, left_proof.0);
    //         assert_eq!(rangeproof.last_key_proof.0, right_proof.0);
    //     }

    //     #[tokio::test]
    //     async fn single_value_range_proof() {
    //         const RANDOM_KEY: u8 = 42;

    //         let mut merkle = create_in_memory_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();
    //         // insert values
    //         for key_val in u8::MIN..=u8::MAX {
    //             let key = &[key_val];
    //             let val = &[key_val];

    //             merkle.insert(key, val.to_vec(), root_addr).unwrap();
    //         }
    //         merkle.flush_dirty();

    //         let rangeproof = merkle
    //             .range_proof(root_addr, Some([RANDOM_KEY]), None, Some(1))
    //             .await
    //             .unwrap()
    //             .unwrap();
    //         assert_eq!(rangeproof.first_key_proof.0, rangeproof.last_key_proof.0);
    //         assert_eq!(rangeproof.middle.len(), 1);
    //     }

    //     #[test]
    //     fn shared_path_proof() {
    //         let mut merkle = create_in_memory_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         let key1 = b"key1";
    //         let value1 = b"1";
    //         merkle.insert(key1, value1.to_vec(), root_addr).unwrap();

    //         let key2 = b"key2";
    //         let value2 = b"2";
    //         merkle.insert(key2, value2.to_vec(), root_addr).unwrap();

    //         let root_hash = merkle.root_hash(root_addr).unwrap();

    //         let verified = {
    //             let key = key1;
    //             let proof = merkle.prove(key, root_addr).unwrap();
    //             proof.verify(key, root_hash.0).unwrap()
    //         };

    //         assert_eq!(verified, Some(value1.to_vec()));

    //         let verified = {
    //             let key = key2;
    //             let proof = merkle.prove(key, root_addr).unwrap();
    //             proof.verify(key, root_hash.0).unwrap()
    //         };

    //         assert_eq!(verified, Some(value2.to_vec()));
    //     }

    //     // this was a specific failing case
    //     #[test]
    //     fn shared_path_on_insert() {
    //         type Bytes = &'static [u8];
    //         let pairs: Vec<(Bytes, Bytes)> = vec![
    //             (
    //                 &[1, 1, 46, 82, 67, 218],
    //                 &[23, 252, 128, 144, 235, 202, 124, 243],
    //             ),
    //             (
    //                 &[1, 0, 0, 1, 1, 0, 63, 80],
    //                 &[99, 82, 31, 213, 180, 196, 49, 242],
    //             ),
    //             (
    //                 &[0, 0, 0, 169, 176, 15],
    //                 &[105, 211, 176, 51, 231, 182, 74, 207],
    //             ),
    //             (
    //                 &[1, 0, 0, 0, 53, 57, 93],
    //                 &[234, 139, 214, 220, 172, 38, 168, 164],
    //             ),
    //         ];

    //         let mut merkle = create_in_memory_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         for (key, val) in &pairs {
    //             let val = val.to_vec();
    //             merkle.insert(key, val.clone(), root_addr).unwrap();

    //             let fetched_val = merkle.get(key, root_addr).unwrap();

    //             // make sure the value was inserted
    //             assert_eq!(fetched_val.as_deref(), val.as_slice().into());
    //         }

    //         for (key, val) in pairs {
    //             let fetched_val = merkle.get(key, root_addr).unwrap();

    //             // make sure the value was inserted
    //             assert_eq!(fetched_val.as_deref(), val.into());
    //         }
    //     }

    //     #[test]
    //     fn overwrite_leaf() {
    //         let key = vec![0x00];
    //         let val = vec![1];
    //         let overwrite = vec![2];

    //         let mut merkle = create_in_memory_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         merkle.insert(&key, val.clone(), root_addr).unwrap();

    //         assert_eq!(
    //             merkle.get(&key, root_addr).unwrap().as_deref(),
    //             Some(val.as_slice())
    //         );

    //         merkle
    //             .insert(&key, overwrite.clone(), root_addr)
    //             .unwrap();

    //         assert_eq!(
    //             merkle.get(&key, root_addr).unwrap().as_deref(),
    //             Some(overwrite.as_slice())
    //         );
    //     }

    #[test]
    fn test_insert_leaf_suffix() {
        // key_2 is a suffix of key, which is a leaf
        let key = vec![0xff];
        let val = [1];
        let key_2 = vec![0xff, 0x00];
        let val_2 = [2];

        let mut merkle = create_in_memory_merkle();

        merkle.insert(&key, Box::new(val)).unwrap();
        merkle.insert(&key_2, Box::new(val_2)).unwrap();

        let got = merkle.get(&key).unwrap().unwrap();

        assert_eq!(*got, val);

        let got = merkle.get(&key_2).unwrap().unwrap();
        assert_eq!(*got, val_2);
    }

    #[test]
    fn test_insert_leaf_prefix() {
        // key_2 is a prefix of key, which is a leaf
        let key = vec![0xff, 0x00];
        let val = [1];
        let key_2 = vec![0xff];
        let val_2 = [2];

        let mut merkle = create_in_memory_merkle();

        merkle.insert(&key, Box::new(val)).unwrap();
        merkle.insert(&key_2, Box::new(val_2)).unwrap();

        let got = merkle.get(&key).unwrap().unwrap();
        assert_eq!(*got, val);

        let got = merkle.get(&key_2).unwrap().unwrap();
        assert_eq!(*got, val_2);
    }

    #[test]
    fn test_insert_sibling_leaf() {
        // The node at key is a branch node with children key_2 and key_3.
        // TODO assert in this test that key is the parent of key_2 and key_3.
        // i.e. the node types are branch, leaf, leaf respectively.
        let key = vec![0xff];
        let val = [1];
        let key_2 = vec![0xff, 0x00];
        let val_2 = [2];
        let key_3 = vec![0xff, 0x0f];
        let val_3 = [3];

        let mut merkle = create_in_memory_merkle();

        merkle.insert(&key, Box::new(val)).unwrap();
        merkle.insert(&key_2, Box::new(val_2)).unwrap();
        merkle.insert(&key_3, Box::new(val_3)).unwrap();

        let got = merkle.get(&key).unwrap().unwrap();
        assert_eq!(*got, val);

        let got = merkle.get(&key_2).unwrap().unwrap();
        assert_eq!(*got, val_2);

        let got = merkle.get(&key_3).unwrap().unwrap();
        assert_eq!(*got, val_3);
    }

    #[test]
    fn test_insert_branch_as_branch_parent() {
        let key = vec![0xff, 0xf0];
        let val = [1];
        let key_2 = vec![0xff, 0xf0, 0x00];
        let val_2 = [2];
        let key_3 = vec![0xff];
        let val_3 = [3];

        let mut merkle = create_in_memory_merkle();

        merkle.insert(&key, Box::new(val)).unwrap();
        // key is a leaf

        merkle.insert(&key_2, Box::new(val_2)).unwrap();
        // key is branch with child key_2

        merkle.insert(&key_3, Box::new(val_3)).unwrap();
        // key_3 is a branch with child key
        // key is a branch with child key_3

        let got = merkle.get(&key).unwrap().unwrap();
        assert_eq!(&*got, val);

        let got = merkle.get(&key_2).unwrap().unwrap();
        assert_eq!(&*got, val_2);

        let got = merkle.get(&key_3).unwrap().unwrap();
        assert_eq!(&*got, val_3);
    }

    #[test]
    fn test_insert_overwrite_branch_value() {
        let key = vec![0xff];
        let val = [1];
        let key_2 = vec![0xff, 0x00];
        let val_2 = [2];
        let overwrite = [3];

        let mut merkle = create_in_memory_merkle();

        merkle.insert(&key, Box::new(val)).unwrap();
        merkle.insert(&key_2, Box::new(val_2)).unwrap();

        let got = merkle.get(&key).unwrap().unwrap();
        assert_eq!(*got, val);

        let got = merkle.get(&key_2).unwrap().unwrap();
        assert_eq!(*got, val_2);

        merkle.insert(&key, Box::new(overwrite)).unwrap();

        let got = merkle.get(&key).unwrap().unwrap();
        assert_eq!(*got, overwrite);

        let got = merkle.get(&key_2).unwrap().unwrap();
        assert_eq!(*got, val_2);
    }

    //     #[test]
    //     fn single_key_proof_with_one_node() {
    //         let mut merkle = create_in_memory_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();
    //         let key = b"key";
    //         let value = b"value";

    //         merkle.insert(key, value.to_vec(), root_addr).unwrap();
    //         let root_hash = merkle.root_hash(root_addr).unwrap();

    //         let proof = merkle.prove(key, root_addr).unwrap();

    //         let verified = proof.verify(key, root_hash.0).unwrap();

    //         assert_eq!(verified, Some(value.to_vec()));
    //     }

    //     #[test]
    //     fn two_key_proof_without_shared_path() {
    //         let mut merkle = create_in_memory_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         let key1 = &[0x00];
    //         let key2 = &[0xff];

    //         merkle.insert(key1, key1.to_vec(), root_addr).unwrap();
    //         merkle.insert(key2, key2.to_vec(), root_addr).unwrap();

    //         let root_hash = merkle.root_hash(root_addr).unwrap();

    //         let verified = {
    //             let proof = merkle.prove(key1, root_addr).unwrap();
    //             proof.verify(key1, root_hash.0).unwrap()
    //         };

    //         assert_eq!(verified.as_deref(), Some(key1.as_slice()));
    //     }

    fn merkle_build_test<K: AsRef<[u8]>, V: AsRef<[u8]>>(
        items: Vec<(K, V)>,
    ) -> Result<Merkle<MemStore>, MerkleError> {
        let mut merkle = Merkle::new(HashedNodeStore::initialize(MemStore::new(vec![])).unwrap());
        for (k, v) in items.iter() {
            merkle.insert(k.as_ref(), Box::from(v.as_ref()))?;
            println!("{}", merkle.dump()?);
        }

        Ok(merkle)
    }

    #[test]
    fn test_root_hash_simple_insertions() -> Result<(), MerkleError> {
        let items = vec![
            ("do", "verb"),
            ("doe", "reindeer"),
            ("dog", "puppy"),
            ("doge", "coin"),
            ("horse", "stallion"),
            ("ddd", "ok"),
        ];
        let merkle = merkle_build_test(items)?;

        merkle.dump().unwrap();
        Ok(())
    }

    #[test_case(vec![], "0000000000000000000000000000000000000000000000000000000000000000"; "empty trie")]
    #[test_case(vec![(&[0],&[0])], "073615413d814b23383fc2c8d8af13abfffcb371b654b98dbf47dd74b1e4d1b9"; "root")]
    #[test_case(vec![(&[0,1],&[0,1])], "28e67ae4054c8cdf3506567aa43f122224fe65ef1ab3e7b7899f75448a69a6fd"; "root with partial path")]
    #[test_case(vec![(&[0],&[1;32])], "ba0283637f46fa807280b7d08013710af08dfdc236b9b22f9d66e60592d6c8a3"; "leaf value >= 32 bytes")]
    #[test_case(vec![(&[0],&[0]),(&[0,1],&[1;32])], "3edbf1fdd345db01e47655bcd0a9a456857c4093188cf35c5c89b8b0fb3de17e"; "branch value >= 32 bytes")]
    #[test_case(vec![(&[0],&[0]),(&[0,1],&[0,1])], "c3bdc20aff5cba30f81ffd7689e94e1dbeece4a08e27f0104262431604cf45c6"; "root with leaf child")]
    #[test_case(vec![(&[0],&[0]),(&[0,1],&[0,1]),(&[0,1,2],&[0,1,2])], "229011c50ad4d5c2f4efe02b8db54f361ad295c4eee2bf76ea4ad1bb92676f97"; "root with branch child")]
    #[test_case(vec![(&[0],&[0]),(&[0,1],&[0,1]),(&[0,8],&[0,8]),(&[0,1,2],&[0,1,2])], "a683b4881cb540b969f885f538ba5904699d480152f350659475a962d6240ef9"; "root with branch child and leaf child")]
    fn test_root_hash_merkledb_compatible(kvs: Vec<(&[u8], &[u8])>, expected_hash: &str) {
        let mut merkle = merkle_build_test(kvs).unwrap().freeze().unwrap();

        // This hash is from merkledb
        let expected_hash: [u8; 32] = hex::decode(expected_hash).unwrap().try_into().unwrap();

        let actual_hash = merkle.root_hash().unwrap();
        assert_eq!(actual_hash, TrieHash::from(expected_hash));
    }

    #[test]
    fn test_root_hash_fuzz_insertions() -> Result<(), MerkleError> {
        use rand::rngs::StdRng;
        use rand::{Rng, SeedableRng};
        let rng = std::cell::RefCell::new(StdRng::seed_from_u64(42));
        let max_len0 = 8;
        let max_len1 = 4;
        let keygen = || {
            let (len0, len1): (usize, usize) = {
                let mut rng = rng.borrow_mut();
                (
                    rng.gen_range(1..max_len0 + 1),
                    rng.gen_range(1..max_len1 + 1),
                )
            };
            let key: Vec<u8> = (0..len0)
                .map(|_| rng.borrow_mut().gen_range(0..2))
                .chain((0..len1).map(|_| rng.borrow_mut().gen()))
                .collect();
            key
        };

        for _ in 0..10 {
            let mut items = Vec::new();

            for _ in 0..10 {
                let val: Vec<u8> = (0..8).map(|_| rng.borrow_mut().gen()).collect();
                items.push((keygen(), val));
            }

            merkle_build_test(items)?;
        }

        Ok(())
    }

    // #[test]
    // #[allow(clippy::unwrap_used)]
    // fn test_root_hash_reversed_deletions() -> Result<(), MerkleError> {
    //     use rand::rngs::StdRng;
    //     use rand::{Rng, SeedableRng};
    //     let rng = std::cell::RefCell::new(StdRng::seed_from_u64(42));
    //     let max_len0 = 8;
    //     let max_len1 = 4;
    //     let keygen = || {
    //         let (len0, len1): (usize, usize) = {
    //             let mut rng = rng.borrow_mut();
    //             (
    //                 rng.gen_range(1..max_len0 + 1),
    //                 rng.gen_range(1..max_len1 + 1),
    //             )
    //         };
    //         let key: Vec<u8> = (0..len0)
    //             .map(|_| rng.borrow_mut().gen_range(0..2))
    //             .chain((0..len1).map(|_| rng.borrow_mut().gen()))
    //             .collect();
    //         key
    //     };

    //     for _ in 0..10 {
    //         let mut items: Vec<_> = (0..10)
    //             .map(|_| keygen())
    //             .map(|key| {
    //                 let val: Box<[u8]> = (0..8).map(|_| rng.borrow_mut().gen()).collect();
    //                 (key, val)
    //             })
    //             .collect();

    //         items.sort();

    //         let mut merkle =
    //             Merkle::new(HashedNodeStore::initialize(MemStore::new(vec![])).unwrap());

    //         let mut hashes = Vec::new();

    //         for (k, v) in items.iter() {
    //             hashes.push((merkle.root_hash()?, merkle.dump()?));
    //             merkle.insert(k, v.clone())?;
    //         }

    //         let mut new_hashes = Vec::new();

    //         for (k, _) in items.iter().rev() {
    //             let before = merkle.dump()?;
    //             merkle.remove(k)?;
    //             new_hashes.push((merkle.root_hash()?, k, before, merkle.dump()?));
    //         }

    //         hashes.reverse();

    //         for i in 0..hashes.len() {
    //             #[allow(clippy::indexing_slicing)]
    //             let (new_hash, key, before_removal, after_removal) = &new_hashes[i];
    //             #[allow(clippy::indexing_slicing)]
    //             let expected_hash = &hashes[i];
    //             let key = key.iter().fold(String::new(), |mut s, b| {
    //                 let _ = write!(s, "{:02x}", b);
    //                 s
    //             });
    //             // assert_eq!(new_hash, expected_hash, "\n\nkey: {key}\nbefore:\n{before_removal}\nafter:\n{after_removal}\n\nexpected:\n{expected_dump}\n");
    //         }
    //     }

    //     Ok(())
    // }

    // #[test]
    // #[allow(clippy::unwrap_used)]
    // fn test_root_hash_random_deletions() -> Result<(), MerkleError> {
    //     use rand::rngs::StdRng;
    //     use rand::seq::SliceRandom;
    //     use rand::{Rng, SeedableRng};
    //     let rng = std::cell::RefCell::new(StdRng::seed_from_u64(42));
    //     let max_len0 = 8;
    //     let max_len1 = 4;
    //     let keygen = || {
    //         let (len0, len1): (usize, usize) = {
    //             let mut rng = rng.borrow_mut();
    //             (
    //                 rng.gen_range(1..max_len0 + 1),
    //                 rng.gen_range(1..max_len1 + 1),
    //             )
    //         };
    //         let key: Vec<u8> = (0..len0)
    //             .map(|_| rng.borrow_mut().gen_range(0..2))
    //             .chain((0..len1).map(|_| rng.borrow_mut().gen()))
    //             .collect();
    //         key
    //     };

    //     for i in 0..10 {
    //         let mut items = std::collections::HashMap::new();

    //         for _ in 0..10 {
    //             let val: Box<[u8]> = (0..8).map(|_| rng.borrow_mut().gen()).collect();
    //             items.insert(keygen(), val);
    //         }

    //         let mut items_ordered: Vec<_> =
    //             items.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    //         items_ordered.sort();
    //         items_ordered.shuffle(&mut *rng.borrow_mut());
    //         let mut merkle =
    //             Merkle::new(HashedNodeStore::initialize(MemStore::new(vec![])).unwrap());

    //         for (k, v) in items.iter() {
    //             merkle.insert(k, v.clone())?;
    //         }

    //         for (k, v) in items.iter() {
    //             assert_eq!(&*merkle.get(k)?.unwrap(), &v[..]);
    //         }

    //         for (k, _) in items_ordered.into_iter() {
    //             assert!(merkle.get(&k)?.is_some());

    //             merkle.remove(&k)?;

    //             assert!(merkle.get(&k)?.is_none());

    //             items.remove(&k);

    //             for (k, v) in items.iter() {
    //                 assert_eq!(&*merkle.get(k)?.unwrap(), &v[..]);
    //             }

    //             let h = triehash::trie_root::<keccak_hasher::KeccakHasher, Vec<_>, _, _>(
    //                 items.iter().collect(),
    //             );

    //             let h0 = merkle.root_hash()?;

    //             if TrieHash::from(h) != h0 {
    //                 println!("{} != {}", hex::encode(h), hex::encode(*h0));
    //             }
    //         }

    //         println!("i = {i}");
    //     }
    //     Ok(())
    // }

    // #[test]
    // #[allow(clippy::unwrap_used, clippy::indexing_slicing)]
    // fn test_proof() -> Result<(), MerkleError> {
    //     let set = fixed_and_pseudorandom_data(500);
    //     let mut items = Vec::from_iter(set.iter());
    //     items.sort();
    //     let merkle = merkle_build_test(items.clone())?;
    //     let (keys, vals): (Vec<[u8; 32]>, Vec<[u8; 20]>) = items.into_iter().unzip();

    //     for (i, key) in keys.iter().enumerate() {
    //         let proof = merkle.prove(key)?;
    //         assert!(!proof.0.is_empty());
    //         let val = merkle.verify_proof(key, &proof)?;
    //         assert!(val.is_some());
    //         assert_eq!(val.unwrap(), vals[i]);
    //     }

    //     Ok(())
    // }

    // #[test]
    // /// Verify the proofs that end with leaf node with the given key.
    // fn test_proof_end_with_leaf() -> Result<(), MerkleError> {
    //     let items = vec![
    //         ("do", "verb"),
    //         ("doe", "reindeer"),
    //         ("dog", "puppy"),
    //         ("doge", "coin"),
    //         ("horse", "stallion"),
    //         ("ddd", "ok"),
    //     ];
    //     let merkle = merkle_build_test(items)?;
    //     let key = "doe".as_ref();

    //     let proof = merkle.prove(key)?;
    //     assert!(!proof.0.is_empty());

    //     let verify_proof = merkle.verify_proof(key, &proof)?;
    //     assert!(verify_proof.is_some());

    //     Ok(())
    // }

    // #[test]
    // /// Verify the proofs that end with branch node with the given key.
    // fn test_proof_end_with_branch() -> Result<(), MerkleError> {
    //     let items = vec![
    //         ("d", "verb"),
    //         ("do", "verb"),
    //         ("doe", "reindeer"),
    //         ("e", "coin"),
    //     ];
    //     let merkle = merkle_build_test(items)?;
    //     let key = "d".as_ref();

    //     let proof = merkle.prove(key)?;
    //     assert!(!proof.0.is_empty());

    //     let verify_proof = merkle.verify_proof(key, &proof)?;
    //     assert!(verify_proof.is_some());

    //     Ok(())
    // }

    // #[test]
    // fn test_bad_proof() -> Result<(), MerkleError> {
    //     let set = fixed_and_pseudorandom_data(800);
    //     let mut items = Vec::from_iter(set.iter());
    //     items.sort();
    //     let merkle = merkle_build_test(items.clone())?;
    //     let (keys, _): (Vec<[u8; 32]>, Vec<[u8; 20]>) = items.into_iter().unzip();

    //     for key in keys.iter() {
    //         let mut proof = merkle.prove(key)?;
    //         assert!(!proof.0.is_empty());

    //         // Delete an entry from the generated proofs.
    //         let len = proof.0.len();
    //         let new_proof = Proof(proof.0.drain().take(len - 1).collect());
    //         assert!(merkle.verify_proof(key, &new_proof).is_err());
    //     }

    //     Ok(())
    // }

    // #[test]
    // // Tests that missing keys can also be proven. The test explicitly uses a single
    // // entry trie and checks for missing keys both before and after the single entry.
    // fn test_missing_key_proof() -> Result<(), MerkleError> {
    //     let items = vec![("k", "v")];
    //     let merkle = merkle_build_test(items)?;
    //     for key in ["a", "j", "l", "z"] {
    //         let proof = merkle.prove(key.as_ref())?;
    //         assert!(!proof.0.is_empty());
    //         assert!(proof.0.len() == 1);

    //         let val = merkle.verify_proof(key.as_ref(), &proof)?;
    //         assert!(val.is_none());
    //     }

    //     Ok(())
    // }

    // #[test]
    // fn test_empty_tree_proof() -> Result<(), MerkleError> {
    //     let items: Vec<(&str, &str)> = Vec::new();
    //     let merkle = merkle_build_test(items)?;
    //     let key = "x".as_ref();

    //     let proof = merkle.prove(key)?;
    //     assert!(proof.0.is_empty());

    //     Ok(())
    // }

    // #[test]
    // // Tests normal range proof with both edge proofs as the existent proof.
    // // The test cases are generated randomly.
    // #[allow(clippy::indexing_slicing)]
    // fn test_range_proof() -> Result<(), ProofError> {
    //     let set = fixed_and_pseudorandom_data(4096);
    //     let mut items = Vec::from_iter(set.iter());
    //     items.sort();
    //     let merkle = merkle_build_test(items.clone())?;

    //     for _ in 0..10 {
    //         let start = rand::thread_rng().gen_range(0..items.len());
    //         let end = rand::thread_rng().gen_range(0..items.len() - start) + start - 1;

    //         if end <= start {
    //             continue;
    //         }

    //         let mut proof = merkle.prove(items[start].0)?;
    //         assert!(!proof.0.is_empty());
    //         let end_proof = merkle.prove(items[end - 1].0)?;
    //         assert!(!end_proof.0.is_empty());
    //         proof.extend(end_proof);

    //         let mut keys = Vec::new();
    //         let mut vals = Vec::new();
    //         for item in items[start..end].iter() {
    //             keys.push(item.0.as_ref());
    //             vals.push(&item.1);
    //         }

    //         merkle.verify_range_proof(&proof, items[start].0, items[end - 1].0, keys, vals)?;
    //     }
    //     Ok(())
    // }

    // #[test]
    // // Tests a few cases which the proof is wrong.
    // // The prover is expected to detect the error.
    // #[allow(clippy::indexing_slicing)]
    // fn test_bad_range_proof() -> Result<(), ProofError> {
    //     let set = fixed_and_pseudorandom_data(4096);
    //     let mut items = Vec::from_iter(set.iter());
    //     items.sort();
    //     let merkle = merkle_build_test(items.clone())?;

    //     for _ in 0..10 {
    //         let start = rand::thread_rng().gen_range(0..items.len());
    //         let end = rand::thread_rng().gen_range(0..items.len() - start) + start - 1;

    //         if end <= start {
    //             continue;
    //         }

    //         let mut proof = merkle.prove(items[start].0)?;
    //         assert!(!proof.0.is_empty());
    //         let end_proof = merkle.prove(items[end - 1].0)?;
    //         assert!(!end_proof.0.is_empty());
    //         proof.extend(end_proof);

    //         let mut keys: Vec<[u8; 32]> = Vec::new();
    //         let mut vals: Vec<[u8; 20]> = Vec::new();
    //         for item in items[start..end].iter() {
    //             keys.push(*item.0);
    //             vals.push(*item.1);
    //         }

    //         let test_case: u32 = rand::thread_rng().gen_range(0..6);
    //         let index = rand::thread_rng().gen_range(0..end - start);
    //         match test_case {
    //             0 => {
    //                 // Modified key
    //                 keys[index] = rand::thread_rng().gen::<[u8; 32]>(); // In theory it can't be same
    //             }
    //             1 => {
    //                 // Modified val
    //                 vals[index] = rand::thread_rng().gen::<[u8; 20]>(); // In theory it can't be same
    //             }
    //             2 => {
    //                 // Gapped entry slice
    //                 if index == 0 || index == end - start - 1 {
    //                     continue;
    //                 }
    //                 keys.remove(index);
    //                 vals.remove(index);
    //             }
    //             3 => {
    //                 // Out of order
    //                 let index_1 = rand::thread_rng().gen_range(0..end - start);
    //                 let index_2 = rand::thread_rng().gen_range(0..end - start);
    //                 if index_1 == index_2 {
    //                     continue;
    //                 }
    //                 keys.swap(index_1, index_2);
    //                 vals.swap(index_1, index_2);
    //             }
    //             4 => {
    //                 // Set random key to empty, do nothing
    //                 keys[index] = [0; 32];
    //             }
    //             5 => {
    //                 // Set random value to nil
    //                 vals[index] = [0; 20];
    //             }
    //             _ => unreachable!(),
    //         }

    //         let keys_slice = keys.iter().map(|k| k.as_ref()).collect::<Vec<&[u8]>>();
    //         assert!(merkle
    //             .verify_range_proof(&proof, items[start].0, items[end - 1].0, keys_slice, vals)
    //             .is_err());
    //     }

    //     Ok(())
    // }

    // #[test]
    // // Tests normal range proof with two non-existent proofs.
    // // The test cases are generated randomly.
    // #[allow(clippy::indexing_slicing)]
    // fn test_range_proof_with_non_existent_proof() -> Result<(), ProofError> {
    //     let set = fixed_and_pseudorandom_data(4096);
    //     let mut items = Vec::from_iter(set.iter());
    //     items.sort();
    //     let merkle = merkle_build_test(items.clone())?;

    //     for _ in 0..10 {
    //         let start = rand::thread_rng().gen_range(0..items.len());
    //         let end = rand::thread_rng().gen_range(0..items.len() - start) + start - 1;

    //         if end <= start {
    //             continue;
    //         }

    //         // Short circuit if the decreased key is same with the previous key
    //         let first = decrease_key(items[start].0);
    //         if start != 0 && first.as_ref() == items[start - 1].0.as_ref() {
    //             continue;
    //         }
    //         // Short circuit if the decreased key is underflow
    //         if &first > items[start].0 {
    //             continue;
    //         }
    //         // Short circuit if the increased key is same with the next key
    //         let last = increase_key(items[end - 1].0);
    //         if end != items.len() && last.as_ref() == items[end].0.as_ref() {
    //             continue;
    //         }
    //         // Short circuit if the increased key is overflow
    //         if &last < items[end - 1].0 {
    //             continue;
    //         }

    //         let mut proof = merkle.prove(&first)?;
    //         assert!(!proof.0.is_empty());
    //         let end_proof = merkle.prove(&last)?;
    //         assert!(!end_proof.0.is_empty());
    //         proof.extend(end_proof);

    //         let mut keys: Vec<&[u8]> = Vec::new();
    //         let mut vals: Vec<[u8; 20]> = Vec::new();
    //         for item in items[start..end].iter() {
    //             keys.push(item.0);
    //             vals.push(*item.1);
    //         }

    //         merkle.verify_range_proof(&proof, &first, &last, keys, vals)?;
    //     }

    //     // Special case, two edge proofs for two edge key.
    //     let first = &[0; 32];
    //     let last = &[255; 32];
    //     let mut proof = merkle.prove(first)?;
    //     assert!(!proof.0.is_empty());
    //     let end_proof = merkle.prove(last)?;
    //     assert!(!end_proof.0.is_empty());
    //     proof.extend(end_proof);

    //     let (keys, vals): (Vec<&[u8; 32]>, Vec<&[u8; 20]>) = items.into_iter().unzip();
    //     let keys = keys.iter().map(|k| k.as_ref()).collect::<Vec<&[u8]>>();
    //     merkle.verify_range_proof(&proof, first, last, keys, vals)?;

    //     Ok(())
    // }

    // #[test]
    // // Tests such scenarios:
    // // - There exists a gap between the first element and the left edge proof
    // // - There exists a gap between the last element and the right edge proof
    // #[allow(clippy::indexing_slicing)]
    // fn test_range_proof_with_invalid_non_existent_proof() -> Result<(), ProofError> {
    //     let set = fixed_and_pseudorandom_data(4096);
    //     let mut items = Vec::from_iter(set.iter());
    //     items.sort();
    //     let merkle = merkle_build_test(items.clone())?;

    //     // Case 1
    //     let mut start = 100;
    //     let mut end = 200;
    //     let first = decrease_key(items[start].0);

    //     let mut proof = merkle.prove(&first)?;
    //     assert!(!proof.0.is_empty());
    //     let end_proof = merkle.prove(items[end - 1].0)?;
    //     assert!(!end_proof.0.is_empty());
    //     proof.extend(end_proof);

    //     start = 105; // Gap created
    //     let mut keys: Vec<&[u8]> = Vec::new();
    //     let mut vals: Vec<[u8; 20]> = Vec::new();
    //     // Create gap
    //     for item in items[start..end].iter() {
    //         keys.push(item.0);
    //         vals.push(*item.1);
    //     }
    //     assert!(merkle
    //         .verify_range_proof(&proof, &first, items[end - 1].0, keys, vals)
    //         .is_err());

    //     // Case 2
    //     start = 100;
    //     end = 200;
    //     let last = increase_key(items[end - 1].0);

    //     let mut proof = merkle.prove(items[start].0)?;
    //     assert!(!proof.0.is_empty());
    //     let end_proof = merkle.prove(&last)?;
    //     assert!(!end_proof.0.is_empty());
    //     proof.extend(end_proof);

    //     end = 195; // Capped slice
    //     let mut keys: Vec<&[u8]> = Vec::new();
    //     let mut vals: Vec<[u8; 20]> = Vec::new();
    //     // Create gap
    //     for item in items[start..end].iter() {
    //         keys.push(item.0);
    //         vals.push(*item.1);
    //     }
    //     assert!(merkle
    //         .verify_range_proof(&proof, items[start].0, &last, keys, vals)
    //         .is_err());

    //     Ok(())
    // }

    // #[test]
    // // Tests the proof with only one element. The first edge proof can be existent one or
    // // non-existent one.
    // #[allow(clippy::indexing_slicing)]
    // fn test_one_element_range_proof() -> Result<(), ProofError> {
    //     let set = fixed_and_pseudorandom_data(4096);
    //     let mut items = Vec::from_iter(set.iter());
    //     items.sort();
    //     let merkle = merkle_build_test(items.clone())?;

    //     // One element with existent edge proof, both edge proofs
    //     // point to the SAME key.
    //     let start = 1000;
    //     let start_proof = merkle.prove(items[start].0)?;
    //     assert!(!start_proof.0.is_empty());

    //     merkle.verify_range_proof(
    //         &start_proof,
    //         items[start].0,
    //         items[start].0,
    //         vec![items[start].0],
    //         vec![&items[start].1],
    //     )?;

    //     // One element with left non-existent edge proof
    //     let first = decrease_key(items[start].0);
    //     let mut proof = merkle.prove(&first)?;
    //     assert!(!proof.0.is_empty());
    //     let end_proof = merkle.prove(items[start].0)?;
    //     assert!(!end_proof.0.is_empty());
    //     proof.extend(end_proof);

    //     merkle.verify_range_proof(
    //         &proof,
    //         &first,
    //         items[start].0,
    //         vec![items[start].0],
    //         vec![*items[start].1],
    //     )?;

    //     // One element with right non-existent edge proof
    //     let last = increase_key(items[start].0);
    //     let mut proof = merkle.prove(items[start].0)?;
    //     assert!(!proof.0.is_empty());
    //     let end_proof = merkle.prove(&last)?;
    //     assert!(!end_proof.0.is_empty());
    //     proof.extend(end_proof);

    //     merkle.verify_range_proof(
    //         &proof,
    //         items[start].0,
    //         &last,
    //         vec![items[start].0],
    //         vec![*items[start].1],
    //     )?;

    //     // One element with two non-existent edge proofs
    //     let mut proof = merkle.prove(&first)?;
    //     assert!(!proof.0.is_empty());
    //     let end_proof = merkle.prove(&last)?;
    //     assert!(!end_proof.0.is_empty());
    //     proof.extend(end_proof);

    //     merkle.verify_range_proof(
    //         &proof,
    //         &first,
    //         &last,
    //         vec![items[start].0],
    //         vec![*items[start].1],
    //     )?;

    //     // Test the mini trie with only a single element.
    //     let key = rand::thread_rng().gen::<[u8; 32]>();
    //     let val = rand::thread_rng().gen::<[u8; 20]>();
    //     let merkle = merkle_build_test(vec![(key, val)])?;

    //     let first = &[0; 32];
    //     let mut proof = merkle.prove(first)?;
    //     assert!(!proof.0.is_empty());
    //     let end_proof = merkle.prove(&key)?;
    //     assert!(!end_proof.0.is_empty());
    //     proof.extend(end_proof);

    //     merkle.verify_range_proof(&proof, first, &key, vec![&key], vec![val])?;

    //     Ok(())
    // }

    // #[test]
    // // Tests the range proof with all elements.
    // // The edge proofs can be nil.
    // #[allow(clippy::indexing_slicing)]
    // fn test_all_elements_proof() -> Result<(), ProofError> {
    //     let set = fixed_and_pseudorandom_data(4096);
    //     let mut items = Vec::from_iter(set.iter());
    //     items.sort();
    //     let merkle = merkle_build_test(items.clone())?;

    //     let item_iter = items.clone().into_iter();
    //     let keys: Vec<&[u8]> = item_iter.clone().map(|item| item.0.as_ref()).collect();
    //     let vals: Vec<&[u8; 20]> = item_iter.map(|item| item.1).collect();

    //     let empty_proof = Proof(HashMap::<[u8; 32], Vec<u8>>::new());
    //     let empty_key: [u8; 32] = [0; 32];
    //     merkle.verify_range_proof(
    //         &empty_proof,
    //         &empty_key,
    //         &empty_key,
    //         keys.clone(),
    //         vals.clone(),
    //     )?;

    //     // With edge proofs, it should still work.
    //     let start = 0;
    //     let end = &items.len() - 1;

    //     let mut proof = merkle.prove(items[start].0)?;
    //     assert!(!proof.0.is_empty());
    //     let end_proof = merkle.prove(items[end].0)?;
    //     assert!(!end_proof.0.is_empty());
    //     proof.extend(end_proof);

    //     merkle.verify_range_proof(
    //         &proof,
    //         items[start].0,
    //         items[end].0,
    //         keys.clone(),
    //         vals.clone(),
    //     )?;

    //     // Even with non-existent edge proofs, it should still work.
    //     let first = &[0; 32];
    //     let last = &[255; 32];
    //     let mut proof = merkle.prove(first)?;
    //     assert!(!proof.0.is_empty());
    //     let end_proof = merkle.prove(last)?;
    //     assert!(!end_proof.0.is_empty());
    //     proof.extend(end_proof);

    //     merkle.verify_range_proof(&proof, first, last, keys, vals)?;

    //     Ok(())
    // }

    // #[test]
    // // Tests the range proof with "no" element. The first edge proof must
    // // be a non-existent proof.
    // #[allow(clippy::indexing_slicing)]
    // fn test_empty_range_proof() -> Result<(), ProofError> {
    //     let set = fixed_and_pseudorandom_data(4096);
    //     let mut items = Vec::from_iter(set.iter());
    //     items.sort();
    //     let merkle = merkle_build_test(items.clone())?;

    //     let cases = [(items.len() - 1, false)];
    //     for c in cases.iter() {
    //         let first = increase_key(items[c.0].0);
    //         let proof = merkle.prove(&first)?;
    //         assert!(!proof.0.is_empty());

    //         // key and value vectors are intentionally empty.
    //         let keys: Vec<&[u8]> = Vec::new();
    //         let vals: Vec<[u8; 20]> = Vec::new();

    //         if c.1 {
    //             assert!(merkle
    //                 .verify_range_proof(&proof, &first, &first, keys, vals)
    //                 .is_err());
    //         } else {
    //             merkle.verify_range_proof(&proof, &first, &first, keys, vals)?;
    //         }
    //     }

    //     Ok(())
    // }

    // #[test]
    // // Focuses on the small trie with embedded nodes. If the gapped
    // // node is embedded in the trie, it should be detected too.
    // #[allow(clippy::indexing_slicing)]
    // fn test_gapped_range_proof() -> Result<(), ProofError> {
    //     let mut items = Vec::new();
    //     // Sorted entries
    //     for i in 0..10_u32 {
    //         let mut key = [0; 32];
    //         for (index, d) in i.to_be_bytes().iter().enumerate() {
    //             key[index] = *d;
    //         }
    //         items.push((key, i.to_be_bytes()));
    //     }
    //     let merkle = merkle_build_test(items.clone())?;

    //     let first = 2;
    //     let last = 8;

    //     let mut proof = merkle.prove(&items[first].0)?;
    //     assert!(!proof.0.is_empty());
    //     let end_proof = merkle.prove(&items[last - 1].0)?;
    //     assert!(!end_proof.0.is_empty());
    //     proof.extend(end_proof);

    //     let middle = (first + last) / 2 - first;
    //     let (keys, vals): (Vec<&[u8]>, Vec<&[u8; 4]>) = items[first..last]
    //         .iter()
    //         .enumerate()
    //         .filter(|(pos, _)| *pos != middle)
    //         .map(|(_, item)| (item.0.as_ref(), &item.1))
    //         .unzip();

    //     assert!(merkle
    //         .verify_range_proof(&proof, &items[0].0, &items[items.len() - 1].0, keys, vals)
    //         .is_err());

    //     Ok(())
    // }

    // #[test]
    // // Tests the element is not in the range covered by proofs.
    // #[allow(clippy::indexing_slicing)]
    // fn test_same_side_proof() -> Result<(), MerkleError> {
    //     let set = fixed_and_pseudorandom_data(4096);
    //     let mut items = Vec::from_iter(set.iter());
    //     items.sort();
    //     let merkle = merkle_build_test(items.clone())?;

    //     let pos = 1000;
    //     let mut last = decrease_key(items[pos].0);
    //     let mut first = last;
    //     first = decrease_key(&first);

    //     let mut proof = merkle.prove(&first)?;
    //     assert!(!proof.0.is_empty());
    //     let end_proof = merkle.prove(&last)?;
    //     assert!(!end_proof.0.is_empty());
    //     proof.extend(end_proof);

    //     assert!(merkle
    //         .verify_range_proof(
    //             &proof,
    //             &first,
    //             &last,
    //             vec![items[pos].0],
    //             vec![items[pos].1]
    //         )
    //         .is_err());

    //     first = increase_key(items[pos].0);
    //     last = first;
    //     last = increase_key(&last);

    //     let mut proof = merkle.prove(&first)?;
    //     assert!(!proof.0.is_empty());
    //     let end_proof = merkle.prove(&last)?;
    //     assert!(!end_proof.0.is_empty());
    //     proof.extend(end_proof);

    //     assert!(merkle
    //         .verify_range_proof(
    //             &proof,
    //             &first,
    //             &last,
    //             vec![items[pos].0],
    //             vec![items[pos].1]
    //         )
    //         .is_err());

    //     Ok(())
    // }

    // #[test]
    // #[allow(clippy::indexing_slicing)]
    // // Tests the range starts from zero.
    // fn test_single_side_range_proof() -> Result<(), ProofError> {
    //     for _ in 0..10 {
    //         let mut set = HashMap::new();
    //         for _ in 0..4096_u32 {
    //             let key = rand::thread_rng().gen::<[u8; 32]>();
    //             let val = rand::thread_rng().gen::<[u8; 20]>();
    //             set.insert(key, val);
    //         }
    //         let mut items = Vec::from_iter(set.iter());
    //         items.sort();
    //         let merkle = merkle_build_test(items.clone())?;

    //         let cases = vec![0, 1, 100, 1000, items.len() - 1];
    //         for case in cases {
    //             let start = &[0; 32];
    //             let mut proof = merkle.prove(start)?;
    //             assert!(!proof.0.is_empty());
    //             let end_proof = merkle.prove(items[case].0)?;
    //             assert!(!end_proof.0.is_empty());
    //             proof.extend(end_proof);

    //             let item_iter = items.clone().into_iter().take(case + 1);
    //             let keys = item_iter.clone().map(|item| item.0.as_ref()).collect();
    //             let vals = item_iter.map(|item| item.1).collect();

    //             merkle.verify_range_proof(&proof, start, items[case].0, keys, vals)?;
    //         }
    //     }
    //     Ok(())
    // }

    // #[test]
    // #[allow(clippy::indexing_slicing)]
    // // Tests the range ends with 0xffff...fff.
    // fn test_reverse_single_side_range_proof() -> Result<(), ProofError> {
    //     for _ in 0..10 {
    //         let mut set = HashMap::new();
    //         for _ in 0..1024_u32 {
    //             let key = rand::thread_rng().gen::<[u8; 32]>();
    //             let val = rand::thread_rng().gen::<[u8; 20]>();
    //             set.insert(key, val);
    //         }
    //         let mut items = Vec::from_iter(set.iter());
    //         items.sort();
    //         let merkle = merkle_build_test(items.clone())?;

    //         let cases = vec![0, 1, 100, 1000, items.len() - 1];
    //         for case in cases {
    //             let end = &[255; 32];
    //             let mut proof = merkle.prove(items[case].0)?;
    //             assert!(!proof.0.is_empty());
    //             let end_proof = merkle.prove(end)?;
    //             assert!(!end_proof.0.is_empty());
    //             proof.extend(end_proof);

    //             let item_iter = items.clone().into_iter().skip(case);
    //             let keys = item_iter.clone().map(|item| item.0.as_ref()).collect();
    //             let vals = item_iter.map(|item| item.1).collect();

    //             merkle.verify_range_proof(&proof, items[case].0, end, keys, vals)?;
    //         }
    //     }
    //     Ok(())
    // }

    // #[test]
    // // Tests the range starts with zero and ends with 0xffff...fff.
    // fn test_both_sides_range_proof() -> Result<(), ProofError> {
    //     for _ in 0..10 {
    //         let mut set = HashMap::new();
    //         for _ in 0..4096_u32 {
    //             let key = rand::thread_rng().gen::<[u8; 32]>();
    //             let val = rand::thread_rng().gen::<[u8; 20]>();
    //             set.insert(key, val);
    //         }
    //         let mut items = Vec::from_iter(set.iter());
    //         items.sort();
    //         let merkle = merkle_build_test(items.clone())?;

    //         let start = &[0; 32];
    //         let end = &[255; 32];

    //         let mut proof = merkle.prove(start)?;
    //         assert!(!proof.0.is_empty());
    //         let end_proof = merkle.prove(end)?;
    //         assert!(!end_proof.0.is_empty());
    //         proof.extend(end_proof);

    //         let (keys, vals): (Vec<&[u8; 32]>, Vec<&[u8; 20]>) = items.into_iter().unzip();
    //         let keys = keys.iter().map(|k| k.as_ref()).collect::<Vec<&[u8]>>();
    //         merkle.verify_range_proof(&proof, start, end, keys, vals)?;
    //     }
    //     Ok(())
    // }

    // #[test]
    // #[allow(clippy::indexing_slicing)]
    // // Tests normal range proof with both edge proofs
    // // as the existent proof, but with an extra empty value included, which is a
    // // noop technically, but practically should be rejected.
    // fn test_empty_value_range_proof() -> Result<(), ProofError> {
    //     let set = fixed_and_pseudorandom_data(512);
    //     let mut items = Vec::from_iter(set.iter());
    //     items.sort();
    //     let merkle = merkle_build_test(items.clone())?;

    //     // Create a new entry with a slightly modified key
    //     let mid_index = items.len() / 2;
    //     let key = increase_key(items[mid_index - 1].0);
    //     let empty_value: [u8; 20] = [0; 20];
    //     items.splice(mid_index..mid_index, [(&key, &empty_value)].iter().cloned());

    //     let start = 1;
    //     let end = items.len() - 1;

    //     let mut proof = merkle.prove(items[start].0)?;
    //     assert!(!proof.0.is_empty());
    //     let end_proof = merkle.prove(items[end - 1].0)?;
    //     assert!(!end_proof.0.is_empty());
    //     proof.extend(end_proof);

    //     let item_iter = items.clone().into_iter().skip(start).take(end - start);
    //     let keys = item_iter.clone().map(|item| item.0.as_ref()).collect();
    //     let vals = item_iter.map(|item| item.1).collect();
    //     assert!(merkle
    //         .verify_range_proof(&proof, items[start].0, items[end - 1].0, keys, vals)
    //         .is_err());

    //     Ok(())
    // }

    // #[test]
    // #[allow(clippy::indexing_slicing)]
    // // Tests the range proof with all elements,
    // // but with an extra empty value included, which is a noop technically, but
    // // practically should be rejected.
    // fn test_all_elements_empty_value_range_proof() -> Result<(), ProofError> {
    //     let set = fixed_and_pseudorandom_data(512);
    //     let mut items = Vec::from_iter(set.iter());
    //     items.sort();
    //     let merkle = merkle_build_test(items.clone())?;

    //     // Create a new entry with a slightly modified key
    //     let mid_index = items.len() / 2;
    //     let key = increase_key(items[mid_index - 1].0);
    //     let empty_value: [u8; 20] = [0; 20];
    //     items.splice(mid_index..mid_index, [(&key, &empty_value)].iter().cloned());

    //     let start = 0;
    //     let end = items.len() - 1;

    //     let mut proof = merkle.prove(items[start].0)?;
    //     assert!(!proof.0.is_empty());
    //     let end_proof = merkle.prove(items[end].0)?;
    //     assert!(!end_proof.0.is_empty());
    //     proof.extend(end_proof);

    //     let item_iter = items.clone().into_iter();
    //     let keys = item_iter.clone().map(|item| item.0.as_ref()).collect();
    //     let vals = item_iter.map(|item| item.1).collect();
    //     assert!(merkle
    //         .verify_range_proof(&proof, items[start].0, items[end].0, keys, vals)
    //         .is_err());

    //     Ok(())
    // }

    // #[test]
    // fn test_range_proof_keys_with_shared_prefix() -> Result<(), ProofError> {
    //     let items = vec![
    //         (
    //             hex::decode("aa10000000000000000000000000000000000000000000000000000000000000")
    //                 .expect("Decoding failed"),
    //             hex::decode("02").expect("Decoding failed"),
    //         ),
    //         (
    //             hex::decode("aa20000000000000000000000000000000000000000000000000000000000000")
    //                 .expect("Decoding failed"),
    //             hex::decode("03").expect("Decoding failed"),
    //         ),
    //     ];
    //     let merkle = merkle_build_test(items.clone())?;

    //     let start = hex::decode("0000000000000000000000000000000000000000000000000000000000000000")
    //         .expect("Decoding failed");
    //     let end = hex::decode("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
    //         .expect("Decoding failed");

    //     let mut proof = merkle.prove(&start)?;
    //     assert!(!proof.0.is_empty());
    //     let end_proof = merkle.prove(&end)?;
    //     assert!(!end_proof.0.is_empty());
    //     proof.extend(end_proof);

    //     let keys = items.iter().map(|item| item.0.as_ref()).collect();
    //     let vals = items.iter().map(|item| item.1.clone()).collect();

    //     merkle.verify_range_proof(&proof, &start, &end, keys, vals)?;

    //     Ok(())
    // }

    // #[test]
    // #[allow(clippy::indexing_slicing)]
    // // Tests a malicious proof, where the proof is more or less the
    // // whole trie. This is to match corresponding test in geth.
    // fn test_bloadted_range_proof() -> Result<(), ProofError> {
    //     // Use a small trie
    //     let mut items = Vec::new();
    //     for i in 0..100_u32 {
    //         let mut key: [u8; 32] = [0; 32];
    //         let mut value: [u8; 20] = [0; 20];
    //         for (index, d) in i.to_be_bytes().iter().enumerate() {
    //             key[index] = *d;
    //             value[index] = *d;
    //         }
    //         items.push((key, value));
    //     }
    //     let merkle = merkle_build_test(items.clone())?;

    //     // In the 'malicious' case, we add proofs for every single item
    //     // (but only one key/value pair used as leaf)
    //     let mut proof = Proof(HashMap::new());
    //     let mut keys = Vec::new();
    //     let mut vals = Vec::new();
    //     for (i, item) in items.iter().enumerate() {
    //         let cur_proof = merkle.prove(&item.0)?;
    //         assert!(!cur_proof.0.is_empty());
    //         proof.extend(cur_proof);
    //         if i == 50 {
    //             keys.push(item.0.as_ref());
    //             vals.push(item.1);
    //         }
    //     }

    //     merkle.verify_range_proof(&proof, keys[0], keys[keys.len() - 1], keys.clone(), vals)?;

    //     Ok(())
    // }

    // generate pseudorandom data, but prefix it with some known data
    // The number of fixed data points is 100; you specify how much random data you want
    // #[allow(clippy::indexing_slicing)]
    // fn fixed_and_pseudorandom_data(random_count: u32) -> HashMap<[u8; 32], [u8; 20]> {
    //     let mut items: HashMap<[u8; 32], [u8; 20]> = HashMap::new();
    //     for i in 0..100_u32 {
    //         let mut key: [u8; 32] = [0; 32];
    //         let mut value: [u8; 20] = [0; 20];
    //         for (index, d) in i.to_be_bytes().iter().enumerate() {
    //             key[index] = *d;
    //             value[index] = *d;
    //         }
    //         items.insert(key, value);

    //         let mut more_key: [u8; 32] = [0; 32];
    //         for (index, d) in (i + 10).to_be_bytes().iter().enumerate() {
    //             more_key[index] = *d;
    //         }
    //         items.insert(more_key, value);
    //     }

    //     // read FIREWOOD_TEST_SEED from the environment. If it's there, parse it into a u64.
    //     let seed = std::env::var("FIREWOOD_TEST_SEED")
    //         .ok()
    //         .map_or_else(
    //             || None,
    //             |s| Some(str::parse(&s).expect("couldn't parse FIREWOOD_TEST_SEED; must be a u64")),
    //         )
    //         .unwrap_or_else(|| thread_rng().gen());

    //     // the test framework will only render this in verbose mode or if the test fails
    //     // to re-run the test when it fails, just specify the seed instead of randomly
    //     // selecting one
    //     eprintln!("Seed {seed}: to rerun with this data, export FIREWOOD_TEST_SEED={seed}");
    //     let mut r = StdRng::seed_from_u64(seed);
    //     for _ in 0..random_count {
    //         let key = r.gen::<[u8; 32]>();
    //         let val = r.gen::<[u8; 20]>();
    //         items.insert(key, val);
    //     }
    //     items
    // }

    // fn increase_key(key: &[u8; 32]) -> [u8; 32] {
    //     let mut new_key = *key;
    //     for ch in new_key.iter_mut().rev() {
    //         let overflow;
    //         (*ch, overflow) = ch.overflowing_add(1);
    //         if !overflow {
    //             break;
    //         }
    //     }
    //     new_key
    // }

    // fn decrease_key(key: &[u8; 32]) -> [u8; 32] {
    //     let mut new_key = *key;
    //     for ch in new_key.iter_mut().rev() {
    //         let overflow;
    //         (*ch, overflow) = ch.overflowing_sub(1);
    //         if !overflow {
    //             break;
    //         }
    //     }
    //     new_key
    // }
}
