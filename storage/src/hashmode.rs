// Copyright (C) 2023, Ava Labs, Inc. All rights reserved.
// See the file LICENSE.md for licensing terms.

//! Runtime-selectable node-hashing scheme.
//!
//! [`HashMode`] is the trait that will eventually carry every behavior that
//! differs between the MerkleDB (SHA-256) and Ethereum (Keccak-256) hashing
//! schemes, implemented by the two zero-sized types [`MerkleDbHash`] and
//! [`EthHash`]. Threading it as a type parameter is how issue #1088 turns the
//! scheme into a per-database runtime choice instead of a compile-time feature.
//!
//! This is the first, additive step: it introduces the trait, the two ZSTs, and
//! the [`DefaultHashMode`] bridge alias, and implements the mode-specific
//! *policy* methods (algorithm identity, empty-root hash, key validity) for the
//! mode selected by the current `ethhash` feature. The hashing methods — and
//! the second `impl`, so both modes are available in one binary — follow once
//! the data types are unified.

use crate::hashednode::{HasUpdate, Hashable};
use crate::node::ExtendableBytes;
use crate::{HashType, NodeHashAlgorithm, Path, TrieHash};
use std::io::{Error, Read};

/// The node-hashing scheme. Implemented by [`MerkleDbHash`] and [`EthHash`].
///
/// The trait is deliberately neither sealed nor `Copy`: leaving it unsealed
/// keeps the door open for a future scheme (e.g. a quantum-safe hash) from
/// outside the crate, and avoiding `Copy` prevents accidental implicit copies.
/// The practical set of schemes stays bounded by the [`NodeHashAlgorithm`]
/// discriminants persisted in file headers.
pub trait HashMode: Default + std::fmt::Debug + Send + Sync + 'static {
    /// The persisted algorithm identity for this scheme. Stamped into file
    /// headers and proof headers.
    const ALGORITHM: NodeHashAlgorithm;

    /// Root hash of an empty trie: `None` for MerkleDB, `keccak256(0x80)` (the
    /// hash of an empty Ethereum trie) for Ethereum.
    fn default_root_hash() -> Option<TrieHash>;

    /// Whether `key` is a structurally valid full trie key under this scheme.
    /// MerkleDB accepts any even nibble length; Ethereum accepts only account
    /// keys (64 nibbles) and storage-slot keys (128 nibbles).
    fn is_valid_key(key: &Path) -> bool;

    /// Hash a node preimage under this scheme.
    fn to_hash<T: Hashable>(node: &T) -> HashType;

    /// Write the hash preimage of `node` to `buf` under this scheme.
    fn write_preimage<T: Hashable>(node: &T, buf: &mut impl HasUpdate);

    /// Serialize a child's hash into a node's on-disk encoding under this
    /// scheme.
    ///
    /// # Errors
    ///
    /// Returns an error if `hash` cannot be represented in this scheme's
    /// on-disk format (e.g. an inline-RLP hash under MerkleDB, which indicates
    /// database corruption).
    fn write_child_hash<W: ExtendableBytes>(hash: &HashType, buf: &mut W) -> Result<(), Error>;

    /// Deserialize a child's hash from a node's on-disk encoding under this
    /// scheme.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes cannot be decoded into a [`HashType`].
    fn read_child_hash(reader: &mut impl Read) -> Result<HashType, Error>;
}

/// MerkleDB hashing: SHA-256 over a length-prefixed node encoding.
#[derive(Debug, Default)]
pub struct MerkleDbHash;

/// Ethereum hashing: Keccak-256 over the Ethereum Modified Merkle Patricia
/// Trie wire format.
#[derive(Debug, Default)]
pub struct EthHash;

/// The hash mode selected by the compile-time `ethhash` feature.
///
/// Used as the default type parameter while `H` is threaded through the stack
/// (`NodeStore<T, S, H = DefaultHashMode>` and friends), so both feature
/// configurations stay green until the feature is removed. Its only job is to
/// pick the default; it carries no behavior of its own.
#[cfg(feature = "ethhash")]
pub type DefaultHashMode = EthHash;

/// The hash mode selected by the compile-time `ethhash` feature.
///
/// See the `ethhash`-enabled definition for details.
#[cfg(not(feature = "ethhash"))]
pub type DefaultHashMode = MerkleDbHash;

// The `impl HashMode for EthHash` and `impl HashMode for MerkleDbHash` blocks
// live in `crate::hashers::{ethhash,merkledb}` alongside the scheme-specific
// preimage hashing and child-hash codecs they carry. Both are compiled in every
// build so a single binary can hash under either scheme.

