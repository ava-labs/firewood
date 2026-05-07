// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! # Ethereum-compatible node hashing
//!
//! This module implements [`Preimage`] for firewood nodes so that the resulting
//! root hash matches what an Ethereum Modified Merkle Patricia Trie (MPT) would
//! produce for the same key/value set. It is only compiled when the `ethhash`
//! feature is enabled (used for Avalanche C-Chain state).
//!
//! ## Why this is non-trivial
//!
//! Firewood stores a trie of (up-to-)16-ary branch nodes with partial-path
//! compression. Ethereum's MPT is conceptually the same, but the on-the-wire
//! encoding differs in several ways that must all be reproduced bit-exactly or
//! the root hash will not match:
//!
//! 1. **Hex-prefix / "compact" path encoding** — partial paths are packed with
//!    a one-byte header encoding `(is_leaf, odd_nibble_count)` plus an optional
//!    leading low nibble. See [`nibbles_to_eth_compact`].
//! 2. **Inline children (the "<32 byte" rule)** — a child whose RLP encoding is
//!    less than 32 bytes is embedded *inline* in the parent's RLP instead of
//!    being replaced by its 32-byte hash. In firewood's [`HashType`] this is
//!    the distinction between [`HashType::Hash`] and [`HashType::Rlp`].
//!    [`Preimage::to_hash`] preserves whichever form fits (`< 32` → `Rlp`, else
//!    `Hash`); branch encoders then inline `Rlp` children via
//!    [`RlpStream::append_raw`] and hash-reference `Hash` children.
//! 3. **17-element branch lists** — a branch is always RLP-encoded as
//!    `[child_0, ..., child_15, value]`. Missing children are `0x80` (empty RLP
//!    bytes), not omitted.
//! 4. **Two-level state trie** — Ethereum's world state is a trie of accounts,
//!    where each account's value is RLP `[nonce, balance, storageRoot, codeHash]`
//!    and `storageRoot` is the root hash of a *separate* storage trie for that
//!    account. See the account-node section below.
//!
//! ## Account nodes (depth 64)
//!
//! In firewood an "account node" is any node whose full key is exactly 64
//! nibbles (= 32 bytes, the Keccak-256 of the account address). At that depth
//! the node's value is account RLP, and its *children*, if any, are the
//! storage trie for that account.
//!
//! Two things happen at account nodes that do not happen elsewhere:
//!
//! - **`storageRoot` is recomputed at hash time.** When computing a node's
//!   hash, we always derive the `storageRoot` slot from the node's children
//!   and splice it into the account RLP via [`replace_hash`], regardless of
//!   whatever value is currently in that slot. This is why [`replace_hash`]
//!   exists. The *persisted* bytes are not updated by this splice, so callers
//!   reading the raw value may observe a stale or zero `storageRoot` even
//!   though the trie's root hash is correct; the proof and iteration paths
//!   re-apply the fix on read via
//!   [`fix_account_storage_root_value`](crate::fix_account_storage_root_value).
//! - **The single-child "fake root" trick**. An account with exactly one
//!   storage slot looks like a firewood branch with one child, but the
//!   equivalent Ethereum storage trie is a single leaf at the *root*. To get
//!   the same hash, we conceptually prepend the child's branch nibble to its
//!   partial path and re-hash it as if it were a standalone root node. See
//!   the `children == 1` arm in [`Preimage::write`] and `hash_helper_inner`'s
//!   `fake_root_extra_nibble` in `nodestore::hash`.
//!
//! ## Where the storage root gets spliced
//!
//! There are two sites that compute and splice `storageRoot` into account
//! RLP, and they must stay in lockstep:
//!
//! 1. **At hash time**, during [`Preimage::write`]: we compute the storage-trie
//!    hash and splice it into the RLP used for the account node's own hash.
//!    This makes the account's contribution to the state root correct.
//!
//! 2. **At proof / iteration time**, in
//!    [`fix_account_storage_root_value`](crate::fix_account_storage_root_value)
//!    (`nodestore::hash`): the persisted account bytes may hold a stale or
//!    zero `storageRoot` (callers supply the bytes verbatim, and hashing does
//!    not write them back), so proof generation and key/value enumeration
//!    re-splice the correct `storageRoot` before returning.
//!
//! Both sites must produce bit-identical bytes for the same inputs or
//! proof verification will disagree with the trie's root hash.
//! `fix_account_storage_root_value` runs at proof time where all storage-trie
//! children are guaranteed to be 32-byte hashes (32-byte storage keys always
//! yield encodings ≥ 32 bytes), so it treats [`HashType::Rlp`] as
//! `unreachable!`; `Preimage::write` runs during hashing and must handle
//! inline [`HashType::Rlp`] children.
//!
//! ## Reviewer checklist
//!
//! When reviewing changes in this module or its callers in `nodestore::hash`:
//!
//! - Does the change alter RLP shape? If so, verify both the multi-child branch
//!   path *and* the single-child "fake root" path produce bit-identical output
//!   to geth/reth for a known fixture.
//! - Does it change where the account depth (64 nibbles) check lives? The two
//!   splice sites must agree on the definition.
//! - Does it change how [`HashType::Rlp`] vs [`HashType::Hash`] is chosen? The
//!   32-byte cutoff in [`Preimage::to_hash`] must match what parent encoders
//!   assume when calling [`RlpStream::append_raw`] vs `append`.
//! - Does it touch [`replace_hash`]? Note that Coreth may append fields beyond
//!   the standard 4 — the `[_, _, storage_root, ..]` pattern is deliberate.

