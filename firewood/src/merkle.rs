use crate::nibbles::Nibbles;
use crate::node::path::Path;
// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.
use crate::node::{BranchNode, LeafNode, Node};
use crate::proof::{Proof, ProofError};
use crate::storage::hashednode::HashedNodeStore;
use crate::storage::linear::{ReadLinearStore, WriteLinearStore};
use crate::storage::node::{LinearAddress, UpdateError};
use crate::stream::{MerkleKeyValueStream, NodeWithKey, PathIterator};
use crate::trie_hash::TrieHash;
use crate::v2::api;
use futures::{StreamExt, TryStreamExt};
use std::future::ready;
use std::io::Write;

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

impl<T: ReadLinearStore> Merkle<T> {
    const EMPTY_HASH: TrieHash = TrieHash([0; 32]);
}

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

impl<T: ReadLinearStore> Merkle<T> {
    pub const fn new(store: HashedNodeStore<T>) -> Merkle<T> {
        Merkle(store)
    }

    // TODO: remove me; callers should use [Merkle::read_node]
    pub fn get_node(&self, _addr: LinearAddress) -> Result<&Node, MerkleError> {
        todo!()
    }

    pub fn root_address(&self) -> Option<LinearAddress> {
        self.0.root_address()
    }

    // TODO: can we make this &self instead of &mut self?
    pub fn root_hash(&mut self) -> Result<TrieHash, std::io::Error> {
        let root = self.root_address();
        match root {
            None => Ok(Self::EMPTY_HASH),
            Some(root) => {
                // TODO: We might be able to get the hash without reading the node...
                let root_node = self.read_node(root)?;
                root_node.hash(root, &mut self.0)
            }
        }
    }

    fn _get_node_by_key<K: AsRef<[u8]>>(&self, _key: K) -> Result<Option<&Node>, MerkleError> {
        todo!()
    }

    /// Constructs a merkle proof for key. The result contains all encoded nodes
    /// on the path to the value at key. The value itself is also included in the
    /// last node and can be retrieved by verifying the proof.
    ///
    /// If the trie does not contain a value for key, the returned proof contains
    /// all nodes of the longest existing prefix of the key, ending with the node
    /// that proves the absence of the key (at least the root node).
    pub fn prove<K>(&self, _key: K) -> Result<Proof<Vec<u8>>, MerkleError>
    where
        K: AsRef<[u8]>,
    {
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

    pub fn get<K: AsRef<[u8]>>(&self, _key: K) -> Result<Option<Box<[u8]>>, MerkleError> {
        todo!()
        // TODO danlaine use or remove the code below
        // if root_addr.is_null() {
        //     todo!()
        //         return Ok(None);
        //     }

        //     let root_node = self.get_node(root_addr)?;
        //     let node_ref = self.get_node_by_key(root_node, key)?;

        //     Ok(node_ref.map(Ref))
    }

    pub fn verify_proof<N: AsRef<[u8]> + Send, K: AsRef<[u8]>>(
        &self,
        _key: K,
        _proof: &Proof<N>,
    ) -> Result<Option<Vec<u8>>, MerkleError> {
        todo!()
    }

    pub fn verify_range_proof<N: AsRef<[u8]> + Send, K: AsRef<[u8]>, V: AsRef<[u8]>>(
        &self,
        _proof: &Proof<N>,
        _first_key: K,
        _last_key: K,
        _keys: Vec<K>,
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
        MerkleKeyValueStream::_from_key(self, key)
    }

    pub(super) async fn _range_proof<K: api::KeyType + Send + Sync>(
        &self,
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
            Some(key) => self._key_value_iter_from_key(key.as_ref().to_vec().into_boxed_slice()),
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
        writer: &mut dyn Write,
    ) -> Result<(), std::io::Error> {
        write!(writer, "{addr} ")?;
        let _node = self.read_node(addr)?;
        todo!()
    }
    pub fn dump(&self) -> Result<String, std::io::Error> {
        let mut result = vec![];
        match self.root_address() {
            Some(addr) => self.dump_node(addr, &mut result)?,
            None => write!(&mut result, "[empty merkle]")?,
        }

        Ok(String::from_utf8_lossy(&result).to_string())
    }
}

impl<T: WriteLinearStore> Merkle<T> {
    pub fn insert<K: AsRef<[u8]>>(&mut self, key: K, val: Vec<u8>) -> Result<(), MerkleError> {
        for addr in self.insert_and_return_ancestors(key, val)? {
            self.0.invalidate_hash(addr);
        }
        Ok(())
    }

    pub fn insert_and_return_ancestors<K: AsRef<[u8]>>(
        &mut self,
        key: K,
        val: Vec<u8>,
    ) -> Result<Vec<LinearAddress>, MerkleError> {
        let mut traversal_path = PathIterator::new(self, key.as_ref())?
            .collect::<Result<Vec<NodeWithKey>, MerkleError>>()?;

        let hash_invalidation_addresses: Vec<_> =
            traversal_path.iter().map(|item| item.addr).collect();

        let Some(last_node) = traversal_path.pop() else {
            let key_nibbles = Nibbles::new(key.as_ref()).into_iter();

            // no root, so create a leaf with this value
            let leaf = Node::Leaf(crate::node::LeafNode {
                partial_path: Path::from_nibbles(key_nibbles),
                value: val,
            });
            let new_root = self.create_node(&leaf)?;
            self.set_root(new_root)?;
            return Ok(Default::default());
        };

        if last_node.key.as_ref() == key.as_ref() {
            match last_node.node {
                Node::Branch(_) => todo!(),
                Node::Leaf(leaf) => {
                    let new_leaf = Node::Leaf(LeafNode {
                        value: val,
                        partial_path: leaf.partial_path.clone(),
                    });
                    let addr = last_node.addr;
                    // TODO: can we remove this clone?
                    let parent = traversal_path.pop().map(|parent_node| {
                        (
                            parent_node.addr,
                            parent_node.node.as_branch().unwrap().clone(),
                        )
                    });
                    match self.update_node(addr, &new_leaf) {
                        Err(crate::storage::node::UpdateError::NodeMoved(new_addr)) => {
                            // update the parent to point to the new node address
                            let Some(branch) = parent else {
                                self.set_root(new_addr)?;
                                return Ok(Default::default());
                            };
                            let mut new_children = branch.1.children;
                            *new_children
                                .iter_mut()
                                .find(|child_addr| **child_addr == Some(addr))
                                .unwrap() = Some(new_addr);
                            let new_branch = Node::Branch(Box::new(BranchNode {
                                children: new_children,
                                partial_path: branch.1.partial_path,
                                value: branch.1.value,
                            }));
                            self.update_node(branch.0, &new_branch)
                                .map_err(|ue| match ue {
                                    UpdateError::Io(e) => MerkleError::Format(e),
                                    UpdateError::NodeMoved(_) => unreachable!(
                                        "only changed the child address, node can't grow"
                                    ),
                                })?;
                        }
                        Err(UpdateError::Io(e)) => return Err(e.into()),
                        _ => {}
                    }
                }
            }
        } else {
            todo!()
        }

        Ok(hash_invalidation_addresses)
    }

    pub fn remove<K: AsRef<[u8]>>(&mut self, _key: K) -> Result<Option<Vec<u8>>, MerkleError> {
        // let Some(root_address) = self.root_address() else {
        //     return Ok(None);
        // };
        todo!()
    }
}

impl<T: WriteLinearStore> Merkle<T> {
    pub fn put_node(&mut self, node: Node) -> Result<LinearAddress, MerkleError> {
        self.create_node(&node).map_err(MerkleError::Format)
    }

