// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::PathComponentSliceExt;
use firewood_storage::Preimage;
use firewood_storage::logger::debug;

use super::*;

#[test]
fn range_proof_invalid_bounds() {
    let merkle = create_in_memory_merkle().hash();

    let start_key = &[0x01];
    let end_key = &[0x00];

    match merkle.range_proof(Some(start_key), Some(end_key), NonZeroUsize::new(1)) {
        Err(api::Error::InvalidRange {
            start_key: first_key,
            end_key: last_key,
        }) if *first_key == *start_key && *last_key == *end_key => (),
        Err(api::Error::InvalidRange { .. }) => panic!("wrong bounds on InvalidRange error"),
        _ => panic!("expected InvalidRange error"),
    }
}

#[test]
fn full_range_proof() {
    let merkle = init_merkle((u8::MIN..=u8::MAX).map(|k| ([k], [k])));

    let rangeproof = merkle.range_proof(None, None, None).unwrap();
    assert_eq!(rangeproof.key_values().len(), u8::MAX as usize + 1);
    assert_eq!(rangeproof.start_proof(), &FrozenProof::default());
    assert_eq!(rangeproof.end_proof(), &FrozenProof::default());

    let rangeproof = roundtrip_range_proof(&rangeproof);
    assert_eq!(rangeproof.key_values().len(), u8::MAX as usize + 1);
    assert_eq!(rangeproof.start_proof(), &FrozenProof::default());
    assert_eq!(rangeproof.end_proof(), &FrozenProof::default());
}

#[test]
fn single_value_range_proof() {
    const RANDOM_KEY: u8 = 42;

    let merkle = init_merkle((u8::MIN..=u8::MAX).map(|k| ([k], [k])));

    let rangeproof = merkle
        .range_proof(Some(&[RANDOM_KEY]), None, NonZeroUsize::new(1))
        .unwrap();
    assert_eq!(rangeproof.start_proof(), rangeproof.end_proof());
    assert_eq!(rangeproof.key_values().len(), 1);

    let rangeproof = roundtrip_range_proof(&rangeproof);
    assert_eq!(rangeproof.start_proof(), rangeproof.end_proof());
    assert_eq!(rangeproof.key_values().len(), 1);
}

#[test]
fn shared_path_proof() {
    let key1 = b"key1";
    let value1 = b"1";

    let key2 = b"key2";
    let value2 = b"2";

    let merkle = init_merkle([(key1, value1), (key2, value2)]);

    let root_hash = merkle.nodestore().root_hash().unwrap();

    let key = key1;
    let proof = merkle.prove(key).unwrap();
    proof.verify(key, Some(value1), &root_hash).unwrap();

    let key = key2;
    let proof = merkle.prove(key).unwrap();
    proof.verify(key, Some(value2), &root_hash).unwrap();
}

#[test]
fn single_key_proof_with_one_node() {
    let key = b"key";
    let value = b"value";

    let merkle = init_merkle([(key, value)]);

    let root_hash = merkle.nodestore().root_hash().unwrap();

    let proof = merkle.prove(key).unwrap();
    proof.verify(key, Some(value), &root_hash).unwrap();
}

#[test]
fn two_key_proof_without_shared_path() {
    let key1 = &[0x00];
    let key2 = &[0xff];

    let merkle = init_merkle([(key1, key1), (key2, key2)]);

    let root_hash = merkle.nodestore().root_hash().unwrap();

    let proof = merkle.prove(key1).unwrap();
    proof.verify(key1, Some(key1), &root_hash).unwrap();

    let proof = merkle.prove(key2).unwrap();
    proof.verify(key2, Some(key2), &root_hash).unwrap();
}

#[test]
fn test_empty_tree_proof() {
    let items: Vec<(&str, &str)> = Vec::new();
    let merkle = init_merkle(items);
    let key = "x".as_ref();

    let proof_err = merkle.prove(key).unwrap_err();
    assert!(matches!(proof_err, ProofError::Empty), "{proof_err:?}");
}

