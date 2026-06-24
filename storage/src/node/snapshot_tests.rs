// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Snapshot tests for `firewood-storage` node serialization.
//!
//! These tests lock down the binary wire format for three `Serializable`
//! implementations and the `Node::as_bytes` / `Node::from_reader` functions.
//!
//! Snapshot files live in `storage/src/node/snapshots/`. Run
//! `just snapshot-nodes` to write them on the first run or to regenerate
//! them after an intentional format change.
//!
//! # Test coverage
//!
//! ## `Serializable` implementations (feature-agnostic)
//!
//! | Module | Test | Subject |
//! |--------|------|---------|
//! | [`trie_hash`] | [`trie_hash::zero`] | All-zero 32-byte hash |
//! | [`trie_hash`] | [`trie_hash::sequential`] | Sequential-byte hash `[0x00..0x1f]` |
//! | [`free_area`] | [`free_area::no_next`] | `FreeArea::new(None)` → `[0xFF, 0x00]` |
//! | [`free_area`] | [`free_area::with_addr`] | Single-byte varint address |
//! | [`free_area`] | [`free_area::with_large_addr`] | Multi-byte varint address |
//!
//! ## `Node` serialization (feature-gated: child hash encoding differs)
//!
//! Leaf nodes produce identical bytes under both hash modes (no child hashes),
//! so leaf tests are at the top level and share a single snapshot file.
//! Branch nodes are in cfg-gated `merkledb` / `ethhash` submodules.
//!
//! | Test | Subject |
//! |------|---------|
//! | [`leaf_with_value`] | Leaf: path `[0,1,2,3]`, value `[4,5,6,7]` |
//! | [`leaf_partial_path`] | Leaf with path length that triggers varint overflow |
//! | `merkledb::branch_with_one_child` | Branch: one child, SHA-256 child hash |
//! | `merkledb::branch_with_value` | Branch: value + one child, SHA-256 child hash |
//! | `ethhash::branch_with_one_child` | Branch: one child, Keccak-256 discriminant |
//! | `ethhash::branch_with_value` | Branch: value + one child, Keccak-256 discriminant |
//!
//! ## `HashOrRlp` serialization (ethhash feature only)
//!
//! | Module | Test | Subject |
//! |--------|------|---------|
//! | `ethhash::hash_or_rlp` | `keccak_hash` | Full 32-byte hash: `[0x00] + 32 bytes` |
//! | `ethhash::hash_or_rlp` | `rlp_bytes` | Short RLP: length byte + payload bytes |

#![expect(clippy::unwrap_used)]

use crate::IntoHashType;
use crate::node::branch::Serializable;
use crate::node::{BranchNode, LeafNode, Node};
use crate::nodestore::alloc::FreeArea;
use crate::{Child, Children, LinearAddress, NibblesIterator, Path, PathComponent, TrieHash};

/// Serializes a [`Serializable`] value into a fresh `Vec<u8>`.
fn write_to_vec<T: Serializable>(t: &T) -> Vec<u8> {
    let mut buf = Vec::new();
    t.write_to(&mut buf);
    buf
}

/// Calls [`Node::as_bytes`] and returns the full encoded bytes (including the
/// leading `AreaIndex` byte at position 0).
fn node_as_bytes(node: &Node) -> Vec<u8> {
    let mut buf = Vec::new();
    node.as_bytes(&mut buf).unwrap();
    buf
}

/// Builds a zero-hash child entry at the given nibble index using
/// [`TrieHash::from`] + [`IntoHashType`], which works under both hash modes.
fn child_at(nibble: u8) -> impl Fn(PathComponent) -> Option<Child> {
    let hash: crate::HashType = TrieHash::from([0u8; 32]).into_hash_type();
    move |i| {
        if i.as_u8() == nibble {
            Some(Child::AddressWithHash(
                LinearAddress::new(1).unwrap(),
                hash.clone(),
            ))
        } else {
            None
        }
    }
}

/// [`TrieHash`] serialization — always 32 flat bytes, no framing.
mod trie_hash {
    use super::*;

    /// All-zero hash: 32 × `0x00`.
    #[test]
    fn zero() {
        let h = TrieHash::from([0u8; 32]);
        insta::assert_snapshot!(hex::encode(write_to_vec(&h)));
    }

    /// Sequential-byte hash: `[0x00, 0x01, ..., 0x1f]`.
    #[test]
    fn sequential() {
        let h = TrieHash::from(std::array::from_fn::<u8, 32, _>(|i| i as u8));
        insta::assert_snapshot!(hex::encode(write_to_vec(&h)));
    }
}

/// [`FreeArea`] serialization — `0xFF` marker + varint-encoded optional address.
mod free_area {
    use super::*;

    /// No next free block: `[0xFF, 0x00]` (zero-varint encodes `None`).
    #[test]
    fn no_next() {
        let fa = FreeArea::new(None);
        insta::assert_snapshot!(hex::encode(write_to_vec(&fa)));
    }

