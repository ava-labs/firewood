// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use integer_encoding::VarInt;
use test_case::test_case;

use firewood_storage::{
    Children, IntoHashType, PathComponent, SeededRng, TrieHash, ValueDigest, logger::debug,
};

use super::{
    header::InvalidHeader,
    magic,
    reader::ReadError,
    types::{Proof, ProofNode, ProofType},
};
use crate::api::{FrozenChangeProof, FrozenRangeProof};
use crate::db::BatchOp;

/// Builds the 32-byte proof header by hand (magic, version 0, hash mode,
/// branch factor, proof type, reserved).
fn raw_header(proof_type: ProofType) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    out.extend_from_slice(magic::PROOF_HEADER);
    out.push(0); // version
    out.push(magic::HASH_MODE);
    out.push(magic::BRANCH_FACTOR);
    out.push(proof_type as u8);
    out.extend_from_slice(&[0u8; 20]);
    out
}

/// An empty proof-node list, shared by the minimal proofs built below.
fn empty_nodes() -> Proof<Box<[ProofNode]>> {
    Proof::new(Box::<[ProofNode]>::from([]))
}

/// Frames `header || canonical body` bytes into wire bytes the way
/// [`FrozenRangeProof::write_to_vec`] does (the wire compresses the body
/// behind the header). The byte-taxonomy tests below craft and mutate the
/// *canonical* bytes — offsets and error expectations target the body
/// parser — so this helper re-frames them before parsing. Inputs shorter
/// than a header are passed through unchanged (header errors fire first).
fn frame_canonical(data: &[u8], proof_type: ProofType) -> Vec<u8> {
    match data.split_at_checked(32) {
        Some((header, body)) => {
            let mut wire = header.to_vec();
            super::frame::write_compressed_body(body, &mut wire, proof_type);
            wire
        }
        None => data.to_vec(),
    }
}

/// Parses `header || canonical body` bytes via [`frame_canonical`].
fn parse_range_canonical(data: &[u8]) -> Result<FrozenRangeProof, ReadError> {
    FrozenRangeProof::from_slice(&frame_canonical(data, ProofType::Range))
}

/// See [`parse_range_canonical`].
fn parse_change_canonical(data: &[u8]) -> Result<FrozenChangeProof, ReadError> {
    FrozenChangeProof::from_slice(&frame_canonical(data, ProofType::Change))
}

/// Returns a valid range proof plus its canonical uncompressed bytes
/// (`header || body`) for the byte-taxonomy tests. Parse the result with
/// [`parse_range_canonical`].
fn create_valid_range_proof() -> (FrozenRangeProof, Vec<u8>) {
    let merkle = crate::merkle::tests::init_merkle((0u8..=10).map(|k| ([k], [k])));
    let proof = merkle
        .range_proof(Some(&[2u8]), Some(&[8u8]), std::num::NonZeroUsize::new(5))
        .unwrap();
    let mut serialized = raw_header(ProofType::Range);
    proof.write_body_to_vec(&mut serialized);
    (proof, serialized)
}