#[test]
fn test_proof() {
    let rng = firewood_storage::SeededRng::from_env_or_random();
    let set = fixed_and_pseudorandom_data(&rng, 500);
    let mut items = set.iter().collect::<Vec<_>>();
    items.sort_unstable();
    let merkle = init_merkle(items.clone());

    let root_hash = merkle.nodestore().root_hash().unwrap();

    for (key, val) in items {
        let proof = merkle.prove(key).unwrap();
        assert!(!proof.is_empty());
        proof.verify(key, Some(val), &root_hash).unwrap();
    }
}

#[test]
fn test_proof_end_with_leaf() {
    let merkle = init_merkle([
        ("do", "verb"),
        ("doe", "reindeer"),
        ("dog", "puppy"),
        ("doge", "coin"),
        ("horse", "stallion"),
        ("ddd", "ok"),
    ]);
    let root_hash = merkle.nodestore().root_hash().unwrap();

    let key = b"doe";

    let proof = merkle.prove(key).unwrap();
    assert!(!proof.is_empty());

    proof.verify(key, Some(b"reindeer"), &root_hash).unwrap();
}

#[test]
fn test_proof_end_with_branch() {
    let items = [
        ("d", "verb"),
        ("do", "verb"),
        ("doe", "reindeer"),
        ("e", "coin"),
    ];
    let merkle = init_merkle(items);
    let root_hash = merkle.nodestore().root_hash().unwrap();

    let key = b"d";

    let proof = merkle.prove(key).unwrap();
    assert!(!proof.is_empty());

    proof.verify(key, Some(b"verb"), &root_hash).unwrap();
}

#[test]
fn test_bad_proof() {
    let rng = firewood_storage::SeededRng::from_env_or_random();
    let set = fixed_and_pseudorandom_data(&rng, 800);
    let mut items = set.iter().collect::<Vec<_>>();
    items.sort_unstable();
    let merkle = init_merkle(items.clone());
    let root_hash = merkle.nodestore().root_hash().unwrap();

    for (key, value) in items {
        let proof = merkle.prove(key).unwrap();
        assert!(!proof.is_empty());

        // Delete an entry from the generated proofs.
        let mut new_proof = proof.into_mutable();
        new_proof.pop();

        // TODO: verify error result matches expected error
        assert!(new_proof.verify(key, Some(value), &root_hash).is_err());
    }
}

#[test]
fn exclusion_with_proof_value_present() {
    // Build a trie where an ancestor on the path has a value
    let mut merkle = crate::merkle::tests::create_in_memory_merkle();
    // Parent has a value
    merkle.insert(&[0u8], Box::from([0u8])).unwrap();
    // Child under the same branch
    merkle.insert(&[0u8, 1u8], Box::from([1u8])).unwrap();

    let merkle = merkle.hash();
    let root_hash = merkle.nodestore.root_hash().unwrap();
    debug!("{}", merkle.dump_to_string().unwrap());
    debug!("root_hash: {root_hash:?}");

    // Non-existent key under the same parent branch
    let missing = [0u8, 2u8];
    let proof = merkle.prove(&missing).unwrap();

    debug!("{proof:#?}");
    assert_eq!(
        proof.as_ref().first().unwrap().to_hash(),
        root_hash.clone().into_hash_type()
    );

    // Ensure at least one node in the proof carries a value (proof value present)
    assert!(proof.as_ref().iter().any(|n| n.value_digest.is_some()));

    // Exclusion should verify with expected None even if proof includes node values
    proof
        .verify(missing, Option::<&[u8]>::None, &root_hash)
        .unwrap();
}

