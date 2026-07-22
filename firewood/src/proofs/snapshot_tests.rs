// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Snapshot tests for [`ProofNode`] wire serialization.
//!
//! Each test constructs a canonical [`ProofNode`] variant and snapshots the
//! hex-encoded bytes that follow the fixed 32-byte proof header when it is
//! serialized inside a minimal [`FrozenRangeProof`]. The header itself is
//! excluded because it encodes only proof-level metadata (magic, version,
//! hash mode, proof type); those node snapshots target only the node encoding.
//!
//! Snapshot files live in `src/proofs/snapshots/`. Run `just snapshot-proof-nodes`
//! to write them on the first run or to regenerate them after an intentional
//! wire-format change.
//!
//! # Structure
//!
//! Each group of structurally-identical tests is a single
//! [`test_case`](test_case::test_case)-parametrized function. The first
//! argument of every case is the explicit `insta` snapshot name (an explicit
//! name is required: `test_case` would otherwise make all cases of one function
//! resolve to the same auto-derived name).
//!
//! The node-hashing scheme is a runtime, per-database choice (issue #1088), so
//! tests whose bytes differ between hash modes (branch child encoding and the
//! header's `hash_mode` byte) take the [`NodeHashAlgorithm`] as an explicit case
//! argument and run once per mode; [`hash_mode_name`] prefixes their snapshot
//! name with `merkledb__` or `ethhash__` so each mode gets its own snapshot
//! file. Tests whose bytes are mode-independent pick a single mode and share one
//! snapshot file.
//!
//! # Test coverage
//!
//! ## `ProofNode` encoding
//!
//! [`proof_node`] cases have no children, so their bytes are identical under
//! both hash modes and share one snapshot file. [`branch_node`] cases carry
//! child hashes (or a hashed large value), whose encoding differs per mode, so
//! they are prefixed via [`hash_mode_name`].
//!
//! | Function | Case | Key nibbles | `partial_len` | Value | Children |
//! |----------|------|-------------|---------------|-------|----------|
//! | [`proof_node`] | `empty` | `[]` | 0 | none | none |
//! | [`proof_node`] | `leaf_with_value` | `[1, 2, 3]` | 0 | `b"hello"` | none |
//! | [`proof_node`] | `partial_path` | `[1, 2, 3, 4, 5]` | 3 | none | none |
//! | [`branch_node`] | `single_child` | `[1]` | 0 | none | nibble 7 |
//! | [`branch_node`] | `all_children` | `[0]` | 0 | none | all 16 |
//! | [`branch_node`] | `large_value_becomes_hash` | `[1, 2]` | 0 | 32 × `0xAB` | none |
//!
//! `large_value_becomes_hash` enumerates only the `merkledb` mode: the
//! large-value → hash behaviour is format-agnostic and is fully covered by the
//! MerkleDB variant, while `single_child` / `all_children` already exercise the
//! complete child-hash encoding under both modes.
//!
//! ## Header, key-values, and batch ops
//!
//! [`proof_header`] snapshots the full 32-byte header (whose `hash_mode` byte
//! differs between SHA-256 and Keccak-256), so it runs under both modes and is
//! prefixed via [`hash_mode_name`]. [`key_value`] and [`batch_op`] snapshot
//! `bytes[32..]` and are mode-independent (raw byte slices / fixed opcodes);
//! their explicit snapshot names carry a `key_values__` / `batch_ops__` group
//! prefix so they stay unique within the flattened module (snapshot names are
//! keyed by module path + name, not by function).
//!
//! | Function | Case | Subject |
//! |----------|------|---------|
//! | [`proof_header`] | `range` | 32-byte header, range proof type |
//! | [`proof_header`] | `change` | 32-byte header, change proof type |
//! | [`key_value`] | `empty` | Empty KV sequence: `varint(0)` |
//! | [`key_value`] | `single_pair` | One `(key, value)` pair |
//! | [`key_value`] | `multiple_pairs` | Two `(key, value)` pairs |
//! | [`batch_op`] | `put` | Single `Put` operation (opcode `0x00`) |
//! | [`batch_op`] | `delete` | Single `Delete` operation (opcode `0x01`) |
//! | [`batch_op`] | `delete_range` | Single `DeleteRange` operation (opcode `0x02`) |
//! | [`batch_op`] | `all_ops` | All three operations in sequence |

use test_case::test_case;

use firewood_storage::{
    Children, IntoHashType, NodeHashAlgorithm, PathComponent, TrieHash, ValueDigest,
};

use super::types::{Proof, ProofNode};
use crate::api::{FrozenChangeProof, FrozenRangeProof};
use crate::db::BatchOp;
use crate::merkle::{Key, Value};