#[cfg(test)]
mod tests {
    #![expect(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn path_of_len(len: usize) -> Path {
        Path((0..len).map(|_| 0u8).collect())
    }

    #[test]
    fn default_hash_mode_matches_compile_option() {
        assert_eq!(
            DefaultHashMode::ALGORITHM,
            NodeHashAlgorithm::compile_option()
        );
    }

    #[test]
    fn default_root_hash_per_mode() {
        if DefaultHashMode::ALGORITHM.is_ethereum() {
            assert!(DefaultHashMode::default_root_hash().is_some());
        } else {
            assert!(DefaultHashMode::default_root_hash().is_none());
        }
    }

    #[test]
    fn is_valid_key_per_mode() {
        if DefaultHashMode::ALGORITHM.is_ethereum() {
            assert!(DefaultHashMode::is_valid_key(&path_of_len(64)));
            assert!(DefaultHashMode::is_valid_key(&path_of_len(128)));
            assert!(!DefaultHashMode::is_valid_key(&path_of_len(66)));
            assert!(!DefaultHashMode::is_valid_key(&path_of_len(63)));
        } else {
            assert!(DefaultHashMode::is_valid_key(&path_of_len(64)));
            assert!(DefaultHashMode::is_valid_key(&path_of_len(2)));
            assert!(!DefaultHashMode::is_valid_key(&path_of_len(3)));
        }
    }

    // The codec and cross-mode tests below call the `EthHash` / `MerkleDbHash`
    // methods directly, so they exercise *both* schemes in a single binary
    // regardless of the active feature flag.

    #[test]
    fn eth_child_hash_codec_round_trips_full_hash() {
        let hash = HashType::from([0x11u8; 32]);
        let mut buf = Vec::new();
        EthHash::write_child_hash(&hash, &mut buf).unwrap();
        // Frozen eth layout: leading 0x00 discriminant + 32 raw hash bytes.
        assert_eq!(buf.len(), 33);
        assert_eq!(buf[0], 0x00);
        assert_eq!(&buf[1..], &[0x11u8; 32]);

        let decoded = EthHash::read_child_hash(&mut std::io::Cursor::new(&buf)).unwrap();
        assert_eq!(decoded, hash);
        assert!(matches!(decoded, HashType::Hash(_)));
    }

    #[test]
    fn eth_child_hash_codec_round_trips_inline_rlp() {
        let rlp = HashType::Rlp(smallvec::SmallVec::from_slice(&[0xAA, 0xBB, 0xCC]));
        let mut buf = Vec::new();
        EthHash::write_child_hash(&rlp, &mut buf).unwrap();
        // Frozen eth layout: leading length byte (1..=31) + the RLP bytes.
        assert_eq!(buf, vec![0x03, 0xAA, 0xBB, 0xCC]);

        let decoded = EthHash::read_child_hash(&mut std::io::Cursor::new(&buf)).unwrap();
        assert_eq!(decoded, rlp);
        assert!(matches!(decoded, HashType::Rlp(_)));
    }

    #[test]
    fn merkledb_child_hash_codec_round_trips_bare_32_bytes() {
        let hash = HashType::from([0x22u8; 32]);
        let mut buf = Vec::new();
        MerkleDbHash::write_child_hash(&hash, &mut buf).unwrap();
        // Frozen merkledb layout: a bare 32 bytes, no prefix.
        assert_eq!(buf, vec![0x22u8; 32]);

        let decoded = MerkleDbHash::read_child_hash(&mut std::io::Cursor::new(&buf)).unwrap();
        assert_eq!(decoded, hash);
        assert!(matches!(decoded, HashType::Hash(_)));
    }

    #[test]
    fn merkledb_child_hash_write_rejects_inline_rlp() {
        let rlp = HashType::Rlp(smallvec::SmallVec::from_slice(&[0xAA, 0xBB]));
        let mut buf = Vec::new();
        let err = MerkleDbHash::write_child_hash(&rlp, &mut buf)
            .expect_err("merkledb must reject an RLP child hash as corruption");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn cross_mode_leaf_preimage_differs_per_scheme() {
        use crate::{Children, HashableShunt, Path, ValueDigest};

        // A single leaf with a short partial path and value. No children, so
        // the hash does not depend on any mode-specific child-hash encoding;
        // the difference is purely the SHA-256 vs Keccak-256 preimage.
        let partial = Path::from(vec![0x01, 0x02, 0x03, 0x04]);
        let value = [0xDEu8, 0xAD, 0xBE, 0xEF];
        let leaf = HashableShunt::new(
            <&[crate::PathComponent]>::default(),
            partial.as_components(),
            Some(ValueDigest::Value(value.as_slice())),
            Children::new(),
        );

        let eth = EthHash::to_hash(&leaf);
        let merkledb = MerkleDbHash::to_hash(&leaf);

        // MerkleDB always produces a full 32-byte hash.
        assert!(matches!(merkledb, HashType::Hash(_)));
        // This leaf's eth preimage is well under 32 bytes, so eth keeps it
        // inline as `Rlp` rather than hashing it — the mode-appropriate shape.
        assert!(matches!(eth, HashType::Rlp(_)));
        // The two schemes must disagree (otherwise the "one binary, two
        // schemes" guarantee would be vacuous). Full-value correctness of each
        // scheme is anchored by the both-mode root vectors in `tries::kvp`.
        assert_ne!(eth, merkledb);
    }
}
