// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

#![expect(clippy::unwrap_used, clippy::indexing_slicing)]

use integer_encoding::VarInt;
use test_case::test_case;

use firewood_storage::{Children, IntoHashType, PathComponent, TrieHash, ValueDigest};

use super::{
    header::InvalidHeader,
    magic,
    reader::ReadError,
    types::{Proof, ProofNode, ProofType},
};
use crate::api::{FrozenChangeProof, FrozenRangeProof};
use crate::db::BatchOp;

fn create_valid_range_proof() -> (FrozenRangeProof, Vec<u8>) {
    let merkle = crate::merkle::tests::init_merkle((0u8..=10).map(|k| ([k], [k])));
    let proof = merkle
        .range_proof(Some(&[2u8]), Some(&[8u8]), std::num::NonZeroUsize::new(5))
        .unwrap();
    let mut serialized = Vec::new();
    proof.write_to_vec(&mut serialized);
    (proof, serialized)
}

fn create_valid_change_proof() -> (FrozenChangeProof, Vec<u8>) {
    let proof = FrozenChangeProof::new(
        Proof::new(Box::<[ProofNode]>::from([])),
        Proof::new(Box::<[ProofNode]>::from([])),
        Box::new([
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
        ]),
    );
    let mut serialized = Vec::new();
    proof.write_to_vec(&mut serialized);
    (proof, serialized)
}

#[test]
fn test_range_proof_roundtrip() {
    let (_, serialized) = create_valid_range_proof();
    let parsed = FrozenRangeProof::from_slice(&serialized).expect("roundtrip should succeed");
    let mut re_serialized = Vec::new();
    parsed.write_to_vec(&mut re_serialized);
    assert_eq!(serialized, re_serialized);
}

#[test]
fn test_change_proof_roundtrip() {
    let (_, serialized) = create_valid_change_proof();
    let parsed = FrozenChangeProof::from_slice(&serialized).expect("roundtrip should succeed");
    let mut re_serialized = Vec::new();
    parsed.write_to_vec(&mut re_serialized);
    assert_eq!(serialized, re_serialized);
}

#[test_case(
    |data| *<&mut [u8; 8]>::try_from(&mut data[0..8]).unwrap() = *b"badmagic",
    |err| matches!(err, InvalidHeader::InvalidMagic { found } if found == b"badmagic");
    "invalid magic"
)]
#[test_case(
    |data| data[8] = 99,
    |err| matches!(err, InvalidHeader::UnsupportedVersion { found: 99 });
    "unsupported version"
)]
#[test_case(
    |data| data[9] = 99,
    |err| matches!(err, InvalidHeader::UnsupportedHashMode { found: 99 });
    "unsupported hash mode"
)]
#[test_case(
    |data| data[10] = 99,
    |err| matches!(err, InvalidHeader::UnsupportedBranchFactor { found: 99 });
    "unsupported branch factor"
)]
#[test_case(
    |data| data[11] = 99,
    |err| matches!(err, InvalidHeader::InvalidProofType { found: 99, expected: Some(ProofType::Range) });
    "invalid proof type"
)]
#[test_case(
    |data| data[11] = ProofType::Change as u8,
    |err| matches!(err, InvalidHeader::InvalidProofType { found: 2, expected: Some(ProofType::Range) });
    "wrong proof type"
)]
fn test_invalid_header(
    mutator: impl FnOnce(&mut Vec<u8>),
    expected: impl FnOnce(&InvalidHeader) -> bool,
) {
    let (_, mut data) = create_valid_range_proof();

    mutator(&mut data);

    match FrozenRangeProof::from_slice(&data) {
        Err(ReadError::InvalidHeader(err)) => assert!(expected(&err), "unexpected error: {err}"),
        other => panic!("Expected ReadError::InvalidHeader, got: {other:?}"),
    }
}