    /// Next free block at address 42: `[0xFF]` + single-byte LEB128 (42 fits
    /// in 7 bits).
    #[test]
    fn with_addr() {
        let fa = FreeArea::new(Some(LinearAddress::new(42).unwrap()));
        insta::assert_snapshot!(hex::encode(write_to_vec(&fa)));
    }

    /// Next free block at address 1024: `[0xFF]` + two-byte LEB128 (1024
    /// requires more than 7 bits: `0x80 0x08`).
    #[test]
    fn with_large_addr() {
        let fa = FreeArea::new(Some(LinearAddress::new(1024).unwrap()));
        insta::assert_snapshot!(hex::encode(write_to_vec(&fa)));
    }
}

/// Leaf node with a short path and value. Leaf encoding is identical under
/// both hash modes (no child hashes), so this test is not cfg-gated.
///
/// Encoded first byte: `is_leaf=1 | path_len_low_bits`. Followed by path
/// nibbles, then varint value length, then value bytes.
#[test]
fn leaf_with_value() {
    let node = Node::Leaf(LeafNode {
        partial_path: Path::from(vec![0, 1, 2, 3]),
        value: vec![4, 5, 6, 7].into(),
    });
    insta::assert_snapshot!(hex::encode(node_as_bytes(&node)));
}

/// Leaf node with a path long enough to trigger the varint overflow for the
/// path length field in the first byte (nibble count > the inline capacity).
/// Also feature-agnostic.
#[test]
fn leaf_partial_path() {
    let node = Node::Leaf(LeafNode {
        partial_path: Path::from_nibbles_iterator(NibblesIterator::new(
            b"this is a really long partial path, like so long it's more than 63 nibbles long which triggers #1056.",
        )),
        value: vec![4, 5, 6, 7].into(),
    });
    insta::assert_snapshot!(hex::encode(node_as_bytes(&node)));
}

/// Branch node tests for MerkleDB (SHA-256) mode. Child hashes are flat
/// 32-byte `TrieHash` values with no prefix byte.
#[cfg(not(feature = "ethhash"))]
mod merkledb {
    use super::*;

    /// Branch with a single child at nibble 15 (`0xF`), no value.
    /// Encoded as a sparse child list: one entry with address + 32-byte hash.
    #[test]
    fn branch_with_one_child() {
        let node = Node::Branch(Box::new(BranchNode {
            partial_path: Path::from(vec![0, 1]),
            value: None,
            children: Children::from_fn(child_at(15)),
        }));
        insta::assert_snapshot!(hex::encode(node_as_bytes(&node)));
    }

    /// Branch with a value and a single child at nibble 0, longer partial
    /// path. Exercises the has-value bit and the value-length varint.
    #[test]
    fn branch_with_value() {
        let node = Node::Branch(Box::new(BranchNode {
            partial_path: Path::from(vec![0, 1, 2, 3]),
            value: Some(vec![4, 5, 6, 7].into()),
            children: Children::from_fn(child_at(0)),
        }));
        insta::assert_snapshot!(hex::encode(node_as_bytes(&node)));
    }
}

/// Branch node tests for Ethereum (Keccak-256) mode. Each child hash is
/// prefixed with a 1-byte discriminant: `0x00` for a full 32-byte Keccak
/// hash, non-zero for an inline RLP value (length byte + payload).
#[cfg(feature = "ethhash")]
mod ethhash {
    use super::*;

    /// Branch with a single child at nibble 15, no value.
    /// Each child encodes as discriminant `0x00` + 32-byte zero hash (33 bytes).
    #[test]
    fn branch_with_one_child() {
        let node = Node::Branch(Box::new(BranchNode {
            partial_path: Path::from(vec![0, 1]),
            value: None,
            children: Children::from_fn(child_at(15)),
        }));
        insta::assert_snapshot!(hex::encode(node_as_bytes(&node)));
    }

    /// Branch with a value and a single child at nibble 0.
    #[test]
    fn branch_with_value() {
        let node = Node::Branch(Box::new(BranchNode {
            partial_path: Path::from(vec![0, 1, 2, 3]),
            value: Some(vec![4, 5, 6, 7].into()),
            children: Children::from_fn(child_at(0)),
        }));
        insta::assert_snapshot!(hex::encode(node_as_bytes(&node)));
    }

    /// [`crate::HashType`] (`HashOrRlp`) serialization — available only when
    /// the `ethhash` feature is enabled.
    mod hash_or_rlp {
        use super::*;
        use crate::HashType;
        use smallvec::SmallVec;

        /// Full Keccak-256 hash: discriminant `0x00` followed by 32 zero bytes.
        #[test]
        fn keccak_hash() {
            let h = HashType::Hash(TrieHash::from([0u8; 32]));
            insta::assert_snapshot!(hex::encode(write_to_vec(&h)));
        }

        /// Inline RLP (single-byte payload `0xC0` = empty RLP list): length
        /// byte `0x01` followed by `0xC0`.
        #[test]
        fn rlp_bytes() {
            let h = HashType::Rlp(SmallVec::from_slice(&[0xC0u8]));
            insta::assert_snapshot!(hex::encode(write_to_vec(&h)));
        }
    }
}