    fn _delete_node(&mut self, _addr: LinearAddress) -> Result<(), MerkleError> {
        todo!()
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

/// TODO danlaine: use or remove PrefixOverlap
/// The [`PrefixOverlap`] type represents the _shared_ and _unique_ parts of two potentially overlapping slices.
/// As the type-name implies, the `shared` property only constitues a shared *prefix*.
/// The `unique_*` properties, [`unique_a`][`PrefixOverlap::unique_a`] and [`unique_b`][`PrefixOverlap::unique_b`]
/// are set based on the argument order passed into the [`from`][`PrefixOverlap::from`] constructor.
// #[derive(Debug)]
// struct PrefixOverlap<'a, T> {
//     shared: &'a [T],
//     unique_a: &'a [T],
//     unique_b: &'a [T],
// }

// impl<'a, T: PartialEq> PrefixOverlap<'a, T> {
//     fn from(a: &'a [T], b: &'a [T]) -> Self {
//         let mut split_index = 0;

//         #[allow(clippy::indexing_slicing)]
//         for i in 0..std::cmp::min(a.len(), b.len()) {
//             if a[i] != b[i] {
//                 break;
//             }

//             split_index += 1;
//         }

//         let (shared, unique_a) = a.split_at(split_index);
//         let (_, unique_b) = b.split_at(split_index);

//         Self {
//             shared,
//             unique_a,
//             unique_b,
//         }
//     }
// }

#[cfg(test)]
#[allow(clippy::indexing_slicing, clippy::unwrap_used)]
mod tests {

    use crate::storage::linear::memory::MemStore;

    use super::*;

    #[test]
    fn insert_empty() {
        let mut merkle = create_in_memory_merkle();
        merkle.insert(b"abc", vec![]).unwrap()
    }

    #[test]
    fn insert_two() {
        let mut merkle = create_in_memory_merkle();
        merkle.insert(b"abc", vec![]).unwrap();
        merkle.insert(b"abc", vec![b'a']).unwrap()
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
    //         let merkle = create_test_merkle();

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

    //     #[test]
    //     fn insert_and_retrieve_one() {
    //         let key = b"hello";
    //         let val = b"world";

    //         let mut merkle = create_test_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         merkle.insert(key, val.to_vec(), root_addr).unwrap();

    //         let fetched_val = merkle.get(key, root_addr).unwrap();

    //         assert_eq!(fetched_val.as_deref(), val.as_slice().into());
    //     }

    //     #[test]
    //     fn insert_and_retrieve_multiple() {
    //         let mut merkle = create_test_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         // insert values
    //         for key_val in u8::MIN..=u8::MAX {
    //             let key = vec![key_val];
    //             let val = vec![key_val];

    //             merkle.insert(&key, val.clone(), root_addr).unwrap();

    //             let fetched_val = merkle.get(&key, root_addr).unwrap();

    //             // make sure the value was inserted
    //             assert_eq!(fetched_val.as_deref(), val.as_slice().into());
    //         }

    //         // make sure none of the previous values were forgotten after initial insert
    //         for key_val in u8::MIN..=u8::MAX {
    //             let key = vec![key_val];
    //             let val = vec![key_val];

    //             let fetched_val = merkle.get(&key, root_addr).unwrap();

    //             assert_eq!(fetched_val.as_deref(), val.as_slice().into());
    //         }
    //     }

    //     #[test]
    //     fn long_insert_and_retrieve_multiple() {
    //         let key_val: Vec<(&'static [u8], _)> = vec![
    //             (
    //                 &[0, 0, 0, 1, 0, 101, 151, 236],
    //                 [16, 15, 159, 195, 34, 101, 227, 73],
    //             ),
    //             (
    //                 &[0, 0, 1, 107, 198, 92, 205],
    //                 [26, 147, 21, 200, 138, 106, 137, 218],
    //             ),
    //             (&[0, 1, 0, 1, 0, 56], [194, 147, 168, 193, 19, 226, 51, 204]),
    //             (&[1, 90], [101, 38, 25, 65, 181, 79, 88, 223]),
    //             (
    //                 &[1, 1, 1, 0, 0, 0, 1, 59],
    //                 [105, 173, 182, 126, 67, 166, 166, 196],
    //             ),
    //             (
    //                 &[0, 1, 0, 0, 1, 1, 55, 33, 38, 194],
    //                 [90, 140, 160, 53, 230, 100, 237, 236],
    //             ),
    //             (
    //                 &[1, 1, 0, 1, 249, 46, 69],
    //                 [16, 104, 134, 6, 57, 46, 200, 35],
    //             ),
    //             (
    //                 &[1, 1, 0, 1, 0, 0, 1, 33, 163],
    //                 [95, 97, 187, 124, 198, 28, 75, 226],
    //             ),
    //             (
    //                 &[1, 1, 0, 1, 0, 57, 156],
    //                 [184, 18, 69, 29, 96, 252, 188, 58],
    //             ),
    //             (&[1, 0, 1, 1, 0, 218], [155, 38, 43, 54, 93, 134, 73, 209]),
    //         ];

    //         let mut merkle = create_test_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         for (key, val) in &key_val {
    //             merkle.insert(key, val.to_vec(), root_addr).unwrap();

    //             let fetched_val = merkle.get(key, root_addr).unwrap();

    //             assert_eq!(fetched_val.as_deref(), val.as_slice().into());
    //         }

    //         for (key, val) in key_val {
    //             let fetched_val = merkle.get(key, root_addr).unwrap();

    //             assert_eq!(fetched_val.as_deref(), val.as_slice().into());
    //         }
    //     }

    //     #[test]
    //     fn remove_one() {
    //         let key = b"hello";
    //         let val = b"world";

    //         let mut merkle = create_test_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         merkle.insert(key, val.to_vec(), root_addr).unwrap();

    //         assert_eq!(
    //             merkle.get(key, root_addr).unwrap().as_deref(),
    //             val.as_slice().into()
    //         );

    //         let removed_val = merkle.remove(key, root_addr).unwrap();
    //         assert_eq!(removed_val.as_deref(), val.as_slice().into());

    //         let fetched_val = merkle.get(key, root_addr).unwrap();
    //         assert!(fetched_val.is_none());
    //     }

    //     #[test]
    //     fn remove_many() {
    //         let mut merkle = create_test_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         // insert values
    //         for key_val in u8::MIN..=u8::MAX {
    //             let key = &[key_val];
    //             let val = &[key_val];

    //             merkle.insert(key, val.to_vec(), root_addr).unwrap();

    //             let fetched_val = merkle.get(key, root_addr).unwrap();

    //             // make sure the value was inserted
    //             assert_eq!(fetched_val.as_deref(), val.as_slice().into());
    //         }

    //         // remove values
    //         for key_val in u8::MIN..=u8::MAX {
    //             let key = &[key_val];
    //             let val = &[key_val];

    //             let Ok(removed_val) = merkle.remove(key, root_addr) else {
    //                 panic!("({key_val}, {key_val}) missing");
    //             };

    //             assert_eq!(removed_val.as_deref(), val.as_slice().into());

    //             let fetched_val = merkle.get(key, root_addr).unwrap();
    //             assert!(fetched_val.is_none());
    //         }
    //     }

    //     #[test]
    //     fn get_empty_proof() {
    //         let merkle = create_test_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         let proof = merkle.prove(b"any-key", root_addr).unwrap();

    //         assert!(proof.0.is_empty());
    //     }

    //     #[tokio::test]
    //     async fn empty_range_proof() {
    //         let merkle = create_test_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         assert!(merkle
    //             .range_proof::<&[u8]>(root_addr, None, None, None)
    //             .await
    //             .unwrap()
    //             .is_none());
    //     }

    //     #[tokio::test]
    //     async fn range_proof_invalid_bounds() {
    //         let merkle = create_test_merkle();
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
    //         let mut merkle = create_test_merkle();
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

    //         let mut merkle = create_test_merkle();
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
    //         let mut merkle = create_test_merkle();
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

    //         let mut merkle = create_test_merkle();
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

    //         let mut merkle = create_test_merkle();
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

    //     #[test]
    //     fn new_leaf_is_a_child_of_the_old_leaf() {
    //         let key = vec![0xff];
    //         let val = vec![1];
    //         let key_2 = vec![0xff, 0x00];
    //         let val_2 = vec![2];

    //         let mut merkle = create_test_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         merkle.insert(&key, val.clone(), root_addr).unwrap();
    //         merkle.insert(&key_2, val_2.clone(), root_addr).unwrap();

    //         assert_eq!(
    //             merkle.get(&key, root_addr).unwrap().as_deref(),
    //             Some(val.as_slice())
    //         );

    //         assert_eq!(
    //             merkle.get(&key_2, root_addr).unwrap().as_deref(),
    //             Some(val_2.as_slice())
    //         );
    //     }

    //     #[test]
    //     fn old_leaf_is_a_child_of_the_new_leaf() {
    //         let key = vec![0xff, 0x00];
    //         let val = vec![1];
    //         let key_2 = vec![0xff];
    //         let val_2 = vec![2];

    //         let mut merkle = create_test_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         merkle.insert(&key, val.clone(), root_addr).unwrap();
    //         merkle.insert(&key_2, val_2.clone(), root_addr).unwrap();

    //         assert_eq!(
    //             merkle.get(&key, root_addr).unwrap().as_deref(),
    //             Some(val.as_slice())
    //         );

    //         assert_eq!(
    //             merkle.get(&key_2, root_addr).unwrap().as_deref(),
    //             Some(val_2.as_slice())
    //         );
    //     }

    //     #[test]
    //     fn new_leaf_is_sibling_of_old_leaf() {
    //         let key = vec![0xff];
    //         let val = vec![1];
    //         let key_2 = vec![0xff, 0x00];
    //         let val_2 = vec![2];
    //         let key_3 = vec![0xff, 0x0f];
    //         let val_3 = vec![3];

    //         let mut merkle = create_test_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         merkle.insert(&key, val.clone(), root_addr).unwrap();
    //         merkle.insert(&key_2, val_2.clone(), root_addr).unwrap();
    //         merkle.insert(&key_3, val_3.clone(), root_addr).unwrap();

    //         assert_eq!(
    //             merkle.get(&key, root_addr).unwrap().as_deref(),
    //             Some(val.as_slice())
    //         );

    //         assert_eq!(
    //             merkle.get(&key_2, root_addr).unwrap().as_deref(),
    //             Some(val_2.as_slice())
    //         );

    //         assert_eq!(
    //             merkle.get(&key_3, root_addr).unwrap().as_deref(),
    //             Some(val_3.as_slice())
    //         );
    //     }

    //     #[test]
    //     fn old_branch_is_a_child_of_new_branch() {
    //         let key = vec![0xff, 0xf0];
    //         let val = vec![1];
    //         let key_2 = vec![0xff, 0xf0, 0x00];
    //         let val_2 = vec![2];
    //         let key_3 = vec![0xff];
    //         let val_3 = vec![3];

    //         let mut merkle = create_test_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         merkle.insert(&key, val.clone(), root_addr).unwrap();
    //         merkle.insert(&key_2, val_2.clone(), root_addr).unwrap();
    //         merkle.insert(&key_3, val_3.clone(), root_addr).unwrap();

    //         assert_eq!(
    //             merkle.get(&key, root_addr).unwrap().as_deref(),
    //             Some(val.as_slice())
    //         );

    //         assert_eq!(
    //             merkle.get(&key_2, root_addr).unwrap().as_deref(),
    //             Some(val_2.as_slice())
    //         );

    //         assert_eq!(
    //             merkle.get(&key_3, root_addr).unwrap().as_deref(),
    //             Some(val_3.as_slice())
    //         );
    //     }

    //     #[test]
    //     fn overlapping_branch_insert() {
    //         let key = vec![0xff];
    //         let val = vec![1];
    //         let key_2 = vec![0xff, 0x00];
    //         let val_2 = vec![2];

    //         let overwrite = vec![3];

    //         let mut merkle = create_test_merkle();
    //         let root_addr = merkle.init_sentinel().unwrap();

    //         merkle.insert(&key, val.clone(), root_addr).unwrap();
    //         merkle.insert(&key_2, val_2.clone(), root_addr).unwrap();

    //         assert_eq!(
    //             merkle.get(&key, root_addr).unwrap().as_deref(),
    //             Some(val.as_slice())
    //         );

    //         assert_eq!(
    //             merkle.get(&key_2, root_addr).unwrap().as_deref(),
    //             Some(val_2.as_slice())
    //         );

    //         merkle
    //             .insert(&key, overwrite.clone(), root_addr)
    //             .unwrap();

    //         assert_eq!(
    //             merkle.get(&key, root_addr).unwrap().as_deref(),
    //             Some(overwrite.as_slice())
    //         );

    //         assert_eq!(
    //             merkle.get(&key_2, root_addr).unwrap().as_deref(),
    //             Some(val_2.as_slice())
    //         );
    //     }

    //     #[test]
    //     fn single_key_proof_with_one_node() {
    //         let mut merkle = create_test_merkle();
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
    //         let mut merkle = create_test_merkle();
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

    //     #[test]
    //     fn update_leaf_with_larger_path() -> Result<(), MerkleError> {
    //         let path = vec![0x00];
    //         let value = vec![0x00];

    //         let double_path = path
    //             .clone()
    //             .into_iter()
    //             .chain(path.clone())
    //             .collect::<Vec<_>>();

    //         let node = Node::from_leaf(LeafNode {
    //             partial_path: Path::from(path),
    //             value: value.clone(),
    //         });

    //         check_node_update(node, double_path, value)
    //     }

    //     #[test]
    //     fn update_leaf_with_larger_value() -> Result<(), MerkleError> {
    //         let path = vec![0x00];
    //         let value = vec![0x00];

    //         let double_value = value
    //             .clone()
    //             .into_iter()
    //             .chain(value.clone())
    //             .collect::<Vec<_>>();

    //         let node = Node::from_leaf(LeafNode {
    //             partial_path: Path::from(path.clone()),
    //             value,
    //         });

    //         check_node_update(node, path, double_value)
    //     }

    //     #[test]
    //     fn update_branch_with_larger_path() -> Result<(), MerkleError> {
    //         let path = vec![0x00];
    //         let value = vec![0x00];

    //         let double_path = path
    //             .clone()
    //             .into_iter()
    //             .chain(path.clone())
    //             .collect::<Vec<_>>();

    //         let node = Node::from_branch(BranchNode {
    //             partial_path: Path::from(path.clone()),
    //             children: Default::default(),
    //             value: Some(value.clone()),
    //             children_encoded: Default::default(),
    //         });

    //         check_node_update(node, double_path, value)
    //     }

    //     #[test]
    //     fn update_branch_with_larger_value() -> Result<(), MerkleError> {
    //         let path = vec![0x00];
    //         let value = vec![0x00];

    //         let double_value = value
    //             .clone()
    //             .into_iter()
    //             .chain(value.clone())
    //             .collect::<Vec<_>>();

    //         let node = Node::from_branch(BranchNode {
    //             partial_path: Path::from(path.clone()),
    //             children: Default::default(),
    //             value: Some(value),
    //             children_encoded: Default::default(),
    //         });

    //         check_node_update(node, path, double_value)
    //     }

    //     fn check_node_update(
    //         node: Node,
    //         new_path: Vec<u8>,
    //         new_value: Vec<u8>,
    //     ) -> Result<(), MerkleError> {
    //         let merkle = create_test_merkle();
    //         let root_addr = merkle.init_sentinel()?;
    //         let sentinel = merkle.get_node(root_addr)?;

    //         let mut node_ref = merkle.put_node(node)?;
    //         let addr = node_ref.as_addr();

    //         // make sure that doubling the path length will fail on a normal write
    //         let write_result = node_ref.write(|node| {
    //             node.inner_mut().set_path(Path(new_path.clone()));
    //             node.inner_mut().set_value(new_value.clone());
    //             node.rehash();
    //         });

    //         assert!(matches!(write_result, Err(ObjWriteSizeError)));

    //         let mut to_delete = vec![];
    //         // could be any branch node, convenient to use the root.
    //         let mut parents = vec![(sentinel, 0)];

    //         let node = merkle.update_path_and_move_node_if_larger(
    //             (&mut parents, &mut to_delete),
    //             node_ref,
    //             Path(new_path.clone()),
    //         )?;

    //         assert_ne!(node.as_addr(), addr);
    //         assert_eq!(&to_delete[0], &addr);

    //         let (path, value) = match node.inner() {
    //             NodeType::Leaf(leaf) => (&leaf.partial_path, Some(&leaf.value)),
    //             NodeType::Branch(branch) => (&branch.partial_path, branch.value.as_ref()),
    //         };

    //         assert_eq!(path, &Path(new_path));
    //         assert_eq!(value, Some(&new_value));

    //         Ok(())
    //     }
}
#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod test {
    use super::*;
    use crate::storage::linear::memory::MemStore;
    use rand::rngs::StdRng;
    use rand::{thread_rng, Rng, SeedableRng as _};
    use std::collections::HashMap;

    fn merkle_build_test<
        K: AsRef<[u8]> + std::cmp::Ord + Clone + std::fmt::Debug,
        V: AsRef<[u8]> + Clone,
    >(
        items: Vec<(K, V)>,
    ) -> Result<Merkle<MemStore>, MerkleError> {
        let mut merkle = Merkle::new(HashedNodeStore::initialize(MemStore::new(vec![])).unwrap());
        for (k, v) in items.iter() {
            merkle.insert(k, v.as_ref().to_vec())?;
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

        merkle.dump()?;

        Ok(())
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

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_root_hash_reversed_deletions() -> Result<(), MerkleError> {
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
            let mut items: Vec<_> = (0..10)
                .map(|_| keygen())
                .map(|key| {
                    let val: Vec<u8> = (0..8).map(|_| rng.borrow_mut().gen()).collect();
                    (key, val)
                })
                .collect();

            items.sort();

            let mut merkle =
                Merkle::new(HashedNodeStore::initialize(MemStore::new(vec![])).unwrap());

            let mut hashes = Vec::new();

            for (k, v) in items.iter() {
                hashes.push((merkle.root_hash()?, merkle.dump()?));
                merkle.insert(k, v.to_vec())?;
            }

            let mut new_hashes = Vec::new();

            for (k, _) in items.iter().rev() {
                let before = merkle.dump()?;
                merkle.remove(k)?;
                new_hashes.push((merkle.root_hash()?, k, before, merkle.dump()?));
            }

            hashes.reverse();

            // TODO danlaine: uncomment below or remove this test
            // for i in 0..hashes.len() {
            //     #[allow(clippy::indexing_slicing)]
            //     let (new_hash, key, before_removal, after_removal) = &new_hashes[i];
            //     #[allow(clippy::indexing_slicing)]
            //     let expected_hash = &hashes[i];
            //     let key = key.iter().fold(String::new(), |mut s, b| {
            //         let _ = write!(s, "{:02x}", b);
            //         s
            //     });
            //     // assert_eq!(new_hash, expected_hash, "\n\nkey: {key}\nbefore:\n{before_removal}\nafter:\n{after_removal}\n\nexpected:\n{expected_dump}\n");
            // }
        }

        Ok(())
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn test_root_hash_random_deletions() -> Result<(), MerkleError> {
        use rand::rngs::StdRng;
        use rand::seq::SliceRandom;
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

        for i in 0..10 {
            let mut items = std::collections::HashMap::new();

            for _ in 0..10 {
                let val: Vec<u8> = (0..8).map(|_| rng.borrow_mut().gen()).collect();
                items.insert(keygen(), val);
            }

            let mut items_ordered: Vec<_> =
                items.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            items_ordered.sort();
            items_ordered.shuffle(&mut *rng.borrow_mut());
            let mut merkle =
                Merkle::new(HashedNodeStore::initialize(MemStore::new(vec![])).unwrap());

            for (k, v) in items.iter() {
                merkle.insert(k, v.to_vec())?;
            }

            for (k, v) in items.iter() {
                assert_eq!(&*merkle.get(k)?.unwrap(), &v[..]);
            }

            for (k, _) in items_ordered.into_iter() {
                assert!(merkle.get(&k)?.is_some());

                merkle.remove(&k)?;

                assert!(merkle.get(&k)?.is_none());

                items.remove(&k);

                for (k, v) in items.iter() {
                    assert_eq!(&*merkle.get(k)?.unwrap(), &v[..]);
                }

                let h = triehash::trie_root::<keccak_hasher::KeccakHasher, Vec<_>, _, _>(
                    items.iter().collect(),
                );

                let h0 = merkle.root_hash()?;

                if h[..] != *h0 {
                    println!("{} != {}", hex::encode(h), hex::encode(*h0));
                }
            }

            println!("i = {i}");
        }
        Ok(())
    }

    #[test]
    #[allow(clippy::unwrap_used, clippy::indexing_slicing)]
    fn test_proof() -> Result<(), MerkleError> {
        let set = fixed_and_pseudorandom_data(500);
        let mut items = Vec::from_iter(set.iter());
        items.sort();
        let merkle = merkle_build_test(items.clone())?;
        let (keys, vals): (Vec<&[u8; 32]>, Vec<&[u8; 20]>) = items.into_iter().unzip();

        for (i, key) in keys.iter().enumerate() {
            let proof = merkle.prove(key)?;
            assert!(!proof.0.is_empty());
            let val = merkle.verify_proof(key, &proof)?;
            assert!(val.is_some());
            assert_eq!(val.unwrap(), vals[i]);
        }

        Ok(())
    }

    #[test]
    /// Verify the proofs that end with leaf node with the given key.
    fn test_proof_end_with_leaf() -> Result<(), MerkleError> {
        let items = vec![
            ("do", "verb"),
            ("doe", "reindeer"),
            ("dog", "puppy"),
            ("doge", "coin"),
            ("horse", "stallion"),
            ("ddd", "ok"),
        ];
        let merkle = merkle_build_test(items)?;
        let key = "doe";

        let proof = merkle.prove(key)?;
        assert!(!proof.0.is_empty());

        let verify_proof = merkle.verify_proof(key, &proof)?;
        assert!(verify_proof.is_some());

        Ok(())
    }

    #[test]
    /// Verify the proofs that end with branch node with the given key.
    fn test_proof_end_with_branch() -> Result<(), MerkleError> {
        let items = vec![
            ("d", "verb"),
            ("do", "verb"),
            ("doe", "reindeer"),
            ("e", "coin"),
        ];
        let merkle = merkle_build_test(items)?;
        let key = "d";

        let proof = merkle.prove(key)?;
        assert!(!proof.0.is_empty());

        let verify_proof = merkle.verify_proof(key, &proof)?;
        assert!(verify_proof.is_some());

        Ok(())
    }

    #[test]
    fn test_bad_proof() -> Result<(), MerkleError> {
        let set = fixed_and_pseudorandom_data(800);
        let mut items = Vec::from_iter(set.iter());
        items.sort();
        let merkle = merkle_build_test(items.clone())?;
        let (keys, _): (Vec<&[u8; 32]>, Vec<&[u8; 20]>) = items.into_iter().unzip();

        for key in keys.iter() {
            let mut proof = merkle.prove(key)?;
            assert!(!proof.0.is_empty());

            // Delete an entry from the generated proofs.
            let len = proof.0.len();
            let new_proof = Proof(proof.0.drain().take(len - 1).collect());
            assert!(merkle.verify_proof(key, &new_proof).is_err());
        }

        Ok(())
    }

    #[test]
    // Tests that missing keys can also be proven. The test explicitly uses a single
    // entry trie and checks for missing keys both before and after the single entry.
    fn test_missing_key_proof() -> Result<(), MerkleError> {
        let items = vec![("k", "v")];
        let merkle = merkle_build_test(items)?;
        for key in &["a", "j", "l", "z"] {
            let proof = merkle.prove(key)?;
            assert!(!proof.0.is_empty());
            assert!(proof.0.len() == 1);

            let val = merkle.verify_proof(key, &proof)?;
            assert!(val.is_none());
        }

        Ok(())
    }

    #[test]
    fn test_empty_tree_proof() -> Result<(), MerkleError> {
        let items: Vec<(&str, &str)> = Vec::new();
        let merkle = merkle_build_test(items)?;
        let key = "x";

        let proof = merkle.prove(key)?;
        assert!(proof.0.is_empty());

        Ok(())
    }

    #[test]
    // Tests normal range proof with both edge proofs as the existent proof.
    // The test cases are generated randomly.
    #[allow(clippy::indexing_slicing)]
    fn test_range_proof() -> Result<(), ProofError> {
        let set = fixed_and_pseudorandom_data(4096);
        let mut items = Vec::from_iter(set.iter());
        items.sort();
        let merkle = merkle_build_test(items.clone())?;

        for _ in 0..10 {
            let start = rand::thread_rng().gen_range(0..items.len());
            let end = rand::thread_rng().gen_range(0..items.len() - start) + start - 1;

            if end <= start {
                continue;
            }

            let mut proof = merkle.prove(items[start].0)?;
            assert!(!proof.0.is_empty());
            let end_proof = merkle.prove(items[end - 1].0)?;
            assert!(!end_proof.0.is_empty());
            proof.extend(end_proof);

            let mut keys = Vec::new();
            let mut vals = Vec::new();
            for item in items[start..end].iter() {
                keys.push(&item.0);
                vals.push(&item.1);
            }

            merkle.verify_range_proof(&proof, &items[start].0, &items[end - 1].0, keys, vals)?;
        }
        Ok(())
    }

    #[test]
    // Tests a few cases which the proof is wrong.
    // The prover is expected to detect the error.
    #[allow(clippy::indexing_slicing)]
    fn test_bad_range_proof() -> Result<(), ProofError> {
        let set = fixed_and_pseudorandom_data(4096);
        let mut items = Vec::from_iter(set.iter());
        items.sort();
        let merkle = merkle_build_test(items.clone())?;

        for _ in 0..10 {
            let start = rand::thread_rng().gen_range(0..items.len());
            let end = rand::thread_rng().gen_range(0..items.len() - start) + start - 1;

            if end <= start {
                continue;
            }

            let mut proof = merkle.prove(items[start].0)?;
            assert!(!proof.0.is_empty());
            let end_proof = merkle.prove(items[end - 1].0)?;
            assert!(!end_proof.0.is_empty());
            proof.extend(end_proof);

            let mut keys: Vec<[u8; 32]> = Vec::new();
            let mut vals: Vec<[u8; 20]> = Vec::new();
            for item in items[start..end].iter() {
                keys.push(*item.0);
                vals.push(*item.1);
            }

            let test_case: u32 = rand::thread_rng().gen_range(0..6);
            let index = rand::thread_rng().gen_range(0..end - start);
            match test_case {
                0 => {
                    // Modified key
                    keys[index] = rand::thread_rng().gen::<[u8; 32]>(); // In theory it can't be same
                }
                1 => {
                    // Modified val
                    vals[index] = rand::thread_rng().gen::<[u8; 20]>(); // In theory it can't be same
                }
                2 => {
                    // Gapped entry slice
                    if index == 0 || index == end - start - 1 {
                        continue;
                    }
                    keys.remove(index);
                    vals.remove(index);
                }
                3 => {
                    // Out of order
                    let index_1 = rand::thread_rng().gen_range(0..end - start);
                    let index_2 = rand::thread_rng().gen_range(0..end - start);
                    if index_1 == index_2 {
                        continue;
                    }
                    keys.swap(index_1, index_2);
                    vals.swap(index_1, index_2);
                }
                4 => {
                    // Set random key to empty, do nothing
                    keys[index] = [0; 32];
                }
                5 => {
                    // Set random value to nil
                    vals[index] = [0; 20];
                }
                _ => unreachable!(),
            }
            assert!(merkle
                .verify_range_proof(&proof, *items[start].0, *items[end - 1].0, keys, vals)
                .is_err());
        }

        Ok(())
    }

    #[test]
    // Tests normal range proof with two non-existent proofs.
    // The test cases are generated randomly.
    #[allow(clippy::indexing_slicing)]
    fn test_range_proof_with_non_existent_proof() -> Result<(), ProofError> {
        let set = fixed_and_pseudorandom_data(4096);
        let mut items = Vec::from_iter(set.iter());
        items.sort();
        let merkle = merkle_build_test(items.clone())?;

        for _ in 0..10 {
            let start = rand::thread_rng().gen_range(0..items.len());
            let end = rand::thread_rng().gen_range(0..items.len() - start) + start - 1;

            if end <= start {
                continue;
            }

            // Short circuit if the decreased key is same with the previous key
            let first = decrease_key(items[start].0);
            if start != 0 && first.as_ref() == items[start - 1].0.as_ref() {
                continue;
            }
            // Short circuit if the decreased key is underflow
            if first.as_ref() > items[start].0.as_ref() {
                continue;
            }
            // Short circuit if the increased key is same with the next key
            let last = increase_key(items[end - 1].0);
            if end != items.len() && last.as_ref() == items[end].0.as_ref() {
                continue;
            }
            // Short circuit if the increased key is overflow
            if last.as_ref() < items[end - 1].0.as_ref() {
                continue;
            }

            let mut proof = merkle.prove(first)?;
            assert!(!proof.0.is_empty());
            let end_proof = merkle.prove(last)?;
            assert!(!end_proof.0.is_empty());
            proof.extend(end_proof);

            let mut keys: Vec<[u8; 32]> = Vec::new();
            let mut vals: Vec<[u8; 20]> = Vec::new();
            for item in items[start..end].iter() {
                keys.push(*item.0);
                vals.push(*item.1);
            }

            merkle.verify_range_proof(&proof, first, last, keys, vals)?;
        }

        // Special case, two edge proofs for two edge key.
        let first: [u8; 32] = [0; 32];
        let last: [u8; 32] = [255; 32];
        let mut proof = merkle.prove(first)?;
        assert!(!proof.0.is_empty());
        let end_proof = merkle.prove(last)?;
        assert!(!end_proof.0.is_empty());
        proof.extend(end_proof);

        let (keys, vals): (Vec<&[u8; 32]>, Vec<&[u8; 20]>) = items.into_iter().unzip();
        merkle.verify_range_proof(&proof, &first, &last, keys, vals)?;

        Ok(())
    }

    #[test]
    // Tests such scenarios:
    // - There exists a gap between the first element and the left edge proof
    // - There exists a gap between the last element and the right edge proof
    #[allow(clippy::indexing_slicing)]
    fn test_range_proof_with_invalid_non_existent_proof() -> Result<(), ProofError> {
        let set = fixed_and_pseudorandom_data(4096);
        let mut items = Vec::from_iter(set.iter());
        items.sort();
        let merkle = merkle_build_test(items.clone())?;

        // Case 1
        let mut start = 100;
        let mut end = 200;
        let first = decrease_key(items[start].0);

        let mut proof = merkle.prove(first)?;
        assert!(!proof.0.is_empty());
        let end_proof = merkle.prove(items[end - 1].0)?;
        assert!(!end_proof.0.is_empty());
        proof.extend(end_proof);

        start = 105; // Gap created
        let mut keys: Vec<[u8; 32]> = Vec::new();
        let mut vals: Vec<[u8; 20]> = Vec::new();
        // Create gap
        for item in items[start..end].iter() {
            keys.push(*item.0);
            vals.push(*item.1);
        }
        assert!(merkle
            .verify_range_proof(&proof, first, *items[end - 1].0, keys, vals)
            .is_err());

        // Case 2
        start = 100;
        end = 200;
        let last = increase_key(items[end - 1].0);

        let mut proof = merkle.prove(items[start].0)?;
        assert!(!proof.0.is_empty());
        let end_proof = merkle.prove(last)?;
        assert!(!end_proof.0.is_empty());
        proof.extend(end_proof);

        end = 195; // Capped slice
        let mut keys: Vec<[u8; 32]> = Vec::new();
        let mut vals: Vec<[u8; 20]> = Vec::new();
        // Create gap
        for item in items[start..end].iter() {
            keys.push(*item.0);
            vals.push(*item.1);
        }
        assert!(merkle
            .verify_range_proof(&proof, *items[start].0, last, keys, vals)
            .is_err());

        Ok(())
    }

    #[test]
    // Tests the proof with only one element. The first edge proof can be existent one or
    // non-existent one.
    #[allow(clippy::indexing_slicing)]
    fn test_one_element_range_proof() -> Result<(), ProofError> {
        let set = fixed_and_pseudorandom_data(4096);
        let mut items = Vec::from_iter(set.iter());
        items.sort();
        let merkle = merkle_build_test(items.clone())?;

        // One element with existent edge proof, both edge proofs
        // point to the SAME key.
        let start = 1000;
        let start_proof = merkle.prove(items[start].0)?;
        assert!(!start_proof.0.is_empty());

        merkle.verify_range_proof(
            &start_proof,
            &items[start].0,
            &items[start].0,
            vec![&items[start].0],
            vec![&items[start].1],
        )?;

        // One element with left non-existent edge proof
        let first = decrease_key(items[start].0);
        let mut proof = merkle.prove(first)?;
        assert!(!proof.0.is_empty());
        let end_proof = merkle.prove(items[start].0)?;
        assert!(!end_proof.0.is_empty());
        proof.extend(end_proof);

        merkle.verify_range_proof(
            &proof,
            first,
            *items[start].0,
            vec![*items[start].0],
            vec![*items[start].1],
        )?;

        // One element with right non-existent edge proof
        let last = increase_key(items[start].0);
        let mut proof = merkle.prove(items[start].0)?;
        assert!(!proof.0.is_empty());
        let end_proof = merkle.prove(last)?;
        assert!(!end_proof.0.is_empty());
        proof.extend(end_proof);

        merkle.verify_range_proof(
            &proof,
            *items[start].0,
            last,
            vec![*items[start].0],
            vec![*items[start].1],
        )?;

        // One element with two non-existent edge proofs
        let mut proof = merkle.prove(first)?;
        assert!(!proof.0.is_empty());
        let end_proof = merkle.prove(last)?;
        assert!(!end_proof.0.is_empty());
        proof.extend(end_proof);

        merkle.verify_range_proof(
            &proof,
            first,
            last,
            vec![*items[start].0],
            vec![*items[start].1],
        )?;

        // Test the mini trie with only a single element.
        let key = rand::thread_rng().gen::<[u8; 32]>();
        let val = rand::thread_rng().gen::<[u8; 20]>();
        let merkle = merkle_build_test(vec![(key, val)])?;

        let first: [u8; 32] = [0; 32];
        let mut proof = merkle.prove(first)?;
        assert!(!proof.0.is_empty());
        let end_proof = merkle.prove(key)?;
        assert!(!end_proof.0.is_empty());
        proof.extend(end_proof);

        merkle.verify_range_proof(&proof, first, key, vec![key], vec![val])?;

        Ok(())
    }

    #[test]
    // Tests the range proof with all elements.
    // The edge proofs can be nil.
    #[allow(clippy::indexing_slicing)]
    fn test_all_elements_proof() -> Result<(), ProofError> {
        let set = fixed_and_pseudorandom_data(4096);
        let mut items = Vec::from_iter(set.iter());
        items.sort();
        let merkle = merkle_build_test(items.clone())?;

        let item_iter = items.clone().into_iter();
        let keys: Vec<&[u8; 32]> = item_iter.clone().map(|item| item.0).collect();
        let vals: Vec<&[u8; 20]> = item_iter.map(|item| item.1).collect();

        let empty_proof = Proof(HashMap::<[u8; 32], Vec<u8>>::new());
        let empty_key: [u8; 32] = [0; 32];
        merkle.verify_range_proof(
            &empty_proof,
            &empty_key,
            &empty_key,
            keys.clone(),
            vals.clone(),
        )?;

        // With edge proofs, it should still work.
        let start = 0;
        let end = &items.len() - 1;

        let mut proof = merkle.prove(items[start].0)?;
        assert!(!proof.0.is_empty());
        let end_proof = merkle.prove(items[end].0)?;
        assert!(!end_proof.0.is_empty());
        proof.extend(end_proof);

        merkle.verify_range_proof(
            &proof,
            items[start].0,
            items[end].0,
            keys.clone(),
            vals.clone(),
        )?;

        // Even with non-existent edge proofs, it should still work.
        let first: [u8; 32] = [0; 32];
        let last: [u8; 32] = [255; 32];
        let mut proof = merkle.prove(first)?;
        assert!(!proof.0.is_empty());
        let end_proof = merkle.prove(last)?;
        assert!(!end_proof.0.is_empty());
        proof.extend(end_proof);

        merkle.verify_range_proof(&proof, &first, &last, keys, vals)?;

        Ok(())
    }

    #[test]
    // Tests the range proof with "no" element. The first edge proof must
    // be a non-existent proof.
    #[allow(clippy::indexing_slicing)]
    fn test_empty_range_proof() -> Result<(), ProofError> {
        let set = fixed_and_pseudorandom_data(4096);
        let mut items = Vec::from_iter(set.iter());
        items.sort();
        let merkle = merkle_build_test(items.clone())?;

        let cases = [(items.len() - 1, false)];
        for c in cases.iter() {
            let first = increase_key(items[c.0].0);
            let proof = merkle.prove(first)?;
            assert!(!proof.0.is_empty());

            // key and value vectors are intentionally empty.
            let keys: Vec<[u8; 32]> = Vec::new();
            let vals: Vec<[u8; 20]> = Vec::new();

            if c.1 {
                assert!(merkle
                    .verify_range_proof(&proof, first, first, keys, vals)
                    .is_err());
            } else {
                merkle.verify_range_proof(&proof, first, first, keys, vals)?;
            }
        }

        Ok(())
    }

    #[test]
    // Focuses on the small trie with embedded nodes. If the gapped
    // node is embedded in the trie, it should be detected too.
    #[allow(clippy::indexing_slicing)]
    fn test_gapped_range_proof() -> Result<(), ProofError> {
        let mut items = Vec::new();
        // Sorted entries
        for i in 0..10_u32 {
            let mut key: [u8; 32] = [0; 32];
            for (index, d) in i.to_be_bytes().iter().enumerate() {
                key[index] = *d;
            }
            items.push((key, i.to_be_bytes()));
        }
        let merkle = merkle_build_test(items.clone())?;

        let first = 2;
        let last = 8;

        let mut proof = merkle.prove(items[first].0)?;
        assert!(!proof.0.is_empty());
        let end_proof = merkle.prove(items[last - 1].0)?;
        assert!(!end_proof.0.is_empty());
        proof.extend(end_proof);

        let middle = (first + last) / 2 - first;
        let (keys, vals): (Vec<&[u8; 32]>, Vec<&[u8; 4]>) = items[first..last]
            .iter()
            .enumerate()
            .filter(|(pos, _)| *pos != middle)
            .map(|(_, item)| (&item.0, &item.1))
            .unzip();

        assert!(merkle
            .verify_range_proof(&proof, &items[0].0, &items[items.len() - 1].0, keys, vals)
            .is_err());

        Ok(())
    }

    #[test]
    // Tests the element is not in the range covered by proofs.
    #[allow(clippy::indexing_slicing)]
    fn test_same_side_proof() -> Result<(), MerkleError> {
        let set = fixed_and_pseudorandom_data(4096);
        let mut items = Vec::from_iter(set.iter());
        items.sort();
        let merkle = merkle_build_test(items.clone())?;

        let pos = 1000;
        let mut last = decrease_key(items[pos].0);
        let mut first = last;
        first = decrease_key(&first);

        let mut proof = merkle.prove(first)?;
        assert!(!proof.0.is_empty());
        let end_proof = merkle.prove(last)?;
        assert!(!end_proof.0.is_empty());
        proof.extend(end_proof);

        assert!(merkle
            .verify_range_proof(&proof, first, last, vec![*items[pos].0], vec![items[pos].1])
            .is_err());

        first = increase_key(items[pos].0);
        last = first;
        last = increase_key(&last);

        let mut proof = merkle.prove(first)?;
        assert!(!proof.0.is_empty());
        let end_proof = merkle.prove(last)?;
        assert!(!end_proof.0.is_empty());
        proof.extend(end_proof);

        assert!(merkle
            .verify_range_proof(&proof, first, last, vec![*items[pos].0], vec![items[pos].1])
            .is_err());

        Ok(())
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    // Tests the range starts from zero.
    fn test_single_side_range_proof() -> Result<(), ProofError> {
        for _ in 0..10 {
            let mut set = HashMap::new();
            for _ in 0..4096_u32 {
                let key = rand::thread_rng().gen::<[u8; 32]>();
                let val = rand::thread_rng().gen::<[u8; 20]>();
                set.insert(key, val);
            }
            let mut items = Vec::from_iter(set.iter());
            items.sort();
            let merkle = merkle_build_test(items.clone())?;

            let cases = vec![0, 1, 100, 1000, items.len() - 1];
            for case in cases {
                let start: [u8; 32] = [0; 32];
                let mut proof = merkle.prove(start)?;
                assert!(!proof.0.is_empty());
                let end_proof = merkle.prove(items[case].0)?;
                assert!(!end_proof.0.is_empty());
                proof.extend(end_proof);

                let item_iter = items.clone().into_iter().take(case + 1);
                let keys = item_iter.clone().map(|item| *item.0).collect();
                let vals = item_iter.map(|item| item.1).collect();

                merkle.verify_range_proof(&proof, start, *items[case].0, keys, vals)?;
            }
        }
        Ok(())
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    // Tests the range ends with 0xffff...fff.
    fn test_reverse_single_side_range_proof() -> Result<(), ProofError> {
        for _ in 0..10 {
            let mut set = HashMap::new();
            for _ in 0..1024_u32 {
                let key = rand::thread_rng().gen::<[u8; 32]>();
                let val = rand::thread_rng().gen::<[u8; 20]>();
                set.insert(key, val);
            }
            let mut items = Vec::from_iter(set.iter());
            items.sort();
            let merkle = merkle_build_test(items.clone())?;

            let cases = vec![0, 1, 100, 1000, items.len() - 1];
            for case in cases {
                let end: [u8; 32] = [255; 32];
                let mut proof = merkle.prove(items[case].0)?;
                assert!(!proof.0.is_empty());
                let end_proof = merkle.prove(end)?;
                assert!(!end_proof.0.is_empty());
                proof.extend(end_proof);

                let item_iter = items.clone().into_iter().skip(case);
                let keys = item_iter.clone().map(|item| item.0).collect();
                let vals = item_iter.map(|item| item.1).collect();

                merkle.verify_range_proof(&proof, items[case].0, &end, keys, vals)?;
            }
        }
        Ok(())
    }

    #[test]
    // Tests the range starts with zero and ends with 0xffff...fff.
    fn test_both_sides_range_proof() -> Result<(), ProofError> {
        for _ in 0..10 {
            let mut set = HashMap::new();
            for _ in 0..4096_u32 {
                let key = rand::thread_rng().gen::<[u8; 32]>();
                let val = rand::thread_rng().gen::<[u8; 20]>();
                set.insert(key, val);
            }
            let mut items = Vec::from_iter(set.iter());
            items.sort();
            let merkle = merkle_build_test(items.clone())?;

            let start: [u8; 32] = [0; 32];
            let end: [u8; 32] = [255; 32];

            let mut proof = merkle.prove(start)?;
            assert!(!proof.0.is_empty());
            let end_proof = merkle.prove(end)?;
            assert!(!end_proof.0.is_empty());
            proof.extend(end_proof);

            let (keys, vals): (Vec<&[u8; 32]>, Vec<&[u8; 20]>) = items.into_iter().unzip();
            merkle.verify_range_proof(&proof, &start, &end, keys, vals)?;
        }
        Ok(())
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    // Tests normal range proof with both edge proofs
    // as the existent proof, but with an extra empty value included, which is a
    // noop technically, but practically should be rejected.
    fn test_empty_value_range_proof() -> Result<(), ProofError> {
        let set = fixed_and_pseudorandom_data(512);
        let mut items = Vec::from_iter(set.iter());
        items.sort();
        let merkle = merkle_build_test(items.clone())?;

        // Create a new entry with a slightly modified key
        let mid_index = items.len() / 2;
        let key = increase_key(items[mid_index - 1].0);
        let empty_value: [u8; 20] = [0; 20];
        items.splice(mid_index..mid_index, [(&key, &empty_value)].iter().cloned());

        let start = 1;
        let end = items.len() - 1;

        let mut proof = merkle.prove(items[start].0)?;
        assert!(!proof.0.is_empty());
        let end_proof = merkle.prove(items[end - 1].0)?;
        assert!(!end_proof.0.is_empty());
        proof.extend(end_proof);

        let item_iter = items.clone().into_iter().skip(start).take(end - start);
        let keys = item_iter.clone().map(|item| item.0).collect();
        let vals = item_iter.map(|item| item.1).collect();
        assert!(merkle
            .verify_range_proof(&proof, items[start].0, items[end - 1].0, keys, vals)
            .is_err());

        Ok(())
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    // Tests the range proof with all elements,
    // but with an extra empty value included, which is a noop technically, but
    // practically should be rejected.
    fn test_all_elements_empty_value_range_proof() -> Result<(), ProofError> {
        let set = fixed_and_pseudorandom_data(512);
        let mut items = Vec::from_iter(set.iter());
        items.sort();
        let merkle = merkle_build_test(items.clone())?;

        // Create a new entry with a slightly modified key
        let mid_index = items.len() / 2;
        let key = increase_key(items[mid_index - 1].0);
        let empty_value: [u8; 20] = [0; 20];
        items.splice(mid_index..mid_index, [(&key, &empty_value)].iter().cloned());

        let start = 0;
        let end = items.len() - 1;

        let mut proof = merkle.prove(items[start].0)?;
        assert!(!proof.0.is_empty());
        let end_proof = merkle.prove(items[end].0)?;
        assert!(!end_proof.0.is_empty());
        proof.extend(end_proof);

        let item_iter = items.clone().into_iter();
        let keys = item_iter.clone().map(|item| item.0).collect();
        let vals = item_iter.map(|item| item.1).collect();
        assert!(merkle
            .verify_range_proof(&proof, items[start].0, items[end].0, keys, vals)
            .is_err());

        Ok(())
    }

    #[test]
    fn test_range_proof_keys_with_shared_prefix() -> Result<(), ProofError> {
        let items = vec![
            (
                hex::decode("aa10000000000000000000000000000000000000000000000000000000000000")
                    .expect("Decoding failed"),
                hex::decode("02").expect("Decoding failed"),
            ),
            (
                hex::decode("aa20000000000000000000000000000000000000000000000000000000000000")
                    .expect("Decoding failed"),
                hex::decode("03").expect("Decoding failed"),
            ),
        ];
        let merkle = merkle_build_test(items.clone())?;

        let start = hex::decode("0000000000000000000000000000000000000000000000000000000000000000")
            .expect("Decoding failed");
        let end = hex::decode("ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff")
            .expect("Decoding failed");

        let mut proof = merkle.prove(&start)?;
        assert!(!proof.0.is_empty());
        let end_proof = merkle.prove(&end)?;
        assert!(!end_proof.0.is_empty());
        proof.extend(end_proof);

        let item_iter = items.into_iter();
        let keys = item_iter.clone().map(|item| item.0).collect();
        let vals = item_iter.map(|item| item.1).collect();

        merkle.verify_range_proof(&proof, start, end, keys, vals)?;

        Ok(())
    }

    #[test]
    #[allow(clippy::indexing_slicing)]
    // Tests a malicious proof, where the proof is more or less the
    // whole trie. This is to match corresponding test in geth.
    fn test_bloadted_range_proof() -> Result<(), ProofError> {
        // Use a small trie
        let mut items = Vec::new();
        for i in 0..100_u32 {
            let mut key: [u8; 32] = [0; 32];
            let mut value: [u8; 20] = [0; 20];
            for (index, d) in i.to_be_bytes().iter().enumerate() {
                key[index] = *d;
                value[index] = *d;
            }
            items.push((key, value));
        }
        let merkle = merkle_build_test(items.clone())?;

        // In the 'malicious' case, we add proofs for every single item
        // (but only one key/value pair used as leaf)
        let mut proof = Proof(HashMap::new());
        let mut keys = Vec::new();
        let mut vals = Vec::new();
        for (i, item) in items.iter().enumerate() {
            let cur_proof = merkle.prove(item.0)?;
            assert!(!cur_proof.0.is_empty());
            proof.extend(cur_proof);
            if i == 50 {
                keys.push(item.0);
                vals.push(item.1);
            }
        }

        merkle.verify_range_proof(&proof, keys[0], keys[keys.len() - 1], keys, vals)?;

        Ok(())
    }

    // generate pseudorandom data, but prefix it with some known data
    // The number of fixed data points is 100; you specify how much random data you want
    #[allow(clippy::indexing_slicing)]
    fn fixed_and_pseudorandom_data(random_count: u32) -> HashMap<[u8; 32], [u8; 20]> {
        let mut items: HashMap<[u8; 32], [u8; 20]> = HashMap::new();
        for i in 0..100_u32 {
            let mut key: [u8; 32] = [0; 32];
            let mut value: [u8; 20] = [0; 20];
            for (index, d) in i.to_be_bytes().iter().enumerate() {
                key[index] = *d;
                value[index] = *d;
            }
            items.insert(key, value);

            let mut more_key: [u8; 32] = [0; 32];
            for (index, d) in (i + 10).to_be_bytes().iter().enumerate() {
                more_key[index] = *d;
            }
            items.insert(more_key, value);
        }

        // read FIREWOOD_TEST_SEED from the environment. If it's there, parse it into a u64.
        let seed = std::env::var("FIREWOOD_TEST_SEED")
            .ok()
            .map_or_else(
                || None,
                |s| Some(str::parse(&s).expect("couldn't parse FIREWOOD_TEST_SEED; must be a u64")),
            )
            .unwrap_or_else(|| thread_rng().gen());

        // the test framework will only render this in verbose mode or if the test fails
        // to re-run the test when it fails, just specify the seed instead of randomly
        // selecting one
        eprintln!("Seed {seed}: to rerun with this data, export FIREWOOD_TEST_SEED={seed}");
        let mut r = StdRng::seed_from_u64(seed);
        for _ in 0..random_count {
            let key = r.gen::<[u8; 32]>();
            let val = r.gen::<[u8; 20]>();
            items.insert(key, val);
        }
        items
    }

    fn increase_key(key: &[u8; 32]) -> [u8; 32] {
        let mut new_key = *key;
        for ch in new_key.iter_mut().rev() {
            let overflow;
            (*ch, overflow) = ch.overflowing_add(1);
            if !overflow {
                break;
            }
        }
        new_key
    }

    fn decrease_key(key: &[u8; 32]) -> [u8; 32] {
        let mut new_key = *key;
        for ch in new_key.iter_mut().rev() {
            let overflow;
            (*ch, overflow) = ch.overflowing_sub(1);
            if !overflow {
                break;
            }
        }
        new_key
    }
}