#![cfg_attr(
    feature = "ethhash",
    expect(
        clippy::too_many_lines,
        reason = "Found 1 occurrences after enabling the lint."
    )
)]

use crate::logger::warn;
use crate::{
    BranchNode, HashType, Hashable, Preimage, TrieHash, TriePath, ValueDigest,
    hashednode::HasUpdate, logger::trace,
};
use ::rlp::{NULL_RLP, Rlp, RlpStream};
use bitfield::bitfield;
use bytes::BytesMut;
use sha3::{Digest, Keccak256};
use smallvec::SmallVec;
use std::iter::once;

impl HasUpdate for Keccak256 {
    fn update<T: AsRef<[u8]>>(&mut self, data: T) {
        sha3::Digest::update(self, data);
    }
}

/// Hex-prefix encoding of a nibble path (Ethereum Yellow Paper, appendix C).
///
/// Produces the byte sequence used as the first element of leaf and extension
/// nodes when RLP-encoding an MPT node. The first output byte is a header of
/// the form `00ABCCCC`:
///
/// - `A` is 1 iff `is_leaf` — distinguishes leaf (`2x`/`3x`) from extension
///   (`0x`/`1x`) nodes.
/// - `B` is 1 iff the input has an odd number of nibbles, in which case
///   `CCCC` is the first (orphan) nibble.
/// - If `B` is 0, `CCCC` is zero and the remaining nibbles pack evenly into
///   bytes.
///
/// Remaining nibble pairs are packed high-nibble-first into subsequent bytes.
/// This must match geth's `hexToCompact` exactly; any deviation breaks root
/// hash compatibility.
pub(crate) fn nibbles_to_eth_compact<T: TriePath>(nibbles: T, is_leaf: bool) -> SmallVec<[u8; 32]> {
    // This is a bitfield that represents the first byte of the output, documented above
    bitfield! {
        struct CompactFirstByte(u8);
        impl Debug;
        impl new;
        u8;
        is_leaf, set_is_leaf: 5;
        odd_nibbles, set_odd_nibbles: 4;
        low_nibble, set_low_nibble: 3, 0;
    }

    let nibbles = nibbles.as_component_slice();

    let mut first_byte = CompactFirstByte(0);
    first_byte.set_is_leaf(is_leaf);

    let (maybe_low_nibble, nibble_pairs) = nibbles.as_rchunks::<2>();
    if let &[low_nibble] = maybe_low_nibble {
        // we have an odd number of nibbles
        first_byte.set_odd_nibbles(true);
        first_byte.set_low_nibble(low_nibble.as_u8());
    } else {
        // as_rchunks can only return 0 or 1 element in the first slice if N is 2
        debug_assert!(maybe_low_nibble.is_empty());
    }

    // now assemble everything: the first byte, and the nibble pairs compacted back together
    once(first_byte.0)
        .chain(nibble_pairs.iter().map(|&[hi, lo]| hi.join(lo)))
        .collect()
}

