// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Builders for Ethereum-shaped test data: RLP account and storage values,
//! storage-slot keys under an account, and the in-place edits that turn a
//! committed [`MemStore`] into a pre-`firewood-v1-hfix` database.
//!
//! Not gated on the `ethhash` feature: hashing is selected per database at
//! run time, so tests over the concrete `EthHash` mode compile in every
//! build.
//!
//! [`MemStore`]: firewood_storage::MemStore

use firewood_storage::{
    Committed, DeletedNodeTracking, HashMode, MemStore, NodeStore, NodeStoreHeader,
};
use sha3::{Digest, Keccak256};

use crate::merkle::Merkle;

/// Keccak256 of empty bytes — the codeHash for accounts with no contract code.
pub(super) fn empty_code_hash() -> [u8; 32] {
    Keccak256::digest([]).into()
}

/// RLP-encode an Ethereum account value: [nonce, balance, storageRoot, codeHash].
pub(super) fn rlp_encode_account(
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
pub(super) fn rlp_encode_storage(value: &[u8; 32]) -> Vec<u8> {
    use rlp::RlpStream;

    let mut rlp = RlpStream::new();
    rlp.append(&value.as_slice());
    rlp.out().to_vec()
}

/// Build a storage-slot key: `account_key` plus a 32-byte suffix of
/// `first_suffix_byte` followed by zeros. At depth 64 the account branch fans
/// out on the next nibble, so the high nibble of `first_suffix_byte` selects
/// which child slot this entry occupies.
pub(super) fn account_storage_key(account_key: &[u8], first_suffix_byte: u8) -> Box<[u8]> {
    let mut suffix = [0u8; 32];
    suffix[0] = first_suffix_byte;
    [account_key, &suffix].concat().into()
}

/// Find `value_bytes` in the raw [`MemStore`] and overwrite it with `replacement`.
/// The two slices must be the same length. Returns the number of occurrences
/// replaced.
///
/// Simulates a legacy database that stored zeroed hashes inside the
/// RLP-encoded account values. The search is for the full serialized value,
/// not just the 32-byte hash, so unrelated node hashes or structural data in
/// the [`MemStore`] are not corrupted.
pub(super) fn clobber_value_in_memstore(
    storage: &firewood_storage::MemStore,
    value_bytes: &[u8],
    replacement: &[u8],
) -> usize {
    use firewood_storage::{ReadableStorage, WritableStorage};
    use std::io::Read;
    assert_eq!(value_bytes.len(), replacement.len());

    let mut buf = Vec::new();
    storage
        .stream_from(0)
        .unwrap()
        .read_to_end(&mut buf)
        .unwrap();

    let len = value_bytes.len();
    let mut count = 0_usize;
    for offset in 0..buf.len().saturating_sub(len.saturating_sub(1)) {
        // `offset + len <= buf.len()` by the loop bound.
        if buf.get(offset..offset.wrapping_add(len)) == Some(value_bytes) {
            storage.write(offset as u64, replacement).unwrap();
            count = count.saturating_add(1);
        }
    }
    count
}

/// Given an RLP-encoded account value, return a copy with field 2 (storageRoot)
/// replaced by the given 32-byte value.
pub(super) fn zero_storage_root_in_rlp(value: &[u8], replacement: &[u8; 32]) -> Vec<u8> {
    let list: Vec<Vec<u8>> = rlp::Rlp::new(value).as_list().unwrap();
    assert!(list.len() >= 3);

    let mut rlp = rlp::RlpStream::new_list(list.len());
    for (i, item) in list.iter().enumerate() {
        if i == 2 {
            rlp.append(&replacement.as_slice());
        } else {
            rlp.append(item);
        }
    }
    rlp.out().to_vec()
}

/// Turn a committed in-memory store into a pre-`firewood-v1-hfix` database
/// and reopen it: every account in `account_keys` has the `storageRoot` field
/// of its stored value overwritten with zeros, exactly as databases written
/// before the fix hold it, and the header's version string is rewritten so
/// `must_recompute_storage_hash()` reports `true` on the reopened store.
/// Node hashes are untouched, so the root hash is unchanged.
///
/// `storage` must be the store `merkle` was committed to.
pub(super) fn reopen_as_legacy<H: HashMode>(
    merkle: &Merkle<NodeStore<Committed, MemStore, H>>,
    account_keys: &[&[u8]],
) -> NodeStore<Committed, MemStore, H> {
    use firewood_storage::WritableStorage;

    let storage = merkle.nodestore().storage().clone();
    for key in account_keys {
        let stored = merkle.get_value(key).unwrap().unwrap();
        let zeroed = zero_storage_root_in_rlp(&stored, &[0; 32]);
        let replaced = clobber_value_in_memstore(&storage, &stored, &zeroed);
        assert_eq!(
            replaced, 1,
            "expected exactly one occurrence of the account value for {key:02x?}"
        );
    }
    storage.write(0, b"firewood-v1\0\0\0\0\0").unwrap();
    let header = NodeStoreHeader::read_from_storage(&*storage, H::ALGORITHM).unwrap();
    NodeStore::open(&header, storage, DeletedNodeTracking::Enabled).unwrap()
}