#[test_case(
    |_, data| data.truncate(20),
    "header",
    32, // expected len
    20; // found len
    "incomplete header"
)]
#[test_case(
    |_, data| data.truncate(31),
    "header",
    32, // expected len
    31; // found len
    "header one byte short"
)]
#[test_case(
    |_, data| data.truncate(32),
    "array length",
    1, // expected len
    0; // found len
    "no varint after header"
)]
#[test_case(
    |proof, data| {
        #[expect(clippy::arithmetic_side_effects)]
        data.truncate(
            32
            + proof.start_proof().len().required_space()
            + proof.start_proof()[0].key.len().required_space()
            // truncate after the key length varint but before the key bytes
        );
    },
    "byte slice",
    1, // expected len
    0; // found len
    "truncated node key"
)]
fn test_incomplete_item(
    mutator: impl FnOnce(&FrozenRangeProof, &mut Vec<u8>),
    item: &'static str,
    expected_len: usize,
    found_len: usize,
) {
    let (proof, mut data) = create_valid_range_proof();

    eprintln!("data len: {}", data.len());
    eprintln!("proof: {proof:#?}");
    eprintln!("data: {}", hex::encode(&data));

    mutator(&proof, &mut data);

    match FrozenRangeProof::from_slice(&data) {
        Err(ReadError::IncompleteItem {
            item: found_item,
            offset: _,
            expected,
            found,
        }) => {
            assert_eq!(
                found_item, item,
                "unexpected `item` value, got: {found_item}, wanted: {item}; {data:?}"
            );
            assert_eq!(
                expected, expected_len,
                "unexpected `expected` value, got: {expected}, wanted: {expected_len}; {data:?}"
            );
            assert_eq!(
                found, found_len,
                "unexpected `found` value, got: {found}, wanted: {found_len}; {data:?}"
            );
        }
        other => panic!("Expected ReadError::IncompleteItem, got: {other:?}"),
    }
}

#[test_case(
    |proof, data| data[32
        + proof.start_proof().len().required_space()
        + proof.start_proof()[0].key.len().required_space()
        + proof.start_proof()[0].key.len()
        + proof.start_proof()[0].partial_len.required_space()
        // Corrupt the option discriminant for the value digest (should be 0 or 1)
    ] = 3, // invalid option discriminant
    "option discriminant",
    "0 or 1",
    "3";
    "invalid option discriminant"
)]
#[test_case(
    |_, data| *<&mut [u8; 10]>::try_from(&mut data[32..42]).unwrap() = [0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89],
    "array length",
    "byte with no MSB within 9 bytes",
    "[128, 129, 130, 131, 132, 133, 134, 135, 136, 137]";
    "invalid varint"
)]
#[test_case(
    |_, data| data.extend_from_slice(&[0xFF; 100]), // extend data with invalid trailing bytes
    "trailing bytes",
    "no data after the proof",
    "100 bytes";
    "extra trailing bytes"
)]
fn test_invalid_item(
    mutator: impl FnOnce(&FrozenRangeProof, &mut Vec<u8>),
    item: &'static str,
    expected: &'static str,
    found: &'static str,
) {
    let (proof, mut data) = create_valid_range_proof();

    mutator(&proof, &mut data);

    match FrozenRangeProof::from_slice(&data) {
        Err(ReadError::InvalidItem {
            item: found_item,
            offset: _,
            expected: found_expected,
            found: found_found,
        }) => {
            assert_eq!(
                found_item, item,
                "unexpected `item` value, got: {found_item}, wanted: {item}"
            );
            assert_eq!(
                found_expected, expected,
                "unexpected `expected` value, got: {found_expected}, wanted: {expected}"
            );
            assert_eq!(
                found_found, found,
                "unexpected `found` value, got: {found_found}, wanted: {found}"
            );
        }
        other => panic!("Expected ReadError::InvalidItem, got: {other:?}"),
    }
}

#[test]
fn test_partial_key_len_exceeds_key_len() {
    let (proof, mut data) = create_valid_range_proof();

    let node = &proof.start_proof()[0];
    let key_len = node.key.len();
    let original_partial_len_size = node.partial_len.required_space();
    let invalid_partial_len: usize = key_len + 1;

    let offset =
        32 + proof.start_proof().len().required_space() + key_len.required_space() + key_len;

    data.splice(
        offset..offset + original_partial_len_size,
        invalid_partial_len.encode_var_vec(),
    );

    match FrozenRangeProof::from_slice(&data) {
        Err(ReadError::InvalidItem {
            item,
            expected,
            found,
            ..
        }) => {
            assert_eq!(item, "partial key length");
            assert_eq!(expected, "value less than or equal to the key length");
            assert_eq!(found, invalid_partial_len.to_string());
        }
        other => panic!("Expected ReadError::InvalidItem, got: {other:?}"),
    }
}

#[test]
fn test_empty_proof() {
    #[rustfmt::skip]
    let bytes = [
        b'f', b'w', b'd', b'p', b'r', b'o', b'o', b'f', // magic
        0, // version
        magic::HASH_MODE,
        magic::BRANCH_FACTOR,
        ProofType::Range as u8,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, // reserved
        0, // start proof length = 0
        0, // end proof length = 0
        0, // key-value pairs length = 0
    ];

    match FrozenRangeProof::from_slice(&bytes) {
        Ok(proof) => {
            assert!(proof.start_proof().is_empty());
            assert!(proof.end_proof().is_empty());
            assert!(proof.key_values().is_empty());
        }
        Err(err) => panic!("Expected valid empty proof, got error: {err}"),
    }
}

