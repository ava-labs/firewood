use crate::db::{MutStore, SharedStore};
// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.
use crate::nibbles::Nibbles;
use crate::shale::{
    self, disk_address::DiskAddress, ObjWriteError,
    ShaleError, ShaleStore,
};
use crate::v2::api;
use futures::{StreamExt, TryStreamExt};
use sha3::Digest;
use std::{
    cmp::Ordering, collections::HashMap, future::ready, io::Write, iter::once, marker::PhantomData,
    sync::OnceLock,
};
use thiserror::Error;

mod node;
pub mod proof;
mod stream;
mod trie_hash;

pub use node::{
    BinarySerde, Bincode, BranchNode, Data, EncodedNode, EncodedNodeType, ExtNode, LeafNode, Node,
    NodeType, PartialPath,
};
pub use proof::{Proof, ProofError};
pub use stream::MerkleKeyValueStream;
pub use trie_hash::{TrieHash, TRIE_HASH_LEN};

type NodeObjRef<'a> = shale::ObjRef<'a, Node>;
type ParentRefs<'a> = Vec<(NodeObjRef<'a>, u8)>;
type ParentAddresses = Vec<(DiskAddress, u8)>;

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
    #[error("merkle serde error: {0}")]
    BinarySerdeError(String),
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
pub struct Merkle<S, T> {
    store: Box<S>,
    phantom: PhantomData<T>,
}

impl<T> From<Merkle<MutStore, T>> for Merkle<SharedStore, T> {
    fn from(value: Merkle<MutStore, T>) -> Self {
        let store = value.store.into();
        Merkle {
            store: Box::new(store),
            phantom: PhantomData,
        }
    }
}

impl<S: ShaleStore<Node>, T> Merkle<S, T> {
    pub fn get_node(&self, ptr: DiskAddress) -> Result<NodeObjRef, MerkleError> {
        self.store.get_item(ptr).map_err(Into::into)
    }

    pub fn put_node(&self, node: Node) -> Result<NodeObjRef, MerkleError> {
        self.store.put_item(node, 0).map_err(Into::into)
    }

    fn free_node(&mut self, ptr: DiskAddress) -> Result<(), MerkleError> {
        self.store.free_item(ptr).map_err(Into::into)
    }
}

impl<'de, S, T> Merkle<S, T>
where
    S: ShaleStore<Node> + Send + Sync,
    T: BinarySerde,
    EncodedNode<T>: serde::Serialize + serde::Deserialize<'de>,
{
    pub fn new(store: Box<S>) -> Self {
        Self {
            store,
            phantom: PhantomData,
        }
    }

    // TODO: use `encode` / `decode` instead of `node.encode` / `node.decode` after extention node removal.
    #[allow(dead_code)]
    fn encode(&self, node: &NodeObjRef) -> Result<Vec<u8>, MerkleError> {
        let encoded = match node.inner() {
            NodeType::Leaf(n) => EncodedNode::new(EncodedNodeType::Leaf(n.clone())),
            NodeType::Branch(n) => {
                // pair up DiskAddresses with encoded children and pick the right one
                let encoded_children = n.chd().iter().zip(n.children_encoded.iter());
                let children = encoded_children
                    .map(|(child_addr, encoded_child)| {
                        child_addr
                            // if there's a child disk address here, get the encoded bytes
                            .map(|addr| self.get_node(addr).and_then(|node| self.encode(&node)))
                            // or look for the pre-fetched bytes
                            .or_else(|| encoded_child.as_ref().map(|child| Ok(child.to_vec())))
                            .transpose()
                    })
                    .collect::<Result<Vec<Option<Vec<u8>>>, MerkleError>>()?
                    .try_into()
                    .expect("MAX_CHILDREN will always be yielded");

                EncodedNode::new(EncodedNodeType::Branch {
                    children,
                    value: n.value.clone(),
                })
            }

            NodeType::Extension(_) => todo!(),
        };

        Bincode::serialize(&encoded).map_err(|e| MerkleError::BinarySerdeError(e.to_string()))
    }

    #[allow(dead_code)]
    fn decode(&self, buf: &'de [u8]) -> Result<NodeType, MerkleError> {
        let encoded: EncodedNode<T> =
            T::deserialize(buf).map_err(|e| MerkleError::BinarySerdeError(e.to_string()))?;

        match encoded.node {
            EncodedNodeType::Leaf(leaf) => Ok(NodeType::Leaf(leaf)),
            EncodedNodeType::Branch { children, value } => {
                let path = Vec::new().into();
                let value = value.map(|v| v.0);
                Ok(NodeType::Branch(
                    BranchNode::new(path, [None; BranchNode::MAX_CHILDREN], value, *children)
                        .into(),
                ))
            }
        }
    }
}

impl<S: ShaleStore<Node> + Send + Sync, T> Merkle<S, T> {
    pub fn init_root(&self) -> Result<DiskAddress, MerkleError> {
        self.store
            .put_item(
                Node::from_branch(BranchNode {
                    // path: vec![].into(),
                    children: [None; BranchNode::MAX_CHILDREN],
                    value: None,
                    children_encoded: Default::default(),
                }),
                Node::max_branch_node_size(),
            )
            .map_err(MerkleError::Shale)
            .map(|node| node.as_ptr())
    }

    pub fn get_store(&self) -> &S {
        self.store.as_ref()
    }

    pub fn empty_root() -> &'static TrieHash {
        static V: OnceLock<TrieHash> = OnceLock::new();
        #[allow(clippy::unwrap_used)]
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
            #[allow(clippy::unwrap_used)]
            if node.is_dirty() {
                node.write(|_| {}).unwrap();
                node.set_dirty(false);
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
            #[allow(clippy::unwrap_used)]
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
    fn split(
        &self,
        mut node_to_split: NodeObjRef,
        parents: &mut [(NodeObjRef, u8)],
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
                let path = node.inner.path_mut();

                *path = PartialPath(new_split_node_path.to_vec());

                node.rehash();
            })?;

