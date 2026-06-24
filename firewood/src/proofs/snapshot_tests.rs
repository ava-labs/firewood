// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Snapshot tests for [`ProofNode`] wire serialization.
//!
//! Each test constructs a canonical [`ProofNode`] variant and snapshots the
//! hex-encoded bytes that follow the fixed 32-byte proof header when it is
//! serialized inside a minimal [`FrozenRangeProof`]. The header itself is
//! excluded because it encodes only proof-level metadata (magic, version,
//! hash mode, proof type); the snapshots target only the node encoding.
//!
//! Snapshot files live in `src/proofs/snapshots/`. Run `just snapshot-proof-nodes`
//! to write them on the first run or to regenerate them after an intentional
//! wire-format change.
//!
//! # Test coverage
//!
//! | Test | Key nibbles | `partial_len` | Value | Children |
//! |------|-------------|---------------|-------|----------|
//! | [`empty`] | `[]` | 0 | none | none |
//! | [`leaf_with_value`] | `[1, 2, 3]` | 0 | `b"hello"` | none |
//! | [`partial_path`] | `[1, 2, 3, 4, 5]` | 3 | none | none |
//! | [`merkledb::single_child`] | `[1]` | 0 | none | nibble 7 |
//! | [`merkledb::all_children`] | `[0]` | 0 | none | all 16 |
//! | [`merkledb::large_value_becomes_hash`] | `[1, 2]` | 0 | 32 × `0xAB` | none |
//! | [`ethhash::single_child`] | `[1]` | 0 | none | nibble 7 |
//! | [`ethhash::all_children`] | `[0]` | 0 | none | all 16 |
//!
//! The `merkledb` sub-module is compiled only without the `ethhash` feature;
//! `ethhash` is compiled only with it. Tests without children (`empty`,
//! `leaf_with_value`, `partial_path`) produce identical bytes under both
//! hash modes and share a single snapshot file.
//!
//! The `ethhash` module has no `large_value_becomes_hash` variant: child
//! hashes in `ethhash` mode already carry a 1-byte discriminant prefix, so
//! `single_child` and `all_children` already cover the complete child
//! encoding format. The large-value → hash behaviour is format-agnostic and
//! is fully covered by the `merkledb` variant.
//!
//! ## Additional coverage
//!
//! The following tests lock down wire-format sections beyond individual node encoding.
//! The payload snapshots (everything after the 32-byte header) are feature-independent
//! for key-values and batch ops; the header snapshots are cfg-gated because the
//! `hash_mode` byte differs between SHA-256 and Keccak-256 modes.
//!
//! | Test | Subject |
//! |------|---------|
//! | `header::merkledb::range` | 32-byte header, SHA-256 mode, range proof type |
//! | `header::merkledb::change` | 32-byte header, SHA-256 mode, change proof type |
//! | `header::ethhash::range` | 32-byte header, Keccak-256 mode, range proof type |
//! | `header::ethhash::change` | 32-byte header, Keccak-256 mode, change proof type |
//! | `key_values::empty` | Empty KV sequence: `varint(0)` |
//! | `key_values::single_pair` | One `(key, value)` pair |
//! | `key_values::multiple_pairs` | Two `(key, value)` pairs |
//! | `batch_ops::put` | Single `Put` operation (opcode `0x00`) |
//! | `batch_ops::delete` | Single `Delete` operation (opcode `0x01`) |
//! | `batch_ops::delete_range` | Single `DeleteRange` operation (opcode `0x02`) |
//! | `batch_ops::all_ops` | All three operations in sequence |

#![expect(clippy::unwrap_used, clippy::indexing_slicing)]

use firewood_storage::{Children, IntoHashType, PathComponent, TrieHash, ValueDigest};

use super::types::{Proof, ProofNode};
use crate::api::{FrozenChangeProof, FrozenRangeProof};
use crate::db::BatchOp;
use crate::merkle::{Key, Value};

