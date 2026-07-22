// Copyright (C) 2025, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

use firewood_storage::logger::debug;

use super::*;

#[firewood_macros::hash_mode]
fn range_proof_invalid_bounds<H: HashMode>() {
    let merkle = create_in_memory_merkle::<H>().hash();

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

#[firewood_macros::hash_mode]
fn full_range_proof<H: HashMode>() {
    const FULL_KEY_RANGE_LEN: usize = 256; // u8::MAX + 1 keys

    let merkle = init_merkle::<H, _, _, _>((u8::MIN..=u8::MAX).map(|k| ([k], [k])));

    let rangeproof = merkle.range_proof(None, None, None).unwrap();
    assert_eq!(rangeproof.key_values().len(), FULL_KEY_RANGE_LEN);
    assert_eq!(rangeproof.start_proof(), &FrozenProof::default());
    assert_eq!(rangeproof.end_proof(), &FrozenProof::default());

    let rangeproof = roundtrip_range_proof(&rangeproof);
    assert_eq!(rangeproof.key_values().len(), FULL_KEY_RANGE_LEN);
    assert_eq!(rangeproof.start_proof(), &FrozenProof::default());
    assert_eq!(rangeproof.end_proof(), &FrozenProof::default());
}

#[firewood_macros::hash_mode]
fn single_value_range_proof<H: HashMode>() {
    const RANDOM_KEY: u8 = 42;

    let merkle = init_merkle::<H, _, _, _>((u8::MIN..=u8::MAX).map(|k| ([k], [k])));

    let rangeproof = merkle
        .range_proof(Some(&[RANDOM_KEY]), None, NonZeroUsize::new(1))
        .unwrap();
    assert_eq!(rangeproof.start_proof(), rangeproof.end_proof());
    assert_eq!(rangeproof.key_values().len(), 1);

    let rangeproof = roundtrip_range_proof(&rangeproof);
    assert_eq!(rangeproof.start_proof(), rangeproof.end_proof());
    assert_eq!(rangeproof.key_values().len(), 1);
}

#[firewood_macros::hash_mode]
fn shared_path_proof<H: HashMode>() {
    let key1 = b"key1";
    let value1 = b"1";

    let key2 = b"key2";
    let value2 = b"2";

    let merkle = init_merkle::<H, _, _, _>([(key1, value1), (key2, value2)]);

    let root_hash = merkle.nodestore().root_hash().unwrap();

    let key = key1;
    let proof = merkle.prove(key).unwrap();
    proof
        .verify(key, Some(value1), &root_hash, H::ALGORITHM)
        .unwrap();

    let key = key2;
    let proof = merkle.prove(key).unwrap();
    proof
        .verify(key, Some(value2), &root_hash, H::ALGORITHM)
        .unwrap();
}

#[firewood_macros::hash_mode]
fn single_key_proof_with_one_node<H: HashMode>() {
    let key = b"key";
    let value = b"value";

    let merkle = init_merkle::<H, _, _, _>([(key, value)]);

    let root_hash = merkle.nodestore().root_hash().unwrap();

    let proof = merkle.prove(key).unwrap();
    proof
        .verify(key, Some(value), &root_hash, H::ALGORITHM)
        .unwrap();
}

#[firewood_macros::hash_mode]
fn two_key_proof_without_shared_path<H: HashMode>() {
    let key1 = &[0x00];
    let key2 = &[0xff];

    let merkle = init_merkle::<H, _, _, _>([(key1, key1), (key2, key2)]);

    let root_hash = merkle.nodestore().root_hash().unwrap();

    let proof = merkle.prove(key1).unwrap();
    proof
        .verify(key1, Some(key1), &root_hash, H::ALGORITHM)
        .unwrap();

    let proof = merkle.prove(key2).unwrap();
    proof
        .verify(key2, Some(key2), &root_hash, H::ALGORITHM)
        .unwrap();
}

#[firewood_macros::hash_mode]
fn test_empty_tree_proof<H: HashMode>() {
    let items: Vec<(&str, &str)> = Vec::new();
    let merkle = init_merkle::<H, _, _, _>(items);
    let key = "x".as_ref();

    let proof_err = merkle.prove(key).unwrap_err();
    assert!(matches!(proof_err, ProofError::Empty), "{proof_err:?}");
}

#[firewood_macros::hash_mode]
fn test_proof<H: HashMode>() {
    let rng = firewood_storage::SeededRng::from_env_or_random();
    let set = fixed_and_pseudorandom_data(&rng, 500);
    let mut items = set.iter().collect::<Vec<_>>();
    items.sort_unstable();
    let merkle = init_merkle::<H, _, _, _>(items.clone());

    let root_hash = merkle.nodestore().root_hash().unwrap();

    for (key, _val) in items {
        let proof = merkle.prove(key).unwrap();
        assert!(!proof.is_empty());
        // Read the stored value rather than using the original input because
        // ethhash mode rewrites the storageRoot field in account-depth values
        // during hashing. The proof must be verified against what was actually
        // committed, not what was originally inserted.
        let stored_val = merkle.get_value(key).unwrap().expect("key should exist");
        proof
            .verify(key, Some(&stored_val), &root_hash, H::ALGORITHM)
            .unwrap();
    }
}

#[firewood_macros::hash_mode]
fn test_proof_end_with_leaf<H: HashMode>() {
    let merkle = init_merkle::<H, _, _, _>([
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

    proof
        .verify(key, Some(b"reindeer"), &root_hash, H::ALGORITHM)
        .unwrap();
}

#[firewood_macros::hash_mode]
fn test_proof_end_with_branch<H: HashMode>() {
    let items = [
        ("d", "verb"),
        ("do", "verb"),
        ("doe", "reindeer"),
        ("e", "coin"),
    ];
    let merkle = init_merkle::<H, _, _, _>(items);
    let root_hash = merkle.nodestore().root_hash().unwrap();

    let key = b"d";

    let proof = merkle.prove(key).unwrap();
    assert!(!proof.is_empty());

    proof
        .verify(key, Some(b"verb"), &root_hash, H::ALGORITHM)
        .unwrap();
}

#[firewood_macros::hash_mode]
fn test_bad_proof<H: HashMode>() {
    let rng = firewood_storage::SeededRng::from_env_or_random();
    let set = fixed_and_pseudorandom_data(&rng, 800);
    let mut items = set.iter().collect::<Vec<_>>();
    items.sort_unstable();
    let merkle = init_merkle::<H, _, _, _>(items.clone());
    let root_hash = merkle.nodestore().root_hash().unwrap();

    for (key, value) in items {
        let proof = merkle.prove(key).unwrap();
        assert!(!proof.is_empty());

        // Delete an entry from the generated proofs.
        let mut new_proof = proof.into_mutable();
        new_proof.pop();

        // TODO(demosdemon): verify error result matches expected error
        assert!(
            new_proof
                .verify(key, Some(value), &root_hash, H::ALGORITHM)
                .is_err()
        );
    }
}

#[firewood_macros::hash_mode]
fn exclusion_with_proof_value_present<H: HashMode>() {
    // Build a trie where an ancestor on the path has a value
    let mut merkle = crate::merkle::tests::create_in_memory_merkle::<H>();
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
    // `Preimage::to_hash` (the inherent `.to_hash()` method) always hashes
    // under the compile-time `DefaultHashMode`, not the mode this trie was
    // actually built with — it hasn't been threaded over `H` yet (see the
    // `Preimage` doc comment in `firewood-storage::hashednode`). Call the
    // mode-aware `HashMode::to_hash` directly so this assertion holds under
    // both wrappers regardless of which mode the binary defaults to.
    assert_eq!(
        H::to_hash(proof.as_ref().first().unwrap()),
        root_hash.clone().into_hash_type()
    );

    // Ensure at least one node in the proof carries a value (proof value present)
    assert!(proof.as_ref().iter().any(|n| n.value_digest.is_some()));

    // Exclusion should verify with expected None even if proof includes node values
    proof
        .verify(missing, Option::<&[u8]>::None, &root_hash, H::ALGORITHM)
        .unwrap();
}

#[firewood_macros::hash_mode]
fn proof_path_construction_and_corruption<H: HashMode>() {
    use crate::{Proof, ProofNode};

    // Build a trie with several entries
    let mut merkle = crate::merkle::tests::create_in_memory_merkle::<H>();
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
    #[expect(clippy::disallowed_methods, reason = "window size is the literal 2")]
    for w in nodes.windows(2) {
        let cur = w[0].key.as_ref();
        let nxt = w[1].key.as_ref();
        assert!(nxt.starts_with(cur), "proof path not prefix-ordered");
    }

    // Sanity: proof verifies
    proof
        .verify(key, Some(val.as_slice()), &root_hash, H::ALGORITHM)
        .unwrap();

    // Negative: corrupt the path by clearing children of the first node
    let mut corrupt: Proof<Vec<ProofNode>> = proof.clone().into_mutable();
    if let Some(first) = (*corrupt).first_mut() {
        // Set all child hashes to empty so traversal fails
        first.child_hashes = Children::new();
    }
    let corrupt = corrupt.into_immutable();
    let err = corrupt
        .verify(key, Some(val.as_slice()), &root_hash, H::ALGORITHM)
        .unwrap_err();
    // Node traversal should fail
    assert!(matches!(
        err,
        crate::ProofError::NodeNotInTrie | crate::ProofError::UnexpectedHash { .. }
    ));
}

#[firewood_macros::hash_mode]
fn range_proof_serialization_roundtrip<H: HashMode>() {
    let merkle = init_merkle::<H, _, _, _>((u8::MIN..=u8::MAX).map(|k| ([k], [k])));

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

/// Truncated exclusion proof must be rejected. If the terminal proof node
/// is an ancestor of the proven key and has a child at the next nibble,
/// the proof is incomplete — it must include that child to demonstrate
/// the key's absence.
#[firewood_macros::hash_mode]
fn truncated_exclusion_proof_rejected<H: HashMode>() {
    // \x10 and \x20 exist. \x15 does not.
    let merkle = init_merkle::<H, _, _, _>([(b"\x10" as &[u8], b"a"), (b"\x20", b"b")]);
    let root_hash = merkle.nodestore().root_hash().unwrap();

    // Full exclusion proof for \x15 — proves it doesn't exist.
    let full_proof = merkle.prove(b"\x15").unwrap();
    assert!(
        full_proof
            .value_digest(b"\x15", &root_hash, H::ALGORITHM)
            .unwrap()
            .is_none(),
        "full proof should be a valid exclusion proof"
    );

    // Truncate: remove the terminal node.
    let mut nodes: Vec<crate::ProofNode> = full_proof.as_ref().to_vec();
    assert!(
        nodes.len() >= 2,
        "proof must have at least 2 nodes to truncate"
    );
    nodes.pop();
    let truncated = crate::Proof::new(nodes.into_boxed_slice());

    // The truncated proof must be rejected — the remaining terminal node
    // has a child toward \x15 but the proof doesn't include it.
    let result = truncated.value_digest(b"\x15", &root_hash, H::ALGORITHM);
    assert!(
        matches!(result, Err(crate::ProofError::ExclusionProofMissingChild)),
        "truncated exclusion proof should be rejected with ExclusionProofMissingChild, got {result:?}"
    );
}