#[test_case(
    |data| *<&mut [u8; 8]>::try_from(&mut data[0..8]).unwrap() = *b"badmagic",
    |err| matches!(err, InvalidHeader::InvalidMagic { found } if found == b"badmagic");
    "invalid magic"
)]
#[test_case(
    |data| data[8] = 99,
    |err| matches!(err, InvalidHeader::UnsupportedVersion { found: 99 });
    "unsupported version"
)]
#[test_case(
    |data| data[9] = 99,
    |err| matches!(err, InvalidHeader::UnsupportedHashMode { found: 99 });
    "unsupported hash mode"
)]
#[test_case(
    |data| data[10] = 99,
    |err| matches!(err, InvalidHeader::UnsupportedBranchFactor { found: 99 });
    "unsupported branch factor"
)]
#[test_case(
    |data| data[11] = 99,
    |err| matches!(err, InvalidHeader::InvalidProofType { found: 99, expected: Some(ProofType::Change) });
    "invalid proof type"
)]
#[test_case(
    |data| data[11] = ProofType::Range as u8,
    |err| matches!(err, InvalidHeader::InvalidProofType { found: 1, expected: Some(ProofType::Change) });
    "wrong proof type"
)]
fn test_change_proof_invalid_header(
    mutator: impl FnOnce(&mut Vec<u8>),
    expected: impl FnOnce(&InvalidHeader) -> bool,
) {
    let (_, mut data) = create_valid_change_proof();

    mutator(&mut data);

    match FrozenChangeProof::from_slice(&data) {
        Err(ReadError::InvalidHeader(err)) => assert!(expected(&err), "unexpected error: {err}"),
        other => panic!("Expected ReadError::InvalidHeader, got: {other:?}"),
    }
}

#[test_case(
    |data| data.truncate(20),
    "header",
    32, // expected len
    20; // found len
    "incomplete header"
)]
#[test_case(
    |data| data.truncate(31),
    "header",
    32, // expected len
    31; // found len
    "header one byte short"
)]
#[test_case(
    |data| data.truncate(32),
    "array length",
    1, // expected len
    0; // found len
    "no varint after header"
)]
fn test_change_proof_incomplete_item(
    mutator: impl FnOnce(&mut Vec<u8>),
    item: &'static str,
    expected_len: usize,
    found_len: usize,
) {
    let (_, mut data) = create_valid_change_proof();

    mutator(&mut data);

    match FrozenChangeProof::from_slice(&data) {
        Err(ReadError::IncompleteItem {
            item: found_item,
            offset: _,
            expected,
            found,
        }) => {
            assert_eq!(
                found_item, item,
                "unexpected `item` value, got: {found_item}, wanted: {item}; {data:?}"
            );
            assert_eq!(
                expected, expected_len,
                "unexpected `expected` value, got: {expected}, wanted: {expected_len}; {data:?}"
            );
            assert_eq!(
                found, found_len,
                "unexpected `found` value, got: {found}, wanted: {found_len}; {data:?}"
            );
        }
        other => panic!("Expected ReadError::IncompleteItem, got: {other:?}"),
    }
}

#[test_case(
    |_, data| *<&mut [u8; 10]>::try_from(&mut data[32..42]).unwrap() = [0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89],
    "array length",
    "byte with no MSB within 9 bytes",
    "[128, 129, 130, 131, 132, 133, 134, 135, 136, 137]";
    "invalid varint"
)]
#[test_case(
    |_, data| data.extend_from_slice(&[0xFF; 100]),
    "trailing bytes",
    "no data after the proof",
    "100 bytes";
    "extra trailing bytes"
)]
#[test_case(
    // Layout: 32 (header) + 1 (start_proof len=0) + 1 (end_proof len=0) + 1 (batch_ops len=3) = offset 35
    // Byte at offset 35 is the first BatchOp discriminant (BATCH_PUT = 0)
    |_, data| data[35] = 99,
    "option discriminant",
    "0, 1, or 2",
    "99";
    "invalid batch op discriminant"
)]
fn test_change_proof_invalid_item(
    mutator: impl FnOnce(&FrozenChangeProof, &mut Vec<u8>),
    item: &'static str,
    expected: &'static str,
    found: &'static str,
) {
    let (proof, mut data) = create_valid_change_proof();

    mutator(&proof, &mut data);

    match FrozenChangeProof::from_slice(&data) {
        Err(ReadError::InvalidItem {
            item: found_item,
            offset: _,
            expected: found_expected,
            found: found_found,
        }) => {
            assert_eq!(
                found_item, item,
                "unexpected `item` value, got: {found_item}, wanted: {item}"
            );
            assert_eq!(
                found_expected, expected,
                "unexpected `expected` value, got: {found_expected}, wanted: {expected}"
            );
            assert_eq!(
                found_found, found,
                "unexpected `found` value, got: {found_found}, wanted: {found}"
            );
        }
        other => panic!("Expected ReadError::InvalidItem, got: {other:?}"),
    }
}

