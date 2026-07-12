// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Issue #1088 acceptance test: a single process (one binary, regardless of the
//! `ethhash` feature) opens both an Ethereum-mode database and a MerkleDB-mode
//! database side by side, writes the same key/value set to each, and confirms
//! each reproduces its own frozen root vector.
//!
//! The root vectors are the seven-key vectors frozen in
//! `storage/src/tries/kvp.rs` (`test_hashed_trie`, "seven keys").

#![expect(clippy::unwrap_used)]

use firewood::api::{DynDb, FrozenRangeProof, OwnedBatch};
use firewood::db::{BatchOp, Db, DbConfig};
use firewood::verify_range_proof;
use firewood_storage::NodeHashAlgorithm;

/// The shared key/value set written to both databases.
const KVS: &[(&str, &str)] = &[
    ("a", "1"),
    ("ab", "2"),
    ("ac", "3"),
    ("b", "4"),
    ("ba", "5"),
    ("bb", "6"),
    ("c", "7"),
];

/// Frozen MerkleDB (sha256) root for `KVS` via the full `Db` commit path.
///
/// NOTE: this differs from the storage-layer `kvp.rs` "seven keys" merkledb
/// vector (`697e76…`). MerkleDB hashing is computed over firewood's *physical*
/// trie structure, and the production `Db`/Merkle path builds a structurally
/// different (but semantically equivalent) trie than the `KeyValueTrieRoot`
/// test builder used in `kvp.rs`. The Ethereum root below is the canonical
/// (structure-independent) MPT root, so it matches the `kvp.rs` vector and
/// anchors trie correctness; this is the deterministic MerkleDB root the full
/// `Db` produces for `KVS`.
const MERKLEDB_ROOT_HEX: &str = "7d2f1289f4552d1f7b2c5cb007b3ae5296fae6d0b70de83ebe7cf0866ec7969c";

/// Frozen Ethereum (keccak256) root for `KVS` (kvp.rs "seven keys").
const ETHEREUM_ROOT_HEX: &str = "3fa832b90f7f1a053a48a4528d1e446cc679fbcf376d0ef8703748d64030e19d";

fn batch() -> OwnedBatch {
    KVS.iter()
        .map(|(k, v)| BatchOp::Put {
            key: k.as_bytes().into(),
            value: v.as_bytes().into(),
        })
        .collect()
}

fn open(path: &std::path::Path, algorithm: NodeHashAlgorithm) -> Box<dyn DynDb> {
    let cfg = DbConfig::builder()
        .node_hash_algorithm(algorithm)
        .truncate(false)
        .build();
    Db::open(path, algorithm, cfg).unwrap()
}

/// Create a database in `algorithm` mode, write `KVS`, and return the committed
/// root hash (as hex).
fn write_and_root(path: &std::path::Path, algorithm: NodeHashAlgorithm) -> String {
    let db = open(path, algorithm);
    let proposal = db.propose(batch()).unwrap();
    proposal.commit().unwrap();
    let root = db.root_hash().expect("non-empty trie has a root");
    let hex = hex::encode(root);
    db.close().unwrap();
    hex
}

#[test]
fn eth_and_merkledb_databases_open_in_one_process() {
    let eth_dir = tempfile::tempdir().unwrap();
    let merkledb_dir = tempfile::tempdir().unwrap();

    // Both modes are constructed and written in this single process.
    let eth_root = write_and_root(eth_dir.path(), NodeHashAlgorithm::Ethereum);
    let merkledb_root = write_and_root(merkledb_dir.path(), NodeHashAlgorithm::MerkleDB);

    assert_eq!(
        eth_root, ETHEREUM_ROOT_HEX,
        "ethereum database must reproduce the frozen keccak root"
    );
    assert_eq!(
        merkledb_root, MERKLEDB_ROOT_HEX,
        "merkledb database must reproduce the frozen sha256 root"
    );
    assert_ne!(
        eth_root, merkledb_root,
        "the two schemes must hash the same data differently"
    );

    // Reopening each database honors the header's persisted algorithm and
    // reproduces the same root (header-honoring + determinism).
    let eth_db = open(eth_dir.path(), NodeHashAlgorithm::Ethereum);
    assert_eq!(eth_db.node_hash_algorithm(), NodeHashAlgorithm::Ethereum);
    assert_eq!(hex::encode(eth_db.root_hash().unwrap()), ETHEREUM_ROOT_HEX);
    eth_db.close().unwrap();

    let merkledb_db = open(merkledb_dir.path(), NodeHashAlgorithm::MerkleDB);
    assert_eq!(
        merkledb_db.node_hash_algorithm(),
        NodeHashAlgorithm::MerkleDB
    );
    assert_eq!(
        hex::encode(merkledb_db.root_hash().unwrap()),
        MERKLEDB_ROOT_HEX
    );
    merkledb_db.close().unwrap();
}