#[test]
fn proof_path_construction_and_corruption() {
    use crate::{Proof, ProofNode};

    // Build a trie with several entries
    let mut merkle = crate::merkle::tests::create_in_memory_merkle();
    merkle.insert(b"a", Box::from(b"1".as_slice())).unwrap();
    merkle.insert(b"ab", Box::from(b"2".as_slice())).unwrap();
    merkle.insert(b"abc", Box::from(b"3".as_slice())).unwrap();
    merkle.insert(b"abd", Box::from(b"4".as_slice())).unwrap();

    let merkle = merkle.hash();
    let root_hash = merkle.nodestore.root_hash().unwrap();

    // Inclusion proof for an existing key
    let key = b"abc";
    let val = b"3";
    let proof = merkle.prove(key).unwrap();
    debug!("proof: {proof:#?}");
    debug!("root_hash: {root_hash:?}");
    debug!("{}", merkle.dump_to_string().unwrap());

    // Positive: check path monotonicity (each node key is a prefix of the next)
    let nodes = proof.as_ref();
    for w in nodes.windows(2) {
        let cur = w[0].key.as_ref();
        let nxt = w[1].key.as_ref();
        assert!(nxt.starts_with(cur), "proof path not prefix-ordered");
    }

    // Sanity: proof verifies
    proof.verify(key, Some(val.as_slice()), &root_hash).unwrap();

    // Negative: corrupt the path by clearing children of the first node
    let mut corrupt: Proof<Vec<ProofNode>> = proof.clone().into_mutable();
    if let Some(first) = (*corrupt).first_mut() {
        // Set all child hashes to empty so traversal fails
        first.child_hashes = Children::new();
    }
    let corrupt = corrupt.into_immutable();
    let err = corrupt
        .verify(key, Some(val.as_slice()), &root_hash)
        .unwrap_err();
    // Node traversal should fail
    assert!(matches!(
        err,
        crate::ProofError::NodeNotInTrie | crate::ProofError::UnexpectedHash
    ));
}

#[test]
fn range_proof_serialization_roundtrip() {
    let merkle = init_merkle((u8::MIN..=u8::MAX).map(|k| ([k], [k])));

    let start_key = &[42u8];
    let end_key = &[84u8];

    let rangeproof = merkle
        .range_proof(Some(start_key), Some(end_key), NonZeroUsize::new(10))
        .unwrap();

    drop(roundtrip_range_proof(&rangeproof));
}

fn roundtrip_range_proof(proof: &FrozenRangeProof) -> FrozenRangeProof {
    let mut serialized = Vec::new();
    proof.write_to_vec(&mut serialized);
    let deserialized = FrozenRangeProof::from_slice(&serialized).unwrap();
    assert_eq!(proof, &deserialized);
    deserialized
}

// ── Exclusion proof divergent child tests ────────────────────────────────

/// When a key doesn't exist and the proof path diverges at a branch,
/// `prove()` must include the divergent child node so `value_digest`
/// can confirm the key's absence.
#[test]
fn test_exclusion_proof_includes_divergent_child() {
    // \x10\x58 only — proving non-existent \x10\x50 diverges at nibble 3
    // (0 vs 8) and should include the divergent child.
    let merkle = init_merkle([(b"\x10\x58".as_ref(), b"b".as_ref())]);
    let root_hash = firewood_storage::HashedNodeReader::root_hash(merkle.nodestore()).unwrap();
    let proof = merkle.prove(b"\x10\x50").unwrap();
    let nodes = proof.as_ref();

    // The last node's key must diverge from the target — it's the
    // divergent child, not just an ancestor.
    let last = nodes.last().unwrap();
    let last_key_nibbles: Vec<u8> = last.key.as_byte_slice().to_vec();
    let target_nibbles: Vec<u8> = firewood_storage::NibblesIterator::new(b"\x10\x50").collect();
    assert_ne!(
        last_key_nibbles, target_nibbles,
        "should be exclusion proof"
    );
    assert!(
        last_key_nibbles.len() > target_nibbles.len()
            || last_key_nibbles
                .iter()
                .zip(target_nibbles.iter())
                .any(|(a, b)| a != b),
        "last node should be the divergent child, not an ancestor"
    );

    // Must validate as a valid exclusion proof.
    assert!(
        proof
            .value_digest(b"\x10\x50", &root_hash)
            .unwrap()
            .is_none(),
        "should be a valid exclusion proof"
    );
}