/// Constructs a `ProofNode` with the given nibble key, partial length, optional
/// value, and children at the specified nibble indices.
fn make_proof_node(
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

/// Wraps a single `ProofNode` in a minimal `FrozenRangeProof` and serializes it.
fn make_range_proof_from_single_node(node: ProofNode) -> (FrozenRangeProof, Vec<u8>) {
    let proof = FrozenRangeProof::new(
        Proof::new(Box::new([node])),
        Proof::new(Box::<[ProofNode]>::from([])),
        Box::new([]),
    );
    let mut serialized = Vec::new();
    proof.write_to_vec(&mut serialized);
    (proof, serialized)
}

/// Verifies that deserializing `serialized` and re-serializing produces the same bytes.
fn assert_range_proof_round_trip(serialized: Vec<u8>) {
    let parsed = FrozenRangeProof::from_slice(&serialized).expect("deserialization should succeed");
    let mut re_serialized = Vec::new();
    parsed.write_to_vec(&mut re_serialized);
    assert_eq!(serialized, re_serialized, "round-trip bytes must match");
}

#[test]
fn test_proof_node_leaf_round_trip() {
    // Leaf: no children, no value, empty key
    let node = make_proof_node(&[], 0, None, &[]);
    let (_, serialized) = make_range_proof_from_single_node(node);
    assert_range_proof_round_trip(serialized);
}

#[test]
fn test_proof_node_single_child_round_trip() {
    // Branch with one child at nibble index 7
    let node = make_proof_node(&[1, 2, 3], 0, None, &[7]);
    let (_, serialized) = make_range_proof_from_single_node(node);
    assert_range_proof_round_trip(serialized);
}

#[test]
fn test_proof_node_all_children_round_trip() {
    // Branch with all 16 children present (ChildMask = 0xFFFF)
    let all_nibbles: Vec<u8> = (0u8..16).collect();
    let node = make_proof_node(&[0], 0, None, &all_nibbles);
    let (_, serialized) = make_range_proof_from_single_node(node);
    assert_range_proof_round_trip(serialized);
}

#[cfg(not(feature = "ethhash"))]
#[test]
fn test_value_digest_hash_round_trip() {
    // Values >= 32 bytes are converted to a hash by make_hash() during serialization.
    // The round-trip bytes should still match because re-serializing a Hash-variant
    // node also produces a hash discriminant (1) rather than a value discriminant (0).
    let value: Box<[u8]> = vec![0xABu8; 32].into_boxed_slice();
    let node = make_proof_node(&[1, 2], 0, Some(value), &[]);
    let (_, serialized) = make_range_proof_from_single_node(node);
    assert_range_proof_round_trip(serialized);
}

#[test_case(
    BatchOp::Put { key: Box::from(b"k".as_slice()), value: Box::from(b"v".as_slice()) };
    "put"
)]
#[test_case(
    BatchOp::Delete { key: Box::from(b"k".as_slice()) };
    "delete"
)]
#[test_case(
    BatchOp::DeleteRange { prefix: Box::from(b"k".as_slice()) };
    "delete range"
)]
fn test_change_proof_batch_op_variant(op: BatchOp<Box<[u8]>, Box<[u8]>>) {
    let proof = FrozenChangeProof::new(
        Proof::new(Box::<[ProofNode]>::from([])),
        Proof::new(Box::<[ProofNode]>::from([])),
        Box::new([op]),
    );
    let mut serialized = Vec::new();
    proof.write_to_vec(&mut serialized);
    let parsed =
        FrozenChangeProof::from_slice(&serialized).expect("deserialization should succeed");
    let mut re_serialized = Vec::new();
    parsed.write_to_vec(&mut re_serialized);
    assert_eq!(serialized, re_serialized, "round-trip bytes must match");
}

#[test]
fn test_proof_node_partial_len_boundaries() {
    // partial_len = 0: no shared prefix with the parent node
    let node = make_proof_node(&[1, 2, 3, 4], 0, None, &[]);
    let (_, serialized) = make_range_proof_from_single_node(node);
    assert_range_proof_round_trip(serialized);

    // partial_len = key.len(): entire key is shared with the parent
    let node = make_proof_node(&[1, 2, 3, 4], 4, None, &[]);
    let (_, serialized) = make_range_proof_from_single_node(node);
    assert_range_proof_round_trip(serialized);
}