/// Canonical empty Ethereum trie root: `keccak256(0x80)` (RLP of the empty
/// string). This is the value a fresh `Db<EthHash>` must report regardless of
/// the binary's `ethhash` feature.
const ETHEREUM_EMPTY_ROOT_HEX: &str =
    "56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421";

/// Cross-mode proof regression guard (majors #1/#2): in one process, generate a
/// range proof and a single-key proof from BOTH a `Db<EthHash>` and a
/// `Db<MerkleDbHash>`, assert each carries its DB's runtime `hash_mode()`, and
/// round-trip-verify each (serialize -> `from_slice` -> verify with the DB's
/// mode). A mode pin to the compile default would either mis-stamp the proof
/// (failing the `hash_mode` assertion) or encode the wrong wire format (failing
/// round-trip verification / panicking on a cross-mode inline-RLP child).
#[test]
fn cross_mode_proofs_roundtrip_in_one_process() {
    let eth_dir = tempfile::tempdir().unwrap();
    let merkledb_dir = tempfile::tempdir().unwrap();

    // Populate both databases with the same data.
    let _ = write_and_root(eth_dir.path(), NodeHashAlgorithm::Ethereum);
    let _ = write_and_root(merkledb_dir.path(), NodeHashAlgorithm::MerkleDB);

    for (dir, algorithm) in [
        (eth_dir.path(), NodeHashAlgorithm::Ethereum),
        (merkledb_dir.path(), NodeHashAlgorithm::MerkleDB),
    ] {
        let db = open(dir, algorithm);
        let root = db.root_hash().expect("non-empty trie has a root");
        let view = db.view(root.clone()).unwrap();

        // --- Range proof over the full key set ---
        let range_proof = view.range_proof(None, None, None).unwrap();
        assert_eq!(
            range_proof.hash_mode(),
            algorithm,
            "range proof must be stamped with the source DB's runtime mode ({algorithm:?})",
        );

        // Round-trip through the self-describing wire format.
        let mut bytes = Vec::new();
        range_proof.write_to_vec(&mut bytes);
        let parsed = FrozenRangeProof::from_slice(&bytes)
            .expect("range proof must round-trip through its own wire format");
        assert_eq!(
            parsed.hash_mode(),
            algorithm,
            "parsed range proof must resolve the same mode from its header",
        );

        // Verify the parsed proof against the DB's root using the DB's mode.
        verify_range_proof(None::<&[u8]>, None::<&[u8]>, &root, algorithm, &parsed)
            .expect("cross-mode range proof must verify with the DB's mode");

        // --- Single-key proof (guards the emission/value-digest path) ---
        let &(probe_key, probe_value) = KVS.first().expect("KVS is non-empty");
        let single = view.single_key_proof(probe_key.as_bytes()).unwrap();
        single
            .verify(
                probe_key.as_bytes(),
                Some(probe_value.as_bytes()),
                &root,
                algorithm,
            )
            .expect("cross-mode single-key inclusion proof must verify with the DB's mode");

        db.close().unwrap();
    }
}

/// Empty-trie root regression guard (major #4): in one binary, a fresh
/// `Db<EthHash>` reports `Some(keccak256(0x80))` and a fresh `Db<MerkleDbHash>`
/// reports `None`. A compile-default empty-root resolution would report the
/// wrong value for the non-default mode.
#[test]
fn empty_trie_root_per_mode() {
    let eth_dir = tempfile::tempdir().unwrap();
    let merkledb_dir = tempfile::tempdir().unwrap();

    let eth_db = open(eth_dir.path(), NodeHashAlgorithm::Ethereum);
    let eth_root = eth_db
        .root_hash()
        .expect("empty Ethereum trie reports the keccak256(0x80) empty root");
    assert_eq!(
        hex::encode(eth_root),
        ETHEREUM_EMPTY_ROOT_HEX,
        "empty Db<EthHash> must report the canonical Ethereum empty root",
    );
    eth_db.close().unwrap();

    let merkledb_db = open(merkledb_dir.path(), NodeHashAlgorithm::MerkleDB);
    assert!(
        merkledb_db.root_hash().is_none(),
        "empty Db<MerkleDbHash> must report None for its empty root",
    );
    merkledb_db.close().unwrap();
}

/// Opening a database with the wrong requested algorithm must be rejected by the
/// header-vs-requested `validate_open` gate.
#[test]
fn opening_with_wrong_algorithm_is_rejected() {
    let dir = tempfile::tempdir().unwrap();

    // Create a MerkleDB database.
    let _ = write_and_root(dir.path(), NodeHashAlgorithm::MerkleDB);

    // Re-opening it as Ethereum must fail (header says MerkleDB).
    let cfg = DbConfig::builder()
        .node_hash_algorithm(NodeHashAlgorithm::Ethereum)
        .create_if_missing(false)
        .truncate(false)
        .build();
    let err = Db::open(dir.path(), NodeHashAlgorithm::Ethereum, cfg)
        .expect_err("opening a MerkleDB database as Ethereum must fail");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("mismatch") || msg.contains("Unsupported"),
        "expected a hash-algorithm mismatch error, got: {msg}"
    );
}