impl<T: Hashable> Preimage for T {
    fn to_hash(&self) -> HashType {
        // first collect the thing that would be hashed, and if it's smaller than a hash,
        // just use it directly
        let mut collector = SmallVec::with_capacity(32);
        self.write(&mut collector);

        trace!(
            "SIZE WAS {} {}",
            self.full_path().len(),
            hex::encode(&collector),
        );

        if collector.len() >= 32 {
            HashType::Hash(Keccak256::digest(collector).into())
        } else {
            HashType::Rlp(collector)
        }
    }

    fn write(&self, buf: &mut impl HasUpdate) {
        let is_account = self.full_path().len() == 64;
        trace!("is_account: {is_account}");

        let child_hashes = self.children();

        let children = child_hashes.count();

        if children == 0 {
            // since there are no children, this must be a leaf
            // we append two items, the partial_path, encoded, and the value
            // note that leaves must always have a value, so we know there
            // will be 2 items

            let mut rlp = RlpStream::new_list(2);

            rlp.append(&&*nibbles_to_eth_compact(self.partial_path(), true));

            if is_account {
                // we are a leaf that is at depth 32
                match self.value_digest() {
                    Some(ValueDigest::Value(bytes)) => {
                        let new_hash = Keccak256::digest(NULL_RLP).as_slice().to_vec();
                        let bytes_mut = BytesMut::from(bytes);
                        if let Some(result) = replace_hash(bytes_mut, &new_hash) {
                            rlp.append(&&*result);
                        } else {
                            rlp.append(&bytes);
                        }
                    }
                    None => {
                        rlp.append_empty_data();
                    }
                }
            } else {
                match self.value_digest() {
                    Some(ValueDigest::Value(bytes)) => rlp.append(&bytes),
                    None => rlp.append_empty_data(),
                };
            }

            let bytes = rlp.out();
            trace!("partial path {:?}", self.partial_path().display());
            trace!("serialized leaf-rlp: {:?}", hex::encode(&bytes));
            buf.update(&bytes);
        } else {
            // for a branch, there are always 16 children and a value
            // Child::None we encode as RLP empty_data (0x80)
            let mut rlp = RlpStream::new_list(const { BranchNode::MAX_CHILDREN + 1 });
            for (_, child) in &child_hashes {
                match child {
                    Some(HashType::Hash(hash)) => rlp.append(&hash.as_slice()),
                    Some(HashType::Rlp(rlp_bytes)) => rlp.append_raw(rlp_bytes, 1),
                    None => rlp.append_empty_data(),
                };
            }

            // For branch nodes, we need to append the value as the 17th element in the RLP list.
            // However, account nodes (depth 32) handle values differently - they don't store
            // the value directly in the branch node, but rather in the account structure itself.
            // This is because account nodes have special handling where the storage root hash
            // gets replaced in the account data structure during serialization.
            let digest = (!is_account).then(|| self.value_digest()).flatten();
            if let Some(ValueDigest::Value(digest)) = digest {
                rlp.append(&digest);
            } else {
                rlp.append_empty_data();
            }
            let bytes = rlp.out();
            trace!("pass 1 bytes {:02X?}", hex::encode(&bytes));

            // we've collected all the children in bytes

            let updated_bytes = if is_account {
                // need to get the value again
                if let Some(ValueDigest::Value(rlp_encoded_bytes)) = self.value_digest() {
                    // rlp_encoded__bytes needs to be decoded
                    // TODO: Handle corruption
                    // needs to be the hash of the RLP encoding of the root node that
                    // would have existed here (instead of this account node)
                    // the "root node" is actually this branch node iff there is
                    // more than one child. If there is only one child, then the
                    // child is actually the root node, so we need the hash of that
                    // child here.
                    let replacement_hash = if children == 1 {
                        // we need to treat this child like it's a root node, so the partial path is
                        // actually one longer than it is reported
                        match child_hashes
                            .iter()
                            .find_map(|(_, c)| c.as_ref())
                            .expect("we know there is exactly one child")
                        {
                            HashType::Hash(hash) => hash.clone(),
                            HashType::Rlp(rlp_bytes) => {
                                let mut rlp = RlpStream::new_list(2);
                                rlp.append(&&*nibbles_to_eth_compact(self.partial_path(), true));
                                rlp.append_raw(rlp_bytes, 1);
                                let bytes = rlp.out();
                                TrieHash::from(Keccak256::digest(bytes))
                            }
                        }
                    } else {
                        TrieHash::from(Keccak256::digest(bytes))
                    };
                    trace!("replacement hash {:?}", hex::encode(&replacement_hash));

                    let bytes = replace_hash(rlp_encoded_bytes, &replacement_hash)
                        .unwrap_or_else(|| BytesMut::from(rlp_encoded_bytes));
                    trace!("updated encoded value {:02X?}", hex::encode(&bytes));
                    bytes
                } else {
                    // treat like non-account since it didn't have a value
                    warn!(
                        "Account node {:x?} without value",
                        self.full_path().display(),
                    );
                    bytes.as_ref().into()
                }
            } else {
                bytes.as_ref().into()
            };

            let partial_path = self.partial_path();
            if partial_path.is_empty() {
                trace!("pass 2=bytes {:02X?}", hex::encode(&updated_bytes));
                buf.update(updated_bytes);
            } else {
                let mut final_bytes = RlpStream::new_list(2);
                final_bytes.append(&&*nibbles_to_eth_compact(partial_path, is_account));
                // if the RLP is short enough, we can use it as-is, otherwise we hash it
                // to make the maximum length 32 bytes
                if updated_bytes.len() > 31 && !is_account {
                    let hashed_bytes = Keccak256::digest(updated_bytes);
                    final_bytes.append(&hashed_bytes.as_slice());
                } else {
                    final_bytes.append(&updated_bytes);
                }
                let final_bytes = final_bytes.out();
                trace!("pass 2 bytes {:02X?}", hex::encode(&final_bytes));
                buf.update(final_bytes);
            }
        }
    }
}

