// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use crate::api::OptionalHashKeyExt;

use super::*;
use ethereum_types::H256;
use hash_db::Hasher;
use plain_hasher::PlainHasher;
use sha3::{Digest, Keccak256};
use test_case::test_case;

#[derive(Default, Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeccakHasher;

impl KeccakHasher {
    fn trie_root<I, K, V>(items: I) -> H256
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<[u8]> + Ord,
        V: AsRef<[u8]>,
    {
        firewood_triehash::trie_root::<Self, _, _, _>(items)
    }
}

impl Hasher for KeccakHasher {
    type Out = H256;
    type StdHasher = PlainHasher;
    const LENGTH: usize = 32;

    #[inline]
    fn hash(x: &[u8]) -> Self::Out {
        let mut hasher = Keccak256::new();
        hasher.update(x);
        let result = hasher.finalize();
        H256::from_slice(result.as_slice())
    }
}

#[test_case([("doe", "reindeer")])]
#[test_case([("doe", "reindeer"),("dog", "puppy"),("dogglesworth", "cat")])]
#[test_case([("doe", "reindeer"),("dog", "puppy"),("dogglesworth", "cacatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatt")])]
#[test_case([("dogglesworth", "cacatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatcatt")])]
fn test_root_hash_eth_compatible<I, K, V>(kvs: I)
where
    I: Clone + IntoIterator<Item = (K, V)>,
    K: AsRef<[u8]> + Ord,
    V: AsRef<[u8]>,
{
    let merkle = init_merkle(kvs.clone());
    let firewood_hash = merkle.nodestore.root_hash().unwrap_or_else(TrieHash::empty);
    let eth_hash: TrieHash = KeccakHasher::trie_root(kvs).to_fixed_bytes().into();
    assert_eq!(firewood_hash, eth_hash);
}

#[test_case(
            "0000000000000000000000000000000000000002",
            "f844802ca00000000000000000000000000000000000000000000000000000000000000000a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
            &[],
            "c00ca9b8e6a74b03f6b1ae2db4a65ead348e61b74b339fe4b117e860d79c7821"
    )]
#[test_case(
            "0000000000000000000000000000000000000002",
            "f844802ca00000000000000000000000000000000000000000000000000000000000000000a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
            &[
                    ("48078cfed56339ea54962e72c37c7f588fc4f8e5bc173827ba75cb10a63a96a5", "a00200000000000000000000000000000000000000000000000000000000000000")
            ],
            "91336bf4e6756f68e1af0ad092f4a551c52b4a66860dc31adbd736f0acbadaf6"
    )]
#[test_case(
            "0000000000000000000000000000000000000002",
            "f844802ca00000000000000000000000000000000000000000000000000000000000000000a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
            &[
                    ("48078cfed56339ea54962e72c37c7f588fc4f8e5bc173827ba75cb10a63a96a5", "a00200000000000000000000000000000000000000000000000000000000000000"),
                    ("0e81f83a84964b811dd1b8328262a9f57e6bc3e5e7eb53627d10437c73c4b8da", "a02800000000000000000000000000000000000000000000000000000000000000"),
            ],
            "c267104830880c966c2cc8c669659e4bfaf3126558dbbd6216123b457944001b"
    )]
fn test_eth_compatible_accounts(
    account: &str,
    account_value: &str,
    key_suffixes_and_values: &[(&str, &str)],
    expected_root: &str,
) {
    use sha3::Digest as _;
    use sha3::Keccak256;

    let account = make_key(account);
    let expected_key_hash = Keccak256::digest(&account);

    let items = once((
        Box::from(expected_key_hash.as_slice()),
        make_key(account_value),
    ))
    .chain(key_suffixes_and_values.iter().map(|(key_suffix, value)| {
        let key = expected_key_hash
            .iter()
            .copied()
            .chain(make_key(key_suffix).iter().copied())
            .collect();
        let value = make_key(value);
        (key, value)
    }))
    .collect::<Vec<(Box<_>, Box<_>)>>();

    let merkle = init_merkle(items);
    let firewood_hash = merkle.nodestore.root_hash();

    assert_eq!(
        firewood_hash,
        TrieHash::try_from(&*make_key(expected_root)).ok()
    );
}

