// Copyright (C) 2026, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Runtime hash-mode coverage for the object-safe database boundary.

use firewood::api::{DbView, OwnedBatch, Reconstructible};
use firewood::db::{BatchOp, DbConfig};
use firewood::open;
use firewood_storage::NodeHashAlgorithm;

const KVS: &[(&str, &str)] = &[
    ("a", "1"),
    ("ab", "2"),
    ("ac", "3"),
    ("b", "4"),
    ("ba", "5"),
    ("bb", "6"),
    ("c", "7"),
];

#[derive(Clone, Copy)]
struct ModeCase {
    algorithm: NodeHashAlgorithm,
    empty_root: Option<&'static str>,
    committed_root: &'static str,
}

const CASES: [ModeCase; 2] = [
    ModeCase {
        algorithm: NodeHashAlgorithm::Ethereum,
        empty_root: Some("56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"),
        committed_root: "3fa832b90f7f1a053a48a4528d1e446cc679fbcf376d0ef8703748d64030e19d",
    },
    ModeCase {
        algorithm: NodeHashAlgorithm::MerkleDB,
        empty_root: None,
        committed_root: "7d2f1289f4552d1f7b2c5cb007b3ae5296fae6d0b70de83ebe7cf0866ec7969c",
    },
];

fn puts(pairs: &[(&str, &str)]) -> OwnedBatch {
    pairs
        .iter()
        .map(|(key, value)| BatchOp::Put {
            key: key.as_bytes().into(),
            value: value.as_bytes().into(),
        })
        .collect()
}

#[test]
fn runtime_hash_modes_work_side_by_side() {
    let dirs = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let databases = CASES
        .iter()
        .zip(&dirs)
        .map(|(case, dir)| {
            open(
                dir.path(),
                DbConfig::builder()
                    .node_hash_algorithm(case.algorithm)
                    .build(),
            )
            .unwrap()
        })
        .collect::<Vec<_>>();

    // Both implementations remain live behind the same object-safe interface.
    for (case, db) in CASES.iter().zip(&databases) {
        assert_eq!(db.node_hash_algorithm(), case.algorithm);
        assert_eq!(db.root_hash().map(hex::encode).as_deref(), case.empty_root);

        db.propose(puts(KVS)).unwrap().commit().unwrap();
        let root = db.root_hash().expect("committed trie has a root");
        assert_eq!(hex::encode(root.clone()), case.committed_root);

        let proof = db
            .view(root)
            .unwrap()
            .range_proof(None, None, None)
            .unwrap();
        assert_eq!(proof.hash_mode(), case.algorithm);
    }

    for db in databases {
        db.close().unwrap();
    }

    // The persisted header restores the selected algorithm and root.
    for (case, dir) in CASES.iter().zip(&dirs) {
        let db = open(
            dir.path(),
            DbConfig::builder()
                .node_hash_algorithm(case.algorithm)
                .build(),
        )
        .unwrap();
        assert_eq!(db.node_hash_algorithm(), case.algorithm);
        assert_eq!(
            db.root_hash().map(hex::encode).as_deref(),
            Some(case.committed_root)
        );
        db.close().unwrap();
    }
}

#[test]
fn reconstructed_views_preserve_runtime_hash_mode() {
    for case in CASES {
        let dir = tempfile::tempdir().unwrap();
        let db = open(
            dir.path(),
            DbConfig::builder()
                .node_hash_algorithm(case.algorithm)
                .build(),
        )
        .unwrap();

        db.propose(puts(KVS)).unwrap().commit().unwrap();
        let base_root = db.root_hash().expect("committed trie has a root");
        db.propose(puts(&[("d", "8")])).unwrap().commit().unwrap();

        let historical = db
            .committed_view(base_root)
            .unwrap()
            .expect("base root is a committed revision");
        let reconstructed = db
            .reconstruct_from_view(&historical, puts(&[("e", "9")]))
            .unwrap();
        assert_eq!(reconstructed.node_hash_algorithm(), case.algorithm);
        assert_eq!(
            reconstructed.val(b"a").unwrap().as_deref(),
            Some(b"1".as_slice())
        );
        assert_eq!(
            reconstructed.val(b"e").unwrap().as_deref(),
            Some(b"9".as_slice())
        );
        assert_eq!(reconstructed.val(b"d").unwrap(), None);

        let reconstructed = reconstructed.reconstruct(puts(&[("f", "10")])).unwrap();
        assert_eq!(reconstructed.node_hash_algorithm(), case.algorithm);
        assert_eq!(
            reconstructed.val(b"e").unwrap().as_deref(),
            Some(b"9".as_slice())
        );
        assert_eq!(
            reconstructed.val(b"f").unwrap().as_deref(),
            Some(b"10".as_slice())
        );

        db.close().unwrap();
    }
}
