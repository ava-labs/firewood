// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![expect(clippy::indexing_slicing, clippy::unwrap_used)]

#[cfg(feature = "ethhash")]
mod ethhash;
// TODO: get the hashes from merkledb and verify compatibility with branch factor 256
#[cfg(not(any(feature = "ethhash", feature = "branch_factor_256")))]
mod triehash;
mod unvalidated;

use std::collections::HashMap;

use super::*;
use firewood_storage::{Committed, MemStore, MutableProposal, NodeStore, RootReader, TrieHash};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng, rng};

// Returns n random key-value pairs.
fn generate_random_kvs(seed: u64, n: usize) -> Vec<(Vec<u8>, Vec<u8>)> {
    eprintln!("Seed {seed}: to rerun with this data, export FIREWOOD_TEST_SEED={seed}");

    let mut rng = StdRng::seed_from_u64(seed);

    let mut kvs: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for _ in 0..n {
        let key_len = rng.random_range(1..=4096);
        let key: Vec<u8> = (0..key_len).map(|_| rng.random()).collect();

        let val_len = rng.random_range(1..=4096);
        let val: Vec<u8> = (0..val_len).map(|_| rng.random()).collect();

        kvs.push((key, val));
    }

    kvs
}

fn into_committed(
    merkle: Merkle<NodeStore<Arc<ImmutableProposal>, MemStore>>,
    parent: &NodeStore<Committed, MemStore>,
) -> Merkle<NodeStore<Committed, MemStore>> {
    let ns = merkle.into_inner();
    ns.flush_freelist().unwrap();
    ns.flush_header().unwrap();
    let mut ns = ns.as_committed(parent);
    ns.flush_nodes().unwrap();
    ns.into()
}

fn init_merkle<I, K, V>(iter: I) -> Merkle<NodeStore<Committed, MemStore>>
where
    I: Clone + IntoIterator<Item = (K, V)>,
    K: AsRef<[u8]>,
    V: AsRef<[u8]>,
{
    let memstore = Arc::new(MemStore::new(Vec::with_capacity(64 * 1024)));
    let base = Merkle::from(NodeStore::new_empty_committed(memstore.clone()).unwrap());
    let mut merkle = base.fork().unwrap();

    for (k, v) in iter.clone() {
        let key = k.as_ref();
        let value = v.as_ref();

        merkle.insert(key, value.into()).unwrap();

        assert_eq!(
            merkle.get_value(key).unwrap().as_deref(),
            Some(value),
            "Failed to insert key: {key:?}",
        );
    }

    for (k, v) in iter.clone() {
        let key = k.as_ref();
        let value = v.as_ref();

        assert_eq!(
            merkle.get_value(key).unwrap().as_deref(),
            Some(value),
            "Failed to get key after insert: {key:?}",
        );
    }

    let merkle = merkle.hash();

    for (k, v) in iter.clone() {
        let key = k.as_ref();
        let value = v.as_ref();

        assert_eq!(
            merkle.get_value(key).unwrap().as_deref(),
            Some(value),
            "Failed to get key after hashing: {key:?}"
        );
    }

    let merkle = into_committed(merkle, base.nodestore());

    for (k, v) in iter {
        let key = k.as_ref();
        let value = v.as_ref();

        assert_eq!(
            merkle.get_value(key).unwrap().as_deref(),
            Some(value),
            "Failed to get key after committing: {key:?}"
        );
    }

    merkle
}