/// helper method to convert a hex encoded string into a boxed slice
fn make_key(hex_str: &str) -> Key {
    hex::decode(hex_str).unwrap().into_boxed_slice()
}

/// Keccak256 of empty bytes — the codeHash for accounts with no contract code.
fn empty_code_hash() -> [u8; 32] {
    Keccak256::digest([]).into()
}

/// Keccak256 of RLP-encoded empty string (0x80) — the hash of an empty storage trie.
fn empty_trie_root() -> [u8; 32] {
    Keccak256::digest(rlp::NULL_RLP).into()
}

/// RLP-encode an Ethereum account value: [nonce, balance, storageRoot, codeHash].
fn rlp_encode_account(
    nonce: u64,
    balance: u64,
    storage_root: &[u8; 32],
    code_hash: &[u8; 32],
) -> Box<[u8]> {
    use rlp::RlpStream;

    let mut rlp = RlpStream::new_list(4);
    rlp.append(&nonce);
    rlp.append(&balance);
    rlp.append(&storage_root.as_slice());
    rlp.append(&code_hash.as_slice());
    rlp.out().to_vec().into_boxed_slice()
}

/// Insert an account (and optional storage entries) into a trie, commit it,
/// then read back the account value and verify the storageRoot field was
/// updated from the original `input_storage_root`.
fn commit_and_read_storage_root(
    account_key: &[u8],
    account_value: &[u8],
    input_storage_root: &[u8; 32],
    storage_entries: &[(&[u8], &[u8])],
) -> Vec<u8> {
    use rlp::Rlp;

    let account_key_hash = Keccak256::digest(account_key);

    let mut items = vec![(
        Box::from(account_key_hash.as_slice()),
        Box::from(account_value),
    )];
    for (suffix, value) in storage_entries {
        let key: Box<[u8]> = [account_key_hash.as_slice(), *suffix].concat().into();
        items.push((key, Box::from(*value)));
    }

    let merkle = init_merkle(items);

    let stored = merkle
        .get_value(account_key_hash.as_slice())
        .unwrap()
        .expect("account should exist");

    let rlp = Rlp::new(&stored);
    let list: Vec<Vec<u8>> = rlp.as_list().unwrap();
    assert!(
        list.len() >= 3,
        "account value should have at least 3 RLP items"
    );

    let persisted_storage_root = list.into_iter().nth(2).unwrap();
    assert_ne!(
        persisted_storage_root.as_slice(),
        input_storage_root.as_slice(),
        "storageRoot should have been updated from its original value"
    );
    persisted_storage_root
}

/// Verify that storageRoot is autocomputed during hashing for both
/// 4-item (standard) and 5-item (coreth with trailing empty byte) account RLP.
#[test_case(&[1u64, 0, 0, 0]; "4 item")]
#[test_case(&[1u64, 0, 0, 0, 0]; "5 item")]
fn test_autocompute_hash(fields: &[u64]) {
    use rlp::RlpStream;

    let account_addr = [0u8; 20];
    let dummy_storage_root = [0u8; 32];

    let mut rlp = RlpStream::new_list(fields.len());
    for field in fields {
        rlp.append(field);
    }
    let account_value: Box<[u8]> = rlp.out().to_vec().into();

    let storage_root =
        commit_and_read_storage_root(&account_addr, &account_value, &dummy_storage_root, &[]);

    assert_eq!(
        storage_root,
        empty_trie_root(),
        "storageRoot should be autocomputed as empty trie root"
    );
}