fn create_valid_change_proof() -> (FrozenChangeProof, Vec<u8>) {
    let proof = FrozenChangeProof::new(
        empty_nodes(),
        empty_nodes(),
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
    let mut serialized = raw_header(ProofType::Change);
    proof.write_body_to_vec(&mut serialized);
    (proof, serialized)
}

#[test]
fn test_range_proof_roundtrip() {
    let (proof, _) = create_valid_range_proof();
    let mut wire = Vec::new();
    proof.write_to_vec(&mut wire);
    let parsed = FrozenRangeProof::from_slice(&wire).expect("roundtrip should succeed");
    assert_eq!(proof, parsed);
    let mut re_serialized = Vec::new();
    parsed.write_to_vec(&mut re_serialized);
    assert_eq!(wire, re_serialized, "re-serialization must be idempotent");
}

#[test]
fn test_change_proof_roundtrip() {
    let (proof, canonical) = create_valid_change_proof();
    let mut wire = Vec::new();
    proof.write_to_vec(&mut wire);
    let parsed = FrozenChangeProof::from_slice(&wire).expect("roundtrip should succeed");
    let mut reparsed_canonical = raw_header(ProofType::Change);
    parsed.write_body_to_vec(&mut reparsed_canonical);
    assert_eq!(canonical, reparsed_canonical);
    let mut re_serialized = Vec::new();
    parsed.write_to_vec(&mut re_serialized);
    assert_eq!(wire, re_serialized, "re-serialization must be idempotent");
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

    match parse_range_canonical(&data) {
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

fn test_incomplete_item(
    mutator: impl FnOnce(&FrozenRangeProof, &mut Vec<u8>),
    item: &'static str,
    expected_len: usize,
    found_len: usize,
) {
    let (proof, mut data) = create_valid_range_proof();

    debug!("data len: {}", data.len());
    debug!("proof: {proof:#?}");
    debug!("data: {}", hex::encode(&data));

    mutator(&proof, &mut data);

    match parse_range_canonical(&data) {
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
#[test_case(
    |proof, data| {
        #[expect(clippy::arithmetic_side_effects)]
        data.truncate(
            32
            + proof.start_proof().len().required_space()
            + proof.start_proof()[0].key.len().required_space()
        );
    },
    "array length",
    "length less than or equal to the maximum possible items in remaining bytes",
    "2 > (1 / 5)";
    "truncated node key"
)]
fn test_invalid_item(
    mutator: impl FnOnce(&FrozenRangeProof, &mut Vec<u8>),
    item: &'static str,
    expected: &'static str,
    found: &'static str,
) {
    let (proof, mut data) = create_valid_range_proof();

    mutator(&proof, &mut data);

    match parse_range_canonical(&data) {
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

    match parse_range_canonical(&data) {
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

    match parse_range_canonical(&bytes) {
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

    match parse_change_canonical(&data) {
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

    match parse_change_canonical(&data) {
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

    match parse_change_canonical(&data) {
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
/// Returns the proof plus its canonical uncompressed bytes
/// (`header || body`); parse with [`parse_range_canonical`].
fn make_range_proof_from_single_node(node: ProofNode) -> (FrozenRangeProof, Vec<u8>) {
    let proof = FrozenRangeProof::new(Proof::new(Box::new([node])), empty_nodes(), Box::new([]));
    let mut serialized = raw_header(ProofType::Range);
    proof.write_body_to_vec(&mut serialized);
    (proof, serialized)
}

/// Verifies that parsing the canonical bytes and re-serializing the
/// canonical body produces the same bytes.
fn assert_range_proof_round_trip(serialized: Vec<u8>) {
    let parsed = parse_range_canonical(&serialized).expect("deserialization should succeed");
    let mut re_serialized = raw_header(ProofType::Range);
    parsed.write_body_to_vec(&mut re_serialized);
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
    let proof = FrozenChangeProof::new(empty_nodes(), empty_nodes(), Box::new([op]));
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

// These tests use manually constructed proofs with known byte layouts.
//
// Layout for make_range_proof_from_single_node(make_proof_node(&[1, 2, 3], 0, None, &[])):
//   [32]=0x01 (start_proof count)
//   [33]=0x03 (key byte length)    [34]=0x01  [35]=0x02  [36]=0x03 (key bytes)
//   [37]=0x00 (partial_len)        [38]=0x00  (option discriminant = None)
//   [39]=0x00  [40]=0x00           (ChildMask = 0)
//   [41]=0x00 (end_proof count)    [42]=0x00  (key_values count)
//
// Layout for make_range_proof_from_single_node(make_proof_node(&[1, 2, 3], 0, Some(b"v"), &[])):
//   [32..37] same as above
//   [38]=0x01 (option discriminant = Some)   [39]=0x00 (value digest discriminant = Value)
//   [40]=0x01 (value byte length)            [41]=0x76 (b'v')
//   [42]=0x00  [43]=0x00 (ChildMask = 0)     [44]=0x00  [45]=0x00
//
// Layout for make_range_proof_from_single_node(make_proof_node(&[1], 0, None, &[7])):
//   [32]=0x01  [33]=0x01  [34]=0x01  (count, key len, key byte)
//   [35]=0x00  [36]=0x00             (partial_len, option discriminant = None)
//   [37]=0x80  [38]=0x00             (ChildMask — nibble 7 = bit 7 of low byte)
//   Non-ethhash: [39..71] = TrieHash (32 bytes)
//   Ethhash:     [39] = HashType discriminant, [40..72] = TrieHash

#[test]
fn test_invalid_path_nibble() {
    let node = make_proof_node(&[1, 2, 3], 0, None, &[]);
    let (_, mut data) = make_range_proof_from_single_node(node);
    data[34] = 0x10; // first key byte set to an invalid nibble (16 > 15)
    match parse_range_canonical(&data) {
        Err(ReadError::InvalidItem { item, .. }) => assert_eq!(item, "path"),
        other => panic!("Expected InvalidItem {{ item: \"path\" }}, got: {other:?}"),
    }
}

#[test]
fn test_invalid_value_digest_discriminant() {
    let node = make_proof_node(&[1, 2, 3], 0, Some(Box::from(b"v".as_slice())), &[]);
    let (_, mut data) = make_range_proof_from_single_node(node);
    data[39] = 2; // invalid ValueDigest discriminant (must be 0 or 1)
    match parse_range_canonical(&data) {
        Err(ReadError::InvalidItem {
            item,
            expected,
            found,
            ..
        }) => {
            assert_eq!(item, "value digest discriminant");
            assert_eq!(expected, "0 (value) or 1 (hash)");
            assert_eq!(found, "2");
        }
        other => {
            panic!("Expected InvalidItem {{ item: \"value digest discriminant\" }}, got: {other:?}")
        }
    }
}

#[test_case(38, "option discriminant", 1, 0; "option discriminant")]
#[test_case(39, "children map", 2, 0; "children map zero bytes")]
#[test_case(40, "children map", 2, 1; "children map one byte")]
fn test_incomplete_item_known_layout(
    truncate_at: usize,
    item: &'static str,
    expected_len: usize,
    found_len: usize,
) {
    let node = make_proof_node(&[1, 2, 3], 0, None, &[]);
    let (_, mut data) = make_range_proof_from_single_node(node);
    data.truncate(truncate_at);
    match parse_range_canonical(&data) {
        Err(ReadError::IncompleteItem {
            item: found_item,
            expected,
            found,
            ..
        }) => {
            assert_eq!(found_item, item);
            assert_eq!(expected, expected_len);
            assert_eq!(found, found_len);
        }
        other => panic!("Expected IncompleteItem {{ item: {item:?} }}, got: {other:?}"),
    }
}

#[cfg(not(feature = "ethhash"))]
#[test]
fn test_incomplete_trie_hash() {
    let node = make_proof_node(&[1], 0, None, &[7]);
    let (_, mut data) = make_range_proof_from_single_node(node);
    data.truncate(39); // ChildMask ends at [38]; TrieHash starts at [39]
    match parse_range_canonical(&data) {
        Err(ReadError::IncompleteItem {
            item,
            expected,
            found,
            ..
        }) => {
            assert_eq!(item, "trie hash");
            assert_eq!(expected, 32);
            assert_eq!(found, 0);
        }
        other => panic!("Expected IncompleteItem {{ item: \"trie hash\" }}, got: {other:?}"),
    }
}

#[cfg(feature = "ethhash")]
#[test]
fn test_incomplete_hash_type_discriminant() {
    let node = make_proof_node(&[1], 0, None, &[7]);
    let (_, mut data) = make_range_proof_from_single_node(node);
    data.truncate(39); // ChildMask ends at [38]; HashType discriminant is at [39]
    match parse_range_canonical(&data) {
        Err(ReadError::IncompleteItem {
            item,
            expected,
            found,
            ..
        }) => {
            assert_eq!(item, "hash type discriminant");
            assert_eq!(expected, 1);
            assert_eq!(found, 0);
        }
        other => {
            panic!("Expected IncompleteItem {{ item: \"hash type discriminant\" }}, got: {other:?}")
        }
    }
}

#[cfg(feature = "ethhash")]
#[test]
fn test_invalid_hash_type_discriminant() {
    let node = make_proof_node(&[1], 0, None, &[7]);
    let (_, mut data) = make_range_proof_from_single_node(node);
    data[39] = 2; // invalid HashType discriminant (must be 0 or 1)
    match parse_range_canonical(&data) {
        Err(ReadError::InvalidItem {
            item,
            expected,
            found,
            ..
        }) => {
            assert_eq!(item, "hash type discriminant");
            assert_eq!(expected, "0 (hash) or 1 (rlp)");
            assert_eq!(found, "2");
        }
        other => {
            panic!("Expected InvalidItem {{ item: \"hash type discriminant\" }}, got: {other:?}")
        }
    }
}

#[test]
fn test_change_proof_incomplete_batch_op_discriminant() {
    // Layout of create_valid_change_proof() after the 32-byte header:
    //   [32]=0x00 (start_proof count=0)  [33]=0x00 (end_proof count=0)
    //   [34]=0x03 (batch_ops count=3)    [35]=0x00 (first BatchOp discriminant)
    let (_, mut data) = create_valid_change_proof();
    data.truncate(35); // cut before the first BatchOp discriminant byte
    match parse_change_canonical(&data) {
        Err(ReadError::InvalidItem {
            item,
            expected,
            found,
            ..
        }) => {
            assert_eq!(item, "array length");
            assert_eq!(
                expected,
                "length less than or equal to the maximum possible items in remaining bytes"
            );
            assert_eq!(found, "3 > (0 / 2)");
        }
        other => {
            panic!("Expected InvalidItem {{ item: \"array length\" }}, got: {other:?}")
        }
    }
}

/// Generates a random `ProofNode` using `rng`.
fn generate_random_proof_node(rng: &SeededRng) -> ProofNode {
    let key_len = rng.random_range(0usize..=32);
    let key = (0..key_len)
        .map(|_| PathComponent::try_new(rng.random_range(0u8..16)).unwrap())
        .collect();
    let partial_len = if key_len == 0 {
        0
    } else {
        rng.random_range(0..=key_len)
    };
    let value_digest = rng.random::<bool>().then(|| {
        let val_len = rng.random_range(0usize..=64);
        let value: Box<[u8]> = (0..val_len).map(|_| rng.random::<u8>()).collect();
        ValueDigest::Value(value)
    });
    let mut child_hashes = Children::new();
    for nibble in 0u8..16 {
        if rng.random::<bool>() {
            child_hashes[PathComponent::try_new(nibble).unwrap()] =
                Some(TrieHash::from(rng.random::<[u8; 32]>()).into_hash_type());
        }
    }
    ProofNode {
        key,
        partial_len,
        value_digest,
        child_hashes,
    }
}

/// Generates a random `FrozenRangeProof`. Callers serialize it themselves:
/// `write_to_vec` for wire bytes, `raw_header` + `write_body_to_vec` for
/// canonical bytes.
///
/// The seed used is printed to stderr by `SeededRng` so failures can be reproduced.
fn generate_random_range_proof(rng: &SeededRng) -> FrozenRangeProof {
    let start_nodes: Box<[ProofNode]> = (0..rng.random_range(0usize..=5))
        .map(|_| generate_random_proof_node(rng))
        .collect();
    let end_nodes: Box<[ProofNode]> = (0..rng.random_range(0usize..=5))
        .map(|_| generate_random_proof_node(rng))
        .collect();
    let key_values: Box<[_]> = (0..rng.random_range(0usize..=10))
        .map(|_| {
            let key_len = rng.random_range(0usize..=32);
            let key: Box<[u8]> = (0..key_len).map(|_| rng.random::<u8>()).collect();
            let val_len = rng.random_range(0usize..=32);
            let val: Box<[u8]> = (0..val_len).map(|_| rng.random::<u8>()).collect();
            (key, val)
        })
        .collect();

    FrozenRangeProof::new(Proof::new(start_nodes), Proof::new(end_nodes), key_values)
}

/// Generates a random `FrozenChangeProof`. See [`generate_random_range_proof`].
fn generate_random_change_proof(rng: &SeededRng) -> FrozenChangeProof {
    let start_nodes: Box<[ProofNode]> = (0..rng.random_range(0usize..=5))
        .map(|_| generate_random_proof_node(rng))
        .collect();
    let end_nodes: Box<[ProofNode]> = (0..rng.random_range(0usize..=5))
        .map(|_| generate_random_proof_node(rng))
        .collect();
    let batch_ops: Box<[_]> = (0..rng.random_range(0usize..=10))
        .map(|_| {
            let key_len = rng.random_range(0usize..=32);
            let key: Box<[u8]> = (0..key_len).map(|_| rng.random::<u8>()).collect();
            match rng.random_range(0u8..3) {
                0 => {
                    let val_len = rng.random_range(0usize..=32);
                    let val: Box<[u8]> = (0..val_len).map(|_| rng.random::<u8>()).collect();
                    BatchOp::Put { key, value: val }
                }
                1 => BatchOp::Delete { key },
                _ => BatchOp::DeleteRange { prefix: key },
            }
        })
        .collect();

    FrozenChangeProof::new(Proof::new(start_nodes), Proof::new(end_nodes), batch_ops)
}

/// Corrupts 1–3 random bytes of `data` in place, logging each mutation.
fn corrupt_random_bytes(rng: &SeededRng, data: &mut [u8], iteration: usize) {
    let num_corruptions = rng.random_range(1usize..=3);
    for _ in 0..num_corruptions {
        if data.is_empty() {
            break;
        }
        let pos = rng.random_range(0..data.len());
        let old = data[pos];
        let new_val = rng.random::<u8>();
        debug!("iteration {iteration}: corrupted byte {pos}: {old} -> {new_val}");
        data[pos] = new_val;
    }
}

/// Asserts that re-serializing `parsed` is stable across two round-trips.
fn assert_reserialize_idempotent(parsed: &FrozenRangeProof) {
    let mut re_bytes = Vec::new();
    parsed.write_to_vec(&mut re_bytes);
    let re_parsed =
        FrozenRangeProof::from_slice(&re_bytes).expect("re-serialized proof should parse cleanly");
    let mut re_re_bytes = Vec::new();
    re_parsed.write_to_vec(&mut re_re_bytes);
    assert_eq!(
        re_bytes, re_re_bytes,
        "re-serialized proof must be idempotent"
    );
}

#[test]
fn test_slow_range_proof_roundtrip_fuzz() {
    let rng = SeededRng::from_env_or_random();
    for i in 0..100 {
        let proof = generate_random_range_proof(&rng);
        let mut bytes = Vec::new();
        proof.write_to_vec(&mut bytes);
        debug!("iteration {i}: proof: {proof:#?}");
        debug!("iteration {i}: bytes: {}", hex::encode(&bytes));

        let parsed = FrozenRangeProof::from_slice(&bytes).expect("generated proof should be valid");
        let mut re_bytes = Vec::new();
        parsed.write_to_vec(&mut re_bytes);
        assert_eq!(bytes, re_bytes, "re-serialized bytes must match original");
    }
}

#[test]
fn test_slow_change_proof_roundtrip_fuzz() {
    let rng = SeededRng::from_env_or_random();
    for i in 0..100 {
        let proof = generate_random_change_proof(&rng);
        let mut bytes = Vec::new();
        proof.write_to_vec(&mut bytes);
        debug!("iteration {i}: proof: {proof:#?}");
        debug!("iteration {i}: bytes: {}", hex::encode(&bytes));

        let parsed =
            FrozenChangeProof::from_slice(&bytes).expect("generated proof should be valid");
        let mut re_bytes = Vec::new();
        parsed.write_to_vec(&mut re_bytes);
        assert_eq!(bytes, re_bytes, "re-serialized bytes must match original");
    }
}

#[test]
fn test_slow_malformed_proof_fuzz() {
    let rng = SeededRng::from_env_or_random();
    for i in 0..200 {
        let proof = generate_random_range_proof(&rng);
        let mut data = raw_header(ProofType::Range);
        proof.write_body_to_vec(&mut data);

        corrupt_random_bytes(&rng, &mut data, i);
        debug!("iteration {i}: corrupted bytes: {}", hex::encode(&data));

        match parse_range_canonical(&data) {
            Err(err) => {
                debug!("iteration {i}: parse error (expected): {err}");
            }
            Ok(parsed) => {
                debug!("iteration {i}: corruption produced valid proof (checking stability)");
                assert_reserialize_idempotent(&parsed);
            }
        }
    }
}

#[test]
fn test_dos_array_length_bounds() {
    use integer_encoding::VarInt;

    let (proof, mut data) = create_valid_range_proof();

    // The first array length (for start_proof) is at offset 32.
    let original_len = proof.start_proof().len();
    let original_len_space = original_len.required_space();

    let malicious_num_items: usize = 10_000_000;
    let encoded = malicious_num_items.encode_var_vec();

    data.splice(32..32 + original_len_space, encoded);

    // Calculate the remaining bytes after parsing the new varint (offset 32 + encoded varint len)
    let remainder_len = data.len() - 32 - malicious_num_items.required_space();

    match parse_range_canonical(&data) {
        Err(ReadError::InvalidItem {
            item,
            expected,
            found,
            ..
        }) => {
            assert_eq!(item, "array length");
            assert_eq!(
                expected,
                "length less than or equal to the maximum possible items in remaining bytes"
            );
            assert_eq!(
                found,
                format!("{malicious_num_items} > ({remainder_len} / 5)")
            );
        }
        other => panic!("Expected ReadError::InvalidItem for DoS vector, got: {other:?}"),
    }
}

mod box_array_deserialization_tests {
    use std::num::NonZeroUsize;

    use super::*;
    use crate::proofs::header::Header;
    use crate::proofs::reader::{ProofReader, ReadError, V0Reader, Version0};
    use integer_encoding::VarInt;

    #[derive(Debug, PartialEq)]
    struct Hash32([u8; 32]);

    impl Version0 for Hash32 {
        const MIN_BYTES_PER_ITEM: NonZeroUsize = NonZeroUsize::new(32).unwrap();

        fn read_v0_item(reader: &mut V0Reader<'_>) -> Result<Self, ReadError> {
            let chunk = reader.read_chunk::<32>()?;
            Ok(Hash32(*chunk))
        }
    }

    impl Version0 for u8 {
        fn read_v0_item(reader: &mut V0Reader<'_>) -> Result<Self, ReadError> {
            let chunk = reader.read_chunk::<1>()?;
            Ok(chunk[0])
        }
    }

    #[derive(Debug, PartialEq)]
    struct VarLenVec(Vec<u8>);

    impl Version0 for VarLenVec {
        fn read_v0_item(reader: &mut V0Reader<'_>) -> Result<Self, ReadError> {
            let len = reader.read_item::<usize>()?;
            let slice = reader.read_slice(len)?;
            Ok(VarLenVec(slice.to_vec()))
        }
    }

    fn v0_reader(data: &[u8]) -> V0Reader<'_> {
        let inner = ProofReader::new(data);
        V0Reader::new(inner, Header::from(ProofType::Range))
    }

    #[test]
    fn rejects_usize_max_items_via_length_guard() {
        // Guard catches extreme case before any iteration
        let mut data = Vec::new();
        data.extend_from_slice(&usize::MAX.encode_var_vec());
        let mut reader = v0_reader(&data);
        let result: Result<Box<[Hash32]>, _> = reader.read_v0_item();

        assert!(matches!(
            result,
            Err(ReadError::InvalidItem {
                item: "array length",
                ..
            })
        ));
    }

    #[test]
    fn rejects_eof_mid_array() {
        // Guard doesn't trigger (5 > 64 is FALSE), but loop fails safely on EOF
        let mut data = Vec::new();
        data.extend_from_slice(&5usize.encode_var_vec());
        data.extend_from_slice(&[0u8; 64]); // only 2 complete Hash32 items
        let mut reader = v0_reader(&data);
        let result: Result<Box<[Hash32]>, _> = reader.read_v0_item();

        assert!(result.is_err());
    }

    #[test]
    fn rejects_var_len_item_with_huge_claimed_length() {
        // Nested length field amplification
        let mut data = Vec::new();
        data.extend_from_slice(&1usize.encode_var_vec());
        data.extend_from_slice(&usize::MAX.encode_var_vec());
        data.extend_from_slice(&[0u8; 10]);
        let mut reader = v0_reader(&data);
        let result: Result<Box<[VarLenVec]>, _> = reader.read_v0_item();

        assert!(result.is_err());
    }

    #[test]
    fn accepts_empty_array() {
        let mut data = Vec::new();
        data.extend_from_slice(&0usize.encode_var_vec());
        let mut reader = v0_reader(&data);
        let result: Result<Box<[Hash32]>, _> = reader.read_v0_item();

        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn boundary_exact_fit_passes() {
        // 10 items, 50 bytes → 10 > 50 / 5 is FALSE → PASS
        // T = u8 so num_items directly equals bytes needed
        let mut data = Vec::new();
        data.extend_from_slice(&10usize.encode_var_vec());
        data.extend_from_slice(&[0u8; 50]);
        let mut reader = v0_reader(&data);
        let result: Result<Box<[u8]>, _> = reader.read_v0_item();

        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 10);
    }

    #[test]
    fn boundary_one_over_fails() {
        // 51 items, 50 bytes → 51 > 50 / 1 is TRUE → FAIL
        // T = u8 so MIN_BYTES_PER_ITEM = 1
        let mut data = Vec::new();
        data.extend_from_slice(&51usize.encode_var_vec());
        data.extend_from_slice(&[0u8; 50]);
        let mut reader = v0_reader(&data);
        let result: Result<Box<[u8]>, _> = reader.read_v0_item();

        assert!(matches!(
            result,
            Err(ReadError::InvalidItem {
                item: "array length",
                ..
            })
        ));
    }
}

// =============================================================================
// Compressed-frame tests (the wire layer added by `proofs::frame`)
// =============================================================================

fn range_wire() -> Vec<u8> {
    let (proof, _) = create_valid_range_proof();
    let mut wire = Vec::new();
    proof.write_to_vec(&mut wire);
    wire
}

#[test]
fn test_frame_wire_is_compressed_and_versioned() {
    // A compressible proof: many repetitive key-value pairs, so the zstd
    // frame must come out genuinely smaller than the canonical body (a
    // stored-uncompressed frame would fail the size assertion below).
    let kvs: Box<[_]> = (0u16..512)
        .map(|i| {
            (
                Box::from(format!("key-{i:05}").as_bytes()),
                Box::from(b"value-value-value-value".as_slice()),
            )
        })
        .collect();
    let proof = FrozenRangeProof::new(empty_nodes(), empty_nodes(), kvs);
    let mut canonical_body = Vec::new();
    proof.write_body_to_vec(&mut canonical_body);
    let mut wire = Vec::new();
    proof.write_to_vec(&mut wire);

    assert_eq!(&wire[..8], magic::PROOF_HEADER);
    assert_eq!(wire[8], 0, "compressed wire keeps version byte 0");
    // A zstd frame (magic 0xFD2FB528, little-endian) follows the header.
    assert_eq!(
        &wire[32..36],
        &[0x28, 0xB5, 0x2F, 0xFD],
        "zstd frame magic must follow the header"
    );
    // The wire is actually compressed, not stored.
    assert!(
        wire.len() < canonical_body.len() + 32,
        "wire ({} bytes) must be smaller than header + canonical body ({} bytes)",
        wire.len(),
        canonical_body.len() + 32
    );
}

#[test]
fn test_frame_rejects_over_cap_content_size() {
    // A frame whose header declares a content size just over the cap: the
    // decoder must reject it before allocating.
    let body = vec![0u8; super::frame::MAX_DECOMPRESSED_LEN + 1];
    let mut wire = raw_header(ProofType::Range);
    super::frame::write_compressed_body(&body, &mut wire, ProofType::Range);
    let err = FrozenRangeProof::from_slice(&wire).expect_err("over-cap content size");
    assert!(
        matches!(
            err,
            ReadError::InvalidItem { item, expected, .. }
                if item == "compressed body length" && expected.contains("MAX_DECOMPRESSED_LEN")
        ),
        "got {err:?}"
    );
}

#[test]
fn test_frame_rejects_trailing_bytes_after_frame() {
    let mut wire = range_wire();
    wire.push(0xAB);
    let err = FrozenRangeProof::from_slice(&wire).expect_err("trailing bytes");
    assert!(
        matches!(err, ReadError::InvalidItem { item, .. } if item == "compressed body frame"),
        "got {err:?}"
    );
}

#[test]
fn test_frame_rejects_corrupt_frame_magic() {
    // Flip the first byte of the zstd frame magic. Deeper *content*
    // corruption is not deterministically detectable at this layer — the
    // producer rule pins "no checksum", so a flipped literal byte can
    // decode to a different-but-still-parseable body; tampering is caught
    // by proof hash verification, not by the frame. Content flips are
    // exercised probabilistically by test_slow_malformed_wire_fuzz.
    let mut wire = range_wire();
    wire[32] ^= 0xFF;
    let err = FrozenRangeProof::from_slice(&wire).expect_err("corrupt frame magic");
    assert!(
        matches!(err, ReadError::InvalidItem { item, .. } if item == "compressed body frame"),
        "got {err:?}"
    );
}

#[test]
fn test_frame_rejects_truncated_frame() {
    let mut wire = range_wire();
    wire.pop();
    let err = FrozenRangeProof::from_slice(&wire).expect_err("truncated frame");
    assert!(
        matches!(err, ReadError::InvalidItem { item, .. } if item == "compressed body frame"),
        "got {err:?}"
    );
}

#[test]
fn test_frame_rejects_missing_frame() {
    // Nothing after the header: the frame is missing.
    let wire = range_wire()[..32].to_vec();
    let err = FrozenRangeProof::from_slice(&wire).expect_err("missing frame");
    assert!(
        matches!(err, ReadError::IncompleteItem { item, .. } if item == "compressed body frame"),
        "got {err:?}"
    );
}

#[test]
fn test_frame_rejects_frame_without_content_size() {
    // The producer's bulk compressor always records the content size in
    // the frame header, but zstd streaming encoders (which don't know the
    // input size up front) omit it. An attacker can hand-craft such a
    // frame; the decoder must reject it before allocating.
    let (_, canonical) = create_valid_range_proof();
    let body = &canonical[32..];
    let frame = zstd::stream::encode_all(body, super::frame::ZSTD_LEVEL)
        .expect("stream compression of an in-memory buffer");
    let mut wire = canonical[..32].to_vec();
    wire.extend_from_slice(&frame);
    let err = FrozenRangeProof::from_slice(&wire).expect_err("frame without content size");
    assert!(
        matches!(
            err,
            ReadError::InvalidItem { item, ref found, .. }
                if item == "compressed body length" && found.contains("no content size")
        ),
        "got {err:?}"
    );
}

/// Appends `extra` after the zstd frame of a valid range-proof wire.
fn wire_with_trailing(extra: &[u8]) -> Vec<u8> {
    let mut wire = range_wire();
    wire.extend_from_slice(extra);
    wire
}

#[test]
fn test_frame_rejects_trailing_skippable_frame() {
    // A zstd skippable frame (magic 0x184D2A50–5F, little-endian) with a
    // 4-byte payload. `ZSTD_decompressDCtx` silently skips these, so only
    // the exact-frame-size check rejects it.
    let mut skippable = vec![0x50, 0x2A, 0x4D, 0x18];
    skippable.extend_from_slice(&4u32.to_le_bytes());
    skippable.extend_from_slice(&[0xAB; 4]);
    let wire = wire_with_trailing(&skippable);
    let err = FrozenRangeProof::from_slice(&wire).expect_err("trailing skippable frame");
    assert!(
        matches!(err, ReadError::InvalidItem { item, .. } if item == "compressed body frame"),
        "got {err:?}"
    );
}

#[test]
fn test_frame_rejects_trailing_concatenated_frame() {
    // A second, fully valid zstd frame (compressing zero bytes) appended
    // after the body frame. `ZSTD_decompressDCtx` decompresses
    // concatenated frames, so only the exact-frame-size check rejects it.
    let empty_frame =
        zstd::bulk::compress(&[], super::frame::ZSTD_LEVEL).expect("compress empty buffer");
    let wire = wire_with_trailing(&empty_frame);
    let err = FrozenRangeProof::from_slice(&wire).expect_err("trailing concatenated frame");
    assert!(
        matches!(err, ReadError::InvalidItem { item, .. } if item == "compressed body frame"),
        "got {err:?}"
    );
}

#[test]
fn test_frame_rejects_excessive_compression_ratio() {
    // A 4 MiB all-zero body compresses to a few hundred bytes: under the
    // length cap and consistent with the frame header's content size, but
    // far over MAX_COMPRESSION_RATIO. The decoder must reject it before
    // allocating the 4 MiB.
    let body = vec![0u8; 4 * 1024 * 1024];
    let mut wire = raw_header(ProofType::Range);
    super::frame::write_compressed_body(&body, &mut wire, ProofType::Range);
    let err = FrozenRangeProof::from_slice(&wire).expect_err("compression bomb");
    assert!(
        matches!(
            err,
            ReadError::InvalidItem { item, expected, .. }
                if item == "compressed body length" && expected.contains("MAX_COMPRESSION_RATIO")
        ),
        "got {err:?}"
    );
}

#[test]
fn test_frame_accepts_high_but_legal_ratio() {
    // A ~100x compression ratio: far above what honest proofs reach
    // (~2.3x) but still under MAX_COMPRESSION_RATIO — the decoder must
    // accept it. A 1 KiB random block repeated 100x compresses to roughly
    // one block plus match commands.
    let rng = SeededRng::from_env_or_random();
    let block: Vec<u8> = (0..1024).map(|_| rng.random::<u8>()).collect();
    let body = block.repeat(100);
    let mut frame = Vec::new();
    super::frame::write_compressed_body(&body, &mut frame, ProofType::Range);
    assert!(
        body.len() > 32 * frame.len(),
        "test premise: ratio must be well above honest proofs (frame is {} bytes)",
        frame.len()
    );
    let decoded = super::frame::decompress_body(&frame, 0, ProofType::Range)
        .expect("ratio under MAX_COMPRESSION_RATIO must decode");
    assert_eq!(decoded, body);
}

#[test]
fn test_frame_accepts_max_decompressed_len_exactly() {
    // A body of exactly MAX_DECOMPRESSED_LEN must pass the length cap
    // (the bound is inclusive). A random 1 MiB block repeated 32x keeps
    // the compression ratio ~32x, safely inside MAX_COMPRESSION_RATIO.
    let rng = SeededRng::from_env_or_random();
    let block: Vec<u8> = (0..1024 * 1024).map(|_| rng.random::<u8>()).collect();
    let body = block.repeat(super::frame::MAX_DECOMPRESSED_LEN / block.len());
    assert_eq!(body.len(), super::frame::MAX_DECOMPRESSED_LEN);
    let mut frame = Vec::new();
    super::frame::write_compressed_body(&body, &mut frame, ProofType::Range);
    let decoded = super::frame::decompress_body(&frame, 0, ProofType::Range)
        .expect("body exactly at MAX_DECOMPRESSED_LEN must decode");
    assert_eq!(decoded, body);
}

#[test]
fn test_frame_change_proof_rejects_trailing_bytes() {
    // The frame layer is shared between proof types; pin the change-proof
    // wire path too.
    let (proof, _) = create_valid_change_proof();
    let mut wire = Vec::new();
    proof.write_to_vec(&mut wire);
    wire.push(0xAB);
    let err = FrozenChangeProof::from_slice(&wire).expect_err("trailing bytes");
    assert!(
        matches!(err, ReadError::InvalidItem { item, .. } if item == "compressed body frame"),
        "got {err:?}"
    );
}

/// The compressed wire tail (the zstd frame) of the proof built by
/// [`golden_proof`], captured from the encoder when the compressed format
/// shipped (zstd 1.5.7).
///
/// zstd's *encoder output* is not bit-stable across library versions, so the
/// encoder is free to produce different bytes than these — but the zstd
/// *format* is stable, so this exact byte string must continue to **decode**
/// forever. This pins the wire framing (single frame, frame content size)
/// against accidental changes; the canonical-body snapshot tests pin the
/// body encoding.
const GOLDEN_WIRE_TAIL: &str =
    "28b52ffd201bd90000000002046b6579310676616c756531046b6579320676616c756532";

fn golden_proof() -> FrozenRangeProof {
    FrozenRangeProof::new(
        empty_nodes(),
        empty_nodes(),
        Box::new([
            (
                Box::from(b"key1".as_slice()),
                Box::from(b"value1".as_slice()),
            ),
            (
                Box::from(b"key2".as_slice()),
                Box::from(b"value2".as_slice()),
            ),
        ]),
    )
}

#[test]
fn test_frame_golden_vector_decodes() {
    // The golden body is key-value-only, so its bytes are identical under
    // both hash modes; raw_header supplies the mode-correct header.
    let mut wire = raw_header(ProofType::Range);
    wire.extend_from_slice(&hex::decode(GOLDEN_WIRE_TAIL).expect("valid hex"));
    let parsed = FrozenRangeProof::from_slice(&wire)
        .expect("checked-in compressed golden vector must always decode");
    assert_eq!(parsed, golden_proof());
}

#[test]
fn test_slow_malformed_wire_fuzz() {
    // Complements test_slow_malformed_proof_fuzz (which corrupts the
    // canonical body and re-frames it): mutating the wire bytes directly
    // exercises the frame layer itself — the header, the zstd frame
    // header, and the frame payload.
    let rng = SeededRng::from_env_or_random();
    for i in 0..200 {
        let proof = generate_random_range_proof(&rng);
        let mut data = Vec::new();
        proof.write_to_vec(&mut data);

        corrupt_random_bytes(&rng, &mut data, i);
        debug!("iteration {i}: corrupted wire: {}", hex::encode(&data));

        match FrozenRangeProof::from_slice(&data) {
            Err(err) => {
                debug!("iteration {i}: parse error (expected): {err}");
            }
            Ok(parsed) => {
                debug!("iteration {i}: corruption produced valid proof (checking stability)");
                assert_reserialize_idempotent(&parsed);
            }
        }
    }
}