            let new_node = Node::from_leaf(LeafNode::new(
                PartialPath(new_node_path.to_vec()),
                Data(val),
            ));
            let leaf_address = self.put_node(new_node)?.as_ptr();

            let mut chd = [None; BranchNode::MAX_CHILDREN];

            #[allow(clippy::indexing_slicing)]
            let last_matching_nibble = matching_path[idx];
            #[allow(clippy::indexing_slicing)]
            (chd[last_matching_nibble as usize] = Some(leaf_address));

            let address = match &node_to_split.inner {
                NodeType::Extension(u) if u.path.len() == 0 => {
                    deleted.push(node_to_split_address);
                    u.chd()
                }
                _ => node_to_split_address,
            };

            #[allow(clippy::indexing_slicing)]
            (chd[n_path[idx] as usize] = Some(address));

            let new_branch = Node::from_branch(BranchNode {
                // path: PartialPath(matching_path[..idx].to_vec()),
                children: chd,
                value: None,
                children_encoded: Default::default(),
            });

            let new_branch_address = self.put_node(new_branch)?.as_ptr();

            if idx > 0 {
                self.put_node(Node::from(NodeType::Extension(ExtNode {
                    #[allow(clippy::indexing_slicing)]
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
                                    NodeType::Leaf(u) => u.data = Data(val),
                                    NodeType::Extension(u) => {
                                        #[allow(clippy::unwrap_used)]
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
                        #[allow(clippy::unwrap_used)]
                        node_to_split
                            .write(|u| {
                                // TODO: handle unwraps better
                                let path = u.inner.path_mut();
                                #[allow(clippy::indexing_slicing)]
                                (*path = PartialPath(n_path[insert_path.len() + 1..].to_vec()));

                                u.rehash();
                            })
                            .unwrap();

                        let leaf_address = match &node_to_split.inner {
                            // TODO: handle BranchNode case
                            NodeType::Extension(u) if u.path.len() == 0 => {
                                deleted.push(node_to_split_address);
                                u.chd()
                            }
                            _ => node_to_split_address,
                        };

                        #[allow(clippy::indexing_slicing)]
                        (
                            leaf_address,
                            insert_path,
                            n_path[insert_path.len()] as usize,
                            Data(val).into(),
                        )
                    }
                    // insert path is greather than the path of the leaf
                    (Ordering::Greater, Some(n_value)) => {
                        let leaf = Node::from_leaf(LeafNode::new(
                            #[allow(clippy::indexing_slicing)]
                            PartialPath(insert_path[n_path.len() + 1..].to_vec()),
                            Data(val),
                        ));

                        let leaf_address = self.put_node(leaf)?.as_ptr();

                        deleted.push(node_to_split_address);

                        #[allow(clippy::indexing_slicing)]
                        (
                            leaf_address,
                            n_path.as_slice(),
                            insert_path[n_path.len()] as usize,
                            n_value.into(),
                        )
                    }
                };

            // [parent] (-> [ExtNode]) -> [branch with v] -> [Leaf]
            let mut children = [None; BranchNode::MAX_CHILDREN];

            #[allow(clippy::indexing_slicing)]
            (children[idx] = leaf_address.into());

            let branch_address = self
                .put_node(Node::from_branch(BranchNode {
                    children,
                    value,
                    children_encoded: Default::default(),
                }))?
                .as_ptr();

            if !prefix.is_empty() {
                self.put_node(Node::from(NodeType::Extension(ExtNode {
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
    ) -> Result<(impl Iterator<Item = NodeObjRef>, Vec<DiskAddress>), MerkleError> {
        // as we split a node, we need to track deleted nodes and parents
        let mut deleted = Vec::new();
        let mut parents = Vec::new();

        // we use Nibbles::<1> so that 1 zero nibble is at the front
        // this is for the sentinel node, which avoids moving the root
        // and always only has one child
        let mut key_nibbles = Nibbles::<1>::new(key.as_ref()).into_iter();

        let mut node = self.get_node(root)?;

        // walk down the merkle tree starting from next_node, currently the root
        // return None if the value is inserted
        let next_node_and_val = loop {
            let Some(current_nibble) = key_nibbles.next() else {
                break Some((node, val));
            };

            let (node_ref, next_node_ptr) = match &node.inner {
                // For a Branch node, we look at the child pointer. If it points
                // to another node, we walk down that. Otherwise, we can store our
                // value as a leaf and we're done
                NodeType::Leaf(n) => {
                    // we collided with another key; make a copy
                    // of the stored key to pass into split
                    let n_path = n.path.to_vec();
                    let n_value = Some(n.data.clone());
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

                NodeType::Branch(n) => {
                    #[allow(clippy::indexing_slicing)]
                    match n.children[current_nibble as usize] {
                        Some(c) => (node, c),
                        None => {
                            // insert the leaf to the empty slot
                            // create a new leaf
                            let leaf_ptr = self
                                .put_node(Node::from_leaf(LeafNode::new(
                                    PartialPath(key_nibbles.collect()),
                                    Data(val),
                                )))?
                                .as_ptr();
                            // set the current child to point to this leaf
                            #[allow(clippy::unwrap_used)]
                            node.write(|u| {
                                let uu = u.inner.as_branch_mut().unwrap();
                                #[allow(clippy::indexing_slicing)]
                                (uu.children[current_nibble as usize] = Some(leaf_ptr));
                                u.rehash();
                            })
                            .unwrap();

                            break None;
                        }
                    }
                }

                NodeType::Extension(n) => {
                    let n_path = n.path.to_vec();
                    let n_ptr = n.chd();
                    let rem_path = once(current_nibble)
                        .chain(key_nibbles.clone())
                        .collect::<Vec<_>>();
                    let n_path_len = n_path.len();
                    let node_ptr = node.as_ptr();

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
            parents.push((node_ref, current_nibble));
            node = self.get_node(next_node_ptr)?;
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
                                if n.path.len() == 0 {
                                    n.data = Data(val);

                                    None
                                } else {
                                    #[allow(clippy::indexing_slicing)]
                                    let idx = n.path[0];
                                    #[allow(clippy::indexing_slicing)]
                                    (n.path = PartialPath(n.path[1..].to_vec()));
                                    u.rehash();

                                    Some((idx, true, None, val))
                                }
                            }
                            NodeType::Extension(n) => {
                                #[allow(clippy::indexing_slicing)]
                                let idx = n.path[0];
                                let more = if n.path.len() > 1 {
                                    #[allow(clippy::indexing_slicing)]
                                    (n.path = PartialPath(n.path[1..].to_vec()));
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
                let mut chd = [None; BranchNode::MAX_CHILDREN];

                let c_ptr = if more {
                    u_ptr
                } else {
                    deleted.push(u_ptr);
                    #[allow(clippy::unwrap_used)]
                    ext.unwrap()
                };

                #[allow(clippy::indexing_slicing)]
                (chd[idx as usize] = Some(c_ptr));

                let branch = self
                    .put_node(Node::from_branch(BranchNode {
                        // path: vec![].into(),
                        children: chd,
                        value: Some(Data(val)),
                        children_encoded: Default::default(),
                    }))?
                    .as_ptr();

                set_parent(branch, &mut parents);
            }
        }

        Ok((parents.into_iter().rev().map(|(node, _)| node), deleted))
    }

    #[allow(clippy::unwrap_used)]
    fn after_remove_leaf(
        &self,
        parents: &mut ParentRefs,
        deleted: &mut Vec<DiskAddress>,
    ) -> Result<(), MerkleError> {
        let (b_chd, val) = {
            let (mut b_ref, b_idx) = parents.pop().unwrap();
            // the immediate parent of a leaf must be a branch
            #[allow(clippy::unwrap_used)]
            b_ref
                .write(|b| {
                    #[allow(clippy::indexing_slicing)]
                    (b.inner.as_branch_mut().unwrap().children[b_idx as usize] = None);
                    b.rehash()
                })
                .unwrap();
            #[allow(clippy::unwrap_used)]
            let b_inner = b_ref.inner.as_branch().unwrap();
            let (b_chd, has_chd) = b_inner.single_child();
            if (has_chd && (b_chd.is_none() || b_inner.value.is_some())) || parents.is_empty() {
                return Ok(());
            }
            deleted.push(b_ref.as_ptr());
            (b_chd, b_inner.value.clone())
        };
        #[allow(clippy::unwrap_used)]
        let (mut p_ref, p_idx) = parents.pop().unwrap();
        let p_ptr = p_ref.as_ptr();
        if let Some(val) = val {
            match &p_ref.inner {
                NodeType::Branch(_) => {
                    // from: [p: Branch] -> [b (v)]x -> [Leaf]x
                    // to: [p: Branch] -> [Leaf (v)]
                    let leaf = self
                        .put_node(Node::from_leaf(LeafNode::new(PartialPath(Vec::new()), val)))?
                        .as_ptr();
                    #[allow(clippy::unwrap_used)]
                    p_ref
                        .write(|p| {
                            #[allow(clippy::indexing_slicing)]
                            (p.inner.as_branch_mut().unwrap().children[p_idx as usize] =
                                Some(leaf));
                            p.rehash()
                        })
                        .unwrap();
                }
                NodeType::Extension(n) => {
                    // from: P -> [p: Ext]x -> [b (v)]x -> [leaf]x
                    // to: P -> [Leaf (v)]
                    let leaf = self
                        .put_node(Node::from_leaf(LeafNode::new(
                            PartialPath(n.path.clone().into_inner()),
                            val,
                        )))?
                        .as_ptr();
                    deleted.push(p_ptr);
                    set_parent(leaf, parents);
                }
                _ => unreachable!(),
            }
        } else {
            #[allow(clippy::unwrap_used)]
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
                                .put_node(Node::from(NodeType::Extension(ExtNode {
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
                                    NodeType::Leaf(n) => &mut n.path,
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

                            #[allow(clippy::unwrap_used)]
                            p_ref
                                .write(|p| {
                                    #[allow(clippy::indexing_slicing)]
                                    (p.inner.as_branch_mut().unwrap().children[p_idx as usize] =
                                        Some(c_ptr));
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
                                        NodeType::Leaf(n) => &mut n.path,
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
        parents: &mut ParentRefs,
        deleted: &mut Vec<DiskAddress>,
    ) -> Result<(), MerkleError> {
        // [b] -> [u] -> [c]
        #[allow(clippy::unwrap_used)]
        let (mut b_ref, b_idx) = parents.pop().unwrap();
        #[allow(clippy::unwrap_used)]
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
                                    #[allow(clippy::indexing_slicing)]
                                    (n.children[b_idx as usize] = Some(
                                        self.put_node(Node::from(NodeType::Extension(ExtNode {
                                            path: PartialPath(vec![idx]),
                                            child: c_ptr,
                                            child_encoded: None,
                                        })))?
                                        .as_ptr(),
                                    ));
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
                            NodeType::Leaf(n) => &mut n.path,
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
                    #[allow(clippy::unwrap_used)]
                    b_ref
                        .write(|b| {
                            #[allow(clippy::indexing_slicing)]
                            (b.inner.as_branch_mut().unwrap().children[b_idx as usize] =
                                Some(c_ptr));
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
                            NodeType::Leaf(n) => &mut n.path,
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
        if root.is_null() {
            return Ok(None);
        }

        let (found, parents, deleted) = {
            let (node_ref, mut parents) =
                self.get_node_and_parents_by_key(self.get_node(root)?, key)?;

            let Some(mut node_ref) = node_ref else {
                return Ok(None);
            };
            let mut deleted = Vec::new();
            let mut found = None;

            match &node_ref.inner {
                NodeType::Branch(n) => {
                    let (c_chd, _) = n.single_child();

                    #[allow(clippy::unwrap_used)]
                    node_ref
                        .write(|u| {
                            found = u.inner.as_branch_mut().unwrap().value.take();
                            u.rehash()
                        })
                        .unwrap();

                    if let Some((c_ptr, idx)) = c_chd {
                        deleted.push(node_ref.as_ptr());
                        self.after_remove_branch((c_ptr, idx), &mut parents, &mut deleted)?
                    }
                }

                NodeType::Leaf(n) => {
                    found = Some(n.data.clone());
                    deleted.push(node_ref.as_ptr());
                    self.after_remove_leaf(&mut parents, &mut deleted)?
                }
                _ => (),
            };

            (found, parents, deleted)
        };

        #[allow(clippy::unwrap_used)]
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

    fn get_node_by_key<'a, K: AsRef<[u8]>>(
        &'a self,
        node_ref: NodeObjRef<'a>,
        key: K,
    ) -> Result<Option<NodeObjRef<'a>>, MerkleError> {
        self.get_node_by_key_with_callbacks(node_ref, key, |_, _| {}, |_, _| {})
    }

    fn get_node_and_parents_by_key<'a, K: AsRef<[u8]>>(
        &'a self,
        node_ref: NodeObjRef<'a>,
        key: K,
    ) -> Result<(Option<NodeObjRef<'a>>, ParentRefs<'a>), MerkleError> {
        let mut parents = Vec::new();
        let node_ref = self.get_node_by_key_with_callbacks(
            node_ref,
            key,
            |_, _| {},
            |node_ref, nib| {
                parents.push((node_ref, nib));
            },
        )?;

        Ok((node_ref, parents))
    }

    fn get_node_and_parent_addresses_by_key<'a, K: AsRef<[u8]>>(
        &'a self,
        node_ref: NodeObjRef<'a>,
        key: K,
    ) -> Result<(Option<NodeObjRef<'a>>, ParentAddresses), MerkleError> {
        let mut parents = Vec::new();
        let node_ref = self.get_node_by_key_with_callbacks(
            node_ref,
            key,
            |_, _| {},
            |node_ref, nib| {
                parents.push((node_ref.into_ptr(), nib));
            },
        )?;

        Ok((node_ref, parents))
    }

    fn get_node_by_key_with_callbacks<'a, K: AsRef<[u8]>>(
        &'a self,
        mut node_ref: NodeObjRef<'a>,
        key: K,
        mut start_loop_callback: impl FnMut(DiskAddress, u8),
        mut end_loop_callback: impl FnMut(NodeObjRef<'a>, u8),
    ) -> Result<Option<NodeObjRef<'a>>, MerkleError> {
        let mut key_nibbles = Nibbles::<1>::new(key.as_ref()).into_iter();

        loop {
            let Some(nib) = key_nibbles.next() else {
                break;
            };

            start_loop_callback(node_ref.as_ptr(), nib);

            let next_ptr = match &node_ref.inner {
                #[allow(clippy::indexing_slicing)]
                NodeType::Branch(n) => match n.children[nib as usize] {
                    Some(c) => c,
                    None => return Ok(None),
                },
                NodeType::Leaf(n) => {
                    let node_ref = if once(nib).chain(key_nibbles).eq(n.path.iter().copied()) {
                        Some(node_ref)
                    } else {
                        None
                    };

                    return Ok(node_ref);
                }
                NodeType::Extension(n) => {
                    let mut n_path_iter = n.path.iter().copied();

                    if n_path_iter.next() != Some(nib) {
                        return Ok(None);
                    }

                    let path_matches = n_path_iter
                        .map(Some)
                        .all(|n_path_nibble| key_nibbles.next() == n_path_nibble);

                    if !path_matches {
                        return Ok(None);
                    }

                    n.chd()
                }
            };

            end_loop_callback(node_ref, nib);

            node_ref = self.get_node(next_ptr)?;
        }

        // when we're done iterating over nibbles, check if the node we're at has a value
        let node_ref = match &node_ref.inner {
            NodeType::Branch(n) if n.value.as_ref().is_some() => Some(node_ref),
            NodeType::Leaf(n) if n.path.len() == 0 => Some(node_ref),
            _ => None,
        };

        Ok(node_ref)
    }

    pub fn get_mut<K: AsRef<[u8]>>(
        &mut self,
        key: K,
        root: DiskAddress,
    ) -> Result<Option<RefMut<S, T>>, MerkleError> {
        if root.is_null() {
            return Ok(None);
        }

        let (ptr, parents) = {
            let root_node = self.get_node(root)?;
            let (node_ref, parents) = self.get_node_and_parent_addresses_by_key(root_node, key)?;

            (node_ref.map(|n| n.into_ptr()), parents)
        };

        Ok(ptr.map(|ptr| RefMut::new(ptr, parents, self)))
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
        let mut proofs = HashMap::new();
        if root.is_null() {
            return Ok(Proof(proofs));
        }

        let root_node = self.get_node(root)?;

        let mut nodes = Vec::new();

        let node = self.get_node_by_key_with_callbacks(
            root_node,
            key,
            |node, _| nodes.push(node),
            |_, _| {},
        )?;

        if let Some(node) = node {
            nodes.push(node.as_ptr());
        }

        // Get the hashes of the nodes.
        for node in nodes.into_iter().skip(1) {
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

        let root_node = self.get_node(root)?;
        let node_ref = self.get_node_by_key(root_node, key)?;

        Ok(node_ref.map(Ref))
    }

    pub fn flush_dirty(&self) -> Option<()> {
        self.store.flush_dirty()
    }

    pub(crate) fn iter(&self, root: DiskAddress) -> MerkleKeyValueStream<'_, S, T> {
        MerkleKeyValueStream::new(self, root)
    }

    pub(crate) fn iter_from(
        &self,
        root: DiskAddress,
        key: Box<[u8]>,
    ) -> MerkleKeyValueStream<'_, S, T> {
        MerkleKeyValueStream::from_key(self, root, key)
    }

    pub(super) async fn range_proof<K: api::KeyType + Send + Sync>(
        &self,
        root: DiskAddress,
        first_key: Option<K>,
        last_key: Option<K>,
        limit: Option<usize>,
    ) -> Result<Option<api::RangeProof<Vec<u8>, Vec<u8>>>, api::Error> {
        if let (Some(k1), Some(k2)) = (&first_key, &last_key) {
            if k1.as_ref() > k2.as_ref() {
                return Err(api::Error::InvalidRange {
                    first_key: k1.as_ref().to_vec(),
                    last_key: k2.as_ref().to_vec(),
                });
            }
        }

        // limit of 0 is always an empty RangeProof
        if limit == Some(0) {
            return Ok(None);
        }

        let mut stream = match first_key {
            // TODO: fix the call-site to force the caller to do the allocation
            Some(key) => self.iter_from(root, key.as_ref().to_vec().into_boxed_slice()),
            None => self.iter(root),
        };

        // fetch the first key from the stream
        let first_result = stream.next().await;

        // transpose the Option<Result<T, E>> to Result<Option<T>, E>
        // If this is an error, the ? operator will return it
        let Some((first_key, first_data)) = first_result.transpose()? else {
            // nothing returned, either the trie is empty or the key wasn't found
            return Ok(None);
        };

        let first_key_proof = self
            .prove(&first_key, root)
            .map_err(|e| api::Error::InternalError(Box::new(e)))?;
        let limit = limit.map(|old_limit| old_limit - 1);

        let mut middle = vec![(first_key.into_vec(), first_data)];

        // we stop streaming if either we hit the limit or the key returned was larger
        // than the largest key requested
        #[allow(clippy::unwrap_used)]
        middle.extend(
            stream
                .take(limit.unwrap_or(usize::MAX))
                .take_while(|kv_result| {
                    // no last key asked for, so keep going
                    let Some(last_key) = last_key.as_ref() else {
                        return ready(true);
                    };

                    // return the error if there was one
                    let Ok(kv) = kv_result else {
                        return ready(true);
                    };

                    // keep going if the key returned is less than the last key requested
                    ready(&*kv.0 <= last_key.as_ref())
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
                .prove(last_key, root)
                .map_err(|e| api::Error::InternalError(Box::new(e)))?,
        };

        Ok(Some(api::RangeProof {
            first_key_proof,
            middle,
            last_key_proof,
        }))
    }
}

fn set_parent(new_chd: DiskAddress, parents: &mut [(NodeObjRef, u8)]) {
    #[allow(clippy::unwrap_used)]
    let (p_ref, idx) = parents.last_mut().unwrap();
    #[allow(clippy::unwrap_used)]
    p_ref
        .write(|p| {
            match &mut p.inner {
                #[allow(clippy::indexing_slicing)]
                NodeType::Branch(pp) => pp.children[*idx as usize] = Some(new_chd),
                NodeType::Extension(pp) => *pp.chd_mut() = new_chd,
                _ => unreachable!(),
            }
            p.rehash();
        })
        .unwrap();
}

pub struct Ref<'a>(NodeObjRef<'a>);

pub struct RefMut<'a, S, T> {
    ptr: DiskAddress,
    parents: ParentAddresses,
    merkle: &'a mut Merkle<S, T>,
}

impl<'a> std::ops::Deref for Ref<'a> {
    type Target = [u8];
    #[allow(clippy::unwrap_used)]
    fn deref(&self) -> &[u8] {
        match &self.0.inner {
            NodeType::Branch(n) => n.value.as_ref().unwrap(),
            NodeType::Leaf(n) => &n.data,
            _ => unreachable!(),
        }
    }
}

impl<'a, S: ShaleStore<Node> + Send + Sync, T> RefMut<'a, S, T> {
    fn new(ptr: DiskAddress, parents: ParentAddresses, merkle: &'a mut Merkle<S, T>) -> Self {
        Self {
            ptr,
            parents,
            merkle,
        }
    }

    #[allow(clippy::unwrap_used)]
    pub fn get(&self) -> Ref {
        Ref(self.merkle.get_node(self.ptr).unwrap())
    }

    pub fn write(&mut self, modify: impl FnOnce(&mut Vec<u8>)) -> Result<(), MerkleError> {
        let mut deleted = Vec::new();
        #[allow(clippy::unwrap_used)]
        {
            let mut u_ref = self.merkle.get_node(self.ptr).unwrap();
            #[allow(clippy::unwrap_used)]
            let mut parents: Vec<_> = self
                .parents
                .iter()
                .map(|(ptr, nib)| (self.merkle.get_node(*ptr).unwrap(), *nib))
                .collect();
            write_node!(
                self.merkle,
                u_ref,
                |u| {
                    #[allow(clippy::unwrap_used)]
                    modify(match &mut u.inner {
                        NodeType::Branch(n) => &mut n.value.as_mut().unwrap().0,
                        NodeType::Leaf(n) => &mut n.data.0,
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
pub const fn to_nibble_array(x: u8) -> [u8; 2] {
    [x >> 4, x & 0b_0000_1111]
}

// given a set of nibbles, take each pair and convert this back into bytes
// if an odd number of nibbles, in debug mode it panics. In release mode,
// the final nibble is dropped
pub fn from_nibbles(nibbles: &[u8]) -> impl Iterator<Item = u8> + '_ {
    debug_assert_eq!(nibbles.len() & 1, 0);
    #[allow(clippy::indexing_slicing)]
    nibbles.chunks_exact(2).map(|p| (p[0] << 4) | p[1])
}

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used)]
mod tests {
    use crate::merkle::node::PlainCodec;

    use super::*;
    use node::tests::{extension, leaf};
    use shale::{cached::DynamicMem, compact::CompactSpace, CachedStore};
    use test_case::test_case;

    #[test_case(vec![0x12, 0x34, 0x56], &[0x1, 0x2, 0x3, 0x4, 0x5, 0x6])]
    #[test_case(vec![0xc0, 0xff], &[0xc, 0x0, 0xf, 0xf])]
    fn to_nibbles(bytes: Vec<u8>, nibbles: &[u8]) {
        let n: Vec<_> = bytes.into_iter().flat_map(to_nibble_array).collect();
        assert_eq!(n, nibbles);
    }

    fn create_generic_test_merkle<'de, T>() -> Merkle<CompactSpace<Node, DynamicMem>, T>
    where
        T: BinarySerde,
        EncodedNode<T>: serde::Serialize + serde::Deserialize<'de>,
    {
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
        let mem_meta = dm;
        let mem_payload = DynamicMem::new(0x10000, 0x1);

        let cache = shale::ObjCache::new(1);
        let space =
            shale::compact::CompactSpace::new(mem_meta, mem_payload, compact_header, cache, 10, 16)
                .expect("CompactSpace init fail");

        let store = Box::new(space);
        Merkle::new(store)
    }

    pub(super) fn create_test_merkle() -> Merkle<CompactSpace<Node, DynamicMem>, Bincode> {
        create_generic_test_merkle::<Bincode>()
    }

    fn branch(value: Option<Vec<u8>>, encoded_child: Option<Vec<u8>>) -> Node {
        let children = Default::default();
        let value = value.map(Data);
        let mut children_encoded = <[Option<Vec<u8>>; BranchNode::MAX_CHILDREN]>::default();

        if let Some(child) = encoded_child {
            children_encoded[0] = Some(child);
        }

        Node::from_branch(BranchNode {
            // path: vec![].into(),
            children,
            value,
            children_encoded,
        })
    }

    #[test_case(leaf(Vec::new(), Vec::new()) ; "empty leaf encoding")]
    #[test_case(leaf(vec![1, 2, 3], vec![4, 5]) ; "leaf encoding")]
    #[test_case(branch(Some(b"value".to_vec()), vec![1, 2, 3].into()) ; "branch with chd")]
    #[test_case(branch(Some(b"value".to_vec()), None); "branch without chd")]
    #[test_case(branch(None, None); "branch without value and chd")]
    #[test_case(extension(vec![1, 2, 3], DiskAddress::null(), vec![4, 5].into()) ; "extension without child address")]
    fn encode_(node: Node) {
        let merkle = create_test_merkle();

        let node_ref = merkle.put_node(node).unwrap();
        let encoded = node_ref.get_encoded(merkle.store.as_ref());
        let new_node = Node::from(NodeType::decode(encoded).unwrap());
        let new_node_encoded = new_node.get_encoded(merkle.store.as_ref());

        assert_eq!(encoded, new_node_encoded);
    }

    #[test_case(Bincode::new(), leaf(Vec::new(), Vec::new()) ; "empty leaf encoding with Bincode")]
    #[test_case(Bincode::new(), leaf(vec![1, 2, 3], vec![4, 5]) ; "leaf encoding with Bincode")]
    #[test_case(Bincode::new(), branch(Some(b"value".to_vec()), vec![1, 2, 3].into()) ; "branch with chd with Bincode")]
    #[test_case(Bincode::new(), branch(Some(b"value".to_vec()), None); "branch without chd with Bincode")]
    #[test_case(Bincode::new(), branch(None, None); "branch without value and chd with Bincode")]
    #[test_case(PlainCodec::new(), leaf(Vec::new(), Vec::new()) ; "empty leaf encoding with PlainCodec")]
    #[test_case(PlainCodec::new(), leaf(vec![1, 2, 3], vec![4, 5]) ; "leaf encoding with PlainCodec")]
    #[test_case(PlainCodec::new(), branch(Some(b"value".to_vec()), vec![1, 2, 3].into()) ; "branch with chd with PlainCodec")]
    #[test_case(PlainCodec::new(), branch(Some(b"value".to_vec()), Some(Vec::new())); "branch with empty chd with PlainCodec")]
    #[test_case(PlainCodec::new(), branch(Some(Vec::new()), vec![1, 2, 3].into()); "branch with empty value with PlainCodec")]
    fn node_encode_decode<T>(_codec: T, node: Node)
    where
        T: BinarySerde,
        for<'de> EncodedNode<T>: serde::Serialize + serde::Deserialize<'de>,
    {
        let merkle = create_generic_test_merkle::<T>();
        let node_ref = merkle.put_node(node.clone()).unwrap();

        let encoded = merkle.encode(&node_ref).unwrap();
        let new_node = Node::from(merkle.decode(encoded.as_ref()).unwrap());

        assert_eq!(node, new_node);
    }

    #[test]
    fn insert_and_retrieve_one() {
        let key = b"hello";
        let val = b"world";

        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        merkle.insert(key, val.to_vec(), root).unwrap();

        let fetched_val = merkle.get(key, root).unwrap();

        assert_eq!(fetched_val.as_deref(), val.as_slice().into());
    }

    #[test]
    fn insert_and_retrieve_multiple() {
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        // insert values
        for key_val in u8::MIN..=u8::MAX {
            let key = vec![key_val];
            let val = vec![key_val];

            merkle.insert(&key, val.clone(), root).unwrap();

            let fetched_val = merkle.get(&key, root).unwrap();

            // make sure the value was inserted
            assert_eq!(fetched_val.as_deref(), val.as_slice().into());
        }

        // make sure none of the previous values were forgotten after initial insert
        for key_val in u8::MIN..=u8::MAX {
            let key = vec![key_val];
            let val = vec![key_val];

            let fetched_val = merkle.get(&key, root).unwrap();

            assert_eq!(fetched_val.as_deref(), val.as_slice().into());
        }
    }

    #[test]
    fn remove_one() {
        let key = b"hello";
        let val = b"world";

        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        merkle.insert(key, val.to_vec(), root).unwrap();

        assert_eq!(
            merkle.get(key, root).unwrap().as_deref(),
            val.as_slice().into()
        );

        let removed_val = merkle.remove(key, root).unwrap();
        assert_eq!(removed_val.as_deref(), val.as_slice().into());

        let fetched_val = merkle.get(key, root).unwrap();
        assert!(fetched_val.is_none());
    }

    #[test]
    fn remove_many() {
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        // insert values
        for key_val in u8::MIN..=u8::MAX {
            let key = &[key_val];
            let val = &[key_val];

            merkle.insert(key, val.to_vec(), root).unwrap();

            let fetched_val = merkle.get(key, root).unwrap();

            // make sure the value was inserted
            assert_eq!(fetched_val.as_deref(), val.as_slice().into());
        }

        // remove values
        for key_val in u8::MIN..=u8::MAX {
            let key = &[key_val];
            let val = &[key_val];

            let removed_val = merkle.remove(key, root).unwrap();
            assert_eq!(removed_val.as_deref(), val.as_slice().into());

            let fetched_val = merkle.get(key, root).unwrap();
            assert!(fetched_val.is_none());
        }
    }

    #[test]
    fn get_empty_proof() {
        let merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        let proof = merkle.prove(b"any-key", root).unwrap();

        assert!(proof.0.is_empty());
    }

    #[tokio::test]
    async fn empty_range_proof() {
        let merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        assert!(merkle
            .range_proof::<&[u8]>(root, None, None, None)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn range_proof_invalid_bounds() {
        let merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();
        let start_key = &[0x01];
        let end_key = &[0x00];

        match merkle
            .range_proof::<&[u8]>(root, Some(start_key), Some(end_key), Some(1))
            .await
        {
            Err(api::Error::InvalidRange {
                first_key,
                last_key,
            }) if first_key == start_key && last_key == end_key => (),
            Err(api::Error::InvalidRange { .. }) => panic!("wrong bounds on InvalidRange error"),
            _ => panic!("expected InvalidRange error"),
        }
    }

    #[tokio::test]
    async fn full_range_proof() {
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();
        // insert values
        for key_val in u8::MIN..=u8::MAX {
            let key = &[key_val];
            let val = &[key_val];

            merkle.insert(key, val.to_vec(), root).unwrap();
        }
        merkle.flush_dirty();

        let rangeproof = merkle
            .range_proof::<&[u8]>(root, None, None, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rangeproof.middle.len(), u8::MAX as usize + 1);
        assert_ne!(rangeproof.first_key_proof.0, rangeproof.last_key_proof.0);
        let left_proof = merkle.prove([u8::MIN], root).unwrap();
        let right_proof = merkle.prove([u8::MAX], root).unwrap();
        assert_eq!(rangeproof.first_key_proof.0, left_proof.0);
        assert_eq!(rangeproof.last_key_proof.0, right_proof.0);
    }

    #[tokio::test]
    async fn single_value_range_proof() {
        const RANDOM_KEY: u8 = 42;

        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();
        // insert values
        for key_val in u8::MIN..=u8::MAX {
            let key = &[key_val];
            let val = &[key_val];

            merkle.insert(key, val.to_vec(), root).unwrap();
        }
        merkle.flush_dirty();

        let rangeproof = merkle
            .range_proof(root, Some([RANDOM_KEY]), None, Some(1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(rangeproof.first_key_proof.0, rangeproof.last_key_proof.0);
        assert_eq!(rangeproof.middle.len(), 1);
    }

    #[test]
    fn shared_path_proof() {
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        let key1 = b"key1";
        let value1 = b"1";
        merkle.insert(key1, value1.to_vec(), root).unwrap();

        let key2 = b"key2";
        let value2 = b"2";
        merkle.insert(key2, value2.to_vec(), root).unwrap();

        let root_hash = merkle.root_hash(root).unwrap();

        let verified = {
            let key = key1;
            let proof = merkle.prove(key, root).unwrap();
            proof.verify(key, root_hash.0).unwrap()
        };

        assert_eq!(verified, Some(value1.to_vec()));

        let verified = {
            let key = key2;
            let proof = merkle.prove(key, root).unwrap();
            proof.verify(key, root_hash.0).unwrap()
        };

        assert_eq!(verified, Some(value2.to_vec()));
    }

    // this was a specific failing case
    #[test]
    fn shared_path_on_insert() {
        type Bytes = &'static [u8];
        let pairs: Vec<(Bytes, Bytes)> = vec![
            (
                &[1, 1, 46, 82, 67, 218],
                &[23, 252, 128, 144, 235, 202, 124, 243],
            ),
            (
                &[1, 0, 0, 1, 1, 0, 63, 80],
                &[99, 82, 31, 213, 180, 196, 49, 242],
            ),
            (
                &[0, 0, 0, 169, 176, 15],
                &[105, 211, 176, 51, 231, 182, 74, 207],
            ),
            (
                &[1, 0, 0, 0, 53, 57, 93],
                &[234, 139, 214, 220, 172, 38, 168, 164],
            ),
        ];

        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        for (key, val) in &pairs {
            let val = val.to_vec();
            merkle.insert(key, val.clone(), root).unwrap();

            let fetched_val = merkle.get(key, root).unwrap();

            // make sure the value was inserted
            assert_eq!(fetched_val.as_deref(), val.as_slice().into());
        }

        for (key, val) in pairs {
            let fetched_val = merkle.get(key, root).unwrap();

            // make sure the value was inserted
            assert_eq!(fetched_val.as_deref(), val.into());
        }
    }

    #[test]
    fn overwrite_leaf() {
        let key = vec![0x00];
        let val = vec![1];
        let overwrite = vec![2];

        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        merkle.insert(&key, val.clone(), root).unwrap();

        assert_eq!(
            merkle.get(&key, root).unwrap().as_deref(),
            Some(val.as_slice())
        );

        merkle.insert(&key, overwrite.clone(), root).unwrap();

        assert_eq!(
            merkle.get(&key, root).unwrap().as_deref(),
            Some(overwrite.as_slice())
        );
    }

    #[test]
    fn new_leaf_is_a_child_of_the_old_leaf() {
        let key = vec![0xff];
        let val = vec![1];
        let key_2 = vec![0xff, 0x00];
        let val_2 = vec![2];

        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        merkle.insert(&key, val.clone(), root).unwrap();
        merkle.insert(&key_2, val_2.clone(), root).unwrap();

        assert_eq!(
            merkle.get(&key, root).unwrap().as_deref(),
            Some(val.as_slice())
        );

        assert_eq!(
            merkle.get(&key_2, root).unwrap().as_deref(),
            Some(val_2.as_slice())
        );
    }

    #[test]
    fn old_leaf_is_a_child_of_the_new_leaf() {
        let key = vec![0xff, 0x00];
        let val = vec![1];
        let key_2 = vec![0xff];
        let val_2 = vec![2];

        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        merkle.insert(&key, val.clone(), root).unwrap();
        merkle.insert(&key_2, val_2.clone(), root).unwrap();

        assert_eq!(
            merkle.get(&key, root).unwrap().as_deref(),
            Some(val.as_slice())
        );

        assert_eq!(
            merkle.get(&key_2, root).unwrap().as_deref(),
            Some(val_2.as_slice())
        );
    }

    #[test]
    fn new_leaf_is_sibling_of_old_leaf() {
        let key = vec![0xff];
        let val = vec![1];
        let key_2 = vec![0xff, 0x00];
        let val_2 = vec![2];
        let key_3 = vec![0xff, 0x0f];
        let val_3 = vec![3];

        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        merkle.insert(&key, val.clone(), root).unwrap();
        merkle.insert(&key_2, val_2.clone(), root).unwrap();
        merkle.insert(&key_3, val_3.clone(), root).unwrap();

        assert_eq!(
            merkle.get(&key, root).unwrap().as_deref(),
            Some(val.as_slice())
        );

        assert_eq!(
            merkle.get(&key_2, root).unwrap().as_deref(),
            Some(val_2.as_slice())
        );

        assert_eq!(
            merkle.get(&key_3, root).unwrap().as_deref(),
            Some(val_3.as_slice())
        );
    }

    #[test]
    fn old_branch_is_a_child_of_new_branch() {
        let key = vec![0xff, 0xf0];
        let val = vec![1];
        let key_2 = vec![0xff, 0xf0, 0x00];
        let val_2 = vec![2];
        let key_3 = vec![0xff];
        let val_3 = vec![3];

        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        merkle.insert(&key, val.clone(), root).unwrap();
        merkle.insert(&key_2, val_2.clone(), root).unwrap();
        merkle.insert(&key_3, val_3.clone(), root).unwrap();

        assert_eq!(
            merkle.get(&key, root).unwrap().as_deref(),
            Some(val.as_slice())
        );

        assert_eq!(
            merkle.get(&key_2, root).unwrap().as_deref(),
            Some(val_2.as_slice())
        );

        assert_eq!(
            merkle.get(&key_3, root).unwrap().as_deref(),
            Some(val_3.as_slice())
        );
    }

    #[test]
    fn overlapping_branch_insert() {
        let key = vec![0xff];
        let val = vec![1];
        let key_2 = vec![0xff, 0x00];
        let val_2 = vec![2];

        let overwrite = vec![3];

        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        merkle.insert(&key, val.clone(), root).unwrap();
        merkle.insert(&key_2, val_2.clone(), root).unwrap();

        assert_eq!(
            merkle.get(&key, root).unwrap().as_deref(),
            Some(val.as_slice())
        );

        assert_eq!(
            merkle.get(&key_2, root).unwrap().as_deref(),
            Some(val_2.as_slice())
        );

        merkle.insert(&key, overwrite.clone(), root).unwrap();

        assert_eq!(
            merkle.get(&key, root).unwrap().as_deref(),
            Some(overwrite.as_slice())
        );

        assert_eq!(
            merkle.get(&key_2, root).unwrap().as_deref(),
            Some(val_2.as_slice())
        );
    }

    #[test]
    fn single_key_proof_with_one_node() {
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();
        let key = b"key";
        let value = b"value";

        merkle.insert(key, value.to_vec(), root).unwrap();
        let root_hash = merkle.root_hash(root).unwrap();

        let proof = merkle.prove(key, root).unwrap();

        let verified = proof.verify(key, root_hash.0).unwrap();

        assert_eq!(verified, Some(value.to_vec()));
    }

    #[test]
    fn two_key_proof_without_shared_path() {
        let mut merkle = create_test_merkle();
        let root = merkle.init_root().unwrap();

        let key1 = &[0x00];
        let key2 = &[0xff];

        merkle.insert(key1, key1.to_vec(), root).unwrap();
        merkle.insert(key2, key2.to_vec(), root).unwrap();

        let root_hash = merkle.root_hash(root).unwrap();

        let verified = {
            let proof = merkle.prove(key1, root).unwrap();
            proof.verify(key1, root_hash.0).unwrap()
        };

        assert_eq!(verified.as_deref(), Some(key1.as_slice()));
    }
}