/// RLP-encode a 32-byte storage slot value.
fn rlp_encode_storage(value: &[u8; 32]) -> Vec<u8> {
    use rlp::RlpStream;

    let mut rlp = RlpStream::new();
    rlp.append(&value.as_slice());
    rlp.out().to_vec()
}

/// A branch account (one storage entry) should have its storageRoot set to
/// the hash of the single-node storage sub-trie.
#[test]
fn test_persisted_storage_root_one_storage_entry() {
    let account_addr = [0u8; 20];
    let dummy_storage_root = [0u8; 32];
    let account_value = rlp_encode_account(0, 44, &dummy_storage_root, &empty_code_hash());

    let storage_key = [1u8; 32];
    let storage_value = rlp_encode_storage(&[2u8; 32]);

    let storage_root = commit_and_read_storage_root(
        &account_addr,
        &account_value,
        &dummy_storage_root,
        &[(&storage_key, &storage_value)],
    );

    assert_ne!(
        storage_root,
        empty_trie_root().to_vec(),
        "should not be empty trie root"
    );
}

/// A branch account (two storage entries) should have its storageRoot set to
/// the hash of the two-node storage sub-trie.
#[test]
fn test_persisted_storage_root_two_storage_entries() {
    let account_addr = [0u8; 20];
    let dummy_storage_root = [0u8; 32];
    let account_value = rlp_encode_account(0, 44, &dummy_storage_root, &empty_code_hash());

    let storage_key_a = [1u8; 32];
    let storage_key_b = [2u8; 32];
    let storage_value_a = rlp_encode_storage(&[0xAAu8; 32]);
    let storage_value_b = rlp_encode_storage(&[0xBBu8; 32]);

    let storage_root = commit_and_read_storage_root(
        &account_addr,
        &account_value,
        &dummy_storage_root,
        &[
            (&storage_key_a, &storage_value_a),
            (&storage_key_b, &storage_value_b),
        ],
    );

    assert_ne!(
        storage_root,
        empty_trie_root().to_vec(),
        "should not be empty trie root"
    );
}

#[test]
fn test_root_hash_random_deletions() {
    use rand::seq::SliceRandom;
    let rng = firewood_storage::SeededRng::from_option(Some(42));
    let max_len0 = 8;
    let max_len1 = 4;
    let keygen = || {
        let (len0, len1): (usize, usize) = {
            (
                rng.random_range(1..=max_len0),
                rng.random_range(1..=max_len1),
            )
        };
        (0..len0)
            .map(|_| rng.random_range(0..2))
            .chain((0..len1).map(|_| rng.random()))
            .collect()
    };

    for i in 0..10 {
        let mut items = std::collections::HashMap::<Key, Value>::new();

        for _ in 0..10 {
            let val = (0..8).map(|_| rng.random()).collect();
            items.insert(keygen(), val);
        }

        let mut items_ordered: Vec<_> = items.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        items_ordered.sort_unstable();
        items_ordered.shuffle(&mut &rng);

        let (mut committed_merkle, mut header) = init_merkle_with_header(&items);

        for (k, v) in items_ordered {
            let mut merkle = committed_merkle.fork().unwrap();
            assert_eq!(merkle.get_value(&k).unwrap().as_deref(), Some(v.as_ref()));

            merkle.remove(&k).unwrap();

            // assert_eq(None) and not assert(is_none) for better error messages
            assert_eq!(merkle.get_value(&k).unwrap().as_deref(), None);

            items.remove(&k);

            for (k, v) in &items {
                assert_eq!(merkle.get_value(k).unwrap().as_deref(), Some(v.as_ref()));
            }

            committed_merkle = into_committed(merkle.hash(), &mut header);

            let h: TrieHash = KeccakHasher::trie_root(&items).to_fixed_bytes().into();

            let h0 = committed_merkle
                .nodestore()
                .root_hash()
                .or_default_root_hash()
                .unwrap();

            assert_eq!(h, h0);
        }

        println!("i = {i}");
    }
}