/// Serializes `node` inside a minimal [`FrozenRangeProof`] and returns the bytes
/// after the fixed 32-byte proof header.
///
/// The returned slice begins with the varint-encoded start-proof node count,
/// followed by the [`ProofNode`] encoding, then the empty end-proof and
/// key-values counts.
fn node_bytes(node: &ProofNode) -> Vec<u8> {
    let proof = FrozenRangeProof::new(
        Proof::new(Box::new([node.clone()])),
        Proof::new(Box::<[ProofNode]>::from([])),
        Box::new([]),
    );
    let mut out = Vec::new();
    proof.write_to_vec(&mut out);
    // Skip the fixed 32-byte proof header (magic, version, hash mode, proof type,
    // reserved); only the node encoding is under scrutiny in these tests.
    out[32..].to_vec()
}

/// Builds a [`ProofNode`] from nibble slices. All child hashes use the zero hash
/// (`[0u8; 32]`) so the encoded bytes are fully deterministic.
fn make_node(
    key_nibbles: &[u8],
    partial_len: usize,
    value: Option<Box<[u8]>>,
    child_nibbles: &[u8],
) -> ProofNode {
    let key = key_nibbles
        .iter()
        .map(|&n| PathComponent::try_new(n).unwrap())
        .collect();
    let mut child_hashes = Children::new();
    for &nibble in child_nibbles {
        child_hashes[PathComponent::try_new(nibble).unwrap()] =
            Some(TrieHash::from([0u8; 32]).into_hash_type());
    }
    ProofNode {
        key,
        partial_len,
        value_digest: value.map(ValueDigest::Value),
        child_hashes,
    }
}

/// Empty leaf: zero-length key, no value, no children.
///
/// Encoded as: `[count=1] [key_len=0] [partial_len=0] [option=0] [mask=0x0000]
/// [end_proof_count=0] [kv_count=0]` — 8 bytes after the header.
#[test]
fn empty() {
    let node = make_node(&[], 0, None, &[]);
    insta::assert_snapshot!(hex::encode(node_bytes(&node)));
}

/// Leaf with a short value (`b"hello"`, 5 bytes). Values shorter than 32 bytes
/// are encoded as `ValueDigest::Value` (discriminant `0x00`) under both hash modes.
#[test]
fn leaf_with_value() {
    let node = make_node(&[1, 2, 3], 0, Some(Box::from(b"hello".as_slice())), &[]);
    insta::assert_snapshot!(hex::encode(node_bytes(&node)));
}

/// Node with `partial_len = 3` on a 5-nibble key. The first 3 nibbles belong to
/// the parent; the remaining 2 are this node's own partial path.
#[test]
fn partial_path() {
    let node = make_node(&[1, 2, 3, 4, 5], 3, None, &[]);
    insta::assert_snapshot!(hex::encode(node_bytes(&node)));
}

/// Tests for MerkleDB hashing (SHA-256, non-`ethhash`). Child hashes are encoded
/// as flat 32-byte `TrieHash` values with no prefix byte.
#[cfg(not(feature = "ethhash"))]
mod merkledb {
    use super::*;

    /// Branch with a single child at nibble 7. `ChildMask` has bit 7 set
    /// (`0x0080`); one 32-byte child hash follows.
    #[test]
    fn single_child() {
        let node = make_node(&[1], 0, None, &[7]);
        insta::assert_snapshot!(hex::encode(node_bytes(&node)));
    }

    /// Branch with all 16 children present. `ChildMask = 0xFFFF`; 16 × 32
    /// bytes of child hashes follow.
    #[test]
    fn all_children() {
        let all: Vec<u8> = (0u8..16).collect();
        let node = make_node(&[0], 0, None, &all);
        insta::assert_snapshot!(hex::encode(node_bytes(&node)));
    }

