// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.
use crate::node::Node;
use crate::proof::{Proof, ProofError};
use crate::storage::linear::{ReadLinearStore, WriteLinearStore};
use crate::storage::node::{self, LinearAddress};
use crate::stream::{MerkleKeyValueStream, PathIterator};
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
pub struct Merkle<T>(node::NodeStore<T>);

impl<T: ReadLinearStore> Deref for Merkle<T> {
    type Target = node::NodeStore<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: WriteLinearStore> DerefMut for Merkle<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: ReadLinearStore> Merkle<T> {
    pub const fn new(store: node::NodeStore<T>) -> Merkle<T> {
        Merkle(store)
    }

    // TODO: remove me
    pub fn get_node(&self, _addr: LinearAddress) -> Result<&Node, MerkleError> {
        todo!()
    }
}

impl<T: WriteLinearStore> Merkle<T> {
    pub fn root_hash(&self) -> Result<TrieHash, MerkleError> {
        todo!()
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
    pub fn insert<K: AsRef<[u8]>>(&mut self, _key: K, _val: Vec<u8>) -> Result<(), MerkleError> {
        todo!()
    }

    pub fn remove<K: AsRef<[u8]>>(&mut self, _key: K) -> Result<Option<Vec<u8>>, MerkleError> {
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

// nibbles, high bits first, then low bits
pub const fn to_nibble_array(x: u8) -> [u8; 2] {
    [x >> 4, x & 0b_0000_1111]
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
pub(super) mod tests {
    use super::*;
    use test_case::test_case;

    #[test_case(vec![0x12, 0x34, 0x56], &[0x1, 0x2, 0x3, 0x4, 0x5, 0x6])]
    #[test_case(vec![0xc0, 0xff], &[0xc, 0x0, 0xf, 0xf])]
    fn to_nibbles(bytes: Vec<u8>, nibbles: &[u8]) {
        let n: Vec<_> = bytes.into_iter().flat_map(to_nibble_array).collect();
        assert_eq!(n, nibbles);
    }

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
