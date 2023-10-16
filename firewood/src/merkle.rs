// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::{nibbles::Nibbles, v2::api::Proof};
use sha3::Digest;
use shale::{disk_address::DiskAddress, ObjRef, ObjWriteError, ShaleError, ShaleStore};
use std::{
    cmp::Ordering,
    collections::HashMap,
    io::Write,
    iter::once,
    sync::{atomic::Ordering::Relaxed, OnceLock},
};
use thiserror::Error;

mod node;
mod partial_path;
mod trie_hash;

pub use node::{BranchNode, Data, ExtNode, LeafNode, Node, NodeType, NBRANCH};
pub use partial_path::PartialPath;
pub use trie_hash::{TrieHash, TRIE_HASH_LEN};

#[derive(Debug, Error)]
pub enum MerkleError {
    #[error("merkle datastore error: {0:?}")]
    Shale(#[from] ShaleError),
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
    #[error("error updating nodes: {0}")]
    WriteError(#[from] ObjWriteError),
}

macro_rules! write_node {
    ($self: expr, $r: expr, $modify: expr, $parents: expr, $deleted: expr) => {
        if let Err(_) = $r.write($modify) {
            let ptr = $self.put_node($r.clone())?.as_ptr();
            set_parent(ptr, $parents);
            $deleted.push($r.as_ptr());
            true
        } else {
            false
        }
    };
}

#[derive(Debug)]
pub struct Merkle<S> {
    store: Box<S>,
}

impl<S: ShaleStore<Node>> Merkle<S> {
    pub fn get_node(&self, ptr: DiskAddress) -> Result<ObjRef<Node>, MerkleError> {
        self.store.get_item(ptr).map_err(Into::into)
    }

    pub fn put_node(&self, node: Node) -> Result<ObjRef<Node>, MerkleError> {
        self.store.put_item(node, 0).map_err(Into::into)
    }

    fn free_node(&mut self, ptr: DiskAddress) -> Result<(), MerkleError> {
        self.store.free_item(ptr).map_err(Into::into)
    }
}

impl<S: ShaleStore<Node> + Send + Sync> Merkle<S> {
    pub fn new(store: Box<S>) -> Self {
        Self { store }
    }

    pub fn init_root(&self) -> Result<DiskAddress, MerkleError> {
        self.store
            .put_item(
                Node::new(NodeType::Branch(BranchNode {
                    children: [None; NBRANCH],
                    value: None,
                    children_encoded: Default::default(),
                })),
                Node::max_branch_node_size(),
            )
            .map_err(MerkleError::Shale)
            .map(|node| node.as_ptr())
    }

    pub fn get_store(&self) -> &dyn ShaleStore<Node> {
        self.store.as_ref()
    }

    pub fn empty_root() -> &'static TrieHash {
        static V: OnceLock<TrieHash> = OnceLock::new();
        V.get_or_init(|| {
            TrieHash(
                hex::decode("56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421")
                    .unwrap()
                    .try_into()
                    .unwrap(),
            )
        })
    }

    pub fn root_hash(&self, root: DiskAddress) -> Result<TrieHash, MerkleError> {
        let root = self
            .get_node(root)?
            .inner
            .as_branch()
            .ok_or(MerkleError::NotBranchNode)?
            .children[0];
        Ok(if let Some(root) = root {
            let mut node = self.get_node(root)?;
            let res = node.get_root_hash::<S>(self.store.as_ref()).clone();
            if node.lazy_dirty.load(Relaxed) {
                node.write(|_| {}).unwrap();
                node.lazy_dirty.store(false, Relaxed);
            }
            res
        } else {
            Self::empty_root().clone()
        })
    }

    fn dump_(&self, u: DiskAddress, w: &mut dyn Write) -> Result<(), MerkleError> {
        let u_ref = self.get_node(u)?;
        write!(
            w,
            "{u:?} => {}: ",
            match u_ref.root_hash.get() {
                Some(h) => hex::encode(**h),
                None => "<lazy>".to_string(),
            }
        )?;
        match &u_ref.inner {
            NodeType::Branch(n) => {
                writeln!(w, "{n:?}")?;
                for c in n.children.iter().flatten() {
                    self.dump_(*c, w)?
                }
            }
            NodeType::Leaf(n) => writeln!(w, "{n:?}").unwrap(),
            NodeType::Extension(n) => {
                writeln!(w, "{n:?}")?;
                self.dump_(n.chd(), w)?
            }
        }
        Ok(())
    }

    pub fn dump(&self, root: DiskAddress, w: &mut dyn Write) -> Result<(), MerkleError> {
        if root.is_null() {
            write!(w, "<Empty>")?;
        } else {
            self.dump_(root, w)?;
        };
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn split<'b>(
        &self,
        mut node_to_split: ObjRef<'b, Node>,
        parents: &mut [(ObjRef<'b, Node>, u8)],
        insert_path: &[u8],
        n_path: Vec<u8>,
        n_value: Option<Data>,
        val: Vec<u8>,
        deleted: &mut Vec<DiskAddress>,
    ) -> Result<Option<Vec<u8>>, MerkleError> {
        let node_to_split_address = node_to_split.as_ptr();
        let split_index = insert_path
            .iter()
            .zip(n_path.iter())
            .position(|(a, b)| a != b);

        let new_child_address = if let Some(idx) = split_index {
            // paths diverge
            let new_split_node_path = n_path.split_at(idx + 1).1;
            let (matching_path, new_node_path) = insert_path.split_at(idx + 1);

            node_to_split.write(|node| {
                // TODO: handle unwrap better
                let path = node.inner.path_mut().unwrap();

                *path = PartialPath(new_split_node_path.to_vec());

                node.rehash();
            })?;

            let new_node = Node::leaf(PartialPath(new_node_path.to_vec()), Data(val));
            let leaf_address = self.put_node(new_node)?.as_ptr();

            let mut chd = [None; NBRANCH];

            let last_matching_nibble = matching_path[idx];
            chd[last_matching_nibble as usize] = Some(leaf_address);

            let address = match &node_to_split.inner {
                NodeType::Extension(u) if u.path.len() == 0 => {
                    deleted.push(node_to_split_address);
                    u.chd()
                }
                _ => node_to_split_address,
            };

            chd[n_path[idx] as usize] = Some(address);

            let new_branch = Node::branch(BranchNode {
                children: chd,
                value: None,
                children_encoded: Default::default(),
            });

            let new_branch_address = self.put_node(new_branch)?.as_ptr();

            if idx > 0 {
                self.put_node(Node::new(NodeType::Extension(ExtNode {
                    path: PartialPath(matching_path[..idx].to_vec()),
                    child: new_branch_address,
                    child_encoded: None,
                })))?
                .as_ptr()
            } else {
                new_branch_address
            }
        } else {
            // paths do not diverge
            let (leaf_address, prefix, idx, value) =
                match (insert_path.len().cmp(&n_path.len()), n_value) {
                    // no node-value means this is an extension node and we can therefore continue walking the tree
                    (Ordering::Greater, None) => return Ok(Some(val)),

                    // if the paths are equal, we overwrite the data
                    (Ordering::Equal, _) => {
                        let mut result = Ok(None);

                        write_node!(
                            self,
                            node_to_split,
                            |u| {
                                match &mut u.inner {
                                    NodeType::Leaf(u) => u.1 = Data(val),
                                    NodeType::Extension(u) => {
                                        let write_result =
                                            self.get_node(u.chd()).and_then(|mut b_ref| {
                                                b_ref
                                                    .write(|b| {
                                                        let branch =
                                                            b.inner.as_branch_mut().unwrap();
                                                        branch.value = Some(Data(val));

                                                        b.rehash()
                                                    })
                                                    // if writing fails, delete the child?
                                                    .or_else(|_| {
                                                        let node = self.put_node(b_ref.clone())?;

                                                        let child = u.chd_mut();
                                                        *child = node.as_ptr();

                                                        deleted.push(b_ref.as_ptr());

                                                        Ok(())
                                                    })
                                            });

                                        if let Err(e) = write_result {
                                            result = Err(e);
                                        }
                                    }
                                    NodeType::Branch(_) => unreachable!(),
                                }

                                u.rehash();
                            },
                            parents,
                            deleted
                        );

                        return result;
                    }

                    // if the node-path is greater than the insert path
                    (Ordering::Less, _) => {
                        // key path is a prefix of the path to u
                        node_to_split
                            .write(|u| {
                                // TODO: handle unwraps better
                                let path = u.inner.path_mut().unwrap();
                                *path = PartialPath(n_path[insert_path.len() + 1..].to_vec());

                                u.rehash();
                            })
                            .unwrap();

                        let leaf_address = match &node_to_split.inner {
                            NodeType::Extension(u) if u.path.len() == 0 => {
                                deleted.push(node_to_split_address);
                                u.chd()
                            }
                            _ => node_to_split_address,
                        };

                        (
                            leaf_address,
                            insert_path,
                            n_path[insert_path.len()] as usize,
                            Data(val).into(),
                        )
                    }
                    // insert path is greather than the path of the leaf
                    (Ordering::Greater, Some(n_value)) => {
                        let leaf = Node::leaf(
                            PartialPath(insert_path[n_path.len() + 1..].to_vec()),
                            Data(val),
                        );

                        let leaf_address = self.put_node(leaf)?.as_ptr();

                        deleted.push(node_to_split_address);

                        (
                            leaf_address,
                            n_path.as_slice(),
                            insert_path[n_path.len()] as usize,
                            n_value.into(),
                        )
                    }
                };

            // [parent] (-> [ExtNode]) -> [branch with v] -> [Leaf]
            let mut children = [None; NBRANCH];

            children[idx] = leaf_address.into();

            let branch_address = self
                .put_node(Node::branch(BranchNode {
                    children,
                    value,
                    children_encoded: Default::default(),
                }))?
                .as_ptr();

            if !prefix.is_empty() {
                self.put_node(Node::new(NodeType::Extension(ExtNode {
                    path: PartialPath(prefix.to_vec()),
                    child: branch_address,
                    child_encoded: None,
                })))?
                .as_ptr()
            } else {
                branch_address
            }
        };

        // observation:
        // - leaf/extension node can only be the child of a branch node
        // - branch node can only be the child of a branch/extension node
        // ^^^ I think a leaf can end up being the child of an extension node
        // ^^^ maybe just on delete though? I'm not sure, removing extension-nodes anyway
        set_parent(new_child_address, parents);

        Ok(None)
    }

    pub fn insert<K: AsRef<[u8]>>(
        &mut self,
        key: K,
        val: Vec<u8>,
        root: DiskAddress,
    ) -> Result<(), MerkleError> {
        let (parents, deleted) = self.insert_and_return_updates(key, val, root)?;

        for mut r in parents {
            r.write(|u| u.rehash())?;
        }

        for ptr in deleted {
            self.free_node(ptr)?
        }

        Ok(())
    }

    fn insert_and_return_updates<K: AsRef<[u8]>>(
        &self,
        key: K,
        mut val: Vec<u8>,
        root: DiskAddress,
    ) -> Result<(impl Iterator<Item = ObjRef<Node>>, Vec<DiskAddress>), MerkleError> {
        // as we split a node, we need to track deleted nodes and parents
        let mut deleted = Vec::new();
        let mut parents = Vec::new();

        let mut next_node = None;

        // we use Nibbles::<1> so that 1 zero nibble is at the front
        // this is for the sentinel node, which avoids moving the root
        // and always only has one child
        let mut key_nibbles = Nibbles::<1>::new(key.as_ref()).into_iter();

        // walk down the merkle tree starting from next_node, currently the root
        // return None if the value is inserted
        let next_node_and_val = loop {
            let mut node = next_node
                .take()
                .map(Ok)
                .unwrap_or_else(|| self.get_node(root))?;
            let node_ptr = node.as_ptr();

            let Some(current_nibble) = key_nibbles.next() else {
                break Some((node, val));
            };

            let (node, next_node_ptr) = match &node.inner {
                // For a Branch node, we look at the child pointer. If it points
                // to another node, we walk down that. Otherwise, we can store our
                // value as a leaf and we're done
                NodeType::Branch(n) => match n.children[current_nibble as usize] {
                    Some(c) => (node, c),
                    None => {
                        // insert the leaf to the empty slot
                        // create a new leaf
                        let leaf_ptr = self
                            .put_node(Node::new(NodeType::Leaf(LeafNode(
                                PartialPath(key_nibbles.collect()),
                                Data(val),
                            ))))?
                            .as_ptr();
                        // set the current child to point to this leaf
                        node.write(|u| {
                            let uu = u.inner.as_branch_mut().unwrap();
                            uu.children[current_nibble as usize] = Some(leaf_ptr);
                            u.rehash();
                        })
                        .unwrap();

                        break None;
                    }
                },

                NodeType::Leaf(n) => {
                    // we collided with another key; make a copy
                    // of the stored key to pass into split
                    let n_path = n.0.to_vec();
                    let n_value = Some(n.1.clone());
                    let rem_path = once(current_nibble).chain(key_nibbles).collect::<Vec<_>>();

                    self.split(
                        node,
                        &mut parents,
                        &rem_path,
                        n_path,
                        n_value,
                        val,
                        &mut deleted,
                    )?;

                    break None;
                }

                NodeType::Extension(n) => {
                    let n_path = n.path.to_vec();
                    let n_ptr = n.chd();
                    let rem_path = once(current_nibble)
                        .chain(key_nibbles.clone())
                        .collect::<Vec<_>>();
                    let n_path_len = n_path.len();

                    if let Some(v) = self.split(
                        node,
                        &mut parents,
                        &rem_path,
                        n_path,
                        None,
                        val,
                        &mut deleted,
                    )? {
                        (0..n_path_len).skip(1).for_each(|_| {
                            key_nibbles.next();
                        });

                        // we couldn't split this, so we
                        // skip n_path items and follow the
                        // extension node's next pointer
                        val = v;

                        (self.get_node(node_ptr)?, n_ptr)
                    } else {
                        // successfully inserted
                        break None;
                    }
                }
            };

            // push another parent, and follow the next pointer
            parents.push((node, current_nibble));
            next_node = self.get_node(next_node_ptr)?.into();
        };

        if let Some((mut node, val)) = next_node_and_val {
            // we walked down the tree and reached the end of the key,
            // but haven't inserted the value yet
            let mut info = None;
            let u_ptr = {
                write_node!(
                    self,
                    node,
                    |u| {
                        info = match &mut u.inner {
                            NodeType::Branch(n) => {
                                n.value = Some(Data(val));
                                None
                            }
                            NodeType::Leaf(n) => {
                                if n.0.len() == 0 {
                                    n.1 = Data(val);

                                    None
                                } else {
                                    let idx = n.0[0];
                                    n.0 = PartialPath(n.0[1..].to_vec());
                                    u.rehash();

                                    Some((idx, true, None, val))
                                }
                            }
                            NodeType::Extension(n) => {
                                let idx = n.path[0];
                                let more = if n.path.len() > 1 {
                                    n.path = PartialPath(n.path[1..].to_vec());
                                    true
                                } else {
                                    false
                                };

                                Some((idx, more, Some(n.chd()), val))
                            }
                        };

                        u.rehash()
                    },
                    &mut parents,
                    &mut deleted
                );

                node.as_ptr()
            };

            if let Some((idx, more, ext, val)) = info {
                let mut chd = [None; NBRANCH];

                let c_ptr = if more {
                    u_ptr
                } else {
                    deleted.push(u_ptr);
                    ext.unwrap()
                };

                chd[idx as usize] = Some(c_ptr);

                let branch = self
                    .put_node(Node::new(NodeType::Branch(BranchNode {
                        children: chd,
                        value: Some(Data(val)),
                        children_encoded: Default::default(),
                    })))?
                    .as_ptr();

                set_parent(branch, &mut parents);
            }
        }

        Ok((parents.into_iter().rev().map(|(node, _)| node), deleted))
    }

    fn after_remove_leaf(
        &self,
        parents: &mut Vec<(ObjRef<'_, Node>, u8)>,
        deleted: &mut Vec<DiskAddress>,
    ) -> Result<(), MerkleError> {
        let (b_chd, val) = {
            let (mut b_ref, b_idx) = parents.pop().unwrap();
            // the immediate parent of a leaf must be a branch
            b_ref
                .write(|b| {
                    b.inner.as_branch_mut().unwrap().children[b_idx as usize] = None;
                    b.rehash()
                })
                .unwrap();
            let b_inner = b_ref.inner.as_branch().unwrap();
            let (b_chd, has_chd) = b_inner.single_child();
            if (has_chd && (b_chd.is_none() || b_inner.value.is_some())) || parents.is_empty() {
                return Ok(());
            }
            deleted.push(b_ref.as_ptr());
            (b_chd, b_inner.value.clone())
        };
        let (mut p_ref, p_idx) = parents.pop().unwrap();
        let p_ptr = p_ref.as_ptr();
        if let Some(val) = val {
            match &p_ref.inner {
                NodeType::Branch(_) => {
                    // from: [p: Branch] -> [b (v)]x -> [Leaf]x
                    // to: [p: Branch] -> [Leaf (v)]
                    let leaf = self
                        .put_node(Node::new(NodeType::Leaf(LeafNode(
                            PartialPath(Vec::new()),
                            val,
                        ))))?
                        .as_ptr();
                    p_ref
                        .write(|p| {
                            p.inner.as_branch_mut().unwrap().children[p_idx as usize] = Some(leaf);
                            p.rehash()
                        })
                        .unwrap();
                }
                NodeType::Extension(n) => {
                    // from: P -> [p: Ext]x -> [b (v)]x -> [leaf]x
                    // to: P -> [Leaf (v)]
                    let leaf = self
                        .put_node(Node::new(NodeType::Leaf(LeafNode(
                            PartialPath(n.path.clone().into_inner()),
                            val,
                        ))))?
                        .as_ptr();
                    deleted.push(p_ptr);
                    set_parent(leaf, parents);
                }
                _ => unreachable!(),
            }
        } else {
            let (c_ptr, idx) = b_chd.unwrap();
            let mut c_ref = self.get_node(c_ptr)?;
            match &c_ref.inner {
                NodeType::Branch(_) => {
                    drop(c_ref);
                    match &p_ref.inner {
                        NodeType::Branch(_) => {
                            //                            ____[Branch]
                            //                           /
                            // from: [p: Branch] -> [b]x*
                            //                           \____[Leaf]x
                            // to: [p: Branch] -> [Ext] -> [Branch]
                            let ext = self
                                .put_node(Node::new(NodeType::Extension(ExtNode {
                                    path: PartialPath(vec![idx]),
                                    child: c_ptr,
                                    child_encoded: None,
                                })))?
                                .as_ptr();
                            set_parent(ext, &mut [(p_ref, p_idx)]);
                        }
                        NodeType::Extension(_) => {
                            //                         ____[Branch]
                            //                        /
                            // from: [p: Ext] -> [b]x*
                            //                        \____[Leaf]x
                            // to: [p: Ext] -> [Branch]
                            write_node!(
                                self,
                                p_ref,
                                |p| {
                                    let pp = p.inner.as_extension_mut().unwrap();
                                    pp.path.0.push(idx);
                                    *pp.chd_mut() = c_ptr;
                                    p.rehash();
                                },
                                parents,
                                deleted
                            );
                        }
                        _ => unreachable!(),
                    }
                }
                NodeType::Leaf(_) | NodeType::Extension(_) => {
                    match &p_ref.inner {
                        NodeType::Branch(_) => {
                            //                            ____[Leaf/Ext]
                            //                           /
                            // from: [p: Branch] -> [b]x*
                            //                           \____[Leaf]x
                            // to: [p: Branch] -> [Leaf/Ext]
                            let write_result = c_ref.write(|c| {
                                let partial_path = match &mut c.inner {
                                    NodeType::Leaf(n) => &mut n.0,
                                    NodeType::Extension(n) => &mut n.path,
                                    _ => unreachable!(),
                                };

                                partial_path.0.insert(0, idx);
                                c.rehash()
                            });

                            let c_ptr = if write_result.is_err() {
                                deleted.push(c_ptr);
                                self.put_node(c_ref.clone())?.as_ptr()
                            } else {
                                c_ptr
                            };

                            drop(c_ref);

                            p_ref
                                .write(|p| {
                                    p.inner.as_branch_mut().unwrap().children[p_idx as usize] =
                                        Some(c_ptr);
                                    p.rehash()
                                })
                                .unwrap();
                        }
                        NodeType::Extension(n) => {
                            //                               ____[Leaf/Ext]
                            //                              /
                            // from: P -> [p: Ext]x -> [b]x*
                            //                              \____[Leaf]x
                            // to: P -> [p: Leaf/Ext]
                            deleted.push(p_ptr);

                            let write_failed = write_node!(
                                self,
                                c_ref,
                                |c| {
                                    let mut path = n.path.clone().into_inner();
                                    path.push(idx);
                                    let path0 = match &mut c.inner {
                                        NodeType::Leaf(n) => &mut n.0,
                                        NodeType::Extension(n) => &mut n.path,
                                        _ => unreachable!(),
                                    };
                                    path.extend(&**path0);
                                    *path0 = PartialPath(path);
                                    c.rehash()
                                },
                                parents,
                                deleted
                            );

                            if !write_failed {
                                drop(c_ref);
                                set_parent(c_ptr, parents);
                            }
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
        Ok(())
    }

    fn after_remove_branch(
        &self,
        (c_ptr, idx): (DiskAddress, u8),
        parents: &mut Vec<(ObjRef<'_, Node>, u8)>,
        deleted: &mut Vec<DiskAddress>,
    ) -> Result<(), MerkleError> {
        // [b] -> [u] -> [c]
        let (mut b_ref, b_idx) = parents.pop().unwrap();
        let mut c_ref = self.get_node(c_ptr).unwrap();
        match &c_ref.inner {
            NodeType::Branch(_) => {
                drop(c_ref);
                let mut err = None;
                write_node!(
                    self,
                    b_ref,
                    |b| {
                        if let Err(e) = (|| {
                            match &mut b.inner {
                                NodeType::Branch(n) => {
                                    // from: [Branch] -> [Branch]x -> [Branch]
                                    // to: [Branch] -> [Ext] -> [Branch]
                                    n.children[b_idx as usize] = Some(
                                        self.put_node(Node::new(NodeType::Extension(ExtNode {
                                            path: PartialPath(vec![idx]),
                                            child: c_ptr,
                                            child_encoded: None,
                                        })))?
                                        .as_ptr(),
                                    );
                                }
                                NodeType::Extension(n) => {
                                    // from: [Ext] -> [Branch]x -> [Branch]
                                    // to: [Ext] -> [Branch]
                                    n.path.0.push(idx);
                                    *n.chd_mut() = c_ptr
                                }
                                _ => unreachable!(),
                            }
                            b.rehash();
                            Ok(())
                        })() {
                            err = Some(Err(e))
                        }
                    },
                    parents,
                    deleted
                );
                if let Some(e) = err {
                    return e;
                }
            }
            NodeType::Leaf(_) | NodeType::Extension(_) => match &b_ref.inner {
                NodeType::Branch(_) => {
                    // from: [Branch] -> [Branch]x -> [Leaf/Ext]
                    // to: [Branch] -> [Leaf/Ext]
                    let write_result = c_ref.write(|c| {
                        match &mut c.inner {
                            NodeType::Leaf(n) => &mut n.0,
                            NodeType::Extension(n) => &mut n.path,
                            _ => unreachable!(),
                        }
                        .0
                        .insert(0, idx);
                        c.rehash()
                    });
                    if write_result.is_err() {
                        deleted.push(c_ptr);
                        self.put_node(c_ref.clone())?.as_ptr()
                    } else {
                        c_ptr
                    };
                    drop(c_ref);
                    b_ref
                        .write(|b| {
                            b.inner.as_branch_mut().unwrap().children[b_idx as usize] = Some(c_ptr);
                            b.rehash()
                        })
                        .unwrap();
                }
                NodeType::Extension(n) => {
                    // from: P -> [Ext] -> [Branch]x -> [Leaf/Ext]
                    // to: P -> [Leaf/Ext]
                    let write_result = c_ref.write(|c| {
                        let mut path = n.path.clone().into_inner();
                        path.push(idx);
                        let path0 = match &mut c.inner {
                            NodeType::Leaf(n) => &mut n.0,
                            NodeType::Extension(n) => &mut n.path,
                            _ => unreachable!(),
                        };
                        path.extend(&**path0);
                        *path0 = PartialPath(path);
                        c.rehash()
                    });

                    let c_ptr = if write_result.is_err() {
                        deleted.push(c_ptr);
                        self.put_node(c_ref.clone())?.as_ptr()
                    } else {
                        c_ptr
                    };

                    deleted.push(b_ref.as_ptr());
                    drop(c_ref);
                    set_parent(c_ptr, parents);
                }
                _ => unreachable!(),
            },
        }
        Ok(())
    }

    pub fn remove<K: AsRef<[u8]>>(
        &mut self,
        key: K,
        root: DiskAddress,
    ) -> Result<Option<Vec<u8>>, MerkleError> {
        let mut chunks = vec![0];
        chunks.extend(key.as_ref().iter().copied().flat_map(to_nibble_array));

        if root.is_null() {
            return Ok(None);
        }

        let mut deleted = Vec::new();
        let mut parents: Vec<(ObjRef<Node>, _)> = Vec::new();
        let mut u_ref = self.get_node(root)?;
        let mut nskip = 0;
        let mut found = None;

        for (i, nib) in chunks.iter().enumerate() {
            if nskip > 0 {
                nskip -= 1;
                continue;
            }
            let next_ptr = match &u_ref.inner {
                NodeType::Branch(n) => match n.children[*nib as usize] {
                    Some(c) => c,
                    None => return Ok(None),
                },
                NodeType::Leaf(n) => {
                    if chunks[i..] != *n.0 {
                        return Ok(None);
                    }
                    found = Some(n.1.clone());
                    deleted.push(u_ref.as_ptr());
                    self.after_remove_leaf(&mut parents, &mut deleted)?;
                    break;
                }
                NodeType::Extension(n) => {
                    let n_path = &*n.path.0;
                    let rem_path = &chunks[i..];
                    if rem_path < n_path || &rem_path[..n_path.len()] != n_path {
                        return Ok(None);
                    }
                    nskip = n_path.len() - 1;
                    n.chd()
                }
            };

            parents.push((u_ref, *nib));
            u_ref = self.get_node(next_ptr)?;
        }
        if found.is_none() {
            match &u_ref.inner {
                NodeType::Branch(n) => {
                    if n.value.is_none() {
                        return Ok(None);
                    }
                    let (c_chd, _) = n.single_child();
                    u_ref
                        .write(|u| {
                            found = u.inner.as_branch_mut().unwrap().value.take();
                            u.rehash()
                        })
                        .unwrap();
                    if let Some((c_ptr, idx)) = c_chd {
                        deleted.push(u_ref.as_ptr());
                        self.after_remove_branch((c_ptr, idx), &mut parents, &mut deleted)?
                    }
                }
                NodeType::Leaf(n) => {
                    if n.0.len() > 0 {
                        return Ok(None);
                    }
                    found = Some(n.1.clone());
                    deleted.push(u_ref.as_ptr());
                    self.after_remove_leaf(&mut parents, &mut deleted)?
                }
                _ => (),
            }
        }

        drop(u_ref);

        for (mut r, _) in parents.into_iter().rev() {
            r.write(|u| u.rehash()).unwrap();
        }

        for ptr in deleted.into_iter() {
            self.free_node(ptr)?;
        }
        Ok(found.map(|e| e.0))
    }

    fn remove_tree_(
        &self,
        u: DiskAddress,
        deleted: &mut Vec<DiskAddress>,
    ) -> Result<(), MerkleError> {
        let u_ref = self.get_node(u)?;
        match &u_ref.inner {
            NodeType::Branch(n) => {
                for c in n.children.iter().flatten() {
                    self.remove_tree_(*c, deleted)?
                }
            }
            NodeType::Leaf(_) => (),
            NodeType::Extension(n) => self.remove_tree_(n.chd(), deleted)?,
        }
        deleted.push(u);
        Ok(())
    }

    pub fn remove_tree(&mut self, root: DiskAddress) -> Result<(), MerkleError> {
        let mut deleted = Vec::new();
        if root.is_null() {
            return Ok(());
        }
        self.remove_tree_(root, &mut deleted)?;
        for ptr in deleted.into_iter() {
            self.free_node(ptr)?;
        }
        Ok(())
    }

    pub fn get_mut<K: AsRef<[u8]>>(
        &mut self,
        key: K,
        root: DiskAddress,
    ) -> Result<Option<RefMut<S>>, MerkleError> {
        let mut chunks = vec![0];
        chunks.extend(key.as_ref().iter().copied().flat_map(to_nibble_array));
        let mut parents = Vec::new();

        if root.is_null() {
            return Ok(None);
        }

        let mut u_ref = self.get_node(root)?;
        let mut nskip = 0;

        for (i, nib) in chunks.iter().enumerate() {
            let u_ptr = u_ref.as_ptr();
            if nskip > 0 {
                nskip -= 1;
                continue;
            }
            let next_ptr = match &u_ref.inner {
                NodeType::Branch(n) => match n.children[*nib as usize] {
                    Some(c) => c,
                    None => return Ok(None),
                },
                NodeType::Leaf(n) => {
                    if chunks[i..] != *n.0 {
                        return Ok(None);
                    }
                    drop(u_ref);
                    return Ok(Some(RefMut::new(u_ptr, parents, self)));
                }
                NodeType::Extension(n) => {
                    let n_path = &*n.path.0;
                    let rem_path = &chunks[i..];
                    if rem_path.len() < n_path.len() || &rem_path[..n_path.len()] != n_path {
                        return Ok(None);
                    }
                    nskip = n_path.len() - 1;
                    n.chd()
                }
            };
            parents.push((u_ptr, *nib));
            u_ref = self.get_node(next_ptr)?;
        }

        let u_ptr = u_ref.as_ptr();
        match &u_ref.inner {
            NodeType::Branch(n) => {
                if n.value.as_ref().is_some() {
                    drop(u_ref);
                    return Ok(Some(RefMut::new(u_ptr, parents, self)));
                }
            }
            NodeType::Leaf(n) => {
                if n.0.len() == 0 {
                    drop(u_ref);
                    return Ok(Some(RefMut::new(u_ptr, parents, self)));
                }
            }
            _ => (),
        }

        Ok(None)
    }

    /// Constructs a merkle proof for key. The result contains all encoded nodes
    /// on the path to the value at key. The value itself is also included in the
    /// last node and can be retrieved by verifying the proof.
    ///
    /// If the trie does not contain a value for key, the returned proof contains
    /// all nodes of the longest existing prefix of the key, ending with the node
    /// that proves the absence of the key (at least the root node).
    pub fn prove<K>(&self, key: K, root: DiskAddress) -> Result<Proof<Vec<u8>>, MerkleError>
    where
        K: AsRef<[u8]>,
    {
        let key_nibbles = Nibbles::<0>::new(key.as_ref());

        let mut proofs = HashMap::new();
        if root.is_null() {
            return Ok(Proof(proofs));
        }

        // Skip the sentinel root
        let root = self
            .get_node(root)?
            .inner
            .as_branch()
            .ok_or(MerkleError::NotBranchNode)?
            .children[0];
        let mut u_ref = match root {
            Some(root) => self.get_node(root)?,
            None => return Ok(Proof(proofs)),
        };

        let mut nskip = 0;
        let mut nodes: Vec<DiskAddress> = Vec::new();
        for (i, nib) in key_nibbles.into_iter().enumerate() {
            if nskip > 0 {
                nskip -= 1;
                continue;
            }
            nodes.push(u_ref.as_ptr());
            let next_ptr: DiskAddress = match &u_ref.inner {
                NodeType::Branch(n) => match n.children[nib as usize] {
                    Some(c) => c,
                    None => break,
                },
                NodeType::Leaf(_) => break,
                NodeType::Extension(n) => {
                    // the key passed in must match the entire remainder of this
                    // extension node, otherwise we break out
                    let n_path = &n.path;
                    let remaining_path = key_nibbles.into_iter().skip(i);
                    if remaining_path.size_hint().0 < n_path.len() {
                        // all bytes aren't there
                        break;
                    }
                    if !remaining_path.take(n_path.len()).eq(n_path.iter().cloned()) {
                        // contents aren't the same
                        break;
                    }
                    nskip = n_path.len() - 1;
                    n.chd()
                }
            };
            u_ref = self.get_node(next_ptr)?;
        }

        match &u_ref.inner {
            NodeType::Branch(n) => {
                if n.value.as_ref().is_some() {
                    nodes.push(u_ref.as_ptr());
                }
            }
            NodeType::Leaf(n) => {
                if n.0.len() == 0 {
                    nodes.push(u_ref.as_ptr());
                }
            }
            _ => (),
        }

        drop(u_ref);
        // Get the hashes of the nodes.
        for node in nodes {
            let node = self.get_node(node)?;
            let encoded = <&[u8]>::clone(&node.get_encoded::<S>(self.store.as_ref()));
            let hash: [u8; TRIE_HASH_LEN] = sha3::Keccak256::digest(encoded).into();
            proofs.insert(hash, encoded.to_vec());
        }
        Ok(Proof(proofs))
    }

    pub fn get<K: AsRef<[u8]>>(
        &self,
        key: K,
        root: DiskAddress,
    ) -> Result<Option<Ref>, MerkleError> {
        if root.is_null() {
            return Ok(None);
        }

        let key_nibbles = Nibbles::<1>::new(key.as_ref());

        let mut u_ref = self.get_node(root)?;
        let mut nskip = 0;

        for (i, nib) in key_nibbles.into_iter().enumerate() {
            if nskip > 0 {
                nskip -= 1;
                continue;
            }
            let next_ptr = match &u_ref.inner {
                NodeType::Branch(n) => match n.children[nib as usize] {
                    Some(c) => c,
                    None => return Ok(None),
                },
                NodeType::Leaf(n) => {
                    if !key_nibbles.into_iter().skip(i).eq(n.0.iter().cloned()) {
                        return Ok(None);
                    }
                    return Ok(Some(Ref(u_ref)));
                }
                NodeType::Extension(n) => {
                    let n_path = &n.path;
                    let rem_path = key_nibbles.into_iter().skip(i);
                    if rem_path.size_hint().0 < n_path.len() {
                        return Ok(None);
                    }
                    if !rem_path.take(n_path.len()).eq(n_path.iter().cloned()) {
                        return Ok(None);
                    }
                    nskip = n_path.len() - 1;
                    n.chd()
                }
            };
            u_ref = self.get_node(next_ptr)?;
        }

        match &u_ref.inner {
            NodeType::Branch(n) => {
                if n.value.as_ref().is_some() {
                    return Ok(Some(Ref(u_ref)));
                }
            }
            NodeType::Leaf(n) => {
                if n.0.len() == 0 {
                    return Ok(Some(Ref(u_ref)));
                }
            }
            _ => (),
        }

        Ok(None)
    }

    pub fn flush_dirty(&self) -> Option<()> {
        self.store.flush_dirty()
    }
}

fn set_parent(new_chd: DiskAddress, parents: &mut [(ObjRef<'_, Node>, u8)]) {
    let (p_ref, idx) = parents.last_mut().unwrap();
    p_ref
        .write(|p| {
            match &mut p.inner {
                NodeType::Branch(pp) => pp.children[*idx as usize] = Some(new_chd),
                NodeType::Extension(pp) => *pp.chd_mut() = new_chd,
                _ => unreachable!(),
            }
            p.rehash();
        })
        .unwrap();
}

pub struct Ref<'a>(ObjRef<'a, Node>);

pub struct RefMut<'a, S> {
    ptr: DiskAddress,
    parents: Vec<(DiskAddress, u8)>,
    merkle: &'a mut Merkle<S>,
}

impl<'a> std::ops::Deref for Ref<'a> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        match &self.0.inner {
            NodeType::Branch(n) => n.value.as_ref().unwrap(),
            NodeType::Leaf(n) => &n.1,
            _ => unreachable!(),
        }
    }
}

impl<'a, S: ShaleStore<Node> + Send + Sync> RefMut<'a, S> {
    fn new(ptr: DiskAddress, parents: Vec<(DiskAddress, u8)>, merkle: &'a mut Merkle<S>) -> Self {
        Self {
            ptr,
            parents,
            merkle,
        }
    }

    pub fn get(&self) -> Ref {
        Ref(self.merkle.get_node(self.ptr).unwrap())
    }

    pub fn write(&mut self, modify: impl FnOnce(&mut Vec<u8>)) -> Result<(), MerkleError> {
        let mut deleted = Vec::new();
        {
            let mut u_ref = self.merkle.get_node(self.ptr).unwrap();
            let mut parents: Vec<_> = self
                .parents
                .iter()
                .map(|(ptr, nib)| (self.merkle.get_node(*ptr).unwrap(), *nib))
                .collect();
            write_node!(
                self.merkle,
                u_ref,
                |u| {
                    modify(match &mut u.inner {
                        NodeType::Branch(n) => &mut n.value.as_mut().unwrap().0,
                        NodeType::Leaf(n) => &mut n.1 .0,
                        _ => unreachable!(),
                    });
                    u.rehash()
                },
                &mut parents,
                &mut deleted
            );
        }
        for ptr in deleted.into_iter() {
            self.merkle.free_node(ptr)?;
        }
        Ok(())
    }
}

// nibbles, high bits first, then low bits
pub fn to_nibble_array(x: u8) -> [u8; 2] {
    [x >> 4, x & 0b_0000_1111]
}

// given a set of nibbles, take each pair and convert this back into bytes
// if an odd number of nibbles, in debug mode it panics. In release mode,
// the final nibble is dropped
pub fn from_nibbles(nibbles: &[u8]) -> impl Iterator<Item = u8> + '_ {
    debug_assert_eq!(nibbles.len() & 1, 0);
    nibbles.chunks_exact(2).map(|p| (p[0] << 4) | p[1])
}

#[cfg(test)]
mod test {
    use super::*;
    use shale::cached::{DynamicMem, PlainMem};
    use shale::{CachedStore, Storable};
    use std::ops::Deref;
    use std::sync::Arc;
    use test_case::test_case;

    #[test_case(vec![0x12, 0x34, 0x56], vec![0x1, 0x2, 0x3, 0x4, 0x5, 0x6])]
    #[test_case(vec![0xc0, 0xff], vec![0xc, 0x0, 0xf, 0xf])]
    fn test_to_nibbles(bytes: Vec<u8>, nibbles: Vec<u8>) {
        let n: Vec<_> = bytes.into_iter().flat_map(to_nibble_array).collect();
        assert_eq!(n, nibbles);
    }

    const ZERO_HASH: TrieHash = TrieHash([0u8; TRIE_HASH_LEN]);

    #[test]
    fn test_hash_len() {
        assert_eq!(TRIE_HASH_LEN, ZERO_HASH.dehydrated_len() as usize);
    }
    #[test]
    fn test_dehydrate() {
        let mut to = [1u8; TRIE_HASH_LEN];
        assert_eq!(
            {
                ZERO_HASH.dehydrate(&mut to).unwrap();
                &to
            },
            ZERO_HASH.deref()
        );
    }

    #[test]
    fn test_hydrate() {
        let mut store = PlainMem::new(TRIE_HASH_LEN as u64, 0u8);
        store.write(0, ZERO_HASH.deref());
        assert_eq!(TrieHash::hydrate(0, &store).unwrap(), ZERO_HASH);
    }
    #[test]
    fn test_partial_path_encoding() {
        let check = |steps: &[u8], term| {
            let (d, t) = PartialPath::decode(&PartialPath(steps.to_vec()).encode(term));
            assert_eq!(d.0, steps);
            assert_eq!(t, term);
        };
        for steps in [
            vec![0x1, 0x2, 0x3, 0x4],
            vec![0x1, 0x2, 0x3],
            vec![0x0, 0x1, 0x2],
            vec![0x1, 0x2],
            vec![0x1],
        ] {
            for term in [true, false] {
                check(&steps, term)
            }
        }
    }
    #[test]
    fn test_merkle_node_encoding() {
        let check = |node: Node| {
            let mut bytes = vec![0; node.dehydrated_len() as usize];
            node.dehydrate(&mut bytes).unwrap();

            let mut mem = PlainMem::new(bytes.len() as u64, 0x0);
            mem.write(0, &bytes);
            println!("{bytes:?}");
            let node_ = Node::hydrate(0, &mem).unwrap();
            assert!(node == node_);
        };
        let chd0 = [None; NBRANCH];
        let mut chd1 = chd0;
        for node in chd1.iter_mut().take(NBRANCH / 2) {
            *node = Some(DiskAddress::from(0xa));
        }
        let mut chd_encoded: [Option<Vec<u8>>; NBRANCH] = Default::default();
        for encoded in chd_encoded.iter_mut().take(NBRANCH / 2) {
            *encoded = Some(vec![0x1, 0x2, 0x3]);
        }
        for node in [
            Node::new_from_hash(
                None,
                None,
                NodeType::Leaf(LeafNode(
                    PartialPath(vec![0x1, 0x2, 0x3]),
                    Data(vec![0x4, 0x5]),
                )),
            ),
            Node::new_from_hash(
                None,
                None,
                NodeType::Extension(ExtNode {
                    path: PartialPath(vec![0x1, 0x2, 0x3]),
                    child: DiskAddress::from(0x42),
                    child_encoded: None,
                }),
            ),
            Node::new_from_hash(
                None,
                None,
                NodeType::Extension(ExtNode {
                    path: PartialPath(vec![0x1, 0x2, 0x3]),
                    child: DiskAddress::null(),
                    child_encoded: Some(vec![0x1, 0x2, 0x3]),
                }),
            ),
            Node::new_from_hash(
                None,
                None,
                NodeType::Branch(BranchNode {
                    children: chd0,
                    value: Some(Data("hello, world!".as_bytes().to_vec())),
                    children_encoded: Default::default(),
                }),
            ),
            Node::new_from_hash(
                None,
                None,
                NodeType::Branch(BranchNode {
                    children: chd1,
                    value: None,
                    children_encoded: chd_encoded,
                }),
            ),
        ] {
            check(node);
        }
    }
    #[test]
    fn test_encode() {
        const RESERVED: usize = 0x1000;

        let mut dm = shale::cached::DynamicMem::new(0x10000, 0);
        let compact_header = DiskAddress::null();
        dm.write(
            compact_header.into(),
            &shale::to_dehydrated(&shale::compact::CompactSpaceHeader::new(
                std::num::NonZeroUsize::new(RESERVED).unwrap(),
                std::num::NonZeroUsize::new(RESERVED).unwrap(),
            ))
            .unwrap(),
        );
        let compact_header = shale::StoredView::ptr_to_obj(
            &dm,
            compact_header,
            shale::compact::CompactHeader::MSIZE,
        )
        .unwrap();
        let mem_meta = Arc::new(dm);
        let mem_payload = Arc::new(DynamicMem::new(0x10000, 0x1));

        let cache = shale::ObjCache::new(1);
        let space =
            shale::compact::CompactSpace::new(mem_meta, mem_payload, compact_header, cache, 10, 16)
                .expect("CompactSpace init fail");

        let store = Box::new(space);
        let merkle = Merkle::new(store);

        {
            let chd = Node::new(NodeType::Leaf(LeafNode(
                PartialPath(vec![0x1, 0x2, 0x3]),
                Data(vec![0x4, 0x5]),
            )));
            let chd_ref = merkle.put_node(chd.clone()).unwrap();
            let chd_encoded = chd_ref.get_encoded(merkle.store.as_ref());
            let new_chd = Node::new(NodeType::decode(chd_encoded).unwrap());
            let new_chd_encoded = new_chd.get_encoded(merkle.store.as_ref());
            assert_eq!(chd_encoded, new_chd_encoded);

            let mut chd_encoded: [Option<Vec<u8>>; NBRANCH] = Default::default();
            chd_encoded[0] = Some(new_chd_encoded.to_vec());
            let node = Node::new(NodeType::Branch(BranchNode {
                children: [None; NBRANCH],
                value: Some(Data("value1".as_bytes().to_vec())),
                children_encoded: chd_encoded,
            }));

            let node_ref = merkle.put_node(node.clone()).unwrap();

            let r = node_ref.get_encoded(merkle.store.as_ref());
            let new_node = Node::new(NodeType::decode(r).unwrap());
            let new_encoded = new_node.get_encoded(merkle.store.as_ref());
            assert_eq!(r, new_encoded);
        }

        {
            let chd = Node::new(NodeType::Branch(BranchNode {
                children: [None; NBRANCH],
                value: Some(Data("value1".as_bytes().to_vec())),
                children_encoded: Default::default(),
            }));
            let chd_ref = merkle.put_node(chd.clone()).unwrap();
            let chd_encoded = chd_ref.get_encoded(merkle.store.as_ref());
            let new_chd = Node::new(NodeType::decode(chd_encoded).unwrap());
            let new_chd_encoded = new_chd.get_encoded(merkle.store.as_ref());
            assert_eq!(chd_encoded, new_chd_encoded);

            let node = Node::new(NodeType::Extension(ExtNode {
                path: PartialPath(vec![0x1, 0x2, 0x3]),
                child: DiskAddress::null(),
                child_encoded: Some(chd_encoded.to_vec()),
            }));
            let node_ref = merkle.put_node(node.clone()).unwrap();

            let r = node_ref.get_encoded(merkle.store.as_ref());
            let new_node = Node::new(NodeType::decode(r).unwrap());
            let new_encoded = new_node.get_encoded(merkle.store.as_ref());
            assert_eq!(r, new_encoded);
        }
    }
}