    /// A 32-byte value is hashed by `make_hash()` during serialization and
    /// encoded as `ValueDigest::Hash` (option `0x01`, digest `0x01`, then 32
    /// SHA-256 bytes).
    #[test]
    fn large_value_becomes_hash() {
        let value: Box<[u8]> = vec![0xABu8; 32].into_boxed_slice();
        let node = make_node(&[1, 2], 0, Some(value), &[]);
        insta::assert_snapshot!(hex::encode(node_bytes(&node)));
    }
}

/// Tests for Ethereum hashing (Keccak-256, `ethhash` feature). Each child hash
/// carries a 1-byte discriminant: `0x00` for `HashType::Hash` (32-byte Keccak
/// hash), `0x01` for `HashType::Rlp` (inline RLP, < 32 bytes).
#[cfg(feature = "ethhash")]
mod ethhash {
    use super::*;

    /// Branch with a single child at nibble 7. The child is encoded as
    /// discriminant `0x00` + 32-byte zero hash → 33 bytes per child.
    #[test]
    fn single_child() {
        let node = make_node(&[1], 0, None, &[7]);
        insta::assert_snapshot!(hex::encode(node_bytes(&node)));
    }

    /// Branch with all 16 children. Each child takes 33 bytes (discriminant +
    /// hash); total child data is 16 × 33 = 528 bytes after the 2-byte
    /// `ChildMask`.
    #[test]
    fn all_children() {
        let all: Vec<u8> = (0u8..16).collect();
        let node = make_node(&[0], 0, None, &all);
        insta::assert_snapshot!(hex::encode(node_bytes(&node)));
    }
}

/// Serializes a [`FrozenRangeProof`] with empty node lists and the given key-value
/// pairs. Returns all bytes including the 32-byte proof header.
fn range_proof_bytes(kvs: &[(&[u8], &[u8])]) -> Vec<u8> {
    let kv_pairs: Box<[(Key, Value)]> = kvs
        .iter()
        .map(|(k, v)| (Box::from(*k), Box::from(*v)))
        .collect();
    let proof = FrozenRangeProof::new(
        Proof::new(Box::<[ProofNode]>::from([])),
        Proof::new(Box::<[ProofNode]>::from([])),
        kv_pairs,
    );
    let mut out = Vec::new();
    proof.write_to_vec(&mut out);
    out
}

/// Serializes a [`FrozenChangeProof`] with empty node lists and the given batch
/// operations. Returns all bytes including the 32-byte proof header.
fn change_proof_bytes(ops: Box<[BatchOp<Key, Value>]>) -> Vec<u8> {
    let proof = FrozenChangeProof::new(
        Proof::new(Box::<[ProofNode]>::from([])),
        Proof::new(Box::<[ProofNode]>::from([])),
        ops,
    );
    let mut out = Vec::new();
    proof.write_to_vec(&mut out);
    out
}

/// Header encoding tests — snapshots the full 32-byte proof header.
///
/// The `hash_mode` byte is `0x00` for SHA-256 (merkledb) and `0x01` for
/// Keccak-256 (ethhash), so the header bytes differ between feature modes.
/// Tests are nested inside cfg-gated submodules following the same convention
/// as the node tests above.
mod header {
    use super::*;

    fn range_header() -> Vec<u8> {
        range_proof_bytes(&[])[..32].to_vec()
    }

    fn change_header() -> Vec<u8> {
        change_proof_bytes(Box::new([]))[..32].to_vec()
    }

    #[cfg(not(feature = "ethhash"))]
    mod merkledb {
        use super::*;

        /// Range proof header: magic `fwdproof`, version `0x00`, `hash_mode` `0x00`
        /// (SHA-256), branch_factor `0x10`, proof_type `0x01` (range), 20 zero
        /// reserved bytes.
        #[test]
        fn range() {
            insta::assert_snapshot!(hex::encode(range_header()));
        }

        /// Change proof header: same as range but proof_type `0x02` (change).
        #[test]
        fn change() {
            insta::assert_snapshot!(hex::encode(change_header()));
        }
    }

    #[cfg(feature = "ethhash")]
    mod ethhash {
        use super::*;