// generate pseudorandom data, but prefix it with some known data
// The number of fixed data points is 100; you specify how much random data you want
#[expect(clippy::arithmetic_side_effects)]
fn fixed_and_pseudorandom_data(random_count: u32) -> HashMap<[u8; 32], [u8; 20]> {
    let mut items = HashMap::new();
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
        .unwrap_or_else(|| rng().random());

    // the test framework will only render this in verbose mode or if the test fails
    // to re-run the test when it fails, just specify the seed instead of randomly
    // selecting one
    eprintln!("Seed {seed}: to rerun with this data, export FIREWOOD_TEST_SEED={seed}");
    let mut r = StdRng::seed_from_u64(seed);
    for _ in 0..random_count {
        let key = r.random::<[u8; 32]>();
        let val = r.random::<[u8; 20]>();
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

#[test]
fn test_get_regression() {
    let mut merkle: Merkle<NodeStore<MutableProposal, MemStore>> = create_in_memory_merkle();

    let root = merkle.nodestore.mut_root();
    let mut root_node = std::mem::take(root); 
    let mut merkle_arc = Arc::new(merkle);
    let worker_pool: WorkerPool<MemStore>= WorkerPool::new(merkle_arc.clone());
/* 
    let _ = worker_pool.insert(None, 0, &[0], Box::new([0]));



    // For now just send it to index 0
    let _ = merkle_arc.insert_worker_pool(None, &worker_pool, 0, &[0], Box::new([0]));
    let _ = merkle_arc.insert_worker_pool(None, &worker_pool, 0, &[1], Box::new([1]));
    let _ = merkle_arc.insert_worker_pool(None, &worker_pool, 0, &[2], Box::new([2]));
*/
    let key_range = 255;
    for j in 0..key_range {
        let key = [j];
        let insert_result = merkle_arc.insert_parallel(root_node, &worker_pool, &key, Box::new([j]));
        match insert_result.unwrap() {
            ParallelInsertReturn::Performed(node) => {
                println!("--> Performed {node:?}");
                root_node = Some(node);
            },
            ParallelInsertReturn::RetryNonThreaded(mut node, value ) => {
                // TODO: Need to update clear merkle
                let mut child_nodes: Vec<Option<Node>> = worker_pool.clear_merkle().expect("file io error");
                println!("-----------*** Current root: {node:?}");

                for (i, cur_node) in child_nodes.iter_mut().enumerate() {
                    // If child_nodes is not empty, then node must be a branch
                    if cur_node.is_none() {
                        println!("Skipping index: {i}");

                        match node {
                            Node::Branch(ref mut branch_node) => {
                                //let (_path, _key, child_array) = *branch_node;
                                //let a = cur_node.take().unwrap();
                                //branch_node.children[i] = Some(Child::Node(a));
                                println!("Branch node children at index {i} is {:?}", branch_node.children[i]);
                            },
                            Node::Leaf(_) => {}
                        }
                        continue;
                    }

                    println!("RetryNonThreaded: index: {i} child root: {cur_node:?}");

                    match node {
                        Node::Branch(ref mut branch_node) => {
                            //let (_path, _key, child_array) = *branch_node;
                            let a = cur_node.take().unwrap();
                            branch_node.children[i] = Some(Child::Node(a));
                        },
                        Node::Leaf(_) => {}
                    }
                }

                // All but one of the references to merkle_arc should be gone. Extract
                // the inner Merkle should we can perform a mut operations on it.
                let mut merkle = Arc::into_inner(merkle_arc).unwrap();
                *merkle.nodestore.mut_root() = Some(node);
                println!("^^^^^^^^^^^^^^^^^^^^ Falling back to serialized Merkle insert for {key:?}");
                merkle.insert(&key, value).unwrap();

                let root = merkle.nodestore.mut_root();
                root_node = std::mem::take(root); 


                println!("-----------*** Current root: {:?}", root_node);

                let root = root_node.unwrap();

                match root.clone() {
                    Node::Branch(mut node) => {
                        for (i, cur_node) in node.children.iter_mut().enumerate() {
                            // If child_nodes is not empty, then node must be a branch
                            println!("Iterating root children: at index {i} is {cur_node:?}");
                        }
                    }
                    Node::Leaf(_) => {}
                }
                root_node = Some(root);

                merkle_arc = Arc::new(merkle);
                worker_pool.set_merkle(merkle_arc.clone());
            }
        }
    }
    
    //merkle_arc.insert_parallel(&worker_pool, &[0], Box::new([0]));
    //merkle_arc.insert_parallel(&worker_pool, &[0], Box::new([0]));


    let mut node = root_node.unwrap();
    println!("-----------*** END: Current root: {node:?}");
    let mut child_nodes: Vec<Option<Node>> = worker_pool.clear_merkle().expect("file io error");
    for (i, cur_node) in child_nodes.iter_mut().enumerate() {
        // If child_nodes is not empty, then node must be a branch
        if cur_node.is_none() {
            println!("Skipping index: {i}");

            match node {
                Node::Branch(ref mut branch_node) => {
                    //let (_path, _key, child_array) = *branch_node;
                    //let a = cur_node.take().unwrap();
                    //branch_node.children[i] = Some(Child::Node(a));
                    println!("Branch node children at index {i} is {:?}", branch_node.children[i]);
                },
                Node::Leaf(_) => {}
            }
            continue;
        }

        println!("END Children: index: {i} child root: {cur_node:?}");
        match node {
            Node::Branch(ref mut branch_node) => {
                //let (_path, _key, child_array) = *branch_node;
                let a = cur_node.take().unwrap();
                branch_node.children[i] = Some(Child::Node(a));
            },
            Node::Leaf(_) => {}
        }
    }

    // All but one of the references to merkle_arc should be gone. Extract
    // the inner Merkle should we can perform a mut operations on it.
    let mut merkle = Arc::into_inner(merkle_arc).unwrap();
    *merkle.nodestore.mut_root() = Some(node);

    

    // Wait until all of the previous inserts are complete
    //let new_root = worker_pool.clear_merkle();

    // All but one of the references to merkle_arc should be gone. Extract
    // the inner Merkle should we can perform a mut operations on it.
    //let mut merkle = Arc::into_inner(merkle_arc).unwrap();
    //*merkle.nodestore.mut_root() = new_root.unwrap();
    //*merkle.nodestore.mut_root() = root_node;

    //merkle.insert(&[1], Box::new([1])).unwrap();
    //merkle.insert(&[2], Box::new([2])).unwrap();

    //merkle.insert(&[0], Box::new([0])).unwrap();

    for j in 0..key_range {
        assert_eq!(merkle.get_value(&[j]).unwrap(), Some(Box::from([j])));
    }

    assert_eq!(merkle.get_value(&[0]).unwrap(), Some(Box::from([0])));

    //merkle.insert(&[1], Box::new([1])).unwrap();
    assert_eq!(merkle.get_value(&[1]).unwrap(), Some(Box::from([1])));

    //merkle.insert(&[2], Box::new([2])).unwrap();
    assert_eq!(merkle.get_value(&[2]).unwrap(), Some(Box::from([2])));

    let merkle = merkle.hash();

    assert_eq!(merkle.get_value(&[0]).unwrap(), Some(Box::from([0])));
    assert_eq!(merkle.get_value(&[1]).unwrap(), Some(Box::from([1])));
    assert_eq!(merkle.get_value(&[2]).unwrap(), Some(Box::from([2])));

    for result in merkle.path_iter(&[2]).unwrap() {
        result.unwrap();
    }
}

#[test]
fn insert_one() {
    let mut merkle = create_in_memory_merkle();
    merkle.insert(b"abc", Box::new([])).unwrap();
}

fn create_in_memory_merkle() -> Merkle<NodeStore<MutableProposal, MemStore>> {
    let memstore = MemStore::new(vec![]);

    let nodestore = NodeStore::new_empty_proposal(memstore.into());

    Merkle { nodestore }
}

#[test]
fn test_insert_and_get() {
    let mut merkle = create_in_memory_merkle();

    // insert values
    for key_val in u8::MIN..=u8::MAX {
        let key = vec![key_val];
        let val = Box::new([key_val]);

        merkle.insert(&key, val.clone()).unwrap();

        let fetched_val = merkle.get_value(&key).unwrap();

        // make sure the value was inserted
        assert_eq!(fetched_val.as_deref(), val.as_slice().into());
    }

    // make sure none of the previous values were forgotten after initial insert
    for key_val in u8::MIN..=u8::MAX {
        let key = vec![key_val];
        let val = vec![key_val];

        let fetched_val = merkle.get_value(&key).unwrap();

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
    assert!(merkle.get_value(&key0).unwrap().is_none());
    // Removing an already removed key is a no-op
    assert!(merkle.remove(&key0).unwrap().is_none());

    // Trie is:
    //   key1
    //  /    \
    // key2  key3
    // Test removal of root when it's a branch with multiple children
    assert_eq!(merkle.remove(&key1).unwrap(), Some(Box::from(val1)));
    assert!(merkle.get_value(&key1).unwrap().is_none());
    assert!(merkle.remove(&key1).unwrap().is_none());

    // Trie is:
    //   key1 (now has no value)
    //  /    \
    // key2  key3
    let removed_val = merkle.remove(&key2).unwrap();
    assert_eq!(removed_val, Some(Box::from(val2)));
    assert!(merkle.get_value(&key2).unwrap().is_none());
    assert!(merkle.remove(&key2).unwrap().is_none());

    // Trie is:
    // key3
    let removed_val = merkle.remove(&key3).unwrap();
    assert_eq!(removed_val, Some(Box::from(val3)));
    assert!(merkle.get_value(&key3).unwrap().is_none());
    assert!(merkle.remove(&key3).unwrap().is_none());

    assert!(merkle.nodestore.root_node().is_none());
}

#[test]
fn remove_prefix_exact() {
    let mut merkle = two_byte_all_keys();
    for key_val in u8::MIN..=u8::MAX {
        let key = [key_val];
        let got = merkle.remove_prefix(&key).unwrap();
        assert_eq!(got, 1);
        let got = merkle.get_value(&key).unwrap();
        assert!(got.is_none());
    }
}

fn two_byte_all_keys() -> Merkle<NodeStore<MutableProposal, MemStore>> {
    let mut merkle = create_in_memory_merkle();
    for key_val in u8::MIN..=u8::MAX {
        let key = [key_val, key_val];
        let val = [key_val];

        merkle.insert(&key, Box::new(val)).unwrap();
        let got = merkle.get_value(&key).unwrap().unwrap();
        assert_eq!(&*got, val);
    }
    merkle
}

#[test]
fn remove_prefix_all() {
    let mut merkle = two_byte_all_keys();
    let got = merkle.remove_prefix(&[]).unwrap();
    assert_eq!(got, 256);
}

#[test]
fn remove_prefix_partial() {
    let mut merkle = create_in_memory_merkle();
    merkle
        .insert(b"abc", Box::from(b"value".as_slice()))
        .unwrap();
    merkle
        .insert(b"abd", Box::from(b"value".as_slice()))
        .unwrap();
    let got = merkle.remove_prefix(b"ab").unwrap();
    assert_eq!(got, 2);
}

#[test]
fn remove_many() {
    let mut merkle = create_in_memory_merkle();

    // insert key-value pairs
    for key_val in u8::MIN..=u8::MAX {
        let key = [key_val];
        let val = [key_val];

        merkle.insert(&key, Box::new(val)).unwrap();
        let got = merkle.get_value(&key).unwrap().unwrap();
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

        let got = merkle.get_value(&key).unwrap();
        assert!(got.is_none());
    }
    assert!(merkle.nodestore.root_node().is_none());
}

#[test]
fn remove_prefix() {
    let mut merkle = create_in_memory_merkle();

    // insert key-value pairs
    for key_val in u8::MIN..=u8::MAX {
        let key = [key_val, key_val];
        let val = [key_val];

        merkle.insert(&key, Box::new(val)).unwrap();
        let got = merkle.get_value(&key).unwrap().unwrap();
        assert_eq!(&*got, val);
    }

    // remove key-value pairs with prefix [0]
    let prefix = [0];
    assert_eq!(merkle.remove_prefix(&[0]).unwrap(), 1);

    // make sure all keys with prefix [0] were removed
    for key_val in u8::MIN..=u8::MAX {
        let key = [key_val, key_val];
        let got = merkle.get_value(&key).unwrap();
        if key[0] == prefix[0] {
            assert!(got.is_none());
        } else {
            assert!(got.is_some());
        }
    }
}

#[test]
fn get_empty_proof() {
    let merkle = create_in_memory_merkle().hash();
    let proof = merkle.prove(b"any-key");
    assert!(matches!(proof.unwrap_err(), ProofError::Empty));
}

#[test]
fn single_key_proof() {
    const TEST_SIZE: usize = 1;

    let mut merkle = create_in_memory_merkle();

    let seed = std::env::var("FIREWOOD_TEST_SEED")
        .ok()
        .map_or_else(
            || None,
            |s| Some(str::parse(&s).expect("couldn't parse FIREWOOD_TEST_SEED; must be a u64")),
        )
        .unwrap_or_else(|| rng().random());

    let kvs = generate_random_kvs(seed, TEST_SIZE);

    for (key, val) in &kvs {
        merkle.insert(key, val.clone().into_boxed_slice()).unwrap();
    }

    let merkle = merkle.hash();

    let root_hash = merkle.nodestore.root_hash().unwrap();

    for (key, value) in kvs {
        let proof = merkle.prove(&key).unwrap();

        proof
            .verify(key.clone(), Some(value.clone()), &root_hash)
            .unwrap();

        {
            // Test that the proof is invalid when the value is different
            let mut value = value.clone();
            value[0] = value[0].wrapping_add(1);
            assert!(proof.verify(key.clone(), Some(value), &root_hash).is_err());
        }

        {
            // Test that the proof is invalid when the hash is different
            assert!(
                proof
                    .verify(key, Some(value), &TrieHash::default())
                    .is_err()
            );
        }
    }
}

#[tokio::test]
async fn empty_range_proof() {
    let merkle = create_in_memory_merkle();

    assert!(matches!(
        merkle.range_proof(None, None, None).await.unwrap_err(),
        api::Error::RangeProofOnEmptyTrie
    ));
}

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

    let got = merkle.get_value(&key).unwrap().unwrap();

    assert_eq!(*got, val);

    let got = merkle.get_value(&key_2).unwrap().unwrap();
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

    let got = merkle.get_value(&key).unwrap().unwrap();
    assert_eq!(*got, val);

    let got = merkle.get_value(&key_2).unwrap().unwrap();
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

    let got = merkle.get_value(&key).unwrap().unwrap();
    assert_eq!(*got, val);

    let got = merkle.get_value(&key_2).unwrap().unwrap();
    assert_eq!(*got, val_2);

    let got = merkle.get_value(&key_3).unwrap().unwrap();
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

    let got = merkle.get_value(&key).unwrap().unwrap();
    assert_eq!(&*got, val);

    let got = merkle.get_value(&key_2).unwrap().unwrap();
    assert_eq!(&*got, val_2);

    let got = merkle.get_value(&key_3).unwrap().unwrap();
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

    let got = merkle.get_value(&key).unwrap().unwrap();
    assert_eq!(*got, val);

    let got = merkle.get_value(&key_2).unwrap().unwrap();
    assert_eq!(*got, val_2);

    merkle.insert(&key, Box::new(overwrite)).unwrap();

    let got = merkle.get_value(&key).unwrap().unwrap();
    assert_eq!(*got, overwrite);

    let got = merkle.get_value(&key_2).unwrap().unwrap();
    assert_eq!(*got, val_2);
}

#[test]
fn test_delete_one_child_with_branch_value() {
    let mut merkle = create_in_memory_merkle();
    // insert a parent with a value
    merkle.insert(&[0], Box::new([42u8])).unwrap();
    // insert child1 with a value
    merkle.insert(&[0, 1], Box::new([43u8])).unwrap();
    // insert child2 with a value
    merkle.insert(&[0, 2], Box::new([44u8])).unwrap();

    // now delete one of the children
    let deleted = merkle.remove(&[0, 1]).unwrap();
    assert_eq!(deleted, Some([43u8].to_vec().into_boxed_slice()));

    // make sure the parent still has the correct value
    let got = merkle.get_value(&[0]).unwrap().unwrap();
    assert_eq!(*got, [42u8]);

    // and check the remaining child
    let other_child = merkle.get_value(&[0, 2]).unwrap().unwrap();
    assert_eq!(*other_child, [44u8]);
}

#[test]
fn test_root_hash_simple_insertions() -> Result<(), Error> {
    init_merkle([
        ("do", "verb"),
        ("doe", "reindeer"),
        ("dog", "puppy"),
        ("doge", "coin"),
        ("horse", "stallion"),
        ("ddd", "ok"),
    ])
    .dump()
    .unwrap();
    Ok(())
}

#[test]
fn test_root_hash_fuzz_insertions() -> Result<(), FileIoError> {
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};
    let rng = std::cell::RefCell::new(StdRng::seed_from_u64(42));
    let max_len0 = 8;
    let max_len1 = 4;
    let keygen = || {
        let (len0, len1): (usize, usize) = {
            let mut rng = rng.borrow_mut();
            (
                rng.random_range(1..=max_len0),
                rng.random_range(1..=max_len1),
            )
        };
        let key: Vec<u8> = (0..len0)
            .map(|_| rng.borrow_mut().random_range(0..2))
            .chain((0..len1).map(|_| rng.borrow_mut().random()))
            .collect();
        key
    };

    // TODO: figure out why this fails if we use more than 27 iterations with branch_factor_256
    for _ in 0..27 {
        let mut items = Vec::new();

        for _ in 0..100 {
            let val: Vec<u8> = (0..256).map(|_| rng.borrow_mut().random()).collect();
            items.push((keygen(), val));
        }

        init_merkle(items);
    }

    Ok(())
}

#[test]
fn test_delete_child() {
    let items = vec![("do", "verb")];
    let merkle = init_merkle(items);
    let mut merkle = merkle.fork().unwrap();

    assert_eq!(merkle.remove(b"does_not_exist").unwrap(), None);
    assert_eq!(&*merkle.get_value(b"do").unwrap().unwrap(), b"verb");
}

#[test]
fn test_delete_some() {
    let items = (0..100)
        .map(|n| {
            let key = format!("key{n}");
            let val = format!("value{n}");
            (key.as_bytes().to_vec(), val.as_bytes().to_vec())
        })
        .collect::<Vec<(Vec<u8>, Vec<u8>)>>();
    let mut merkle = init_merkle(items.clone()).fork().unwrap();
    merkle.remove_prefix(b"key1").unwrap();
    for item in items {
        let (key, val) = item;
        if key.starts_with(b"key1") {
            assert!(merkle.get_value(&key).unwrap().is_none());
        } else {
            assert_eq!(&*merkle.get_value(&key).unwrap().unwrap(), val.as_slice());
        }
    }
}

#[test]
fn test_root_hash_reversed_deletions() -> Result<(), FileIoError> {
    use rand::rngs::StdRng;
    use rand::{Rng, SeedableRng};

    let _ = env_logger::Builder::new().is_test(true).try_init();

    let seed = std::env::var("FIREWOOD_TEST_SEED")
        .ok()
        .map_or_else(
            || None,
            |s| Some(str::parse(&s).expect("couldn't parse FIREWOOD_TEST_SEED; must be a u64")),
        )
        .unwrap_or_else(|| rng().random());

    eprintln!("Seed {seed}: to rerun with this data, export FIREWOOD_TEST_SEED={seed}");
    let rng = std::cell::RefCell::new(StdRng::seed_from_u64(seed));

    let max_len0 = 8;
    let max_len1 = 4;
    let keygen = || {
        let (len0, len1): (usize, usize) = {
            let mut rng = rng.borrow_mut();
            (
                rng.random_range(1..=max_len0),
                rng.random_range(1..=max_len1),
            )
        };
        (0..len0)
            .map(|_| rng.borrow_mut().random_range(0..2))
            .chain((0..len1).map(|_| rng.borrow_mut().random()))
            .collect()
    };

    for _ in 0..10 {
        let mut items: Vec<(Key, Value)> = (0..10)
            .map(|_| keygen())
            .map(|key| {
                let val = (0..8).map(|_| rng.borrow_mut().random()).collect();
                (key, val)
            })
            .collect();

        items.sort_unstable();
        items.dedup_by_key(|(k, _)| k.clone());

        let init_merkle = create_in_memory_merkle();
        let init_immutable_merkle = init_merkle.hash();

        let (hashes, complete_immutable_merkle) = items.iter().fold(
            (vec![], init_immutable_merkle),
            |(mut hashes, immutable_merkle), (k, v)| {
                let root_hash = immutable_merkle.nodestore.root_hash();
                hashes.push(root_hash);
                let mut merkle = immutable_merkle.fork().unwrap();
                merkle.insert(k, v.clone()).unwrap();
                (hashes, merkle.hash())
            },
        );

        let (new_hashes, _) = items.iter().rev().fold(
            (vec![], complete_immutable_merkle),
            |(mut new_hashes, immutable_merkle_before_removal), (k, _)| {
                let before = immutable_merkle_before_removal.dump().unwrap();
                let mut merkle = Merkle::from(
                    NodeStore::new(immutable_merkle_before_removal.nodestore()).unwrap(),
                );
                merkle.remove(k).unwrap();
                let immutable_merkle_after_removal: Merkle<NodeStore<Arc<ImmutableProposal>, _>> =
                    merkle.try_into().unwrap();
                new_hashes.push((
                    immutable_merkle_after_removal.nodestore.root_hash(),
                    k,
                    before,
                    immutable_merkle_after_removal.dump().unwrap(),
                ));
                (new_hashes, immutable_merkle_after_removal)
            },
        );

        for (expected_hash, (actual_hash, key, before_removal, after_removal)) in
            hashes.into_iter().rev().zip(new_hashes)
        {
            let key = key.iter().fold(String::new(), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            });
            assert_eq!(
                actual_hash, expected_hash,
                "\n\nkey: {key}\nbefore:\n{before_removal}\nafter:\n{after_removal}\n\nexpected:\n{expected_hash:?}\nactual:\n{actual_hash:?}\n",
            );
        }
    }

    Ok(())
}

#[test]
fn remove_nonexistent_with_one() {
    let items = [("do", "verb")];
    let mut merkle = init_merkle(items).fork().unwrap();

    assert_eq!(merkle.remove(b"does_not_exist").unwrap(), None);
    assert_eq!(&*merkle.get_value(b"do").unwrap().unwrap(), b"verb");
}
