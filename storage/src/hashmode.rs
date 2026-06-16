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

use crate::{NodeHashAlgorithm, Path, TrieHash};

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
// live in `crate::hashers::{ethhash,merkledb}`, next to the scheme-specific
// hashing they will carry. Exactly one of those modules is compiled per build
// (selected by the `ethhash` feature), so only the active mode's impl exists
// until the data types are unified and both modules compile together.

#[cfg(test)]
mod tests {
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
}
