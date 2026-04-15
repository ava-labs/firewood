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

/// RLP-encode a 32-byte storage slot value.
fn rlp_encode_storage(value: &[u8; 32]) -> Vec<u8> {
    use rlp::RlpStream;

    let mut rlp = RlpStream::new();
    rlp.append(&value.as_slice());
    rlp.out().to_vec()
}

/// Verify that a range proof bounded to account keys contains the corrected
/// storageRoot fields. The left account has no storage children so its
/// storageRoot must be keccak256(0x80) (empty trie root). The right account
/// has one storage child so its storageRoot must be the computed hash of that
/// storage sub-trie — neither the dummy zeros nor the empty trie root.
///
/// The range proof spans exactly the two account keys and excludes the
/// storage entry that lives under the right account.
#[test]
fn test_range_proof_accounts_have_computed_storage_root() {
    type BoxedAccounts = Box<[(Box<[u8]>, Box<[u8]>)]>;

    let dummy_storage_root = [0u8; 32];
    let empty_root = empty_trie_root();

    // Two accounts sorted by their keccak256 trie keys (left < right).
    let mut accounts: BoxedAccounts = [[0x01u8; 20], [0x02u8; 20]]
        .into_iter()
        .enumerate()
        .map(|(i, addr)| {
            let key = Box::from(Keccak256::digest(addr).as_slice());
            let value = rlp_encode_account(
                i as u64,
                (i as u64 + 1) * 100,
                &dummy_storage_root,
                &empty_code_hash(),
            );
            (key, value)
        })
        .collect();
    accounts.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));
    let left_key = &accounts[0].0;
    let right_key = &accounts[1].0;

    // One storage entry under the right account. Its 64-byte key is
    // right_account_key || storage_suffix, placing it beyond the right
    // account in trie order.
    let storage_key: Box<[u8]> = [right_key.as_ref(), &[0xAAu8; 32]].concat().into();
    let storage_value: Box<[u8]> = rlp_encode_storage(&[0x42u8; 32]).into();

    let items: BoxedAccounts = accounts
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .chain(once((storage_key, storage_value)))
        .collect();
    let merkle = init_merkle(items);
    let root_hash = merkle.nodestore().root_hash().unwrap();

    // Build a range proof bounded to just the two account keys.
    let range_proof = merkle
        .range_proof(Some(left_key.as_ref()), Some(right_key.as_ref()), None)
        .unwrap();

    // The range proof must verify against the committed root hash.
    verify_range_proof(
        Some(left_key.as_ref()),
        Some(right_key.as_ref()),
        &root_hash,
        &range_proof,
    )
    .unwrap();

    // Exactly two key-value pairs in the proof.
    assert_eq!(range_proof.iter().len(), 2);

    // Decode and check each account's storageRoot.
    for (key, value) in &range_proof {
        let rlp = rlp::Rlp::new(value.as_ref());
        let list: Vec<Vec<u8>> = rlp
            .as_list()
            .expect("account value should be valid RLP list");
        assert!(
            list.len() >= 4,
            "account RLP should have at least 4 fields, got {} for key {:?}",
            list.len(),
            key.as_ref(),
        );

        let storage_root = &list[2];
        assert_ne!(
            storage_root.as_slice(),
            &dummy_storage_root,
            "storageRoot must not be the original dummy zeros",
        );

        if key.as_ref() == left_key.as_ref() {
            // Left account has no storage children → empty trie root.
            assert_eq!(
                storage_root.as_slice(),
                &empty_root,
                "left account storageRoot should be the empty trie root",
            );
        } else {
            // Right account has a storage child → real computed hash.
            assert_ne!(
                storage_root.as_slice(),
                &empty_root,
                "right account storageRoot should NOT be the empty trie root",
            );
        }
    }
}