        /// Range proof header, Keccak-256 mode: `hash_mode` byte is `0x01`
        /// instead of `0x00`.
        #[test]
        fn range() {
            insta::assert_snapshot!(hex::encode(range_header()));
        }

        /// Change proof header, Keccak-256 mode.
        #[test]
        fn change() {
            insta::assert_snapshot!(hex::encode(change_header()));
        }
    }
}

/// Key-value pair encoding tests — snapshots `bytes[32..]` of a [`FrozenRangeProof`]
/// with empty node lists. Key-value encoding uses raw byte slices and is identical
/// under both hash modes; no cfg-gating is needed.
///
/// The payload begins with `varint(0)` twice (empty start- and end-proof counts),
/// then `varint(N)` followed by N length-prefixed `(key, value)` pairs.
mod key_values {
    use super::*;

    /// No key-value pairs: the KV sequence is encoded as a single `varint(0)`.
    #[test]
    fn empty() {
        insta::assert_snapshot!(hex::encode(&range_proof_bytes(&[])[32..]));
    }

    /// Single pair `b"key1"` → `b"value1"`.
    #[test]
    fn single_pair() {
        insta::assert_snapshot!(hex::encode(
            &range_proof_bytes(&[(b"key1", b"value1")])[32..]
        ));
    }

    /// Two pairs in sequence: `b"key1"`→`b"val1"`, `b"key2"`→`b"val2"`.
    #[test]
    fn multiple_pairs() {
        insta::assert_snapshot!(hex::encode(
            &range_proof_bytes(&[(b"key1", b"val1"), (b"key2", b"val2")])[32..]
        ));
    }
}

/// Batch operation encoding tests — snapshots `bytes[32..]` of a
/// [`FrozenChangeProof`] with empty node lists. Batch ops use fixed opcodes
/// and raw byte slices; encoding is identical under both hash modes.
///
/// The payload begins with `varint(0)` twice (empty start- and end-proof counts),
/// then `varint(N)` followed by N operations. Each operation starts with a 1-byte
/// opcode: `0x00` = `Put`, `0x01` = `Delete`, `0x02` = `DeleteRange`.
mod batch_ops {
    use super::*;

    /// Single `Put` operation: opcode `0x00`, key length-prefixed, then value
    /// length-prefixed.
    #[test]
    fn put() {
        let ops = Box::new([BatchOp::Put {
            key: Box::from(b"key1".as_slice()),
            value: Box::from(b"val1".as_slice()),
        }]);
        insta::assert_snapshot!(hex::encode(&change_proof_bytes(ops)[32..]));
    }

    /// Single `Delete` operation: opcode `0x01`, then key length-prefixed.
    #[test]
    fn delete() {
        let ops = Box::new([BatchOp::Delete {
            key: Box::from(b"key2".as_slice()),
        }]);
        insta::assert_snapshot!(hex::encode(&change_proof_bytes(ops)[32..]));
    }

    /// Single `DeleteRange` operation: opcode `0x02`, then prefix length-prefixed.
    #[test]
    fn delete_range() {
        let ops = Box::new([BatchOp::DeleteRange {
            prefix: Box::from(b"key3".as_slice()),
        }]);
        insta::assert_snapshot!(hex::encode(&change_proof_bytes(ops)[32..]));
    }

    /// All three operations in sequence. Matches the proof constructed by
    /// `create_valid_change_proof()` in `tests.rs`.
    #[test]
    fn all_ops() {
        let ops = Box::new([
            BatchOp::Put {
                key: Box::from(b"key1".as_slice()),
                value: Box::from(b"val1".as_slice()),
            },
            BatchOp::Delete {
                key: Box::from(b"key2".as_slice()),
            },
            BatchOp::DeleteRange {
                prefix: Box::from(b"key3".as_slice()),
            },
        ]);
        insta::assert_snapshot!(hex::encode(&change_proof_bytes(ops)[32..]));
    }
}