/// Prefixes a snapshot name with the given hash mode so mode-split tests write
/// distinct snapshot files. Bytes differ between modes, so `merkledb` and
/// `ethhash` snapshots must not share a filename.
fn hash_mode_name(algorithm: NodeHashAlgorithm, name: &str) -> String {
    let mode = if algorithm.is_ethereum() {
        "ethhash"
    } else {
        "merkledb"
    };
    format!("{mode}__{name}")
}

/// Serializes `node` inside a minimal [`FrozenRangeProof`] built in the given
/// runtime hash mode and returns the bytes after the fixed 32-byte proof header.
///
/// The returned slice begins with the varint-encoded start-proof node count,
/// followed by the [`ProofNode`] encoding, then the empty end-proof and
/// key-values counts.
fn node_bytes(node: &ProofNode, algorithm: NodeHashAlgorithm) -> Vec<u8> {
    let proof = FrozenRangeProof::new_with_hash_mode(
        Proof::new(Box::new([node.clone()])),
        Proof::new(Box::<[ProofNode]>::from([])),
        Box::new([]),
        algorithm,
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

/// Serializes a [`FrozenRangeProof`] built in the given runtime hash mode with
/// empty node lists and the given key-value pairs. Returns all bytes including
/// the 32-byte proof header.
fn range_proof_bytes(algorithm: NodeHashAlgorithm, kvs: &[(&[u8], &[u8])]) -> Vec<u8> {
    let kv_pairs: Box<[(Key, Value)]> = kvs
        .iter()
        .map(|(k, v)| (Box::from(*k), Box::from(*v)))
        .collect();
    let proof = FrozenRangeProof::new_with_hash_mode(
        Proof::new(Box::<[ProofNode]>::from([])),
        Proof::new(Box::<[ProofNode]>::from([])),
        kv_pairs,
        algorithm,
    );
    let mut out = Vec::new();
    proof.write_to_vec(&mut out);
    out
}

/// Serializes a [`FrozenChangeProof`] built in the given runtime hash mode with
/// empty node lists and the given batch operations. Returns all bytes including
/// the 32-byte proof header.
fn change_proof_bytes(algorithm: NodeHashAlgorithm, ops: Box<[BatchOp<Key, Value>]>) -> Vec<u8> {
    let proof = FrozenChangeProof::new_with_hash_mode(
        Proof::new(Box::<[ProofNode]>::from([])),
        Proof::new(Box::<[ProofNode]>::from([])),
        ops,
        algorithm,
    );
    let mut out = Vec::new();
    proof.write_to_vec(&mut out);
    out
}

/// [`ProofNode`] encodings with no children — identical bytes under both hash
/// modes, so a single (unprefixed) snapshot file is shared and a fixed mode
/// (MerkleDB) is used to produce it.
///
/// - `empty`: zero-length key, no value, no children — 8 bytes after the header.
/// - `leaf_with_value`: short value (`b"hello"`, < 32 bytes) encoded as
///   `ValueDigest::Value` (discriminant `0x00`).
/// - `partial_path`: `partial_len = 3` on a 5-nibble key; the first 3 nibbles
///   belong to the parent, the remaining 2 are this node's own partial path.
#[test_case("empty", &[], 0, None, &[] ; "empty")]
#[test_case("leaf_with_value", &[1, 2, 3], 0, Some(Box::from(b"hello".as_slice())), &[] ; "leaf_with_value")]
#[test_case("partial_path", &[1, 2, 3, 4, 5], 3, None, &[] ; "partial_path")]
fn proof_node(
    name: &str,
    key: &[u8],
    partial_len: usize,
    value: Option<Box<[u8]>>,
    children: &[u8],
) {
    let node = make_node(key, partial_len, value, children);
    insta::assert_snapshot!(
        name,
        hex::encode(node_bytes(&node, NodeHashAlgorithm::MerkleDB))
    );
}

/// [`ProofNode`] encodings whose bytes depend on the hash mode, so each case
/// runs under both modes and [`hash_mode_name`] keeps the two modes in separate
/// files.
///
/// MerkleDB encodes each child hash as a flat 32-byte `TrieHash`; ethhash
/// prefixes each with a 1-byte discriminant (`0x00` = 32-byte Keccak hash,
/// `0x01` = inline RLP).
///
/// - `single_child`: one child at nibble 7 (`ChildMask` bit 7, `0x0080`).
/// - `all_children`: all 16 children (`ChildMask = 0xFFFF`).
/// - `large_value_becomes_hash` (merkledb-only): a 32-byte value is hashed by
///   `make_hash()` and encoded as `ValueDigest::Hash`.
#[test_case("single_child", NodeHashAlgorithm::MerkleDB, &[1], 0, None, &[7] ; "single_child_merkledb")]
#[test_case("single_child", NodeHashAlgorithm::Ethereum, &[1], 0, None, &[7] ; "single_child_ethhash")]
#[test_case("all_children", NodeHashAlgorithm::MerkleDB, &[0], 0, None, &[0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15] ; "all_children_merkledb")]
#[test_case("all_children", NodeHashAlgorithm::Ethereum, &[0], 0, None, &[0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15] ; "all_children_ethhash")]
#[test_case("large_value_becomes_hash", NodeHashAlgorithm::MerkleDB, &[1, 2], 0, Some(vec![0xABu8; 32].into_boxed_slice()), &[] ; "large_value_becomes_hash")]
fn branch_node(
    name: &str,
    algorithm: NodeHashAlgorithm,
    key: &[u8],
    partial_len: usize,
    value: Option<Box<[u8]>>,
    children: &[u8],
) {
    let node = make_node(key, partial_len, value, children);
    insta::assert_snapshot!(
        hash_mode_name(algorithm, name),
        hex::encode(node_bytes(&node, algorithm))
    );
}

/// Proof header encoding — the full 32-byte header. The `hash_mode` byte is
/// `0x00` for SHA-256 (merkledb) and `0x01` for Keccak-256 (ethhash), so the
/// bytes differ between modes; each case runs under both and [`hash_mode_name`]
/// keeps them in separate files.
///
/// `range` (`proof_type` `0x01`) and `change` (`proof_type` `0x02`) otherwise
/// share the same magic `fwdproof`, version `0x00`, `branch_factor` `0x10`, and
/// 20 zero reserved bytes.
#[test_case("range", NodeHashAlgorithm::MerkleDB, false ; "range_merkledb")]
#[test_case("range", NodeHashAlgorithm::Ethereum, false ; "range_ethhash")]
#[test_case("change", NodeHashAlgorithm::MerkleDB, true ; "change_merkledb")]
#[test_case("change", NodeHashAlgorithm::Ethereum, true ; "change_ethhash")]
fn proof_header(name: &str, algorithm: NodeHashAlgorithm, change: bool) {
    let bytes = if change {
        change_proof_bytes(algorithm, Box::new([]))[..32].to_vec()
    } else {
        range_proof_bytes(algorithm, &[])[..32].to_vec()
    };
    insta::assert_snapshot!(hash_mode_name(algorithm, name), hex::encode(bytes));
}

/// Key-value pair encoding — snapshots `bytes[32..]` of a [`FrozenRangeProof`]
/// with empty node lists. Key-value encoding uses raw byte slices and is
/// identical under both hash modes, so no prefix is needed and a fixed mode
/// (MerkleDB) is used.
///
/// The payload begins with `varint(0)` twice (empty start- and end-proof
/// counts), then `varint(N)` followed by N length-prefixed `(key, value)` pairs.
#[test_case("empty", &[] ; "empty")]
#[test_case("single_pair", &[(b"key1".as_slice(), b"value1".as_slice())] ; "single_pair")]
#[test_case("multiple_pairs", &[(b"key1".as_slice(), b"val1".as_slice()), (b"key2".as_slice(), b"val2".as_slice())] ; "multiple_pairs")]
fn key_value(name: &str, pairs: &[(&[u8], &[u8])]) {
    insta::assert_snapshot!(
        format!("key_values__{name}"),
        hex::encode(&range_proof_bytes(NodeHashAlgorithm::MerkleDB, pairs)[32..])
    );
}

/// Batch operation encoding — snapshots `bytes[32..]` of a [`FrozenChangeProof`]
/// with empty node lists. Batch ops use fixed opcodes and raw byte slices;
/// encoding is identical under both hash modes, so a fixed mode (MerkleDB) is
/// used.
///
/// The payload begins with `varint(0)` twice (empty start- and end-proof
/// counts), then `varint(N)` followed by N operations. Each operation starts
/// with a 1-byte opcode: `0x00` = `Put`, `0x01` = `Delete`,
/// `0x02` = `DeleteRange`. `all_ops` matches the proof constructed by
/// `create_valid_change_proof()` in `tests.rs`.
#[test_case("put", Box::new([BatchOp::Put { key: Box::from(b"key1".as_slice()), value: Box::from(b"val1".as_slice()) }]) ; "put")]
#[test_case("delete", Box::new([BatchOp::Delete { key: Box::from(b"key2".as_slice()) }]) ; "delete")]
#[test_case("delete_range", Box::new([BatchOp::DeleteRange { prefix: Box::from(b"key3".as_slice()) }]) ; "delete_range")]
#[test_case(
    "all_ops",
    Box::new([
        BatchOp::Put { key: Box::from(b"key1".as_slice()), value: Box::from(b"val1".as_slice()) },
        BatchOp::Delete { key: Box::from(b"key2".as_slice()) },
        BatchOp::DeleteRange { prefix: Box::from(b"key3".as_slice()) },
    ])
    ; "all_ops"
)]
fn batch_op(name: &str, ops: Box<[BatchOp<Key, Value>]>) {
    insta::assert_snapshot!(
        format!("batch_ops__{name}"),
        hex::encode(&change_proof_bytes(NodeHashAlgorithm::MerkleDB, ops)[32..])
    );
}