/// Splice a new `storageRoot` into an RLP-encoded account value.
///
/// Decodes `bytes` as an RLP list, replaces element index 2 with `new_hash`,
/// and re-encodes. Returns `None` if `bytes` is not a well-formed RLP list of
/// at least 3 elements — in that case callers treat the value as opaque
/// (non-account) data and leave it untouched.
///
/// The standard Ethereum account is `[nonce, balance, storageRoot, codeHash]`
/// (4 elements). Coreth may append additional trailing fields, so the match
/// pattern deliberately uses `[_, _, storage_root, ..]` rather than asserting
/// a fixed length.
pub(crate) fn replace_hash<T: AsRef<[u8]>, U: AsRef<[u8]>>(
    bytes: T,
    new_hash: U,
) -> Option<BytesMut> {
    let rlp = Rlp::new(bytes.as_ref());
    let mut list: Vec<Vec<u8>> = rlp.as_list().ok()?;
    let [_, _, storage_root, ..] = list.as_mut_slice() else {
        return None;
    };
    *storage_root = Vec::from(new_hash.as_ref());

    trace!("inbound bytes: {}", hex::encode(bytes.as_ref()));
    trace!("list length was {}", list.len());
    trace!("replacement hash {:?}", hex::encode(&new_hash));

    let mut rlp = RlpStream::new_list(list.len());
    for item in list {
        rlp.append(&item);
    }
    let bytes = rlp.out();
    trace!("updated encoded value {:02X?}", hex::encode(&bytes));
    Some(bytes)
}

#[cfg(test)]
mod test {
    use test_case::test_case;

    use crate::{PathComponent, TriePathFromUnpackedBytes};

    #[test_case(&[], false, &[0x00])]
    #[test_case(&[], true, &[0x20])]
    #[test_case(&[1, 2, 3, 4, 5], false, &[0x11, 0x23, 0x45])]
    #[test_case(&[0, 1, 2, 3, 4, 5], false, &[0x00, 0x01, 0x23, 0x45])]
    #[test_case(&[15, 1, 12, 11, 8], true, &[0x3f, 0x1c, 0xb8])]
    #[test_case(&[0, 15, 1, 12, 11, 8], true, &[0x20, 0x0f, 0x1c, 0xb8])]
    fn test_hex_to_compact(hex: &[u8], has_value: bool, expected_compact: &[u8]) {
        let path = <&[PathComponent]>::path_from_unpacked_bytes(hex).expect("valid path");
        assert_eq!(
            &*super::nibbles_to_eth_compact(path, has_value),
            expected_compact
        );
    }
}