/// When the child at the next nibble doesn't exist (None), the exclusion
/// proof should NOT append a divergent child. The proof should still be
/// a valid exclusion proof.
#[test]
fn test_exclusion_proof_no_child_at_next_nibble() {
    // Keys at \x20 and \x30. Proving \x10: the root has children at
    // nibbles 2 and 3, but NO child at nibble 1.
    let merkle = init_merkle([(b"\x20".as_ref(), b"a".as_ref()), (b"\x30", b"b")]);
    let root_hash = firewood_storage::HashedNodeReader::root_hash(merkle.nodestore()).unwrap();
    let proof = merkle.prove(b"\x10").unwrap();

    assert!(
        proof.value_digest(b"\x10", &root_hash).unwrap().is_none(),
        "should be a valid exclusion proof without divergent child"
    );
}

/// An incomplete exclusion proof (missing the divergent child when one
/// exists) must be rejected by `value_digest` with `ExclusionProofMissingChild`.
#[test]
fn test_crafted_incomplete_exclusion_proof_rejected() {
    // \x10\x58 and \x30\x00 — proving deleted \x10\x50 should include
    // the divergent child at \x10\x58.
    let merkle = init_merkle([(b"\x10\x58".as_ref(), b"b".as_ref()), (b"\x30\x00", b"c")]);
    let root_hash = firewood_storage::HashedNodeReader::root_hash(merkle.nodestore()).unwrap();
    let full_proof = merkle.prove(b"\x10\x50").unwrap();

    let nodes = full_proof.as_ref();
    assert!(
        nodes.len() >= 2,
        "expected at least 2 nodes (ancestor + divergent child), got {}",
        nodes.len()
    );

    // Strip the last node (the divergent child).
    let truncated: Vec<crate::ProofNode> = nodes[..nodes.len() - 1].to_vec();
    let incomplete = crate::Proof::new(truncated.into_boxed_slice());

    let err = incomplete
        .value_digest(b"\x10\x50", &root_hash)
        .unwrap_err();
    assert!(
        matches!(err, crate::ProofError::ExclusionProofMissingChild),
        "expected ExclusionProofMissingChild, got {err:?}"
    );
}

/// A proof for key B must NOT validate as an exclusion proof for unrelated
/// key C. The proof path traces toward B (nibble 3), not toward C (nibble 5)
/// — it has no information about C's subtree.
#[test]
fn test_value_digest_rejects_wrong_key_proof() {
    let keys: [[u8; 32]; 3] = {
        let mut arr = [[0u8; 32]; 3];
        arr[0][0] = 0x10; // A
        arr[1][0] = 0x30; // B
        arr[2][0] = 0x50; // C
        arr
    };

    let merkle = init_merkle([
        (keys[0], [0xAAu8; 20]),
        (keys[1], [0xBBu8; 20]),
        (keys[2], [0xCCu8; 20]),
    ]);
    let root_hash = firewood_storage::HashedNodeReader::root_hash(merkle.nodestore()).unwrap();

    let proof_for_b = merkle.prove(&keys[1]).unwrap();

    // Inclusion proof for B should work.
    proof_for_b
        .verify(keys[1], Some(&[0xBBu8; 20]), &root_hash)
        .expect("proof for B should verify B");

    // The same proof queried for C should be rejected.
    let result = proof_for_b.value_digest(keys[2], &root_hash);
    assert!(
        result.is_err(),
        "proof for B should NOT be accepted when queried for C, \
         but value_digest returned {result:?}"
    );
}
